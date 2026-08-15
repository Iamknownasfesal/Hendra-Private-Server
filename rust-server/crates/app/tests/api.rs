//! The app server's endpoints, driven end to end.
//!
//! These call the real router against a real database rather than the handler functions directly,
//! because most of what could go wrong here is in the seams: what status a refusal carries, whether
//! a body that is not what the handler expects becomes a rejection or a panic, and whether the
//! token that comes back out is one the game server would accept.
//!
//! Set `HENDRA_TEST_DATABASE` to point at a Postgres. Without it these skip rather than fail.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hendra_app::{App, router};
use hendra_auth::TokenKey;
use hendra_store::Store;
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";

/// A class identity for characters that only need to exist.
///
/// Any UUID does: these tests never ask the catalog what the class is, only that the character is
/// there and belongs to the right account.
fn a_class() -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0300)
}

/// The identity the catalog gave a class, which is what progress is keyed by.
fn class_uuid(app: &App, object_type: u16) -> uuid::Uuid {
    app.catalog
        .object(hendra_content::ObjectType(object_type))
        .map(|desc| desc.uuid)
        .expect("the class is in the catalog")
}

/// The warrior, which is what unlocks the knight.
fn warrior(app: &App) -> uuid::Uuid {
    class_uuid(app, 0x031d)
}

/// The key both servers share in these tests, and the only one they share.
fn key() -> TokenKey {
    TokenKey::new(vec![3u8; 32]).unwrap()
}

/// An app on a schema of its own, or `None` when no database is configured.
///
/// A schema per test, so tests that create accounts with the same name do not collide and none of
/// them depends on the order the others ran in.
/// The connection string for a schema, so a second app can share one database with the first.
fn scoped_url(schema: &str) -> String {
    let url = std::env::var("HENDRA_TEST_DATABASE").unwrap_or_default();
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}options=-csearch_path%3D{schema}")
}

async fn app(schema: &str) -> Option<Arc<App>> {
    let url = std::env::var("HENDRA_TEST_DATABASE").ok()?;

    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .ok()?;
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(&admin)
        .await
        .ok()?;
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .ok()?;
    admin.close().await;

    let separator = if url.contains('?') { '&' } else { '?' };
    let scoped = format!("{url}{separator}options=-csearch_path%3D{schema}");

    let store = Store::connect(&scoped).await.ok()?;
    Some(Arc::new(App::with_content(
        store,
        key(),
        Arc::new(content()),
    )))
}

/// The real class files, so the class tests are about the game's fourteen classes rather than a
/// fixture that agrees with whatever the code does.
fn content() -> hendra_content::Catalog {
    let dir = std::path::Path::new("../../../godot-client/assets/xml");
    match hendra_content::Catalog::load_dir(dir) {
        Ok((catalog, _)) => catalog,
        Err(_) => hendra_content::Catalog::default(),
    }
}

/// Sends one request and returns the status and the body as JSON.
///
/// The router is consumed per call because that is what `oneshot` takes; building it again is
/// cheap, and it means no test can leave state in a router another test then uses.
async fn send(app: &Arc<App>, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// The same as `post`, named apart so a test may shadow `post` with a captured mailbox.
fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    post(path, body)
}

