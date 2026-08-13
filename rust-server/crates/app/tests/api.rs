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
    Some(Arc::new(App { store, key: key() }))
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
    // The failure this guards against is two deployments configured with different secrets, which
    // otherwise shows up as players who log in successfully and are then refused by the game.
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
        .create_character(account, 0x0300, "Wizard", 800)
        .await
        .unwrap();
    app.store
        .create_character(other, 0x0301, "Archer", 800)
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
    // Argon2 is deliberately slow, so an unauthenticated endpoint that hashes whatever it is sent
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
