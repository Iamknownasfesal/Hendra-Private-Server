//! The Hendra game server.
//!
//! Loads content, builds a world from a map, and serves it over QUIC.
//!
//! ```text
//!   hendra-server [--map Nexus.jm] [--port 2050] [--content DIR] [--worlds DIR]
//!                 [--behaviours DIR] [--music DIR]
//!                 [--cert fullchain.pem --key privkey.pem]
//! ```
//!
//! Every directory defaults into `rust-server/content`, which is this project's own copy of the
//! game content. `Server-Side/` and `Client-Side/` are the specification this server is measured
//! against; nothing here reads them while it runs, so the room a player stands in can be changed
//! without altering the record of what the original shipped.
//!
//! Without a certificate it generates a self-signed one and says so. That pairs only with a client
//! willing to accept any certificate, which is fine on a development machine and is not what a
//! public server should run.

mod accounts;
mod chat;
mod commands;
mod dungeons;
mod queue;
mod session;
mod trades;
mod vault;
mod world_task;
mod worlds;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hendra_content::Catalog;
use hendra_sim::{Terrain, World};
use hendra_transport::{Listener, ServerIdentity};

/// Where the durable half lives.
const DEFAULT_DATABASE: &str = "postgres://localhost/hendra";

/// The object and tile definitions: `content/xmls`.
const DEFAULT_CONTENT: &str = "content/xmls";

/// The world definitions and the maps they name: `content/worlds`.
const DEFAULT_WORLDS: &str = "content/worlds";

/// The compiled enemy scripts: `content/behaviours`.
const DEFAULT_BEHAVIOURS: &str = "content/behaviours";

/// The tracks `/music` may name: `content/music`.
const DEFAULT_MUSIC: &str = "content/music";

struct Options {
    port: u16,
    map: String,
    content: PathBuf,
    worlds: PathBuf,

    /// Where the converted enemy behaviours are.
    behaviours: PathBuf,

    /// Where the music the `/music` command may name is listed.
    music: PathBuf,
    certificate: Option<(PathBuf, PathBuf)>,
    database: String,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            port: 2050,
            map: "Nexus.jm".into(),
            content: PathBuf::from(DEFAULT_CONTENT),
            worlds: PathBuf::from(DEFAULT_WORLDS),
            behaviours: PathBuf::from(DEFAULT_BEHAVIOURS),
            music: PathBuf::from(DEFAULT_MUSIC),
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
            "--behaviours" => {
                options.behaviours = args.next().map(PathBuf::from).unwrap_or(options.behaviours)
            }
            "--music" => options.music = args.next().map(PathBuf::from).unwrap_or(options.music),
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

/// Every track this deployment can play, by the name the client asks for it by.
///
/// The stems of the mp3s in the music directory, which is what `Resources.music` reads
/// (`common/resources/Resources.cs:98-111`). The audio itself is fetched by the client from a web
/// root rather than from this server, so a deployment that keeps its mp3s elsewhere can list their
/// names in `INDEX.txt` instead, one per line: this needs the names and never the sound.
///
/// Sorted and deduplicated, so `/music` with no argument lists them in a stable order. Nothing
/// found gives an empty list, which turns off the name check rather than refusing every name.
fn music_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("mp3"))
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect();

    if let Ok(listed) = std::fs::read_to_string(directory.join("INDEX.txt")) {
        names.extend(
            listed
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string),
        );
    }

    names.sort();
    names.dedup();
    tracing::info!(tracks = names.len(), path = %directory.display(), "music list loaded");
    names
}

