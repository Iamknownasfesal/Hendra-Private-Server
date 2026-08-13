//! The app server: registration, login, and the character list.
//!
//! Everything a player does before they are in a world happens here, over HTTPS. It is the only
//! thing that ever sees a password, and the only thing that mints a session token. The game server
//! sees neither — it takes a token and checks the signature.
//!
//! ```text
//!   POST /register   {name, password}  ->  {token, account_id}
//!   POST /login      {name, password}  ->  {token, account_id, characters}
//!   GET  /characters (Bearer token)    ->  {characters}
//! ```
//!
//! # On running this behind something
//!
//! There is no TLS here. Terminating it belongs to whatever sits in front — a reverse proxy that
//! also handles certificate renewal — and putting a second, worse implementation of it in this
//! process would be a way to get it wrong twice. The server refuses to bind a public address
//! without being told that something is in front of it.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use hendra_auth::{Claims, TokenKey, hash_password, mint, now, verify, verify_password};
use hendra_store::{Store, StoreError};
use serde::{Deserialize, Serialize};

pub struct App {
    pub store: Store,
    pub key: TokenKey,
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

    let hash = hash_password(&body.password)
        .map_err(|err| refuse(StatusCode::BAD_REQUEST, &err.to_string()))?;

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
    let account = match app.store.account_by_name(body.name.trim()).await {
        Ok(account) => account,
        Err(StoreError::NoSuchAccount(_)) => return Err(bad_credentials()),
        Err(err) => {
            tracing::error!(%err, "could not read an account");
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "try again shortly",
            ));
        }
    };

    // An account made before authentication existed has no password. Refusing by name is right:
    // the alternative is comparing against nothing and letting anyone in.
    let Some(stored) = account.password_hash.as_deref() else {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "this account has no password set; contact an administrator",
        ));
    };

    match verify_password(&body.password, stored) {
        Ok(true) => {}
        Ok(false) => return Err(bad_credentials()),
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
            object_type: summary.object_type,
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
                object_type: summary.object_type,
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
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/characters", get(characters))
        // A body limit, because both credential endpoints hash what they are given and Argon2 is
        // meant to be slow.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(8 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(app)
}