fn post(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

/// A form post, which is how the legacy endpoints are asked for anything.
fn post_form(path: &str, fields: &[(&str, String)]) -> Request<Body> {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");

    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

/// Sends one request and returns the status and the body as it was written, for the endpoints that
/// answer in XML rather than JSON.
async fn send_text(app: &Arc<App>, request: Request<Body>) -> (StatusCode, String) {
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn credentials(name: &str, password: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "password": password })
}

macro_rules! app_or_skip {
    ($schema:literal) => {
        match app($schema).await {
            Some(app) => app,
            None => {
                eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_then_logging_in_gives_a_usable_token() {
    let app = app_or_skip!("a_register");

    let (status, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    assert_eq!(status, StatusCode::OK);
    let registered = body["token"].as_str().unwrap().to_string();

    // The token the game server would be handed, checked the way the game server checks it.
    let claims = hendra_auth::verify(&key(), &registered, hendra_auth::now()).unwrap();
    assert_eq!(claims.account_id, body["account_id"].as_i64().unwrap());

    let (status, body) = send(&app, post("/login", credentials("Fesal", PASSWORD))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["account_id"].as_i64(), Some(claims.account_id));
    assert!(
        hendra_auth::verify(&key(), body["token"].as_str().unwrap(), hendra_auth::now()).is_ok()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_from_this_server_is_not_valid_at_a_server_with_another_key() {
    // Two deployments configured with different secrets show up as players who log in
    // successfully and are then refused by the game.
    let app = app_or_skip!("a_otherkey");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap();

    let elsewhere = TokenKey::new(vec![9u8; 32]).unwrap();
    assert_eq!(
        hendra_auth::verify(&elsewhere, token, hendra_auth::now()),
        Err(hendra_auth::TokenError::BadSignature)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_name_and_a_wrong_password_are_refused_identically() {
    // Telling them apart makes login an oracle for which accounts exist, which is the first thing
    // worth knowing to anyone attacking it.
    let app = app_or_skip!("a_oracle");

    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;

    let (unknown_status, unknown_body) =
        send(&app, post("/login", credentials("NobodyHere", PASSWORD))).await;
    let (wrong_status, wrong_body) = send(
        &app,
        post("/login", credentials("Fesal", "not the password")),
    )
    .await;

    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_status, unknown_status);
    assert_eq!(wrong_body, unknown_body);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_can_only_be_registered_once() {
    let app = app_or_skip!("a_taken");

    let (first, _) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    assert_eq!(first, StatusCode::OK);

    let (second, _) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    assert_eq!(second, StatusCode::CONFLICT);

    // And the second attempt did not overwrite the first one's password.
    let (status, _) = send(&app, post("/login", credentials("Fesal", PASSWORD))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_short_password_is_refused_at_registration() {
    let app = app_or_skip!("a_short");

    let (status, body) = send(&app, post("/register", credentials("Fesal", "short"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("at least"));

    // And nothing was created, so the name is still free.
    let (status, _) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_or_oversized_name_is_refused() {
    let app = app_or_skip!("a_names");

    for name in ["", "   ", &"a".repeat(33)] {
        let (status, _) = send(&app, post("/register", credentials(name, PASSWORD))).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "name {name:?} was accepted"
        );
    }

    assert_eq!(
        send(
            &app,
            post("/register", credentials(&"a".repeat(32), PASSWORD))
        )
        .await
        .0,
        StatusCode::OK,
        "thirty-two characters is the limit, not one past it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_password_never_comes_back_out() {
    let app = app_or_skip!("a_leak");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let text = body.to_string();

    assert!(!text.contains(PASSWORD));
    assert!(!text.contains("argon2"), "not the hash either");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_account_is_refused_only_after_the_password_is_checked() {
    // Refusing a banned account before checking the password would make the ban itself a way to
    // confirm that an account exists without knowing anything about it.
    let app = app_or_skip!("a_banned");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let account_id = body["account_id"].as_i64().unwrap();

    sqlx::query("UPDATE account SET banned = true WHERE id = $1")
        .bind(account_id)
        .execute(app.store.pool())
        .await
        .unwrap();

    let (banned, _) = send(&app, post("/login", credentials("Fesal", PASSWORD))).await;
    assert_eq!(banned, StatusCode::FORBIDDEN);

    let (wrong, _) = send(&app, post("/login", credentials("Fesal", "not it"))).await;
    assert_eq!(
        wrong,
        StatusCode::UNAUTHORIZED,
        "a wrong password on a banned account must look like any other wrong password"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_with_no_password_cannot_be_logged_into() {
    // Accounts predating authentication have a null hash. The alternative to refusing them is
    // comparing against nothing.
    let app = app_or_skip!("a_nopassword");

    app.store.create_account("Ancient").await.unwrap();

    let (status, _) = send(&app, post("/login", credentials("Ancient", PASSWORD))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn characters_needs_a_token_and_lists_only_that_account_s() {
    let app = app_or_skip!("a_characters");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let (_, theirs) = send(&app, post("/register", credentials("Someone", PASSWORD))).await;

    let account = mine["account_id"].as_i64().unwrap();
    let other = theirs["account_id"].as_i64().unwrap();

    app.store
        .create_character(account, a_class(), 800)
        .await
        .unwrap();
    app.store
        .create_character(other, a_class(), 800)
        .await
        .unwrap();

    let (status, body) = send(
        &app,
        get("/characters", Some(mine["token"].as_str().unwrap())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["name"], "Fesal");
}

#[tokio::test(flavor = "multi_thread")]
async fn characters_refuses_a_missing_forged_or_expired_token() {
    let app = app_or_skip!("a_bearer");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    assert_eq!(
        send(&app, get("/characters", None)).await.0,
        StatusCode::UNAUTHORIZED
    );

    // The right shape, signed by the wrong key.
    let forged = hendra_auth::mint(
        &TokenKey::new(vec![9u8; 32]).unwrap(),
        hendra_auth::Claims {
            account_id: account,
            character_id: 0,
            expires_at: hendra_auth::now() + 600,
        },
    );
    assert_eq!(
        send(&app, get("/characters", Some(&forged.0))).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Correctly signed, and stale.
    let expired = hendra_auth::mint(
        &key(),
        hendra_auth::Claims {
            account_id: account,
            character_id: 0,
            expires_at: hendra_auth::now() - 1,
        },
    );
    assert_eq!(
        send(&app, get("/characters", Some(&expired.0))).await.0,
        StatusCode::UNAUTHORIZED
    );

    // And the live one still works, so the refusals above are about the tokens and not the route.
    assert_eq!(
        send(&app, get("/characters", Some(&token))).await.0,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_without_the_bearer_prefix_is_refused() {
    let app = app_or_skip!("a_prefix");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap();

    let request = Request::builder()
        .method("GET")
        .uri("/characters")
        .header("authorization", token)
        .body(Body::empty())
        .unwrap();

    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_that_is_not_what_the_endpoint_expects_is_a_refusal_rather_than_a_fault() {
    let app = app_or_skip!("a_junk");

    for body in [
        "".to_string(),
        "not json".to_string(),
        "{}".to_string(),
        serde_json::json!({ "name": 7, "password": PASSWORD }).to_string(),
        serde_json::json!({ "name": "Fesal" }).to_string(),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap();

        let (status, _) = send(&app, request).await;
        assert!(
            status.is_client_error(),
            "body {body:?} answered {status} rather than a client error"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_enormous_body_is_rejected_without_hashing_it() {
    // Argon2 is slow by design, so an unauthenticated endpoint that hashes whatever it is sent
    // is a way to spend the server's time cheaply.
    let app = app_or_skip!("a_huge");

    let request = post("/register", credentials("Fesal", &"a".repeat(64 * 1024)));
    let (status, _) = send(&app, request).await;
    assert!(status.is_client_error(), "answered {status}");
}

#[tokio::test(flavor = "multi_thread")]
async fn health_needs_nothing() {
    let app = app_or_skip!("a_health");

    let response = router(app).oneshot(get("/health", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn guessing_stops_after_enough_failures() {
    let app = app_or_skip!("a_throttle");

    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;

    for attempt in 0..hendra_app::throttle::FAILURES_ALLOWED {
        let (status, _) = send(&app, post("/login", credentials("Fesal", "wrong"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }

    let (status, body) = send(&app, post("/login", credentials("Fesal", "wrong"))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body["error"].as_str().unwrap().contains("try again"));

    // And the right password is refused too, which is the cost of the limit rather than a bug: the
    // alternative is a check that tells the guesser when they have found it.
    let (status, _) = send(&app, post("/login", credentials("Fesal", PASSWORD))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correct_password_clears_the_failures_before_the_limit_is_reached() {
    let app = app_or_skip!("a_throttle_clear");

    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;

    for _ in 0..hendra_app::throttle::FAILURES_ALLOWED - 1 {
        send(&app, post("/login", credentials("Fesal", "wrong"))).await;
    }
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::OK
    );

    // The count is back to nothing, so a full run of failures is available again.
    for attempt in 0..hendra_app::throttle::FAILURES_ALLOWED {
        assert_eq!(
            send(&app, post("/login", credentials("Fesal", "wrong")))
                .await
                .0,
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} should not have been throttled yet"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn throttling_one_name_does_not_throttle_another() {
    let app = app_or_skip!("a_throttle_apart");

    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    send(&app, post("/register", credentials("Someone", PASSWORD))).await;

    for _ in 0..hendra_app::throttle::FAILURES_ALLOWED + 2 {
        send(&app, post("/login", credentials("Fesal", "wrong"))).await;
    }

    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        send(&app, post("/login", credentials("Someone", PASSWORD)))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_that_does_not_exist_throttles_like_one_that_does() {
    // Otherwise the limiter is the oracle that login refuses to be: the name that never locks out
    // is the name with no account behind it.
    let app = app_or_skip!("a_throttle_unknown");

    for _ in 0..hendra_app::throttle::FAILURES_ALLOWED {
        send(&app, post("/login", credentials("NobodyHere", PASSWORD))).await;
    }

    let (status, _) = send(&app, post("/login", credentials("NobodyHere", PASSWORD))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "multi_thread")]
async fn selecting_a_character_puts_it_in_the_token() {
    let app = app_or_skip!("a_select");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    let character = app
        .store
        .create_character(account, a_class(), 800)
        .await
        .unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/select")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({ "character_id": character.id }).to_string(),
        ))
        .unwrap();

    let (status, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK);

    let claims =
        hendra_auth::verify(&key(), body["token"].as_str().unwrap(), hendra_auth::now()).unwrap();
    assert_eq!(claims.account_id, account);
    assert_eq!(claims.character_id, character.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_character_belonging_to_someone_else_cannot_be_selected() {
    // The game server checks ownership too, so this is the second of two independent checks. It
    // matters because a signed token naming someone else's character should not exist at all.
    let app = app_or_skip!("a_select_theirs");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let (_, theirs) = send(&app, post("/register", credentials("Someone", PASSWORD))).await;

    let other = app
        .store
        .create_character(theirs["account_id"].as_i64().unwrap(), a_class(), 800)
        .await
        .unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/select")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", mine["token"].as_str().unwrap()),
        )
        .body(Body::from(
            serde_json::json!({ "character_id": other.id }).to_string(),
        ))
        .unwrap();

    let (status, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The same answer as a character that never existed, so this is not a way to enumerate ids.
    let request = Request::builder()
        .method("POST")
        .uri("/select")
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", mine["token"].as_str().unwrap()),
        )
        .body(Body::from(
            serde_json::json!({ "character_id": 999_999 }).to_string(),
        ))
        .unwrap();
    assert_eq!(send(&app, request).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_character_takes_its_items_with_it() {
    let app = app_or_skip!("a_delete");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    let character = app
        .store
        .create_character(account, a_class(), 800)
        .await
        .unwrap();
    app.store
        .set_inventory(
            character.id,
            &[
                (0, uuid::Uuid::from_u128(0x900)),
                (1, uuid::Uuid::from_u128(0x901)),
            ],
        )
        .await
        .unwrap();

    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/characters/{}", character.id))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, request).await.0, StatusCode::OK);

    assert!(app.store.characters(account).await.unwrap().is_empty());

    let (orphans,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM inventory_slot WHERE character_id = $1")
            .bind(character.id)
            .fetch_one(app.store.pool())
            .await
            .unwrap();
    assert_eq!(orphans, 0, "the items should have gone with the character");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_character_belonging_to_someone_else_cannot_be_deleted() {
    let app = app_or_skip!("a_delete_theirs");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let (_, theirs) = send(&app, post("/register", credentials("Someone", PASSWORD))).await;
    let other_account = theirs["account_id"].as_i64().unwrap();

    let other = app
        .store
        .create_character(other_account, a_class(), 800)
        .await
        .unwrap();

    let request = Request::builder()
        .method("DELETE")
        .uri(format!("/characters/{}", other.id))
        .header(
            "authorization",
            format!("Bearer {}", mine["token"].as_str().unwrap()),
        )
        .body(Body::empty())
        .unwrap();

    assert_eq!(send(&app, request).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        app.store.characters(other_account).await.unwrap().len(),
        1,
        "it should still be there"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_a_password_needs_the_old_one() {
    // A token alone is not enough. A token expires in fifteen minutes; a password someone else set
    // does not, so a stolen token must not become permanent ownership of the account.
    let app = app_or_skip!("a_password");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();

    let change = |old: &str, new: &str| {
        Request::builder()
            .method("POST")
            .uri("/password")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(
                serde_json::json!({ "old_password": old, "new_password": new }).to_string(),
            ))
            .unwrap()
    };

    assert_eq!(
        send(&app, change("not the password", "a new password"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // And it did not change anyway.
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::OK
    );

    assert_eq!(
        send(&app, change(PASSWORD, "a new password")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", "a new password")))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "the old password should have stopped working"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_password_that_is_too_short_is_refused_and_changes_nothing() {
    let app = app_or_skip!("a_password_short");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/password")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::json!({ "old_password": PASSWORD, "new_password": "short" }).to_string(),
        ))
        .unwrap();

    assert_eq!(send(&app, request).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::OK,
        "the old password should still work"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_a_password_needs_a_token() {
    let app = app_or_skip!("a_password_token");

    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;

    let request = Request::builder()
        .method("POST")
        .uri("/password")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "old_password": PASSWORD, "new_password": "a new password" })
                .to_string(),
        ))
        .unwrap();

    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

/// The wizard, which is the only class the shipped files open with.
const WIZARD: u16 = 0x030e;

/// The knight, which needs a warrior at level twenty.
const KNIGHT: u16 = 0x031e;

fn authed(method: &str, path: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

macro_rules! content_or_skip {
    ($app:expr) => {
        if $app.catalog.classes().is_empty() {
            eprintln!("skipping: the class files are not where the test looks for them");
            return;
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn the_class_list_says_which_are_locked_and_why() {
    let app = app_or_skip!("a_classes");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap();

    let (status, body) = send(&app, get("/classes", Some(token))).await;
    assert_eq!(status, StatusCode::OK);

    let classes = body.as_array().unwrap();
    assert_eq!(classes.len(), 14, "the shipped files have fourteen");

    let open: Vec<&str> = classes
        .iter()
        .filter(|class| class["locked"].is_null())
        .map(|class| class["id"].as_str().unwrap())
        .collect();
    assert_eq!(open, ["Wizard"], "a new account starts on one class");

    let knight = classes
        .iter()
        .find(|class| class["id"] == "Knight")
        .unwrap();
    assert_eq!(
        knight["locked"].as_str().unwrap(),
        "reach level 20 with a Warrior"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_created_character_gets_its_own_class_s_gear_and_health() {
    // The whole point of reading classes: a warrior is not a wizard with a different sprite.
    let app = app_or_skip!("a_create_class");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();

    let (status, made) = send(
        &app,
        authed(
            "POST",
            "/characters",
            &token,
            serde_json::json!({ "class": WIZARD }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A class is all that is asked for, and the character comes back wearing the account's name:
    // `Player.cs:423` reads the name off the account and `DbChar` never held one.
    assert_eq!(made["name"], "Fesal");

    let character = app
        .store
        .character(made["id"].as_i64().unwrap())
        .await
        .unwrap();

    assert_eq!(character.max_hp, 100, "the wizard's own starting health");
    assert_eq!(character.hp, 100);

    let named = |slot: i16| {
        character
            .inventory
            .iter()
            .find(|(at, _)| *at == slot)
            .and_then(|(_, item)| {
                app.catalog
                    .type_of_uuid(*item)
                    .and_then(|found| app.catalog.object(found))
                    .map(|desc| desc.id.clone())
            })
    };

    // The three the Wizard's own Equipment list names, and nothing after them: the ring slot and
    // the whole pack start empty, because the class grants no ring and no potions.
    assert_eq!(named(0).as_deref(), Some("Energy Staff"));
    assert_eq!(named(1).as_deref(), Some("Fire Spray Spell"));
    assert_eq!(named(2).as_deref(), Some("Robe of the Neophyte"));
    assert_eq!(named(3), None, "the class grants no ring");
    assert_eq!(named(4), None, "the class grants nothing carried");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_locked_class_is_refused_and_nothing_is_created() {
    let app = app_or_skip!("a_create_locked");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    let (status, refusal) = send(
        &app,
        authed(
            "POST",
            "/characters",
            &token,
            serde_json::json!({ "class": KNIGHT }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(refusal["error"].as_str().unwrap().contains("Warrior"));
    assert!(app.store.characters(account).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn levelling_a_class_unlocks_the_next() {
    // The reason class progress is its own table: the warrior that unlocked the knight may be long
    // dead and deleted, and the unlock has to survive that.
    let app = app_or_skip!("a_unlock");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    let knight = || {
        authed(
            "POST",
            "/characters",
            &token,
            serde_json::json!({ "class": KNIGHT }),
        )
    };
    assert_eq!(send(&app, knight()).await.0, StatusCode::FORBIDDEN);

    // Nineteen is not twenty.
    app.store
        .record_class_progress(account, warrior(&app), 19, 0)
        .await
        .unwrap();
    assert_eq!(send(&app, knight()).await.0, StatusCode::FORBIDDEN);

    app.store
        .record_class_progress(account, warrior(&app), 20, 0)
        .await
        .unwrap();
    assert_eq!(send(&app, knight()).await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_only_ever_rises() {
    // Two characters of one class finishing at once must not let the lower overwrite the higher.
    let app = app_or_skip!("a_progress");

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let account = body["account_id"].as_i64().unwrap();

    app.store
        .record_class_progress(account, warrior(&app), 20, 500)
        .await
        .unwrap();
    app.store
        .record_class_progress(account, warrior(&app), 3, 10)
        .await
        .unwrap();

    let progress = app.store.class_progress(account).await.unwrap();
    assert_eq!(progress.get(&warrior(&app)), Some(&(20, 500)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bought_class_needs_no_levelling() {
    let app = app_or_skip!("a_bought");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();
    let account = body["account_id"].as_i64().unwrap();

    app.store
        .purchase_class(account, class_uuid(&app, KNIGHT))
        .await
        .unwrap();

    let (status, _) = send(
        &app,
        authed(
            "POST",
            "/characters",
            &token,
            serde_json::json!({ "class": KNIGHT }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_a_character_needs_a_token() {
    let app = app_or_skip!("a_create_token");

    let request = Request::builder()
        .method("POST")
        .uri("/characters")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "class": WIZARD }).to_string(),
        ))
        .unwrap();

    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_class_that_does_not_exist_is_refused() {
    let app = app_or_skip!("a_create_nonsense");
    content_or_skip!(app);

    let (_, body) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = body["token"].as_str().unwrap().to_string();

    let (status, _) = send(
        &app,
        authed(
            "POST",
            "/characters",
            &token,
            // A real object, but a sheep rather than a class.
            serde_json::json!({ "class": 0x0500 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_list_needs_no_token() {
    // A client needs somewhere to go before it has anywhere to send a password.
    let mut app = app_or_skip!("a_servers");
    Arc::get_mut(&mut app).unwrap().servers = vec![hendra_app::GameServer {
        name: "EU West".to_string(),
        host: "eu.example.com".to_string(),
        port: 7777,
        region: "Europe".to_string(),
    }];

    let (status, body) = send(&app, get("/servers", None)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["name"], "EU West");
    assert_eq!(body[0]["port"], 7777);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_server_list_is_an_empty_list_rather_than_an_error() {
    // A server with nothing configured should say so plainly, not fail in a way a client has to
    // tell apart from being unreachable.
    let app = app_or_skip!("a_servers_empty");

    let (status, body) = send(&app, get("/servers", None)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().map(Vec::len), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn init_tells_a_client_everything_it_needs_before_it_has_an_account() {
    // One request rather than three, because three round trips is three chances to be halfway
    // configured at a title screen.
    let app = app_or_skip!("a_init");

    let (status, body) = send(&app, get("/init", None)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["protocol"].as_u64().is_some());
    assert!(body["servers"].is_array());
}

/// Asks for a name on behalf of a token.
async fn claim(app: &Arc<App>, token: &str, name: &str) -> StatusCode {
    send(
        app,
        authed("POST", "/name", token, serde_json::json!({ "name": name })),
    )
    .await
    .0
}

/// What every account in a schema holds together, so a rename can be checked against it.
async fn all_fame(app: &Arc<App>) -> i64 {
    sqlx::query_scalar::<_, Option<i64>>("SELECT SUM(fame) FROM account")
        .fetch_one(app.store.pool())
        .await
        .unwrap()
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_can_be_renamed_but_not_to_one_that_is_taken() {
    let app = app_or_skip!("a_rename");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    send(&app, post("/register", credentials("Taken", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();
    let account = mine["account_id"].as_i64().unwrap();

    // `Fesal` is a name somebody chose, so choosing again is the five thousand fame
    // `ChooseNameHandler.cs:63` asks for. Without it there is no rename at any name.
    assert_eq!(
        claim(&app, &token, "Renamed").await,
        StatusCode::PAYMENT_REQUIRED
    );

    app.store
        .credit(account, hendra_store::Currency::Fame, 5000)
        .await
        .unwrap();

    // A name already worn is refused, and refusing costs nothing: the original takes the fame
    // first and then loops forever on the collision, which would leave five thousand fame spent
    // for a name never given.
    let before = all_fame(&app).await;
    assert_eq!(claim(&app, &token, "Taken").await, StatusCode::CONFLICT);
    assert_eq!(all_fame(&app).await, before);

    // A character made under the old name, to be found under the new one.
    let character = app
        .store
        .create_character(account, a_class(), 800)
        .await
        .unwrap();
    assert_eq!(character.name, "Fesal");

    assert_eq!(claim(&app, &token, "Renamed").await, StatusCode::OK);
    assert_eq!(all_fame(&app).await, before - 5000);

    // The character is renamed with the account, because it never had a name to be left holding:
    // `Player.cs:423` reads `client.Account.Name` afresh every time a character is put in a world.
    assert_eq!(
        app.store.character(character.id).await.unwrap().name,
        "Renamed"
    );

    // And the new name is the one that logs in.
    assert_eq!(
        send(&app, post("/login", credentials("Renamed", PASSWORD)))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_that_never_chose_a_name_chooses_one_for_nothing() {
    let app = app_or_skip!("a_firstname");

    // `Darq` is one of the reserved names an unnamed account is given, which is what stands for
    // `Account.NameChosen` being false (`common/Database.cs:82-84`).
    let (_, mine) = send(&app, post("/register", credentials("Darq", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();

    let before = all_fame(&app).await;
    assert_eq!(claim(&app, &token, "Fesal").await, StatusCode::OK);
    assert_eq!(all_fame(&app).await, before);

    // Having chosen once, the next one is paid for.
    assert_eq!(
        claim(&app, &token, "Second").await,
        StatusCode::PAYMENT_REQUIRED
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_must_be_three_to_ten_letters_and_not_a_reserved_one() {
    let app = app_or_skip!("a_namerules");

    let (_, mine) = send(&app, post("/register", credentials("Drol", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();

    // `ChooseNameHandler.cs:38-39`: letters only, three to ten of them, and none of the reserved.
    for refused in ["Ab", "Elevenchars", "Fesal1", "Two Words", "Eango", "eango"] {
        assert_eq!(
            claim(&app, &token, refused).await,
            StatusCode::BAD_REQUEST,
            "{refused:?} should not be a name"
        );
    }

    // The first letter is raised before the rules are read, so a lowercase name is the same name.
    assert_eq!(claim(&app, &token, "fesal").await, StatusCode::OK);
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn friends_appear_on_both_lists_once_both_have_asked() {
    let app = app_or_skip!("a_friends");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let (_, theirs) = send(&app, post("/register", credentials("Someone", PASSWORD))).await;
    let one = mine["token"].as_str().unwrap().to_string();
    let two = theirs["token"].as_str().unwrap().to_string();

    let (status, body) = send(
        &app,
        authed(
            "POST",
            "/friends",
            &one,
            serde_json::json!({ "name": "Someone" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mutual"], false, "asking alone is not a friendship");

    // It shows as a request on the other side before it is answered.
    let (_, waiting) = send(&app, get("/friends", Some(&two))).await;
    assert_eq!(waiting["requests"][0]["name"], "Fesal");

    let (_, answered) = send(
        &app,
        authed(
            "POST",
            "/friends",
            &two,
            serde_json::json!({ "name": "Fesal" }),
        ),
    )
    .await;
    assert_eq!(answered["mutual"], true);

    let (_, list) = send(&app, get("/friends", Some(&one))).await;
    assert_eq!(list["friends"][0]["name"], "Someone");
    assert_eq!(list["friends"][0]["accepted"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn befriending_somebody_who_does_not_exist_says_so_without_saying_who_does() {
    let app = app_or_skip!("a_friends_nobody");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap();

    let (status, _) = send(
        &app,
        authed(
            "POST",
            "/friends",
            token,
            serde_json::json!({ "name": "Nobody" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fame_list_is_a_players_own_characters_best_first() {
    let app = app_or_skip!("a_fame");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap();
    let account = mine["account_id"].as_i64().unwrap();

    for fame in [10, 900, 100] {
        let character = app
            .store
            .create_character(account, a_class(), 800)
            .await
            .unwrap();
        app.store
            .save_character(
                character.id,
                &hendra_store::Saved {
                    hp: 800,
                    mp: 100,
                    max_hp: 800,
                    max_mp: 100,
                    level: 20,
                    experience: 0,
                    fame,
                    stats: [0; 8],
                },
                None,
            )
            .await
            .unwrap();
    }

    let (status, body) = send(&app, get("/fame", Some(token))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body[0]["fame"], 900);
    assert_eq!(body[2]["fame"], 10);

    // All three wear the account's name, because that is the only name a character has.
    for character in body.as_array().unwrap() {
        assert_eq!(character["name"], "Fesal");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_characters_fame_is_served_split_into_what_it_earned_and_what_it_was_paid() {
    // The original serves the base fame, the total, and one `Bonus` element per bonus with its name
    // and its wording (`XmlModels.cs:699-734`); the client adds the bonuses onto the base to arrive
    // at the total (`TotalFame.as:16-30`). Without the elements a death screen has a total and no
    // way to say where it came from.
    let app = app_or_skip!("a_charfame");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let account = mine["account_id"].as_i64().unwrap();

    let character = app
        .store
        .create_character(account, a_class(), 800)
        .await
        .unwrap();

    app.store
        .save_character(
            character.id,
            &hendra_store::Saved {
                hp: 800,
                mp: 100,
                max_hp: 800,
                max_mp: 100,
                level: 20,
                experience: 0,
                fame: 100,
                stats: [0; 8],
            },
            None,
        )
        .await
        .unwrap();

    app.store
        .record_death(
            hendra_store::Death {
                account_id: account,
                character_id: character.id,
                killed_by: "Lava".to_string(),
                final_fame: 178,
                first_born: true,
                bonuses: vec![
                    hendra_store::Awarded {
                        name: "Ancestor".to_string(),
                        fame: 30,
                    },
                    hendra_store::Awarded {
                        name: "Thirsty".to_string(),
                        fame: 32,
                    },
                    hendra_store::Awarded {
                        name: "First Born".to_string(),
                        fame: 16,
                    },
                ],
            },
            None,
        )
        .await
        .unwrap();

    let (status, body) = send_text(
        &app,
        post_form(
            "/char/fame",
            &[
                ("accountId", account.to_string()),
                ("charId", character.id.to_string()),
            ],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<BaseFame>100</BaseFame>"), "{body}");
    assert!(body.contains("<TotalFame>178</TotalFame>"), "{body}");

    // Each bonus carries the wording that goes with its name, which is what the death screen prints
    // beside the number.
    assert!(
        body.contains(
            "<Bonus id=\"Ancestor\" desc=\"one of the first two characters you made\">30</Bonus>"
        ),
        "{body}"
    );
    assert!(
        body.contains("<Bonus id=\"Thirsty\" desc=\"never drank a potion\">32</Bonus>"),
        "{body}"
    );
    assert!(
        body.contains("<Bonus id=\"First Born\" desc=\"your best character yet\">16</Bonus>"),
        "{body}"
    );

    // Nothing appears between the two numbers: the base plus the bonuses served is the total served.
    let awarded: i32 = body
        .split("<Bonus ")
        .skip(1)
        .filter_map(|element| element.split_once('>')?.1.split_once('<'))
        .filter_map(|(fame, _)| fame.parse::<i32>().ok())
        .sum();
    assert_eq!(100 + awarded, 178);
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_are_only_shown_to_who_they_were_sent_to() {
    let app = app_or_skip!("a_messages");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let (_, theirs) = send(&app, post("/register", credentials("Someone", PASSWORD))).await;

    app.store
        .send_message(
            theirs["account_id"].as_i64().unwrap(),
            mine["account_id"].as_i64().unwrap(),
            "are you there",
        )
        .await
        .unwrap();

    let (_, inbox) = send(
        &app,
        get("/messages", Some(mine["token"].as_str().unwrap())),
    )
    .await;
    assert_eq!(inbox[0]["from"], "Someone");
    assert_eq!(inbox[0]["read"], false);

    let (_, empty) = send(
        &app,
        get("/messages", Some(theirs["token"].as_str().unwrap())),
    )
    .await;
    assert_eq!(empty.as_array().map(Vec::len), Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_endpoint_that_should_need_a_token_needs_one() {
    // A route added without its authenticate call would be a route that answers anybody.
    let app = app_or_skip!("a_authwall");

    for (method, path) in [
        ("GET", "/friends"),
        ("GET", "/messages"),
        ("GET", "/fame"),
        ("GET", "/classes"),
        ("GET", "/characters"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            send(&app, request).await.0,
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered without a token"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shared_limit_holds_even_for_a_server_that_has_seen_nothing() {
    // Per-process throttling means two servers behind a load balancer allow twice the attempts.
    // The database is the one thing they share, so a second server must refuse what the first
    // already counted.
    let first = app_or_skip!("a_shared_throttle");
    send(&first, post("/register", credentials("Fesal", PASSWORD))).await;

    for _ in 0..hendra_app::throttle::FAILURES_ALLOWED {
        send(&first, post("/login", credentials("Fesal", "wrong"))).await;
    }

    // A second server, sharing the database and nothing else.
    let second = Arc::new(App::with_content(
        Store::connect(&scoped_url("a_shared_throttle"))
            .await
            .unwrap(),
        key(),
        Arc::new(hendra_content::Catalog::default()),
    ));

    let (status, _) = send(&second, post("/login", credentials("Fesal", PASSWORD))).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "a fresh process must still honour the shared count"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correct_password_clears_the_shared_count_too() {
    let app = app_or_skip!("a_shared_clear");
    send(&app, post("/register", credentials("Fesal", PASSWORD))).await;

    for _ in 0..hendra_app::throttle::FAILURES_ALLOWED - 1 {
        send(&app, post("/login", credentials("Fesal", "wrong"))).await;
    }
    send(&app, post("/login", credentials("Fesal", PASSWORD))).await;

    assert_eq!(
        app.store.recent_failed_logins("Fesal", 300).await.unwrap(),
        0,
        "getting in should forget the misses"
    );
}

/// An app whose mail is captured, so a test can read the link that was sent.
async fn app_with_mail(schema: &str) -> Option<(Arc<App>, Arc<hendra_app::mail::Captured>)> {
    let mut app = Arc::try_unwrap(app(schema).await?).ok()?;
    let mailbox = Arc::new(hendra_app::mail::Captured::default());
    app.mail = mailbox.clone();
    Some((Arc::new(app), mailbox))
}

macro_rules! mailed_or_skip {
    ($schema:literal) => {
        match app_with_mail($schema).await {
            Some(pair) => pair,
            None => {
                eprintln!("skipping: HENDRA_TEST_DATABASE is not set");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_is_confirmed_by_the_code_sent_to_it() {
    let (app, mailbox) = mailed_or_skip!("a_email_verify");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();

    let (status, _) = send(
        &app,
        authed(
            "POST",
            "/email",
            &token,
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let sent = mailbox.sent();
    assert_eq!(sent.len(), 1, "something was sent");
    assert_eq!(sent[0].0, "a@example.com");

    let code = sent[0].2.rsplit(' ').next().unwrap().to_string();
    let (status, _) = send(
        &app,
        post_json("/email/verify", serde_json::json!({ "token": code })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // And the same code does not work twice.
    assert_eq!(
        send(
            &app,
            post_json("/email/verify", serde_json::json!({ "token": code }))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reset_code_changes_the_password_and_is_spent() {
    let (app, mailbox) = mailed_or_skip!("a_email_reset");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();
    let (status, body) = send(
        &app,
        authed(
            "POST",
            "/email",
            &token,
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "setting the address: {body}");

    send(
        &app,
        post_json(
            "/password/forgot",
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;

    let code = mailbox
        .sent()
        .last()
        .unwrap()
        .2
        .rsplit(' ')
        .next()
        .unwrap()
        .to_string();
    let (status, _) = send(
        &app,
        post_json(
            "/password/reset",
            serde_json::json!({ "token": code, "password": "a whole new password" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        send(
            &app,
            post("/login", credentials("Fesal", "a whole new password"))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, post("/login", credentials("Fesal", PASSWORD)))
            .await
            .0,
        StatusCode::UNAUTHORIZED,
        "the old one stopped working"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn asking_to_reset_an_unknown_address_says_nothing_about_it() {
    // Telling a known address from an unknown one turns this into a way to find out which have
    // accounts, which is worse than the confusion of somebody who mistyped their own.
    let (app, mailbox) = mailed_or_skip!("a_email_unknown");

    let (status, _) = send(
        &app,
        post_json(
            "/password/forgot",
            serde_json::json!({ "email": "nobody@example.com" }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "the same answer either way");
    assert!(mailbox.sent().is_empty(), "and nothing was sent");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_the_server_would_refuse_does_not_burn_the_link() {
    // Otherwise one mistyped password costs a player the only link they have.
    let (app, mailbox) = mailed_or_skip!("a_email_short");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();
    send(
        &app,
        authed(
            "POST",
            "/email",
            &token,
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;
    send(
        &app,
        post_json(
            "/password/forgot",
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;

    let code = mailbox
        .sent()
        .last()
        .unwrap()
        .2
        .rsplit(' ')
        .next()
        .unwrap()
        .to_string();

    assert_eq!(
        send(
            &app,
            post_json(
                "/password/reset",
                serde_json::json!({ "token": code, "password": "short" })
            )
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    // The link still works.
    assert_eq!(
        send(
            &app,
            post_json(
                "/password/reset",
                serde_json::json!({ "token": code, "password": "a whole new password" })
            )
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_verify_code_cannot_reset_a_password() {
    // Both reach the same inbox, and one that worked for either would let a confirmation link
    // change a password.
    let (app, mailbox) = mailed_or_skip!("a_email_purpose");

    let (_, mine) = send(&app, post("/register", credentials("Fesal", PASSWORD))).await;
    let token = mine["token"].as_str().unwrap().to_string();
    send(
        &app,
        authed(
            "POST",
            "/email",
            &token,
            serde_json::json!({ "email": "a@example.com" }),
        ),
    )
    .await;

    let code = mailbox
        .sent()
        .last()
        .unwrap()
        .2
        .rsplit(' ')
        .next()
        .unwrap()
        .to_string();

    assert_eq!(
        send(
            &app,
            post_json(
                "/password/reset",
                serde_json::json!({ "token": code, "password": "a whole new password" })
            )
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_content_summary_needs_no_token_and_says_what_the_server_is_running() {
    // A client ships its own content and always has. What it cannot know is whether the server
    // agrees, and a count that differs is the cheapest signal that the two have drifted.
    let app = app_or_skip!("a_content");
    content_or_skip!(app);

    let (status, body) = send(&app, get("/content", None)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["objects"].as_u64().unwrap_or(0) > 1000);
    assert_eq!(body["classes"], 14, "the shipped files have fourteen");
    assert!(
        body["class_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "Wizard")
    );
}

/// A legacy account, made the way the game's clients make one.
///
/// `/account/register` takes an address and a password and answers `<Success />`; everything after
/// it is asked for with that same pair as `guid` and `password`.
async fn legacy_account(app: &Arc<App>, address: &str) -> Vec<(&'static str, String)> {
    let (status, body) = send_text(
        app,
        post_form(
            "/account/register",
            &[
                ("guid", String::new()),
                ("password", String::new()),
                ("newGUID", address.to_string()),
                ("newPassword", PASSWORD.replace(' ', "%20")),
            ],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Success />");

    vec![
        ("guid", address.to_string()),
        ("password", PASSWORD.replace(' ', "%20")),
    ]
}

/// Every endpoint the original's router serves is served here, and none answers a 404.
///
/// A missing route is the one failure a client cannot work around: it reads the body to decide what
/// went wrong, and a 404 from axum has no body it recognises. `RequestHandler.cs:96-137` is the list
/// this checks against.
#[tokio::test(flavor = "multi_thread")]
async fn every_route_the_original_serves_is_served() {
    let app = app_or_skip!("a_routes");

    let routed = [
        "/char/list",
        "/char/delete",
        "/char/fame",
        "/char/purchaseClassUnlock",
        "/account/register",
        "/account/verify",
        "/account/forgotPassword",
        "/account/rp",
        "/account/sendVerifyEmail",
        "/account/changePassword",
        "/account/purchaseCharSlot",
        "/account/setName",
        "/account/verifyage",
        "/account/purchaseSkin",
        "/account/rank",
        "/account/registerDiscord",
        "/account/unregisterDiscord",
        "/credits/getoffers",
        "/credits/add",
        "/fame/list",
        "/picture/get",
        "/app/getLanguageStrings",
        "/app/init",
        "/app/globalNews",
        "/app/getServerXmls",
        "/app/getTextures",
        "/guild/listMembers",
        "/guild/getBoard",
        "/guild/setBoard",
        "/privateMessage/send",
        "/privateMessage/list",
        "/privateMessage/delete",
        "/dailyLogin/fetchCalendar",
        "/weekQuest/getQuests",
        "/inGameNews/getNews",
        "/friends/getList",
        "/friends/getRequests",
    ];

    assert_eq!(
        routed.len(),
        37,
        "the original registers thirty-seven posts"
    );

    for path in routed {
        let (status, _) = send_text(&app, post_form(path, &[])).await;
        assert_ne!(status, StatusCode::NOT_FOUND, "{path} is not served");
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{path} is not served by POST"
        );
    }

    // The one the original also answers on GET, for the link in a reset mail.
    let (status, _) = send_text(&app, get("/account/rp?a=1&b=nope", None)).await;
    assert_eq!(status, StatusCode::OK);
}

/// The endpoints that carry a fixed answer carry the original's, word for word.
///
/// These are the ones a client branches on without ever seeing an account: an offer list it parses,
/// a refusal it shows. A word out of place is a client that does not recognise its own reply.
#[tokio::test(flavor = "multi_thread")]
async fn the_fixed_answers_are_the_originals() {
    let app = app_or_skip!("a_fixed");

    for (path, expected) in [
        ("/credits/add", "<Error>Nope</Error>"),
        ("/account/sendVerifyEmail", "<Error>Nope.</Error>"),
        ("/friends/getList", "<Friends></Friends>"),
        ("/friends/getRequests", "<Requests></Requests>"),
    ] {
        let (status, body) = send_text(&app, post_form(path, &[])).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, expected, "{path}");
    }

    let (_, offers) = send_text(&app, post_form("/credits/getoffers", &[])).await;
    assert_eq!(
        offers,
        "<Offers><Tok>WUT</Tok><Exp>STH</Exp><Offer><Id>0</Id><Price>0</Price><RealmGold>1000\
         </RealmGold><CheckoutJWT>1000</CheckoutJWT><Data>YO</Data><Currency>HKD</Currency>\
         </Offer></Offers>"
    );

    // `FameList.FromDb` reads no rows: the document is one element carrying the timespan asked for,
    // lowercased, and nothing else.
    let (_, fame) = send_text(
        &app,
        post_form("/fame/list", &[("timespan", "WEEK".to_string())]),
    )
    .await;
    assert_eq!(fame, "<FameList timespan=\"week\" />");

    // And with no timespan it throws there, which the router turns into a 500.
    let (status, body) = send_text(&app, post_form("/fame/list", &[])).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "<Error>Internal server error</Error>");
}

/// The static documents are documents, whether or not a deployment shipped the files.
#[tokio::test(flavor = "multi_thread")]
async fn the_static_documents_are_always_parseable() {
    let app = app_or_skip!("a_static");

    let (_, init) = send_text(&app, post_form("/app/init", &[])).await;
    assert!(init.starts_with("<AppSettings>"), "{init}");
    assert!(init.contains("<MaxCharSlot>2</MaxCharSlot>"), "{init}");

    // `app/init.cs:20-30` treats these two as paths and empties them when they do not resolve, so
    // neither ever reaches a client as a filename.
    assert!(!init.contains(".xml</SkinsList>"), "{init}");

    let (_, quests) = send_text(&app, post_form("/weekQuest/getQuests", &[])).await;
    assert!(quests.contains("QuestsResponse"), "{quests}");

    let (_, calendar) = send_text(&app, post_form("/dailyLogin/fetchCalendar", &[])).await;
    assert!(calendar.contains("LoginRewards"), "{calendar}");

    let (_, news) = send_text(&app, post_form("/app/globalNews", &[])).await;
    assert!(news.starts_with('['), "{news}");

    // The texture blob is read as a count and that many named images; an empty pack is a count of
    // zero rather than an empty body, which is what tells the client it asked correctly.
    let response = router(app.clone())
        .oneshot(post_form("/app/getTextures", &[]))
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() >= 4);
    let count = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    assert!(count >= 0, "a count of {count} is not a count");
}

/// The language table arrives as the triples both clients parse.
#[tokio::test(flavor = "multi_thread")]
async fn the_language_table_is_a_list_of_triples() {
    let app = app_or_skip!("a_language");

    app.store
        .set_string("en", "greeting", "hello")
        .await
        .unwrap();

    let (status, body) = send_text(
        &app,
        post_form(
            "/app/getLanguageStrings",
            &[("languageType", "en".to_string())],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let parsed: Vec<Vec<String>> = serde_json::from_str(&body).expect("a list of triples");
    assert!(parsed.contains(&vec![
        "greeting".to_string(),
        "hello".to_string(),
        "en".to_string()
    ]));
}

/// Signing in is refused in the specification's words, and the words differ per endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn the_credential_refusals_are_the_specifications_words() {
    let app = app_or_skip!("a_refusals");
    let account = legacy_account(&app, "namer@example.com").await;

    let mut wrong = account.clone();
    wrong[1] = ("password", "not it".to_string());

    for path in [
        "/account/verify",
        "/char/list",
        "/guild/getBoard",
        "/account/changePassword",
    ] {
        let (status, body) = send_text(&app, post_form(path, &wrong)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, "<Error>Bad Login</Error>", "{path}");
    }

    // An account in no guild is told so in the wording `guild/getBoard.cs:15` uses, which is not
    // the wording an English sentence would use.
    let (_, body) = send_text(&app, post_form("/guild/getBoard", &account)).await;
    assert_eq!(body, "<Error>Not in guild</Error>");

    let (_, body) = send_text(&app, post_form("/guild/listMembers", &account)).await;
    assert_eq!(body, "<Error>Not in guild</Error>");

    // `setBoard` refuses for want of rank rather than for want of a guild, which is a different
    // sentence again.
    let (_, body) = send_text(&app, post_form("/guild/setBoard", &account)).await;
    assert_eq!(body, "<Error>No permission</Error>");

    // Nothing here manages ranks, and an ordinary account is told so.
    let (_, body) = send_text(&app, post_form("/account/rank", &account)).await;
    assert_eq!(body, "<Error>Account not allowed to manage ranks.</Error>");

    let (_, body) = send_text(&app, post_form("/account/registerDiscord", &account)).await;
    assert_eq!(body, "<Error>No permission</Error>");
}

/// A password can be changed and the old one stops working.
#[tokio::test(flavor = "multi_thread")]
async fn changing_a_password_takes_effect_on_the_next_request() {
    let app = app_or_skip!("a_changepass");
    let account = legacy_account(&app, "changer@example.com").await;

    let mut asking = account.clone();
    asking.push(("newPassword", "a whole new password".replace(' ', "%20")));

    let (status, body) = send_text(&app, post_form("/account/changePassword", &asking)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Success />");

    let (_, body) = send_text(&app, post_form("/account/verify", &account)).await;
    assert_eq!(body, "<Error>Bad Login</Error>", "the old one still works");

    let fresh = vec![
        ("guid", "changer@example.com".to_string()),
        ("password", "a whole new password".replace(' ', "%20")),
    ];
    let (_, body) = send_text(&app, post_form("/account/verify", &fresh)).await;
    assert!(body.starts_with("<Account>"), "{body}");
}

/// Naming is free once and costs credits after, as `account/setName.cs:38-40` prices it.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_name_costs_a_thousand_credits() {
    let app = app_or_skip!("a_rename");
    let account = legacy_account(&app, "renamer@example.com").await;

    let mut first = account.clone();
    first.push(("name", "Alphabet".to_string()));
    let (_, body) = send_text(&app, post_form("/account/setName", &first)).await;
    assert_eq!(body, "<Success />", "the first name is free");

    let mut second = account.clone();
    second.push(("name", "Betelgeuse".to_string()));
    let (_, body) = send_text(&app, post_form("/account/setName", &second)).await;
    assert_eq!(body, "<Error>Not enough credits</Error>");

    // With the price in hand it goes through, and the price is gone.
    let id = app.store.account_by_name("Alphabet").await.unwrap().id;
    sqlx::query("UPDATE account SET credits = 1500 WHERE id = $1")
        .bind(id)
        .execute(app.store.pool())
        .await
        .unwrap();

    let (_, body) = send_text(&app, post_form("/account/setName", &second)).await;
    assert_eq!(body, "<Success />");
    assert_eq!(app.store.account(id).await.unwrap().credits, 500);
    assert_eq!(app.store.account(id).await.unwrap().name, "Betelgeuse");
}

/// A character slot is refused for want of fame rather than sold, and nothing is taken either way.
#[tokio::test(flavor = "multi_thread")]
async fn a_character_slot_is_never_charged_for_without_being_given() {
    let app = app_or_skip!("a_charslot");
    let account = legacy_account(&app, "slots@example.com").await;

    let (_, body) = send_text(&app, post_form("/account/purchaseCharSlot", &account)).await;
    assert_eq!(body, "<Error>Insufficient funds</Error>");

    let id = app
        .store
        .account_by_email("slots@example.com")
        .await
        .unwrap();
    app.store
        .set_currency(id, hendra_store::Currency::Fame, 5000)
        .await
        .unwrap();

    let (_, body) = send_text(&app, post_form("/account/purchaseCharSlot", &account)).await;
    assert_eq!(body, "<Error>Internal Server Error</Error>");
    assert_eq!(
        app.store.account(id).await.unwrap().fame,
        5000,
        "a refused purchase costs nothing"
    );
}

/// The mail list is JSON, and empty is the literal the original writes rather than an empty object.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_mailbox_is_the_originals_literal() {
    let app = app_or_skip!("a_pmlist");
    let account = legacy_account(&app, "mail@example.com").await;

    let (status, body) = send_text(&app, post_form("/privateMessage/list", &account)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "{\"messages\":[]}");

    // Credentials that do not check out get the same answer rather than a refusal: `list.cs`
    // catches everything and writes this.
    let mut wrong = account.clone();
    wrong[1] = ("password", "not it".to_string());
    let (_, body) = send_text(&app, post_form("/privateMessage/list", &wrong)).await;
    assert_eq!(body, "{\"messages\":[]}");
}

/// A skin that is not a skin is not sold, whatever number is asked for.
#[tokio::test(flavor = "multi_thread")]
async fn a_number_that_is_not_a_skin_buys_nothing() {
    let app = app_or_skip!("a_skins");
    content_or_skip!(app);
    let account = legacy_account(&app, "dresser@example.com").await;

    let mut asking = account.clone();
    asking.push(("skinType", "0x0300".to_string()));

    // 0x0300 is the wizard, not a skin. `GameData.Skins[skinType]` throws there.
    let (status, _) = send_text(&app, post_form("/account/purchaseSkin", &asking)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let id = app
        .store
        .account_by_email("dresser@example.com")
        .await
        .unwrap();
    assert!(app.store.owned_skins(id).await.unwrap().is_empty());
}

/// Deleting a character says it did, and the character is gone.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_character_removes_it() {
    let app = app_or_skip!("a_chardelete");
    let account = legacy_account(&app, "deleter@example.com").await;
    let id = app
        .store
        .account_by_email("deleter@example.com")
        .await
        .unwrap();

    let character = app
        .store
        .create_character(id, a_class(), 100)
        .await
        .unwrap();

    let mut asking = account.clone();
    asking.push(("charId", character.id.to_string()));

    let (status, body) = send_text(&app, post_form("/char/delete", &asking)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Success />");
    assert!(app.store.characters(id).await.unwrap().is_empty());

    // Asked again, it still says it did: the caller wanted it gone and it is.
    let (_, body) = send_text(&app, post_form("/char/delete", &asking)).await;
    assert_eq!(body, "<Success />");

    // An id that is not a number throws there rather than being read as zero.
    let mut nonsense = account.clone();
    nonsense.push(("charId", "seventeen".to_string()));
    let (status, _) = send_text(&app, post_form("/char/delete", &nonsense)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // But a wrong password is refused as one first: the parse sits inside the branch the login
    // check passes into, so it is never reached by somebody who cannot sign in.
    let mut stranger = nonsense.clone();
    stranger[1] = ("password", "not it".to_string());
    let (status, body) = send_text(&app, post_form("/char/delete", &stranger)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Error>Bad Login</Error>");
}

/// A picture that is not held is a 404, and an id with too many colons is refused.
#[tokio::test(flavor = "multi_thread")]
async fn an_unheld_picture_is_a_not_found() {
    let app = app_or_skip!("a_picture");

    let (status, _) = send_text(
        &app,
        post_form("/picture/get", &[("id", "nothing".to_string())]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = send_text(
        &app,
        post_form("/picture/get", &[("id", "a:b:c".to_string())]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Error>Invalid input</Error>");
}

/// An address nobody registered and one that is not an address get the same answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_forgotten_password_never_says_whether_the_address_is_known() {
    let app = app_or_skip!("a_forgot");
    legacy_account(&app, "forgetful@example.com").await;

    for asked in ["nobody@example.com", "notanaddress"] {
        let (_, body) = send_text(
            &app,
            post_form("/account/forgotPassword", &[("guid", asked.to_string())]),
        )
        .await;
        assert_eq!(body, "<Error>Email not recognized</Error>", "{asked}");
    }

    let (status, body) = send_text(
        &app,
        post_form(
            "/account/forgotPassword",
            &[("guid", "forgetful@example.com".to_string())],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<Success />");
}

/// A reset link with a token nobody issued shows the error page rather than a new password.
#[tokio::test(flavor = "multi_thread")]
async fn a_reset_link_with_a_bad_token_changes_nothing() {
    let app = app_or_skip!("a_resetlink");
    let account = legacy_account(&app, "resetter@example.com").await;

    let (status, body) = send_text(&app, get("/account/rp?a=1&b=nonsense", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Unable to Reset Password"), "{body}");

    // And the password it had still works.
    let (_, body) = send_text(&app, post_form("/account/verify", &account)).await;
    assert!(body.starts_with("<Account>"), "{body}");
}

/// A class unlock is charged for once and granted once.
///
/// `purchaseClassUnlock.cs:15-34` is four answers in order: a number that names no class throws
/// where the dictionary is indexed, a class whose `Unlock.Cost` is absent is "Bad input to
/// character unlock", one the account cannot afford is "Not enough gold", and anything else is
/// paid for and granted. A cost of zero is not an absent cost, so the wizard -- the one class the
/// files price at nothing -- is bought for nothing rather than refused. The reference agrees on
/// every one of those: it answers `<Success />` to `classType=0x030e` for an account holding no
/// gold, and 500 to `classType=0x0400`.
///
/// One rule here is deliberately not the original's. The original never asks whether the class is
/// already unlocked, so it charges again every time: on the reference, buying the archer twice with
/// 5000 gold left 4801 and then 4602. The answer it gives is `<Success />` both times, which is all
/// the client reads, and that is what is kept -- the second charge is not.
#[tokio::test(flavor = "multi_thread")]
async fn a_class_unlock_is_paid_for_once() {
    let app = app_or_skip!("a_unlock");
    content_or_skip!(app);
    let account = legacy_account(&app, "unlocker@example.com").await;
    let id = app
        .store
        .account_by_email("unlocker@example.com")
        .await
        .unwrap();

    // The archer, whose unlock the shipped content prices.
    let mut asking = account.clone();
    asking.push(("classType", "0x0307".to_string()));

    let (_, body) = send_text(&app, post_form("/char/purchaseClassUnlock", &asking)).await;
    assert_eq!(
        body, "<Error>Not enough gold</Error>",
        "with nothing to spend"
    );
    assert!(app.store.purchased_classes(id).await.unwrap().is_empty());

    // The wizard, priced at nothing, bought by the same account that cannot afford the archer.
    let mut free = account.clone();
    free.push(("classType", "0x030e".to_string()));
    let (_, body) = send_text(&app, post_form("/char/purchaseClassUnlock", &free)).await;
    assert_eq!(body, "<Success />", "a class priced at nothing is free");
    assert_eq!(app.store.account(id).await.unwrap().credits, 0);
    assert_eq!(app.store.purchased_classes(id).await.unwrap().len(), 1);

    sqlx::query("UPDATE account SET credits = 5000 WHERE id = $1")
        .bind(id)
        .execute(app.store.pool())
        .await
        .unwrap();

    let (_, body) = send_text(&app, post_form("/char/purchaseClassUnlock", &asking)).await;
    assert_eq!(body, "<Success />");
    let paid = app.store.account(id).await.unwrap().credits;
    assert_eq!(paid, 5000 - 199, "the archer's price, from the class files");
    assert_eq!(app.store.purchased_classes(id).await.unwrap().len(), 2);

    // Asked again it still says yes, and costs nothing the second time.
    let (_, body) = send_text(&app, post_form("/char/purchaseClassUnlock", &asking)).await;
    assert_eq!(body, "<Success />");
    assert_eq!(app.store.account(id).await.unwrap().credits, paid);
    assert_eq!(app.store.purchased_classes(id).await.unwrap().len(), 2);

    // The number is cast to `ushort` before it is looked up, so a wider one names the class its low
    // half names: `0x1030e` is the wizard. The reference answers `<Success />` to it.
    let mut wide = account.clone();
    wide.push(("classType", "0x1030e".to_string()));
    let (_, body) = send_text(&app, post_form("/char/purchaseClassUnlock", &wide)).await;
    assert_eq!(body, "<Success />", "the low half of the number is the class");
    assert_eq!(app.store.purchased_classes(id).await.unwrap().len(), 2);
    assert_eq!(app.store.account(id).await.unwrap().credits, paid);

    // A number that is no class at all throws there rather than being refused in words.
    let mut nonsense = account.clone();
    nonsense.push(("classType", "0x0400".to_string()));
    let (status, _) = send_text(&app, post_form("/char/purchaseClassUnlock", &nonsense)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // Only a lowercase `0x` marks a hexadecimal number. `Int32.Parse("0X030E")` throws, which
    // `Utils.FromString` swallows as zero, and zero is no class -- the reference throws here too.
    let mut shouting = account.clone();
    shouting.push(("classType", "0X030E".to_string()));
    let (status, _) = send_text(&app, post_form("/char/purchaseClassUnlock", &shouting)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// The character list is the document `char/list.cs` returns, element for element.
///
/// Pinned against what a built copy of the original actually answered rather than against a
/// reading of `XmlModels.cs`, because the two disagree in places: the reference's own reply for a
/// freshly registered account is
///
/// ```text
/// <Chars nextCharId="0" maxNumChars="2">
///   <Account>
///     <AccountId>19</AccountId><Name>Oshyu</Name><Rank>0</Rank>
///     <LastSeen>1786778040</LastSeen><IsAgeVerified>1</IsAgeVerified>
///     <isFirstDeath></isFirstDeath><Credits>0</Credits>
///     <NextCharSlotPrice>550</NextCharSlotPrice><CharSlotCurrency>1</CharSlotCurrency>
///     <MenuMusic></MenuMusic><DeadMusic></DeadMusic><Vault />
///     <Stats>...14 ClassStats...<BestCharFame>0</BestCharFame>
///          <TotalFame>0</TotalFame><Fame>0</Fame></Stats>
///     <Guild id="0"><Name /><Rank>0</Rank></Guild>
///   </Account>
///   <ClassAvailabilityList>...</ClassAvailabilityList><News /><Servers>...</Servers>
///   <ItemCosts>...191...</ItemCosts><MaxClassLevelList>...14...</MaxClassLevelList>
/// </Chars>
/// ```
///
/// Note what is *not* there: no `<Fame>` directly under `<Account>`, which is where this server
/// used to write it, and `<Rank>` before `<LastSeen>` before `<VerifiedEmail>`.
///
/// `<Vault>` reports one chest fewer than the account owns, which is the reference's own
/// arithmetic: `Enumerable.Range(0, acc.VaultCount - 1)` (`XmlModels.cs:252`) passes a count where
/// it reads as an end, while its world server stands up all `VaultCount` of them
/// (`wServer/realm/worlds/logic/Vault.cs:96`). Kept, and confirmed live against the pristine app
/// server: `vaultCount` 1 answers `<Vault />`, `vaultCount` 3 answers two `<Chest>` elements.
///
/// One value is deliberately not the reference's: `<ClassStats>` and `<ClassAvailability>` follow
/// this server's class unlocks rather than the reference's `<ClassesUnlocked>1</ClassesUnlocked>`,
/// which hands every class to every new account.
#[tokio::test(flavor = "multi_thread")]
async fn the_character_list_is_the_specifications_document() {
    let app = app_or_skip!("a_charlist");
    let account = legacy_account(&app, "charlist@example.com").await;

    let (status, body) = send_text(&app, post_form("/char/list", &account)).await;
    assert_eq!(status, StatusCode::OK);

    let id = app.store.account_by_email("charlist@example.com").await.unwrap();
    let name = app.store.account(id).await.unwrap().name;
    let seen = app.store.last_seen(id).await.unwrap();

    // A class the account may play and one it may not, so the two words are both exercised.
    let unrestricted: Vec<&str> = ["Wizard"].into();

    let mut expected = format!(
        "<Chars nextCharId=\"0\" maxNumChars=\"2\">\
         <Account><AccountId>{id}</AccountId><Name>{name}</Name>\
         <Rank>0</Rank><LastSeen>{seen}</LastSeen>\
         <IsAgeVerified>1</IsAgeVerified><isFirstDeath></isFirstDeath>\
         <Credits>0</Credits><NextCharSlotPrice>550</NextCharSlotPrice>\
         <CharSlotCurrency>1</CharSlotCurrency><MenuMusic></MenuMusic><DeadMusic></DeadMusic>"
    );

    // One fewer than the account owns, and self-closing when that leaves none -- the original's
    // `Enumerable.Range(0, acc.VaultCount - 1)` (`XmlModels.cs:252`). A new account owns one, so a
    // new account's vault is written `<Vault />`, which is what the pristine app server answers.
    let chests = app.store.account(id).await.unwrap().vault_chests - 1;
    if chests == 0 {
        expected.push_str("<Vault />");
    } else {
        expected.push_str("<Vault>");
        for _ in 0..chests {
            expected.push_str("<Chest>-1, -1, -1, -1, -1, -1, -1, -1</Chest>");
        }
        expected.push_str("</Vault>");
    }
    expected.push_str("<Stats>");

    for class in app.catalog.classes() {
        let object_id = &app.catalog.object(class.object_type).unwrap().id;
        if unrestricted.contains(&object_id.as_str()) {
            expected.push_str(&format!(
                "<ClassStats objectType=\"0x{:04x}\"><BestLevel>0</BestLevel>\
                 <BestFame>0</BestFame></ClassStats>",
                class.object_type.0
            ));
        }
    }

    expected.push_str(
        "<BestCharFame>0</BestCharFame><TotalFame>0</TotalFame><Fame>0</Fame></Stats>\
         <Guild id=\"0\"><Name /><Rank>0</Rank></Guild></Account><ClassAvailabilityList>",
    );

    for class in app.catalog.classes() {
        let object_id = &app.catalog.object(class.object_type).unwrap().id;
        let availability = if unrestricted.contains(&object_id.as_str()) {
            "unrestricted"
        } else {
            "available"
        };
        expected.push_str(&format!(
            "<ClassAvailability id=\"{object_id}\">{availability}</ClassAvailability>"
        ));
    }

    expected.push_str("</ClassAvailabilityList><News /><Servers></Servers><ItemCosts>");

    for skin in app.catalog.skins() {
        expected.push_str(&format!(
            "<ItemCost type=\"{}\" expires=\"0\" purchasable=\"1\">1000</ItemCost>",
            skin.object_type.0
        ));
    }

    expected.push_str("</ItemCosts><MaxClassLevelList>");
    for class in app.catalog.classes() {
        expected.push_str(&format!(
            "<MaxClassLevel maxLevel=\"0\" classType=\"{}\" />",
            class.object_type.0
        ));
    }
    expected.push_str("</MaxClassLevelList></Chars>");

    assert_eq!(body, expected);

    // The blocks the reference carries are the sizes the reference carries them at: fourteen
    // classes twice over, and the 191 skins in `EmbeddedData_SkinsCXML`.
    assert_eq!(body.matches("<ClassAvailability ").count(), 14);
    assert_eq!(body.matches("<MaxClassLevel ").count(), 14);
    assert_eq!(body.matches("<ItemCost ").count(), 191);

    // The account block no longer carries a bare `<Fame>`; the original keeps it inside `<Stats>`,
    // which is where the AS3 client reads it from (`SavedCharactersList.as:115`).
    let account_block = body.split("</Account>").next().unwrap();
    assert!(!account_block.contains("<Fame>0</Fame><NextCharSlotPrice>"), "{account_block}");
}

/// Progress, deaths and a guild all reach the character list, and from the sources that record
/// them rather than from a second tally kept for the screen.
///
/// The reference answers the same shapes for the same facts: seeding its redis with a levelled
/// class, a guild and a gravestone moved `<BestLevel>`, `<MaxClassLevel maxLevel>`, `<Guild id>`
/// and one `<Item>` in `<News>`, and left everything else where it was.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_character_earned_reaches_the_character_list() {
    let app = app_or_skip!("a_charlist_full");
    let account = legacy_account(&app, "veteran@example.com").await;
    let id = app.store.account_by_email("veteran@example.com").await.unwrap();

    // A warrior taken to twenty, which is what the knight is locked behind.
    app.store
        .record_class_progress(id, warrior(&app), 20, 640)
        .await
        .unwrap();

    let (_, body) = send_text(&app, post_form("/char/list", &account)).await;

    // The picker and the level list agree, because both read the same progress.
    assert!(body.contains("<ClassAvailability id=\"Knight\">unrestricted</ClassAvailability>"), "{body}");
    assert!(body.contains("<MaxClassLevel maxLevel=\"20\" classType=\"797\" />"), "{body}");
    assert!(
        body.contains("<ClassStats objectType=\"0x031d\"><BestLevel>20</BestLevel><BestFame>640</BestFame></ClassStats>"),
        "{body}"
    );
    assert!(body.contains("<BestCharFame>640</BestCharFame>"), "{body}");

    // What the class endpoint offers is what the character list calls unrestricted: one answer,
    // not two.
    let offers = hendra_characters::offers(&app.store, &app.catalog, id).await.unwrap();
    for offer in &offers {
        let expected = if offer.locked.is_none() {
            "unrestricted"
        } else {
            "available"
        };
        assert!(
            body.contains(&format!(
                "<ClassAvailability id=\"{}\">{expected}</ClassAvailability>",
                offer.id
            )),
            "{} disagrees with /classes",
            offer.id
        );
    }

    // A gravestone becomes news in the original's exact words, and clears `isFirstDeath`.
    assert!(body.contains("<isFirstDeath></isFirstDeath>"), "nothing has died yet");

    let character = hendra_characters::create(
        &app.store,
        &app.catalog,
        id,
        hendra_content::ObjectType(0x030e),
    )
    .await
    .unwrap();

    app.store
        .record_death(
            hendra_store::Death {
                account_id: id,
                character_id: character.id,
                killed_by: "a Sprite God".to_string(),
                final_fame: 1200,
                first_born: true,
                bonuses: Vec::new(),
            },
            None,
        )
        .await
        .unwrap();

    let (_, body) = send_text(&app, post_form("/char/list", &account)).await;
    assert!(!body.contains("<isFirstDeath>"), "the first death has happened");
    assert!(
        body.contains("<Icon>fame</Icon><Title>Your Wizard died at level 1</Title>\
                       <TagLine>You earned 1200 glorious Fame</TagLine>"),
        "{body}"
    );
    assert!(body.contains(&format!("<Link>fame:{}</Link>", character.id)), "{body}");

    // The counter only goes up: a character that has died still counts against the next id.
    assert!(body.starts_with("<Chars nextCharId=\"1\""), "{body}");
}
