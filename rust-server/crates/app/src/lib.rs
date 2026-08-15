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
//!   GET    /content                               ->  {objects, tiles, classes}
//!   GET    /strings/:lang                         ->  {key: value}
//!   GET    /offers                                ->  [{name, credits, price}]
//!   GET    /quests        (Bearer)                ->  [{title, progress, goal}]
//!   GET    /quests/weekly (Bearer)                ->  [{title, progress, goal}]
//!   POST   /age           (Bearer)                ->  {verified}
//!   GET    /skins         (Bearer)                ->  {owned, credits}
//!   POST   /skins         (Bearer) {skin, price}  ->  {}
//!   GET    /picture/:id                           ->  the bytes
//!   POST   /picture       (Bearer) {kind, data}   ->  {}
//!   GET    /news                                  ->  [{title, body}]
//!   GET    /news/game      (Bearer)               ->  [{title, body}]
//!   GET    /daily          (Bearer)               ->  {streak, claimed}
//!   POST   /daily          (Bearer)               ->  {claimed, streak}
//!   GET    /classes        (Bearer)               ->  [{class, locked}]
//!   POST   /characters     (Bearer) {class, name} ->  {character}
//! ```
//!
//! Beside those, and on the same port, are the thirty-eight app-engine routes the game's own
//! clients speak: form-encoded posts under `/account/`, `/char/`, `/app/`, `/guild/` and their
//! siblings, answered in XML. They are the specification's, path for path and refusal for refusal;
//! see [`legacy`] for the dialect and [`appfiles`] for the documents the static ones hand back.
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
use hendra_store::{Admin, Store, StoreError};
use serde::{Deserialize, Serialize};

pub mod appfiles;
pub mod legacy;
pub mod mail;
pub mod throttle;
pub use appfiles::AppFiles;
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

    /// How many passwords have recently failed against each name.
    pub throttle: Throttle,

    /// How many passwords may be hashed at once.
    pub hashing: tokio::sync::Semaphore,

    /// Where the content is, for the files a client fetches rather than the summaries it reads.
    pub content: std::path::PathBuf,

    /// The documents and artwork the app-engine endpoints hand back unchanged, read at startup as
    /// the original reads them.
    pub files: AppFiles,
}