/// Reads and compiles every converted behaviour.
///
/// A world without these is a world where nothing moves: every enemy stands where it was placed and
/// waits to be shot. Loading them is therefore not optional, but a directory that cannot be read is
/// still not a reason to refuse to start, because a server full of motionless enemies is easier to
/// diagnose than a server that would not boot.
fn load_behaviours(directory: &Path) -> hendra_behavior::Programs {
    let mut source = String::new();
    let mut files = 0;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("beh"))
        .collect();
    paths.sort();

    for path in &paths {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                source.push_str(&text);
                source.push('\n');
                files += 1;
            }
            Err(err) => tracing::warn!(path = %path.display(), %err, "cannot read behaviours"),
        }
    }

    if files == 0 {
        tracing::error!(
            path = %directory.display(),
            "no behaviours were read; every enemy will stand still"
        );
        return hendra_behavior::Programs::default();
    }

    let parsed = match hendra_behavior::parse::parse(&source) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::error!(%err, "the behaviours do not parse; every enemy will stand still");
            return hendra_behavior::Programs::default();
        }
    };

    let (programs, diagnostics) = hendra_behavior::compile::compile(&parsed);

    // Reported rather than swallowed. A behaviour that did not compile is an enemy that does
    // nothing, and an enemy that does nothing looks exactly like one that is working.
    for diagnostic in &diagnostics {
        tracing::warn!(message = %diagnostic.message, "behaviour problem");
    }

    tracing::info!(
        files,
        enemies = programs.programs.len(),
        problems = diagnostics.len(),
        "behaviours loaded"
    );

    programs
}

/// How many players may be in the server at once.
///
/// Anybody arriving past this waits in a line rather than being refused, because a refusal makes
/// everybody retry and everybody retrying makes a busy server hardest to get into exactly when it
/// is busiest.
const MAX_PLAYERS: usize = 200;

/// How often the nexus's portal labels and the marketplace's stalls are brought up to date.
const PORTAL_REFRESH: std::time::Duration = std::time::Duration::from_secs(5);

/// How many server-wide lines may be in flight before a stalled world starts missing them.
///
/// These are rare -- a realm closing, a server coming up -- and a world that has fallen this far
/// behind has worse problems than a missed announcement.
const SERVER_ANNOUNCEMENTS: usize = 64;

/// The world a player's own listings stand in.
const MARKETPLACE: &str = "Marketplace";

/// How many listings the marketplace stands merchants for.
///
/// More than any map has squares for, so the limit that decides what is shown is the map rather
/// than this.
const LISTINGS_SHOWN: i64 = 200;

