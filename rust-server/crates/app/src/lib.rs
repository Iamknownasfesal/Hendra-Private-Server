//! The app server: registration, login, and the character list.
//!
//! Everything a player does before they are in a world happens here, over HTTPS. It is the only
//! thing that ever sees a password, and the only thing that mints a session token. The game server
//! sees neither; it takes a token and checks the signature.
//!
//! ```text
//!   POST   /register       {name, password}       ->  {token, account_id}
//!   POST   /login          {name, password}       ->  {token, account_id, characters}
//!   GET    /characters     (Bearer)               ->  [{character}]
//!   POST   /select         (Bearer) {character_id}->  {token, ...}
//!   DELETE /characters/:id (Bearer)               ->  {}
//!   POST   /password       (Bearer) {old, new}    ->  {}
//!   GET    /servers                               ->  [{name, host, port}]
//!   GET    /init                                  ->  {protocol, servers, classes}
//!   POST   /name           (Bearer) {name}        ->  {}
//!   GET    /friends        (Bearer)               ->  {friends, requests}
//!   POST   /friends        (Bearer) {name}        ->  {mutual}
//!   DELETE /friends/:id    (Bearer)               ->  {}
//!   GET    /messages       (Bearer)               ->  [{message}]
//!   GET    /fame           (Bearer)               ->  [{name, fame}]
//!   POST   /email          (Bearer) {email}       ->  {}
//!   POST   /email/verify   {token}                ->  {}
//!   POST   /password/forgot {email}               ->  {}
//!   POST   /password/reset {token, password}      ->  {}
//!   GET    /classes        (Bearer)               ->  [{class, locked}]
//!   POST   /characters     (Bearer) {class, name} ->  {character}
//! ```
//!
//! # On running this behind something
//!
//! There is no TLS here. Terminating it belongs to whatever sits in front, a reverse proxy that
//! also handles certificate renewal, and putting a second, worse implementation of it in this
//! process would be a way to get it wrong twice. The server refuses to bind a public address
//! without being told that something is in front of it.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hendra_auth::{Claims, TokenKey, hash_password, mint, now, verify, verify_password};
use hendra_store::{Store, StoreError};
use serde::{Deserialize, Serialize};

pub mod mail;
pub mod throttle;
pub use throttle::Throttle;

pub struct App {
    pub store: Store,
    pub key: TokenKey,

    /// Where the game servers are, for a client that has just logged in and has nowhere to go.
    pub servers: Vec<GameServer>,

    /// Where verification and reset links are sent.
    pub mail: mail::Sender,

    /// The content, for the character-select screen. Read-only and shared with nothing.
    pub catalog: Arc<hendra_content::Catalog>,

    /// What every class carries beyond the gear its own slots decide.
    pub common_items: hendra_characters::CommonItems,

    /// How many passwords have recently failed against each name.
    pub throttle: Throttle,

    /// How many passwords may be hashed at once.
    pub hashing: tokio::sync::Semaphore,
}

impl App {
    pub fn with_content(
        store: Store,
        key: TokenKey,
        catalog: Arc<hendra_content::Catalog>,
        common_items: hendra_characters::CommonItems,
    ) -> App {
        App {
            store,
            key,
            servers: Vec::new(),
            mail: Arc::new(mail::Logged),
            catalog,
            common_items,
            throttle: Throttle::new(),
            hashing: tokio::sync::Semaphore::new(throttle::CONCURRENT_HASHES),
        }
    }
}