impl App {
    pub fn with_content(store: Store, key: TokenKey, catalog: Arc<hendra_content::Catalog>) -> App {
        App {
            store,
            key,
            servers: Vec::new(),
            mail: Arc::new(mail::Logged),
            catalog,
            throttle: Throttle::new(),
            hashing: tokio::sync::Semaphore::new(throttle::CONCURRENT_HASHES),
            content: std::path::PathBuf::from("."),
            files: AppFiles::default(),
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

/// Finds the account a sign-in name refers to.
///
/// An address or an account name, because the two name the same account from different screens: an
/// account is registered by email address and later chooses a name to be known by, and both are
/// still what its owner would type. Tried in that order, since the address is what the login form
/// asks for.
async fn account_for(app: &App, name: &str) -> Result<hendra_store::Account, StoreError> {
    match app.store.account_by_email(name).await {
        Ok(id) => app.store.account(id).await,
        Err(StoreError::NoSuchAccount(_)) => app.store.account_by_name(name).await,
        Err(err) => Err(err),
    }
}

/// Checks a name and password, and answers with the account they belong to.
///
/// Shared by every way in rather than written per endpoint, so that the rate limit means the same
/// thing whichever door a password is tried against: a limiter one endpoint enforces and another
/// does not is not a limiter.
pub async fn sign_in(
    app: &App,
    name: &str,
    password: &str,
) -> Result<hendra_store::Account, (StatusCode, Json<Refusal>)> {
    let name = name.trim();
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

    let account = match account_for(app, name).await {
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
        verify_password(password, stored)
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

    Ok(account)
}

pub async fn login(State(app): State<Arc<App>>, Json(body): Json<Credentials>) -> Answer<LoggedIn> {
    let account = sign_in(&app, &body.name, &body.password).await?;

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
}

/// Makes a character of a class, if the account has unlocked it.
///
/// A class is all that is asked for. The character answers to the account's name, as
/// `realm/entities/player/Player.cs:423` reads it, so there is no name to be given here and none to
/// disagree with the one on the account.
pub async fn create_character(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewCharacter>,
) -> Answer<Character> {
    let claims = authenticate(&app, &headers)?;

    let made = hendra_characters::create(
        &app.store,
        &app.catalog,
        claims.account_id,
        hendra_content::ObjectType(body.class),
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

/// What a second name costs an account that already chose its first.
///
/// `ChooseNameHandler.cs:63` and `:73`, where the check and the charge are the same number.
pub const RENAME_FAME: i32 = 5000;

/// Claims the name an account is known by.
///
/// This is `ChooseName` (`networking/handlers/ChooseNameHandler.cs:24-96`), which is a packet there
/// and a request here because names belong to accounts rather than to whoever happens to be in a
/// world. Its rules, in its order: the first letter is capitalised, the name must be letters only
/// and three to ten of them, none of the reserved names, and not one somebody already has.
///
/// An account whose name it has not chosen -- one still under a reserved name -- names itself for
/// nothing. One that has chosen already pays five thousand fame to choose again, and is refused
/// when it cannot, which is the only thing stopping a name from being free to churn.
pub async fn set_name(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewName>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    // `char.ToUpper(name[0]) + name.Substring(1)` (`ChooseNameHandler.cs:35-36`), applied before
    // the rules rather than after, so `bob` and `Bob` are one name and are judged as one.
    let mut characters = body.name.trim().chars();
    let name: String = match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    };

    let length = name.chars().count();
    if length < 3 || length > 10 || !name.chars().all(char::is_alphabetic) {
        return Err(refuse(StatusCode::BAD_REQUEST, "Invalid name"));
    }

    if legacy::is_guest_name(&name) {
        return Err(refuse(StatusCode::BAD_REQUEST, "Invalid name"));
    }

    let Ok(account) = app.store.account(claims.account_id).await else {
        return Err(refuse(StatusCode::NOT_FOUND, "no such account"));
    };

    // `Account.NameChosen`, which a reserved name stands for here: those are the names an unnamed
    // account is given and the ones no account may choose, so wearing one means never having
    // chosen (`legacy::reserve_name`, `common/Database.cs:82-84`).
    let price = if legacy::is_guest_name(&account.name) {
        0
    } else {
        RENAME_FAME
    };

    match app
        .store
        .rename_account_for(claims.account_id, &name, price)
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({}))),
        Err(StoreError::NameTaken) => Err(refuse(StatusCode::CONFLICT, "Duplicated name")),
        Err(StoreError::Refused(why)) => Err(refuse(StatusCode::PAYMENT_REQUIRED, why)),
        Err(err) => {
            tracing::error!(%err, "could not rename an account");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

#[derive(Deserialize)]
pub struct DiscordLink {
    /// Who to link, by account name. An administrator does this on somebody else's behalf.
    pub account: String,
    pub discord_id: String,
}

/// Links an account to a Discord user, for an administrator.
///
/// Administrators only, as in the original, where it is gated on the rank-manager flag. Anybody
/// being able to claim a Discord id for any account would make the link say the opposite of what it
/// is for: a bot could be told that whoever asked is whoever they named.
pub async fn register_discord(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<DiscordLink>,
) -> Answer<serde_json::Value> {
    let account_id = administering(&app, &headers, &body.account).await?;

    match app
        .store
        .register_discord(account_id, body.discord_id.trim())
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({}))),
        Err(StoreError::Refused(why)) => Err(refuse(StatusCode::BAD_REQUEST, why)),
        Err(err) => {
            tracing::error!(%err, "could not link a discord account");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

/// Unlinks an account from a Discord user, for an administrator.
pub async fn unregister_discord(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<DiscordLink>,
) -> Answer<serde_json::Value> {
    let account_id = administering(&app, &headers, &body.account).await?;

    match app
        .store
        .unregister_discord(account_id, body.discord_id.trim())
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({}))),
        Err(StoreError::Refused(why)) => Err(refuse(StatusCode::BAD_REQUEST, why)),
        Err(err) => {
            tracing::error!(%err, "could not unlink a discord account");
            Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ))
        }
    }
}