/// What is for sale, as the marketplace needs it.
async fn shop_listings(
    store: &hendra_store::Store,
    catalog: &hendra_content::Catalog,
) -> Vec<world_task::Listed> {
    store
        .listings(LISTINGS_SHOWN)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|listing| {
            let kind = catalog.type_of_uuid(listing.item)?;

            Some(world_task::Listed {
                listing: listing.id,
                item: kind,
                price: listing.price,
            })
        })
        .collect()
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
    //
    // Asked of the characters crate rather than worked out here, so the rule that decides which
    // class a new account starts on is written once: the account path that makes a character on
    // first arrival reads the same answer.
    let default_class = hendra_characters::default_class(&catalog)
        .and_then(|object_type| catalog.class(object_type))
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

    // The content numbers loot colours from zero upward and names the bags in the same order, nine
    // of them from the brown bag to the troll white. The red bag a loot-drop boost produces goes on
    // the end as colour nine, since the original addresses it separately rather than as another
    // step on the same scale. Anything the catalog does not have leaves a hole, which falls back to
    // the plain bag rather than dropping nothing.
    let bag_types: Vec<hendra_content::ObjectType> = std::iter::once("Loot Bag".to_string())
        .chain((2..=9).map(|colour| format!("Loot Bag {colour}")))
        .chain(std::iter::once("Loot Bag Boost".to_string()))
        .map(|name| {
            catalog
                .type_of(&name)
                .unwrap_or(hendra_content::ObjectType::NONE)
        })
        .collect();

    let behaviours = Arc::new(load_behaviours(&options.behaviours));

    // What may live outdoors, read from the content: every enemy that names a terrain.
    let spawnable = hendra_sim::realm::spawnable(&catalog);
    tracing::info!(kinds = spawnable.len(), "enemies that can populate a realm");

    // One clock for the whole server, because the realm's half-hour and Oryx's ten seconds are both
    // read off `RealmTime.TotalElapsedMs` rather than off any one world's age.
    let started = std::time::Instant::now();

    // Anything said to everyone wherever they are. A world subscribes as it starts and relays what
    // it hears to the people in it, which is what makes a realm closing news in the nexus.
    let (announcements, _) = tokio::sync::broadcast::channel(SERVER_ANNOUNCEMENTS);

    // A portal opening reaches everybody too, but the line each world says depends on which world
    // it is, so what travels is the destination rather than a finished sentence.
    let (portals_opened, _) = tokio::sync::broadcast::channel(SERVER_ANNOUNCEMENTS);

    let loadout = world_task::Loadout {
        bag_types,
        spawnable,
        is_realm: false,
        maps: options.worlds.clone(),
        persistent: false,
        started,
        announcements,
        portals_opened,
        blueprint: None,
        avatar: default_class.object_type,
        weapon: default_class.equipment.first().copied().flatten(),
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
        Arc::clone(&behaviours),
        loadout.clone(),
    ));

    // The entry world resolves its map through its own definition, exactly as every world reached
    // by a portal does. `--map` names a world rather than a file, and a name no definition claims
    // is still taken as a file so an unmentioned map can be served by naming it.
    let (name, map_file) = registry.entry_map(&options.map);

    let map_path = options.worlds.join(&map_file);
    let raw = match std::fs::read(&map_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(path = %map_path.display(), %err, "cannot read map");
            std::process::exit(1);
        }
    };

    let Some(map) = worlds::load_map_bytes(&raw, &map_file, &catalog) else {
        tracing::error!(path = %map_path.display(), "cannot load map");
        std::process::exit(1);
    };
    let terrain = Terrain::build(map, &catalog);
    tracing::info!(
        world = %name,
        width = terrain.width(),
        height = terrain.height(),
        walkable = terrain.walkable_count(),
        "entry world loaded"
    );

    let mut world = World::new(name.clone(), terrain, &catalog);
    world.set_behaviours(&catalog, (*behaviours).clone());
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

    // Handed to the registry so that everything sent to this world by name -- a portal, a death, a
    // realm closing -- arrives in the world people log into rather than in a second copy of it.
    registry.adopt(&name, entry.clone());

    // The realm exists from the moment the server does. `RealmManager.Initialize` builds it at
    // startup and hands its portal to the nexus, which is why a nexus with nobody in it still has
    // a realm to walk into; started on demand, the first player to arrive would find no way out.
    if registry.get_or_start_for(worlds::REALM, 0).is_none() {
        tracing::warn!(world = worlds::REALM, "cannot start the realm");
    }

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

    let trades = Arc::new(trades::Trades::new());

    // The nexus is where a player chooses where to go, and the counts are what makes the choice
    // mean anything. Refreshed on a timer rather than on every arrival: a portal label that changed
    // twenty times a second would cost more to send than it tells anybody.
    {
        let registry = Arc::clone(&registry);
        let roster = Arc::clone(&trades);
        let nexus = entry.clone();
        let store = store.clone();
        let catalog = Arc::clone(&catalog);

        tokio::spawn(async move {
            let mut every = tokio::time::interval(PORTAL_REFRESH);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                every.tick().await;

                let portals = registry.signposts(&roster.counts());
                if !nexus
                    .send(world_task::ToWorld::ShowPortals { portals })
                    .await
                {
                    break;
                }

                // The marketplace stands a merchant for every listing, in the row its kind belongs
                // to. Refreshed on the same timer: the set changes whenever anybody buys or lists,
                // and a stall for something already sold refuses whoever walks up to it.
                let Some(marketplace) = registry.get_or_start_for(MARKETPLACE, 0) else {
                    continue;
                };

                let listings = shop_listings(&store, &catalog).await;
                let _ = marketplace
                    .send(world_task::ToWorld::ShowListings { listings })
                    .await;
            }
        });
    }

    let context = Arc::new(session::Context {
        trades: Arc::clone(&trades),
        started: std::time::Instant::now(),
        capacity: MAX_PLAYERS,
        queue: std::sync::Mutex::new(queue::Queue::new()),
        guild_invites: std::sync::Mutex::new(std::collections::HashMap::new()),
        worlds: Arc::clone(&registry),
        store,
        catalog: Arc::clone(&catalog),
        kit: accounts::StartingKit {
            default_class: default_class.object_type,
        },
        key,
        music: music_names(&options.music),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, which is what a relative default is resolved against when the server is
    /// started from `rust-server/`. A test runs with the crate directory as its own, so the two
    /// levels back are what make the same string mean the same directory in both.
    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Nothing this server reads while it runs may live in the specification.
    ///
    /// `Server-Side/` and `Client-Side/` are the record of what the original shipped, and every
    /// parity test in this project measures us against them. A runtime default pointing into that
    /// tree makes it the source of our content as well as the bar we are judged by, and the two
    /// pull in opposite directions: changing our vault's layout then means editing the file that
    /// proves what the original's vault was. It has already happened once. This is what stops it
    /// happening again.
    #[test]
    fn no_default_content_path_reads_the_specification() {
        let options = Options::default();
        let defaults = [
            ("--content", &options.content),
            ("--worlds", &options.worlds),
            ("--behaviours", &options.behaviours),
            ("--music", &options.music),
        ];

        for (flag, path) in defaults {
            let shown = path.display().to_string();
            for reference in ["Server-Side", "Client-Side"] {
                assert!(
                    !shown.contains(reference),
                    "the default for {flag} is {shown}, which reads the {reference} \
                     specification at runtime. Copy what the server needs into rust-server/content \
                     and point it there; the reference is the bar, not our content source."
                );
            }
        }
    }

    /// A server started with no flags at all has to find everything it needs.
    ///
    /// The check above only says where the defaults do *not* point. Without this one they could be
    /// moved somewhere harmless and empty, and the first sign would be a server that boots into a
    /// world with no objects in it.
    #[test]
    fn the_defaults_name_content_that_is_there() {
        let root = workspace_root();
        let options = Options::default();

        for path in [
            &options.content,
            &options.worlds,
            &options.behaviours,
            &options.music,
        ] {
            let full = root.join(path);
            assert!(
                full.is_dir(),
                "a server started with no flags reads {}, which is not a directory",
                path.display()
            );
        }

        // The entry world's map, named by no flag, comes out of the worlds directory the same way
        // every other world's does.
        let worlds = root.join(&options.worlds);
        let definitions = std::fs::read_dir(&worlds)
            .expect("the worlds directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jw"))
            .count();
        assert!(
            definitions > 30,
            "only {definitions} world definitions in {}",
            worlds.display()
        );

        let objects = std::fs::read_dir(root.join(&options.content))
            .expect("the content directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                matches!(
                    entry.path().extension().and_then(|e| e.to_str()),
                    Some("xml") | Some("dat")
                )
            })
            .count();
        assert!(
            objects > 50,
            "only {objects} content files in {}",
            options.content.display()
        );
    }

    /// The tracks `/music` accepts are read from the names alone.
    ///
    /// The mp3s are served to the client by a web root rather than by this server, so our copy
    /// lists their names instead of carrying two hundred megabytes of audio. A list that stopped
    /// being read would turn the name check off silently, and a mistyped `/music` would then leave
    /// a world playing nothing.
    #[test]
    fn the_music_list_is_read_from_the_names() {
        let directory = workspace_root().join(Options::default().music);
        let names = music_names(&directory);

        assert!(
            names.len() > 50,
            "only {} tracks listed in {}",
            names.len(),
            directory.display()
        );
        assert!(
            names.iter().any(|name| name == "Nexus"),
            "the nexus's own track is not among them"
        );

        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, names, "the list is sorted and holds no duplicates");
    }
}
