//! Starting the app server.
//!
//! Everything the server *does* lives in the library beside this; this is the part that reads the
//! environment and decides whether it is safe to listen at all.

use std::net::SocketAddr;
use std::sync::Arc;

use hendra_auth::TokenKey;
use hendra_store::Store;

/// The resource folder this server reads when `HENDRA_CONTENT` names none.
///
/// `rust-server/content`, which is this project's own copy of the game content. The specification
/// in `Server-Side/` is what the parity tests measure us against and is never read while running.
const DEFAULT_CONTENT: &str = "content";

/// Where the object definitions are inside a resource folder.
///
/// A resource folder keeps the documents in `data/` and the definitions in `xmls/`, which is the
/// layout `rust-server/content` and the original's `XmlDatas` both have, so one directory names
/// everything this server serves. A folder holding the definitions directly is read as it stands,
/// which is what naming a bare directory of XML still means.
fn definitions_in(content: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(content);
    let xmls = root.join("xmls");

    if xmls.is_dir() { xmls } else { root }
}

/// Where the game servers are, read from the environment.
///
/// `name|host|port|region`, separated by commas:
///
/// ```text
/// HENDRA_GAME_SERVERS="EU West|eu.example.com|7777|Europe,US East|us.example.com|7777|America"
/// ```
///
/// Configuration rather than a compiled-in list, so moving a server does not need a new client.
/// An entry that does not parse is skipped and reported rather than refusing the rest: one typo
/// should cost that server, not every server.
fn game_servers() -> Vec<hendra_app::GameServer> {
    let Ok(written) = std::env::var("HENDRA_GAME_SERVERS") else {
        return Vec::new();
    };

    written
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split('|').map(str::trim).collect();
            let [name, host, port, region] = parts.as_slice() else {
                tracing::warn!(%entry, "a game server entry needs name|host|port|region");
                return None;
            };

            let Ok(port) = port.parse::<u16>() else {
                tracing::warn!(%entry, "a game server entry has an unreadable port");
                return None;
            };

            Some(hendra_app::GameServer {
                name: name.to_string(),
                host: host.to_string(),
                port,
                region: region.to_string(),
            })
        })
        .collect()
}

/// Where to listen, from `--address`, `--port`, `HENDRA_APP_ADDRESS`, or the default in that order.
///
/// An argument this does not recognise is an error rather than something to step over. Several
/// processes of this server run side by side during development, told apart only by their port, and
/// a flag that is accepted and ignored puts them all on the same one: the first wins the bind and
/// the rest exit, while every launch command still looks right.
fn listen_address() -> Result<SocketAddr, String> {
    let mut address = std::env::var("HENDRA_APP_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let mut port: Option<u16> = None;
    let mut args = std::env::args().skip(1);

    while let Some(argument) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value after it"))
        };

        match argument.as_str() {
            "--address" => address = value("--address")?,
            "--port" => {
                port = Some(
                    value("--port")?
                        .parse()
                        .map_err(|_| "--port needs a number from 1 to 65535".to_string())?,
                );
            }
            other => {
                return Err(format!(
                    "unrecognised argument {other}. This server takes --address and --port, and \
                     reads HENDRA_APP_ADDRESS, HENDRA_DATABASE, HENDRA_TOKEN_KEY, HENDRA_CONTENT \
                     and HENDRA_GAME_SERVERS from the environment."
                ));
            }
        }
    }

    let mut address: SocketAddr = address
        .parse()
        .map_err(|_| format!("{address} is not a host:port to listen on"))?;

    if let Some(port) = port {
        address.set_port(port);
    }

    Ok(address)
}

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

    // The signing key is shared with the game server. Inventing one here would be worse than
    // refusing, because a
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

    let address = match listen_address() {
        Ok(address) => address,
        Err(err) => {
            tracing::error!("{err}");
            std::process::exit(1);
        }
    };

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
    let content =
        std::env::var("HENDRA_CONTENT").unwrap_or_else(|_| DEFAULT_CONTENT.to_string());
    let catalog = match hendra_content::Catalog::load_dir(&definitions_in(&content)) {
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

    let mut app = hendra_app::App::with_content(store, key, Arc::new(catalog));
    app.servers = game_servers();
    app.content = std::path::PathBuf::from(&content);

    // The documents and artwork the app-engine endpoints hand straight back. Read once here rather
    // than per request, as the original reads them in `InitHandler`, so a missing file is known at
    // boot instead of the first time somebody asks for it.
    app.files = hendra_app::AppFiles::load(&app.content);
    tracing::info!(
        textures = app.files.texture_index.len(),
        languages = app.files.languages.len(),
        "app-engine files loaded"
    );

    // The texture pack is one of the few files a client fetches rather than reads a summary of, and
    // a client that cannot get it falls back to the copy it shipped with. Saying so at boot beats
    // finding out from a support message.
    if !app.content.join(hendra_app::TEXTURE_PACK).exists() {
        tracing::warn!(
            path = %app.content.join(hendra_app::TEXTURE_PACK).display(),
            "no texture pack, so clients will use whatever they shipped with"
        );
    }

    // Delivery is the one part of the flow that leaves the process. Without somewhere to send to,
    // links are logged rather than sent, which is useful on a laptop and wrong in production, so
    // it says so.
    if !app.mail.delivers() {
        tracing::warn!(
            "no mail sender is configured, so verification and reset links will be logged \
             rather than sent"
        );
    }

    if app.servers.is_empty() {
        tracing::warn!(
            "HENDRA_GAME_SERVERS is not set, so /servers is empty and a client has nowhere to go"
        );
    }

    let app = Arc::new(app);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// This server serves our content, not the specification.
    ///
    /// `Server-Side/` and `Client-Side/` are what the parity tests measure us against. A default
    /// reaching into either makes the record of what the original shipped into our content source
    /// as well, and then changing what we serve means editing the record.
    #[test]
    fn the_default_content_directory_is_not_the_specification() {
        for reference in ["Server-Side", "Client-Side"] {
            assert!(
                !DEFAULT_CONTENT.contains(reference),
                "the default content directory is {DEFAULT_CONTENT}, which reads the {reference} \
                 specification at runtime"
            );
        }
    }

    /// An app server started with no environment beyond its secrets finds everything it serves.
    #[test]
    fn the_default_content_directory_holds_what_is_served() {
        let root = workspace_root().join(DEFAULT_CONTENT);
        let definitions = definitions_in(&root.display().to_string());

        assert!(
            definitions.ends_with("xmls"),
            "the definitions were looked for in {}, not the resource folder's xmls",
            definitions.display()
        );
        assert!(
            definitions.join("EmbeddedData_PlayersCXML.dat").is_file(),
            "no player definitions in {}, so nobody could make a character",
            definitions.display()
        );
        assert!(
            root.join("data").join("init.xml").is_file(),
            "no data/init.xml in {}, so the app settings would be the built-in fallback",
            root.display()
        );
    }

    /// A directory of definitions with nothing around it is still read as one.
    ///
    /// Naming a bare folder of XML is what a test fixture and a partial deployment both do, and
    /// neither has a `data/` beside it to make it look like a resource folder.
    #[test]
    fn a_bare_directory_of_definitions_is_read_where_it_stands() {
        let bare = workspace_root().join("content/xmls");
        assert_eq!(definitions_in(&bare.display().to_string()), bare);
    }
}