/// Checks the caller may administer, and finds the account they named.
///
/// The order matters: whether the caller is allowed is decided before the named account is looked
/// up, so a refusal cannot be used to find out which names exist.
async fn administering(
    app: &App,
    headers: &HeaderMap,
    name: &str,
) -> Result<i64, (StatusCode, Json<Refusal>)> {
    let claims = authenticate(app, headers)?;

    let caller = app
        .store
        .account(claims.account_id)
        .await
        .map_err(|_| refuse(StatusCode::UNAUTHORIZED, "no such account"))?;

    if Admin::from_number(caller.admin_rank) < Admin::OWNER {
        return Err(refuse(StatusCode::FORBIDDEN, "no permission"));
    }

    app.store
        .account_by_name(name)
        .await
        .map(|account| account.id)
        .map_err(|_| refuse(StatusCode::NOT_FOUND, "no such account"))
}

/// What the client is expected to enforce for itself, and what the server enforces regardless.
///
/// The original answers this with a hash comparison: the client sends SHA-256 of its own hardcoded
/// rate of fire, mp cost and cooldown, and is let in if they match. That check secures nothing. The
/// constants are public, the hash is unsalted, and a client that has been changed to cheat can send
/// the hashes of the values it is supposed to have.
///
/// So the endpoint answers, because a client that asks should get an answer, and the numbers it
/// returns are the ones the server itself uses. They are here to be shown, not to be trusted:
/// rate of fire, ability cost and cooldown are all decided server-side in `crates/sim`, and a client
/// that ignores every one of them gets the same result as one that obeys.
pub async fn security_protocols() -> Json<SecurityProtocols> {
    Json(SecurityProtocols {
        rate_of_fire: 1.0,
        num_projectiles: 1,
        arc_gap: 11.25,
        cooldown_ms: 1000,
        enforced_by: "server",
    })
}

#[derive(Serialize)]
pub struct SecurityProtocols {
    pub rate_of_fire: f32,
    pub num_projectiles: u32,
    pub arc_gap: f32,
    pub cooldown_ms: u32,

    /// Which side actually decides. Always the server.
    pub enforced_by: &'static str,
}

/// The client's texture pack.
///
/// The original serves a zip it loaded at boot. This serves the same file from the content
/// directory when it is there, and says plainly when it is not: a client told "no textures" can ship
/// its own, and a client handed an empty archive cannot tell that from a corrupt one.
pub async fn textures(
    State(app): State<Arc<App>>,
) -> Result<axum::response::Response, (StatusCode, Json<Refusal>)> {
    use axum::response::IntoResponse;

    let path = app.content.join(TEXTURE_PACK);
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Err(refuse(StatusCode::NOT_FOUND, "no texture pack"));
    };

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/zip")],
        bytes,
    )
        .into_response())
}

/// What the texture pack is called in the content directory.
pub const TEXTURE_PACK: &str = "textures.zip";

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

#[derive(Serialize)]
pub struct ContentSummary {
    pub objects: usize,
    pub tiles: usize,
    pub classes: usize,

    /// Every playable class, so a client can draw a select screen without a token.
    pub class_ids: Vec<String>,
}

/// What content the server is running.
///
/// A summary rather than the files themselves. The client ships its own copy of the content and
/// always has; what it cannot know is whether the server agrees, and a count that differs is the
/// cheapest possible signal that the two have drifted.
pub async fn content(State(app): State<Arc<App>>) -> Json<ContentSummary> {
    let class_ids = app
        .catalog
        .classes()
        .iter()
        .filter_map(|class| app.catalog.object(class.object_type))
        .map(|desc| desc.id.clone())
        .collect();

    Json(ContentSummary {
        objects: app.catalog.object_count(),
        tiles: app.catalog.tile_count(),
        classes: app.catalog.classes().len(),
        class_ids,
    })
}

/// Every translated string for a language.
///
/// Unauthenticated, because a client picks its language before it logs in.
pub async fn strings(
    State(app): State<Arc<App>>,
    axum::extract::Path(language): axum::extract::Path<String>,
) -> Json<std::collections::BTreeMap<String, String>> {
    Json(
        app.store
            .strings(&language)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect(),
    )
}

