//! The Hendra game server.
//!
//! Loads content, builds a world from a map, and serves it over QUIC.
//!
//! ```text
//!   hendra-server [--map Nexus.jm] [--port 2050] [--content DIR] [--worlds DIR]
//!                 [--cert fullchain.pem --key privkey.pem]
//! ```
//!
//! Without a certificate it generates a self-signed one and says so. That pairs only with a client
//! willing to accept any certificate, which is fine on a development machine and is not what a
//! public server should run.

mod accounts;
mod chat;
mod session;
mod world_task;
mod worlds;

use std::path::PathBuf;
use std::sync::Arc;

use hendra_content::Catalog;
use hendra_sim::{Terrain, World};
use hendra_transport::{Listener, ServerIdentity};

/// What a player arrives holding, until character classes decide it.
/// What every class carries on top of the gear its own slots decide.
const COMMON_ITEMS: &[&str] = &["Health Potion", "Magic Potion"];

/// Where the durable half lives.
const DEFAULT_DATABASE: &str = "postgres://localhost/hendra";

struct Options {
    port: u16,
    map: String,
    content: PathBuf,
    worlds: PathBuf,
    certificate: Option<(PathBuf, PathBuf)>,
    database: String,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            port: 2050,
            map: "Nexus.jm".into(),
            content: PathBuf::from("../Server-Side/XmlDatas/xmls/client"),
            worlds: PathBuf::from("../Server-Side/XmlDatas/worlds"),
            certificate: None,
            database: std::env::var("HENDRA_DATABASE")
                .unwrap_or_else(|_| DEFAULT_DATABASE.to_string()),
        }
    }
}