#[derive(Deserialize)]
pub struct Credentials {
    pub name: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct Character {
    pub id: i64,
    pub name: String,

    /// The runtime number, for a client that draws by type. Resolved from the identity at read
    /// time rather than stored, because a runtime number is not durable.
    pub object_type: i32,
    pub level: i16,
    pub fame: i32,
}

#[derive(Serialize)]
pub struct LoggedIn {
    pub token: String,
    pub account_id: i64,
    pub expires_at: u64,
    pub characters: Vec<Character>,
}

#[derive(Serialize)]
pub struct Refusal {
    pub error: String,
}

pub type Answer<T> = Result<Json<T>, (StatusCode, Json<Refusal>)>;

fn refuse(status: StatusCode, message: &str) -> (StatusCode, Json<Refusal>) {
    (
        status,
        Json(Refusal {
            error: message.to_string(),
        }),
    )
}

/// The same refusal for a name that does not exist and a password that is wrong.
///
/// Telling them apart turns the login endpoint into a way to find out which accounts exist, which
/// is the first thing anyone attacking it wants to know.
fn bad_credentials() -> (StatusCode, Json<Refusal>) {
    refuse(
        StatusCode::UNAUTHORIZED,
        "that name and password do not match",
    )
}

/// Refuses a name that has failed too often, and says when it may try again.
///
/// Answered before the password is looked at, so a locked-out name costs no hashing. The limit is
/// meant to stop guessing being cheap for the server as well as bounded for the attacker.
fn too_many_attempts(after: std::time::Duration) -> (StatusCode, Json<Refusal>) {
    let seconds = after.as_secs().max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(Refusal {
            error: format!("too many failed attempts; try again in {seconds} seconds"),
        }),
    )
}

pub async fn register(
    State(app): State<Arc<App>>,
    Json(body): Json<Credentials>,
) -> Answer<LoggedIn> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "a name must be between one and thirty-two characters",
        ));
    }

    let hash = {
        let _permit = app.hashing.acquire().await;
        hash_password(&body.password)
            .map_err(|err| refuse(StatusCode::BAD_REQUEST, &err.to_string()))?
    };

    let account = match app.store.create_account(name).await {
        Ok(account) => account,
        Err(StoreError::NameTaken) => {
            return Err(refuse(StatusCode::CONFLICT, "that name is taken"));
        }
        Err(err) => {
            tracing::error!(%err, "could not create an account");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ));
        }
    };

    if let Err(err) = app.store.set_password(account.id, &hash).await {
        tracing::error!(%err, "could not store a password");
        return Err(refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "try again shortly",
        ));
    }

    Ok(Json(issue(&app, account.id, Vec::new())))
}

pub async fn login(State(app): State<Arc<App>>, Json(body): Json<Credentials>) -> Answer<LoggedIn> {
    let name = body.name.trim();
    let now = Instant::now();

    if let Some(after) = app.throttle.locked_out(name, now) {
        return Err(too_many_attempts(after));
    }

    // Shared as well as local. The in-process limiter is the fast path and catches a burst against
    // one server; this one is what makes the limit mean the same thing when there are four.
    let window = throttle::WINDOW.as_secs() as i64;
    if app
        .store
        .recent_failed_logins(name, window)
        .await
        .is_ok_and(|failures| failures >= throttle::FAILURES_ALLOWED as i64)
    {
        return Err(too_many_attempts(throttle::WINDOW));
    }

    let account = match app.store.account_by_name(name).await {
        Ok(account) => account,
        Err(StoreError::NoSuchAccount(_)) => {
            // Counted even though there is nothing to guess here, because not counting it would
            // make the limiter answer the question the refusal above refuses to: a name that never
            // locks out is a name that does not exist.
            app.throttle.failed(name, now);
            let _ = app.store.record_failed_login(name).await;
            return Err(bad_credentials());
        }
        Err(err) => {
            tracing::error!(%err, "could not read an account");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ));
        }
    };

    // An account made before authentication existed has no password. Refusing by name is right:
    // comparing against nothing would let anyone in.
    let Some(stored) = account.password_hash.as_deref() else {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "this account has no password set; contact an administrator",
        ));
    };

    let checked = {
        let _permit = app.hashing.acquire().await;
        verify_password(&body.password, stored)
    };

    match checked {
        Ok(true) => {
            app.throttle.succeeded(name);
            let _ = app.store.clear_failed_logins(name).await;
        }
        Ok(false) => {
            app.throttle.failed(name, now);
            let _ = app.store.record_failed_login(name).await;
            return Err(bad_credentials());
        }
        Err(err) => {
            tracing::error!(%err, account = account.id, "a stored password hash is unreadable");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ));
        }
    }

    // Checked after the password, so a banned account is not a way to learn a password is correct.
    if account.banned {
        return Err(refuse(StatusCode::FORBIDDEN, "this account is banned"));
    }

    let characters = app
        .store
        .characters(account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|summary| Character {
            id: summary.id,
            name: summary.name,
            object_type: class_number(&app.catalog, summary.class),
            level: summary.level,
            fame: summary.fame,
        })
        .collect();

    Ok(Json(issue(&app, account.id, characters)))
}