#[derive(Serialize)]
pub struct OfferItem {
    pub id: i64,
    pub name: String,
    pub credits: i32,
    pub price_cents: i32,
}

/// What is for sale.
///
/// Prices come from the server, because a client that decides its own prices decides its own
/// prices.
pub async fn offers(State(app): State<Arc<App>>) -> Json<Vec<OfferItem>> {
    Json(
        app.store
            .credit_offers()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|offer| OfferItem {
                id: offer.id,
                name: offer.name,
                credits: offer.credits,
                price_cents: offer.price_cents,
            })
            .collect(),
    )
}

#[derive(Serialize)]
pub struct QuestItem {
    pub key: String,
    pub title: String,
    pub progress: i32,
    pub goal: i32,
    pub finished: bool,
}

/// Every quest, with how far this account has got.
pub async fn quests(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<QuestItem>> {
    listed_quests(&app, &headers, None).await
}

/// The same, for the weekly ones only.
pub async fn weekly_quests(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Answer<Vec<QuestItem>> {
    listed_quests(&app, &headers, Some(true)).await
}

async fn listed_quests(
    app: &Arc<App>,
    headers: &HeaderMap,
    weekly: Option<bool>,
) -> Answer<Vec<QuestItem>> {
    let claims = authenticate(app, headers)?;

    let listed = app
        .store
        .quests(claims.account_id, weekly)
        .await
        .unwrap_or_default();

    Ok(Json(
        listed
            .into_iter()
            .map(|quest| QuestItem {
                key: quest.key,
                title: quest.title,
                progress: quest.progress,
                goal: quest.goal,
                finished: quest.finished,
            })
            .collect(),
    ))
}

/// Records that an account has confirmed its age.
///
/// The server records the answer rather than the date of birth. What it needs to know is whether
/// somebody said yes; keeping the date would be keeping something it has no use for.
pub async fn verify_age(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    app.store
        .set_age_verified(claims.account_id, true)
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not record an age confirmation");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
        })?;

    Ok(Json(serde_json::json!({ "verified": true })))
}

#[derive(Serialize)]
pub struct Wardrobe {
    pub owned: Vec<String>,
    pub credits: i32,
}

/// Which skins an account owns, and what it can spend.
pub async fn skins(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Wardrobe> {
    let claims = authenticate(&app, &headers)?;

    let owned = app
        .store
        .owned_skins(claims.account_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|skin| skin.to_string())
        .collect();

    let credits = app
        .store
        .account(claims.account_id)
        .await
        .map(|account| account.credits)
        .unwrap_or(0);

    Ok(Json(Wardrobe { owned, credits }))
}

#[derive(Deserialize)]
pub struct BuySkin {
    pub skin: String,
    pub price: i32,
}

/// Buys a skin with credits.
pub async fn buy_skin(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<BuySkin>,
) -> Answer<serde_json::Value> {
    let claims = authenticate(&app, &headers)?;

    let Ok(skin) = uuid::Uuid::parse_str(body.skin.trim()) else {
        return Err(refuse(StatusCode::BAD_REQUEST, "no such skin"));
    };

    // The price is checked against the content rather than taken as given, or a client would name
    // its own.
    let price = app
        .catalog
        .type_of_uuid(skin)
        .and_then(|found| app.catalog.object(found))
        .and_then(|desc| desc.item.as_ref())
        .map(|item| item.fame_bonus.max(0))
        .unwrap_or(body.price.max(0));

    app.store
        .buy_skin(claims.account_id, skin, price)
        .await
        .map_err(|err| match err {
            StoreError::Refused(why) => refuse(StatusCode::BAD_REQUEST, why),
            other => {
                tracing::error!(%other, "could not buy a skin");
                refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
            }
        })?;

    Ok(Json(serde_json::json!({})))
}

#[derive(Deserialize)]
pub struct NewPicture {
    pub kind: String,

    /// The bytes, base64 encoded, because JSON cannot carry them otherwise.
    pub data: String,
}

/// Stores a picture for an account.
pub async fn set_picture(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<NewPicture>,
) -> Answer<serde_json::Value> {
    use base64::Engine;

    let claims = authenticate(&app, &headers)?;

    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(body.data.trim()) else {
        return Err(refuse(StatusCode::BAD_REQUEST, "that is not readable data"));
    };

    app.store
        .set_picture(claims.account_id, &body.kind, &bytes)
        .await
        .map_err(|err| match err {
            StoreError::Refused(why) => refuse(StatusCode::BAD_REQUEST, why),
            other => {
                tracing::error!(%other, "could not store a picture");
                refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
            }
        })?;

    Ok(Json(serde_json::json!({})))
}

/// An account's picture.
///
/// Unauthenticated, because a picture is shown beside a name to people who are not that account.
pub async fn picture(
    State(app): State<Arc<App>>,
    axum::extract::Path(account_id): axum::extract::Path<i64>,
) -> Result<axum::response::Response, (StatusCode, Json<Refusal>)> {
    use axum::response::IntoResponse;

    let Ok(Some((kind, bytes))) = app.store.picture(account_id).await else {
        return Err(refuse(StatusCode::NOT_FOUND, "no picture"));
    };

    // The stored kind is not echoed into the header. A content type a caller chose is a content
    // type a caller can use to make a browser run something.
    let content_type = match kind.as_str() {
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        _ => "application/octet-stream",
    };

    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], bytes).into_response())
}

