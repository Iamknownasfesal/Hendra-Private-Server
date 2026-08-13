//! Starting the app server.
//!
//! Everything the server *does* lives in the library beside this; this is the part that reads the
//! environment and decides whether it is safe to listen at all.

use std::net::SocketAddr;
use std::sync::Arc;

use hendra_auth::TokenKey;
use hendra_store::Store;

/// What every class carries on top of the gear its own slots decide.
///
/// Deliberately the same list the game server uses: a character made through the front door and
/// one made on first connection should arrive with the same things.
const COMMON_ITEMS: &[&str] = &["Health Potion", "Magic Potion"];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hendra_app=info,tower_http=warn".into()),
        )
        .init();

    let database = std::env::var("HENDRA_DATABASE")
        .unwrap_or_else(|_| "postgres://localhost/hendra".to_string());

    // The signing key is shared with the game server. Refusing to invent one is deliberate: a
    // generated default would work perfectly until the two processes were restarted separately and
    // every token silently stopped verifying.
    let secret = match std::env::var("HENDRA_TOKEN_KEY") {
        Ok(secret) => secret,
        Err(_) => {
            tracing::error!(
                "HENDRA_TOKEN_KEY is not set. It must be at least 32 bytes and identical to the \
                 game server's, or the tokens this mints will not verify there."
            );
            std::process::exit(1);
        }
    };

    let key = match TokenKey::new(secret.into_bytes()) {
        Ok(key) => key,
        Err(err) => {
            tracing::error!(%err, "HENDRA_TOKEN_KEY is not usable");
            std::process::exit(1);
        }
    };

    let store = match Store::connect(&database).await {
        Ok(store) => store,
        Err(err) => {
            tracing::error!(%err, %database, "cannot reach the database");
            std::process::exit(1);
        }
    };

    let address: SocketAddr = std::env::var("HENDRA_APP_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()
        .expect("a valid address");

    // Passwords cross this connection, so a public bind without something terminating TLS in front
    // is a mistake worth refusing rather than warning about.
    if !address.ip().is_loopback() && std::env::var("HENDRA_BEHIND_PROXY").is_err() {
        tracing::error!(
            %address,
            "refusing to serve passwords unencrypted on a public address. Put a TLS-terminating \
             proxy in front and set HENDRA_BEHIND_PROXY=1, or bind to loopback."
        );
        std::process::exit(1);
    }

    // The content, for the character-select screen. Loaded here rather than reached for lazily so
    // a directory that is missing or broken is a startup failure rather than a request that fails
    // once someone tries to make a character.
    let content = std::env::var("HENDRA_CONTENT").unwrap_or_else(|_| "content".to_string());
    let catalog = match hendra_content::Catalog::load_dir(std::path::Path::new(&content)) {
        Ok((catalog, report)) => {
            tracing::info!(
                classes = report.classes,
                objects = report.objects,
                problems = report.problems.len(),
                "content loaded"
            );
            if report.classes == 0 {
                tracing::warn!("the content has no playable classes; nobody can make a character");
            }
            catalog
        }
        Err(err) => {
            tracing::error!(%err, %content, "cannot read the content directory");
            std::process::exit(1);
        }
    };

    let app = Arc::new(hendra_app::App::with_content(
        store,
        key,
        Arc::new(catalog),
        hendra_characters::CommonItems::new(COMMON_ITEMS.iter().copied()),
    ));

    let router = hendra_app::router(app);

    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(%err, %address, "cannot bind");
            std::process::exit(1);
        }
    };

    tracing::info!(%address, "app server listening");
    if let Err(err) = axum::serve(listener, router).await {
        tracing::error!(%err, "the app server stopped");
    }
}