pub async fn characters(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<Character>> {
    let claims = authenticate(&app, &headers)?;

    let listed = app
        .store
        .characters(claims.account_id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not list characters");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(
        listed
            .into_iter()
            .map(|summary| Character {
                id: summary.id,
                name: summary.name,
                object_type: class_number(&app.catalog, summary.class),
                level: summary.level,
                fame: summary.fame,
            })
            .collect(),
    ))
}

/// Reads and checks a bearer token.
fn authenticate(app: &App, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Refusal>)> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "no token"))?;

    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "expected a bearer token"))?;

    verify(&app.key, token.trim(), now())
        .map_err(|err| refuse(StatusCode::UNAUTHORIZED, &err.to_string()))
}

fn issue(app: &App, account_id: i64, characters: Vec<Character>) -> LoggedIn {
    let expires_at = now() + hendra_auth::LIFETIME_SECONDS;
    let token = mint(
        &app.key,
        Claims {
            account_id,
            character_id: 0,
            expires_at,
        },
    );

    LoggedIn {
        token: token.0,
        account_id,
        expires_at,
        characters,
    }
}

#[derive(Deserialize)]
pub struct Selection {
    pub character_id: i64,
}

/// Mints a token naming one character.
///
/// The game server takes a character from the token in preference to the one the client asks for
/// separately, so this is what makes character choice something the player cannot lie about. The
/// ownership check is here rather than only in the game server because a token that names a
/// character the account does not own should never exist in the first place.
pub async fn select(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Selection>,
) -> Answer<LoggedIn> {
    let claims = authenticate(&app, &headers)?;

    let owns = app
        .store
        .owns_character(claims.account_id, body.character_id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not check character ownership");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    if !owns {
        // The same answer whether it belongs to someone else or does not exist, so this is not a
        // way to enumerate which characters other people have.
        return Err(refuse(StatusCode::NOT_FOUND, "no such character"));
    }

    let expires_at = now() + hendra_auth::LIFETIME_SECONDS;
    let token = mint(
        &app.key,
        Claims {
            account_id: claims.account_id,
            character_id: body.character_id,
            expires_at,
        },
    );

    Ok(Json(LoggedIn {
        token: token.0,
        account_id: claims.account_id,
        expires_at,
        characters: Vec::new(),
    }))
}

/// Deletes a character and everything it was carrying.
pub async fn delete_character(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    let deleted = app
        .store
        .delete_character(claims.account_id, id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not delete a character");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    if !deleted {
        return Err(refuse(StatusCode::NOT_FOUND, "no such character"));
    }

    Ok(Json(serde_json::json!({})))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    pub old_password: String,
    pub new_password: String,
}

/// Changes a password, given the current one.
///
/// A valid token is not enough on its own. Requiring the old password is what stops a token taken
/// from a log or a shared machine being turned into permanent ownership of the account. The token
/// expires in fifteen minutes, and a changed password does not.
pub async fn change_password(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<PasswordChange>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    let account = app.store.account(claims.account_id).await.map_err(|err| {
        tracing::error!(%err, "could not read an account");
        refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
    })?;

    let Some(stored) = account.password_hash.as_deref() else {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "this account has no password set; contact an administrator",
        ));
    };

    let now_at = Instant::now();
    if let Some(after) = app.throttle.locked_out(&account.name, now_at) {
        return Err(too_many_attempts(after));
    }

    let (checked, hashed) = {
        let _permit = app.hashing.acquire().await;
        let checked = verify_password(&body.old_password, stored);
        // Hashed under the same permit, so one request cannot hold two of them and the limit means
        // what it says.
        let hashed = match &checked {
            Ok(true) => Some(hash_password(&body.new_password)),
            _ => None,
        };
        (checked, hashed)
    };

    match checked {
        Ok(true) => app.throttle.succeeded(&account.name),
        Ok(false) => {
            app.throttle.failed(&account.name, now_at);
            return Err(bad_credentials());
        }
        Err(err) => {
            tracing::error!(%err, account = account.id, "a stored password hash is unreadable");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ));
        }
    }

    let hash = hashed
        .expect("a correct old password produces a new hash")
        .map_err(|err| refuse(StatusCode::BAD_REQUEST, &err.to_string()))?;

    app.store
        .set_password(account.id, &hash)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not store a password");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(serde_json::json!({})))
}