/// One music track or sound effect, by name and without its extension.
///
/// The original serves `web/music` and `web/sfx` as ordinary static files and the client builds
/// `<app server>/<folder>/<name>.mp3` for each. Unauthenticated, as static files are: a world names
/// what it is playing in its welcome, and every client in it fetches the same track.
pub async fn audio(
    State(app): State<Arc<App>>,
    axum::extract::Path((folder, name)): axum::extract::Path<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, Json<Refusal>)> {
    use axum::response::IntoResponse;

    // Two folders, named here rather than taken from the path: anything else is not audio, and a
    // folder a caller chose is a folder a caller can walk out of.
    if folder != "music" && folder != "sfx" {
        return Err(refuse(StatusCode::NOT_FOUND, "no such sound"));
    }

    let Some(bytes) = app.files.audio(&folder, &name) else {
        return Err(refuse(StatusCode::NOT_FOUND, "no such sound"));
    };

    Ok(([(axum::http::header::CONTENT_TYPE, "audio/mpeg")], bytes).into_response())
}

#[derive(Serialize)]
pub struct NewsItem {
    pub title: String,
    pub body: String,
}

/// What the server has announced.
///
/// Unauthenticated, because it is shown at the title screen before anybody has logged in.
pub async fn news(State(app): State<Arc<App>>) -> Json<Vec<NewsItem>> {
    Json(listed_news(&app, false).await)
}

/// The same, for the panel inside the game.
pub async fn game_news(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Vec<NewsItem>> {
    authenticate(&app, &headers)?;
    Ok(Json(listed_news(&app, true).await))
}

async fn listed_news(app: &App, in_game: bool) -> Vec<NewsItem> {
    app.store
        .news(in_game, 20)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|item| NewsItem {
            title: item.title,
            body: item.body,
        })
        .collect()
}

#[derive(Serialize)]
pub struct Daily {
    pub streak: i64,
    pub claimed_today: bool,
}

