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

mod session;
mod world_task;

use std::path::PathBuf;
use std::sync::Arc;

use hendra_content::{Catalog, ObjectType, legacy};
use hendra_sim::{Terrain, World};
use hendra_transport::{Listener, ServerIdentity};

/// Where a player's own avatar comes from. Any player-classed object will do until characters are
/// real; this is the wizard.
const DEFAULT_PLAYER_OBJECT: &str = "Wizard";

/// What a player arrives holding, until inventories exist.
const DEFAULT_WEAPON: &str = "Wand of Dark Magic";

struct Options {
    port: u16,
    map: String,
    content: PathBuf,
    worlds: PathBuf,
    certificate: Option<(PathBuf, PathBuf)>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            port: 2050,
            map: "Nexus.jm".into(),
            content: PathBuf::from("../Server-Side/XmlDatas/xmls/client"),
            worlds: PathBuf::from("../Server-Side/XmlDatas/worlds"),
            certificate: None,
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
            "--content" => options.content = args.next().map(PathBuf::from).unwrap_or(options.content),
            "--worlds" => options.worlds = args.next().map(PathBuf::from).unwrap_or(options.worlds),
            "--cert" => cert = args.next().map(PathBuf::from),
            "--key" => key = args.next().map(PathBuf::from),
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

    let map_path = options.worlds.join(&options.map);
    let raw = match std::fs::read(&map_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(path = %map_path.display(), %err, "cannot read map");
            std::process::exit(1);
        }
    };

    // Our own format when the file is one, and the legacy importers otherwise, so a half-converted
    // content directory still boots.
    let map = if raw.starts_with(&hendra_content::map::MAGIC) {
        hendra_content::Map::read(&raw).map_err(|err| err.to_string())
    } else if options.map.ends_with(".wmap") {
        legacy::from_wmap(&raw, &catalog)
            .map(|(map, _)| map)
            .map_err(|err| err.to_string())
    } else {
        std::str::from_utf8(&raw)
            .map_err(|err| err.to_string())
            .and_then(|text| {
                legacy::from_jm(text, &catalog)
                    .map(|(map, _)| map)
                    .map_err(|err| err.to_string())
            })
    };

    let map = match map {
        Ok(map) => map,
        Err(err) => {
            tracing::error!(path = %map_path.display(), %err, "cannot load map");
            std::process::exit(1);
        }
    };

    let name = options.map.trim_end_matches(".jm").trim_end_matches(".wmap").to_string();
    let terrain = Terrain::build(map, &catalog);
    tracing::info!(
        world = %name,
        width = terrain.width(),
        height = terrain.height(),
        walkable = terrain.walkable_count(),
        "map loaded"
    );

    let catalog = Arc::new(catalog);
    let loadout = world_task::Loadout {
        avatar: catalog
            .type_of(DEFAULT_PLAYER_OBJECT)
            .unwrap_or(ObjectType(0x0300)),
        weapon: catalog.type_of(DEFAULT_WEAPON),
    };
    if loadout.weapon.is_none() {
        tracing::warn!(weapon = DEFAULT_WEAPON, "starter weapon not in the catalog; players cannot shoot");
    }

    let world = World::new(name.clone(), terrain, &catalog);
    tracing::info!(world = %name, entities = world.len(), "world populated");

    let handle = world_task::spawn(world, Arc::clone(&catalog), loadout);

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

    tracing::info!(
        address = %listener.local_address().expect("a bound address"),
        world = %name,
        "listening"
    );

    let accepting = async {
        while let Some(incoming) = listener.accept().await {
            match incoming {
                Ok(link) => {
                    tokio::spawn(session::serve(link, handle.clone()));
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