/// One game server a client may connect to.
#[derive(Debug, Clone, Serialize)]
pub struct GameServer {
    pub name: String,
    pub host: String,
    pub port: u16,

    /// Where it is, for a client choosing the nearest.
    pub region: String,
}

/// Where the game is.
///
/// Unauthenticated, because a client needs it before it has anywhere to send a password, and it
/// says nothing a port scan would not. Returning it from the app server rather than compiling it
/// into the client is what makes moving a server a configuration change.
pub async fn servers(State(app): State<Arc<App>>) -> Json<Vec<GameServer>> {
    Json(app.servers.clone())
}

#[derive(Serialize)]
pub struct ClassOffer {
    pub object_type: u16,
    pub id: String,
    pub starting_hp: i32,
    pub starting_mp: i32,

    /// `null` when the class can be played now.
    pub locked: Option<String>,

    pub cost: Option<u32>,
}

/// Every class, and whether this account may play it.
pub async fn classes(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<ClassOffer>> {
    let claims = authenticate(&app, &headers)?;

    let offers = hendra_characters::offers(&app.store, &app.catalog, claims.account_id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not read class progress");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(
        offers
            .into_iter()
            .map(|offer| ClassOffer {
                object_type: offer.object_type.0,
                id: offer.id,
                starting_hp: offer.starting_hp,
                starting_mp: offer.starting_mp,
                // Said in words rather than as a code, because the only thing a client does with
                // it is show it to someone.
                locked: offer.locked.map(|locked| match locked {
                    hendra_content::player::Locked::NeedsLevel { class, level } => {
                        let name = app
                            .catalog
                            .object(class)
                            .map(|desc| desc.id.clone())
                            .unwrap_or_else(|| format!("class {}", class.0));
                        format!("reach level {level} with a {name}")
                    }
                }),
                cost: offer.cost,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct NewCharacter {
    pub class: u16,
    pub name: Option<String>,
}

/// Makes a character of a class, if the account has unlocked it.
pub async fn create_character(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewCharacter>,
) -> Answer<Character> {
    let claims = authenticate(&app, &headers)?;

    let name = body.name.as_deref().map(str::trim).unwrap_or("Adventurer");
    if name.is_empty() || name.chars().count() > 32 {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "a name must be between one and thirty-two characters",
        ));
    }

    let made = hendra_characters::create(
        &app.store,
        &app.catalog,
        &app.common_items,
        claims.account_id,
        hendra_content::ObjectType(body.class),
        name,
    )
    .await;

    match made {
        Ok(character) => Ok(Json(Character {
            id: character.id,
            name: character.name,
            object_type: class_number(&app.catalog, character.class),
            level: character.level,
            fame: character.fame,
        })),
        Err(hendra_characters::CreateError::NoSuchClass) => {
            Err(refuse(StatusCode::BAD_REQUEST, "no such class"))
        }
        Err(hendra_characters::CreateError::Locked(locked)) => {
            let hendra_content::player::Locked::NeedsLevel { class, level } = locked;
            let name = app
                .catalog
                .object(class)
                .map(|desc| desc.id.clone())
                .unwrap_or_else(|| format!("class {}", class.0));
            Err(refuse(
                StatusCode::FORBIDDEN,
                &format!("that class is locked; reach level {level} with a {name}"),
            ))
        }
        Err(err) => {
            tracing::error!(%err, "could not create a character");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

/// The runtime number a class identity currently has, or zero if the catalog does not hold it.
///
/// Resolved on the way out rather than stored, because the number is assigned at load and only the
/// identity is durable.
fn class_number(catalog: &hendra_content::Catalog, class: uuid::Uuid) -> i32 {
    catalog
        .type_of_uuid(class)
        .map(|found| found.0 as i32)
        .unwrap_or(0)
}

#[derive(Serialize)]
pub struct Init {
    /// What the game socket expects. A client that does not match is told before it tries.
    pub protocol: u32,
    pub servers: Vec<GameServer>,
    pub classes: usize,
}

/// Everything a client needs before it has an account.
///
/// One request rather than three, because a client at the title screen has nothing else to do and
/// three round trips is three chances to be halfway configured.
pub async fn init(State(app): State<Arc<App>>) -> Json<Init> {
    Json(Init {
        protocol: hendra_net::message::PROTOCOL_VERSION,
        servers: app.servers.clone(),
        classes: app.catalog.classes().len(),
    })
}

#[derive(Deserialize)]
pub struct NewName {
    pub name: String,
}

/// Renames an account.
pub async fn set_name(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewName>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "a name must be between one and thirty-two characters",
        ));
    }

    match app.store.rename_account(claims.account_id, name).await {
        Ok(()) => Ok(Json(serde_json::json!({}))),
        Err(StoreError::NameTaken) => Err(refuse(StatusCode::CONFLICT, "that name is taken")),
        Err(err) => {
            tracing::error!(%err, "could not rename an account");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

#[derive(Serialize)]
pub struct FriendList {
    pub friends: Vec<Person>,
    pub requests: Vec<Person>,
}

#[derive(Serialize)]
pub struct Person {
    pub account_id: i64,
    pub name: String,
    pub accepted: bool,
}

/// Who an account knows, and who is waiting for an answer.
pub async fn friends(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<FriendList> {
    let claims = authenticate(&app, &headers)?;

    let (friends, requests) = tokio::join!(
        app.store.friends(claims.account_id),
        app.store.friend_requests(claims.account_id)
    );

    let listed = |people: Vec<hendra_store::Friend>| {
        people
            .into_iter()
            .map(|person| Person {
                account_id: person.account_id,
                name: person.name,
                accepted: person.accepted,
            })
            .collect()
    };

    Ok(Json(FriendList {
        friends: listed(friends.unwrap_or_default()),
        requests: listed(requests.unwrap_or_default()),
    }))
}

/// Asks somebody to be a friend, or accepts their asking.
pub async fn add_friend(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewName>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    let Ok(other) = app.store.account_by_name(body.name.trim()).await else {
        // The same answer as a name that exists but has blocked you would be, so this is not a way
        // to find out which accounts exist.
        return Err(refuse(StatusCode::NOT_FOUND, "no such player"));
    };

    match app.store.befriend(claims.account_id, other.id).await {
        Ok(mutual) => Ok(Json(serde_json::json!({ "mutual": mutual }))),
        Err(StoreError::Refused(why)) => Err(refuse(StatusCode::BAD_REQUEST, why)),
        Err(err) => {
            tracing::error!(%err, "could not add a friend");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

/// Removes a friend, from both sides.
pub async fn remove_friend(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    app.store
        .unfriend(claims.account_id, id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not remove a friend");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(serde_json::json!({})))
}

#[derive(Serialize)]
pub struct Note {
    pub id: i64,
    pub from: String,
    pub body: String,
    pub read: bool,
}

/// Messages waiting for an account.
pub async fn messages(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<Note>> {
    let claims = authenticate(&app, &headers)?;

    let held = app
        .store
        .messages(claims.account_id, 50)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not read messages");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(
        held.into_iter()
            .map(|message| Note {
                id: message.id,
                from: message.from,
                body: message.body,
                read: message.read,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct FameEntry {
    pub name: String,
    pub fame: i32,
    pub level: i16,
}

/// An account's characters by fame, which is what a fame list is for one player.
pub async fn fame(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<FameEntry>> {
    let claims = authenticate(&app, &headers)?;

    let mut listed: Vec<FameEntry> = app
        .store
        .characters(claims.account_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|summary| FameEntry {
            name: summary.name,
            fame: summary.fame,
            level: summary.level,
        })
        .collect();

    listed.sort_by_key(|entry| std::cmp::Reverse(entry.fame));
    Ok(Json(listed))
}

#[derive(Deserialize)]
pub struct EmailBody {
    pub email: String,
}

#[derive(Deserialize)]
pub struct TokenBody {
    pub token: String,
}

#[derive(Deserialize)]
pub struct ResetBody {
    pub token: String,
    pub password: String,
}

/// Records an address and sends something to it to prove it arrives.
pub async fn set_email(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    app.store
        .set_email(claims.account_id, &body.email)
        .await
        .map_err(|err| match err {
            StoreError::Refused(why) => refuse(StatusCode::BAD_REQUEST, why),
            other => {
                tracing::error!(%other, "could not set an address");
                refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
            }
        })?;

    send_link(
        &app,
        claims.account_id,
        &body.email,
        hendra_store::email::Purpose::Verify,
    )
    .await;
    Ok(Json(serde_json::json!({})))
}

/// Confirms an address from the token that was sent to it.
pub async fn verify_email(
    State(app): State<Arc<App>>,
    Json(body): Json<TokenBody>,
) -> Answer<serde_json::Value> {
    let account_id = app
        .store
        .spend_email_token(&body.token, hendra_store::email::Purpose::Verify)
        .await
        .map_err(|_| refuse(StatusCode::BAD_REQUEST, "that link is no longer valid"))?;

    app.store
        .mark_email_verified(account_id)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not mark an address verified");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(serde_json::json!({})))
}

/// Starts a password reset.
///
/// Answers the same way whether or not the address is known. Telling them apart turns this into a
/// way to find out which addresses have accounts, which is worse than the small confusion of a
/// player who mistyped their own.
pub async fn forgot_password(
    State(app): State<Arc<App>>,
    Json(body): Json<EmailBody>,
) -> Answer<serde_json::Value> {
    if let Ok(account_id) = app.store.account_by_email(&body.email).await {
        send_link(
            &app,
            account_id,
            &body.email,
            hendra_store::email::Purpose::Reset,
        )
        .await;
    }

    Ok(Json(serde_json::json!({})))
}

/// Finishes a password reset.
pub async fn reset_password(
    State(app): State<Arc<App>>,
    Json(body): Json<ResetBody>,
) -> Answer<serde_json::Value> {
    // Hashed before the token is spent, so a password the server would refuse does not burn the
    // one link the player has.
    let hash = {
        let _permit = app.hashing.acquire().await;
        hash_password(&body.password)
            .map_err(|err| refuse(StatusCode::BAD_REQUEST, &err.to_string()))?
    };

    let account_id = app
        .store
        .spend_email_token(&body.token, hendra_store::email::Purpose::Reset)
        .await
        .map_err(|_| refuse(StatusCode::BAD_REQUEST, "that link is no longer valid"))?;

    app.store
        .set_password(account_id, &hash)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not reset a password");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    // Every failure recorded against the account is forgotten, or somebody who reset their password
    // because they were locked out would still be locked out.
    if let Ok(account) = app.store.account(account_id).await {
        let _ = app.store.clear_failed_logins(&account.name).await;
        app.throttle.succeeded(&account.name);
    }

    Ok(Json(serde_json::json!({})))
}

/// Makes a token and sends it.
async fn send_link(app: &App, account_id: i64, email: &str, purpose: hendra_store::email::Purpose) {
    let token = match app.store.issue_email_token(account_id, purpose).await {
        Ok(token) => token,
        Err(err) => {
            // Reported rather than returned quietly. A link that is never made looks exactly like
            // a link that was never delivered, and one of those is a bug in here.
            tracing::error!(%err, "could not make a link");
            return;
        }
    };

    let (subject, body) = match purpose {
        hendra_store::email::Purpose::Verify => (
            "Confirm your address",
            format!("Your confirmation code is {}", token.0),
        ),
        hendra_store::email::Purpose::Reset => (
            "Reset your password",
            format!("Your reset code is {}", token.0),
        ),
    };

    if let Err(why) = app.mail.send(email, subject, &body) {
        tracing::error!(%why, "could not send a link");
    }
}

async fn health() -> &'static str {
    "ok"
}

/// Every route, given the state they share.
///
/// Separate from startup so the endpoints can be exercised without a socket: a test drives this
/// router directly, which is the difference between testing the server and testing a port.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/servers", get(servers))
        .route("/init", get(init))
        .route("/name", post(set_name))
        .route("/fame", get(fame))
        .route("/email", post(set_email))
        .route("/email/verify", post(verify_email))
        .route("/password/forgot", post(forgot_password))
        .route("/password/reset", post(reset_password))
        .route("/messages", get(messages))
        .route("/friends", get(friends).post(add_friend))
        .route("/friends/{id}", axum::routing::delete(remove_friend))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/characters", get(characters))
        .route("/characters/{id}", axum::routing::delete(delete_character))
        .route("/characters", post(create_character))
        .route("/classes", get(classes))
        .route("/select", post(select))
        .route("/password", post(change_password))
        // A body limit, because both credential endpoints hash what they are given and Argon2 is
        // meant to be slow.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app)
}