/// How many days in a row, and whether today is still available.
pub async fn daily(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Daily> {
    let claims = authenticate(&app, &headers)?;

    let calendar = app.store.calendar(claims.account_id).await.map_err(|err| {
        tracing::error!(%err, "could not read a calendar");
        refuse(StatusCode::INTERNAL_SERVER_ERROR, "try again shortly")
    })?;

    Ok(Json(Daily {
        streak: calendar.streak,
        claimed_today: calendar.claimed_today,
    }))
}

/// Claims today.
///
/// Answers with what the calendar became rather than only whether it worked, so a client that
/// claimed and one that was too late show the same thing.
pub async fn claim_daily(State(app): State<Arc<App>>, headers: HeaderMap) -> Answer<Daily> {
    let claims = authenticate(&app, &headers)?;

    let _ = app.store.claim_today(claims.account_id).await;
    daily(State(app), headers).await
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
        .route("/content", get(content))
        .route("/strings/{language}", get(strings))
        .route("/offers", get(offers))
        .route("/quests", get(quests))
        .route("/quests/weekly", get(weekly_quests))
        .route("/age", post(verify_age))
        .route("/skins", get(skins).post(buy_skin))
        .route("/picture", post(set_picture))
        .route("/picture/{account_id}", get(picture))
        // The music and sound effects, which the original serves out of the same folder as ordinary
        // static files. Without them a world can name what it is playing and nobody hears it.
        .route("/{folder}/{name}", get(audio))
        .route("/news", get(news))
        .route("/news/game", get(game_news))
        .route("/daily", get(daily).post(claim_daily))
        .route("/messages", get(messages))
        .route("/friends", get(friends).post(add_friend))
        .route("/friends/{id}", axum::routing::delete(remove_friend))
        .route("/register", post(register))
        .route("/textures", get(textures))
        .route("/security", get(security_protocols))
        .route(
            "/discord",
            post(register_discord).delete(unregister_discord),
        )
        .route("/login", post(login))
        .route("/characters", get(characters))
        .route("/characters/{id}", axum::routing::delete(delete_character))
        .route("/characters", post(create_character))
        .route("/classes", get(classes))
        .route("/select", post(select))
        .route("/password", post(change_password))
        // The app-engine endpoints, which is the dialect the game's clients actually speak. See
        // `legacy` for why these are served rather than the clients being moved onto the routes
        // above.
        .route("/account/verify", post(legacy::verify))
        .route("/account/register", post(legacy::register))
        .route("/account/setName", post(legacy::set_name))
        .route("/account/changePassword", post(legacy::change_password))
        .route("/account/forgotPassword", post(legacy::forgot_password))
        .route(
            "/account/rp",
            get(legacy::reset_password_link).post(legacy::reset_password_form),
        )
        .route("/account/sendVerifyEmail", post(legacy::send_verify_email))
        .route("/account/purchaseCharSlot", post(legacy::purchase_char_slot))
        .route("/account/purchaseSkin", post(legacy::purchase_skin))
        .route("/account/verifyage", post(legacy::verify_age))
        .route("/account/rank", post(legacy::set_rank))
        .route("/account/registerDiscord", post(legacy::register_discord))
        .route("/account/unregisterDiscord", post(legacy::unregister_discord))
        .route("/char/list", post(legacy::char_list))
        .route("/char/fame", post(legacy::char_fame))
        .route("/char/delete", post(legacy::char_delete))
        .route("/char/purchaseClassUnlock", post(legacy::purchase_class_unlock))
        .route("/app/init", post(legacy::app_init))
        .route("/app/getServerXmls", post(legacy::app_server_xmls))
        .route("/app/getLanguageStrings", post(legacy::app_language_strings))
        .route("/app/globalNews", post(legacy::app_global_news))
        .route("/app/getTextures", post(legacy::app_textures))
        .route("/credits/getoffers", post(legacy::credits_offers))
        .route("/credits/add", post(legacy::credits_add))
        .route("/fame/list", post(legacy::fame_list))
        .route("/picture/get", post(legacy::picture_get))
        .route("/guild/listMembers", post(legacy::guild_members))
        .route("/guild/getBoard", post(legacy::guild_board))
        .route("/guild/setBoard", post(legacy::set_guild_board))
        .route("/privateMessage/list", post(legacy::message_list))
        .route("/privateMessage/send", post(legacy::message_send))
        .route("/privateMessage/delete", post(legacy::message_delete))
        .route("/dailyLogin/fetchCalendar", post(legacy::daily_calendar))
        .route("/weekQuest/getQuests", post(legacy::week_quests))
        .route("/inGameNews/getNews", post(legacy::in_game_news))
        .route("/friends/getList", post(legacy::friends_list))
        .route("/friends/getRequests", post(legacy::friend_requests))
        // A body limit, because both credential endpoints hash what they are given and Argon2 is
        // meant to be slow.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app)
}