fn parse_options() -> Options {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    let (mut cert, mut key) = (None, None);

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--port" => options.port = args.next().and_then(|v| v.parse().ok()).unwrap_or(2050),
            "--map" => options.map = args.next().unwrap_or(options.map),
            "--content" => {
                options.content = args.next().map(PathBuf::from).unwrap_or(options.content)
            }
            "--worlds" => options.worlds = args.next().map(PathBuf::from).unwrap_or(options.worlds),
            "--cert" => cert = args.next().map(PathBuf::from),
            "--key" => key = args.next().map(PathBuf::from),
            "--database" => options.database = args.next().unwrap_or(options.database),
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }

    if let (Some(cert), Some(key)) = (cert, key) {
        options.certificate = Some((cert, key));
    }
    options
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hendra_server=info,hendra_transport=warn".into()),
        )
        .init();

    let options = parse_options();

    let (catalog, report) = match Catalog::load_dir(&options.content) {
        Ok(loaded) => loaded,
        Err(err) => {
            tracing::error!(path = %options.content.display(), %err, "cannot read content");
            std::process::exit(1);
        }
    };
    tracing::info!(
        objects = report.objects,
        tiles = report.tiles,
        problems = report.problems.len(),
        "content loaded"
    );

    let catalog = Arc::new(catalog);

    // The class a new character is made as, and the one a broken one falls back to. The first free
    // class in the files rather than a name written here, so content decides it. In the shipped
    // files that is the wizard, which is what the game has always started people on.
    let default_class = catalog
        .classes()
        .iter()
        .find(|class| class.unlock.free())
        .or_else(|| catalog.classes().first())
        .cloned();

    let Some(default_class) = default_class else {
        tracing::error!("the content has no playable classes; nobody could make a character");
        std::process::exit(1);
    };

    tracing::info!(
        classes = catalog.classes().len(),
        default = catalog
            .object(default_class.object_type)
            .map(|desc| desc.id.as_str())
            .unwrap_or("?"),
        "classes loaded"
    );

    // The content numbers loot colours from zero upward and names the bags in the same order.
    // Anything the catalog does not have leaves a hole, which falls back to the plain bag rather
    // than dropping nothing.
    let bag_types: Vec<hendra_content::ObjectType> = std::iter::once("Loot Bag".to_string())
        .chain((2..=9).map(|colour| format!("Loot Bag {colour}")))
        .map(|name| {
            catalog
                .type_of(&name)
                .unwrap_or(hendra_content::ObjectType::NONE)
        })
        .collect();

    // What may live outdoors, read from the content: every enemy that names a terrain.
    let spawnable = hendra_sim::realm::spawnable(&catalog);
    tracing::info!(kinds = spawnable.len(), "enemies that can populate a realm");

    let loadout = world_task::Loadout {
        bag_types,
        spawnable,
        is_realm: false,
        persistent: false,
        avatar: default_class.object_type,
        weapon: default_class
            .slot_type(0)
            .and_then(|slot| catalog.lowest_tier_for_slot(slot)),
    };

    let store = match hendra_store::Store::connect(&options.database).await {
        Ok(store) => {
            tracing::info!("connected to the database");
            store
        }
        Err(err) => {
            tracing::error!(%err, database = %options.database, "cannot reach the database");
            std::process::exit(1);
        }
    };

    // Shared with the app server, which mints the tokens this verifies. Refusing to invent one is
    // refused rather than generated: a default works until the two processes restart separately, at which
    // point every token silently stops verifying and nobody can log in for no visible reason.
    let key = match std::env::var("HENDRA_TOKEN_KEY") {
        Ok(secret) => match hendra_auth::TokenKey::new(secret.into_bytes()) {
            Ok(key) => key,
            Err(err) => {
                tracing::error!(%err, "HENDRA_TOKEN_KEY is not usable");
                std::process::exit(1);
            }
        },
        Err(_) => {
            tracing::error!(
                "HENDRA_TOKEN_KEY is not set. It must be at least 32 bytes and identical to the \
                 app server's, which is what mints the tokens this checks."
            );
            std::process::exit(1);
        }
    };

    let registry = Arc::new(worlds::Worlds::load(
        &options.worlds,
        Arc::clone(&catalog),
        loadout.clone(),
    ));

    // The entry world is built from the requested map directly, so `--map` still works for a map
    // that no world definition mentions.
    let map_path = options.worlds.join(&options.map);
    let raw = match std::fs::read(&map_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(path = %map_path.display(), %err, "cannot read map");
            std::process::exit(1);
        }
    };

    let Some(map) = worlds::load_map_bytes(&raw, &options.map, &catalog) else {
        tracing::error!(path = %map_path.display(), "cannot load map");
        std::process::exit(1);
    };

    let name = options
        .map
        .trim_end_matches(".jm")
        .trim_end_matches(".wmap")
        .to_string();
    let terrain = Terrain::build(map, &catalog);
    tracing::info!(
        world = %name,
        width = terrain.width(),
        height = terrain.height(),
        walkable = terrain.walkable_count(),
        "entry world loaded"
    );

    let world = World::new(name.clone(), terrain, &catalog);
    tracing::info!(
        world = %name,
        entities = world.len(),
        portals = world_task::portals_in(&world).len(),
        "world populated"
    );
    for (handle, object_type) in world_task::portals_in(&world) {
        match registry.destination_of(object_type) {
            Some(destination) => tracing::info!(
                ?handle, object_type = format!("0x{object_type:04x}"), %destination, "portal"
            ),
            None => tracing::warn!(
                object_type = format!("0x{object_type:04x}"),
                "portal leads nowhere: no world definition claims this type"
            ),
        }
    }

    // The entry world has to exist before anyone is in it, so it is the one that never closes.
    let entry = world_task::spawn(
        world,
        Arc::clone(&catalog),
        world_task::Loadout {
            persistent: true,
            ..loadout.clone()
        },
    );

    let identity = match &options.certificate {
        Some((cert, key)) => match ServerIdentity::from_pem_files(cert, key) {
            Ok(identity) => {
                tracing::info!(chain = identity.chain_len(), "loaded certificate");
                identity
            }
            Err(err) => {
                tracing::error!(%err, "cannot load certificate");
                std::process::exit(1);
            }
        },
        None => {
            tracing::warn!(
                "no --cert/--key given; generating a self-signed certificate. Clients must \
                 accept any certificate, which is for development only."
            );
            ServerIdentity::self_signed(&["localhost"]).expect("a development certificate")
        }
    };

    let address = format!("0.0.0.0:{}", options.port)
        .parse()
        .expect("a valid bind address");

    let listener = match Listener::bind(address, identity) {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!(%err, "cannot bind");
            std::process::exit(1);
        }
    };

    let context = Arc::new(session::Context {
        worlds: Arc::clone(&registry),
        store,
        catalog: Arc::clone(&catalog),
        kit: accounts::StartingKit {
            default_class: default_class.object_type,
            common: hendra_characters::CommonItems::new(COMMON_ITEMS.iter().copied()),
        },
        key,
    });

    tracing::info!(
        address = %listener.local_address().expect("a bound address"),
        world = %name,
        known_worlds = registry.known(),
        "listening"
    );

    let accepting = async {
        while let Some(incoming) = listener.accept().await {
            match incoming {
                Ok(link) => {
                    tokio::spawn(session::serve(link, Arc::clone(&context), entry.clone()));
                }
                Err(err) => tracing::warn!(%err, "handshake failed"),
            }
        }
    };

    tokio::select! {
        _ = accepting => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
}
