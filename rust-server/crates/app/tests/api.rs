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
        hendra_characters::CommonItems::new(["Health Potion"]),
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
        .create_character(account, a_class(), "Wizard", 800)
        .await
        .unwrap();
    app.store
        .create_character(other, a_class(), "Archer", 800)
        .await
        .unwrap();

    let (status, body) = send(
        &app,
        get("/characters", Some(mine["token"].as_str().unwrap())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["name"], "Wizard");
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
        .create_character(account, a_class(), "Wizard", 800)
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
        .create_character(
            theirs["account_id"].as_i64().unwrap(),
            a_class(),
            "Theirs",
            800,
        )
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
        .create_character(account, a_class(), "Wizard", 800)
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
        .create_character(other_account, a_class(), "Theirs", 800)
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
            serde_json::json!({ "class": WIZARD, "name": "Merlin" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(made["name"], "Merlin");

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

    assert_eq!(named(0).as_deref(), Some("Energy Staff"));
    assert_eq!(named(1).as_deref(), Some("Fire Spray Spell"));
    assert_eq!(named(2).as_deref(), Some("Robe of the Neophyte"));
    assert_eq!(named(4).as_deref(), Some("Health Potion"), "the common kit");
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
