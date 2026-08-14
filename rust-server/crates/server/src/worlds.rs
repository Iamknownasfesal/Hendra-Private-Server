//! Which worlds exist, and which portal leads where.
//!
//! Worlds are started on demand rather than at boot. There are forty-two definitions and some of
//! their maps are large; a server whose players are all in the nexus should not be ticking the
//! other forty-one. A world that has been started stays running, because the cost of keeping an
//! idle world is one task doing nothing twenty times a second.
//!
//! # Personal worlds
//!
//! A world marked `isLimbo` is a place rather than a destination, such as the vault or the shop, and each
//! account gets its own. Sharing one would mean everybody's vault chests standing in the same room,
//! which is both wrong and a way to show one player another's belongings.
//!
//! Everywhere else is shared, so a nexus is a nexus.
//!
//! # Where a portal leads
//!
//! The mapping is inverted from what it looks like. A portal object does not name its destination.
//! each world definition lists the portal object types that lead *to* it. So the table is built by
//! reading every definition once and turning it inside out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hendra_content::{Catalog, Map, WorldDef, legacy};
use hendra_sim::{Terrain, World};

use crate::world_task::{self, Loadout, WorldHandle};

/// The world definition that fills itself with enemies and closes half an hour later.
///
/// Named rather than flagged because that is how the original decides: `DynamicWorld` matches a
/// definition's name against a class, and only `Realm` gets an overseer.
pub const REALM: &str = "Realm";

/// Everything needed to bring a world into being.
pub struct Worlds {
    catalog: Arc<Catalog>,

    /// Every enemy's compiled behaviour, given to each world as it starts.
    ///
    /// Compiled once and cloned per world rather than shared, because a world resolves the names in
    /// them against the catalog and holds the result: the programs are read every tick and lifted
    /// in and out of the world while it runs, which a shared reference cannot allow.
    behaviours: Arc<hendra_behavior::Programs>,

    loadout: Loadout,
    directory: PathBuf,

    /// By world name, with the filename it came from.
    ///
    /// The filename matters: a definition with no `maps` field takes its map from the sibling file
    /// of the same name, and a world's `name` is not always its filename.
    definitions: HashMap<String, (WorldDef, String)>,

    /// Portal object type to the world it leads to.
    destinations: HashMap<u16, String>,

    running: Mutex<HashMap<String, WorldHandle>>,
}

impl Worlds {
    /// Reads every world definition in a directory.
    pub fn load(
        directory: &Path,
        catalog: Arc<Catalog>,
        behaviours: Arc<hendra_behavior::Programs>,
        loadout: Loadout,
    ) -> Worlds {
        let mut definitions = HashMap::new();
        let mut destinations = HashMap::new();

        let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jw"))
            .collect();
        paths.sort();

        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match WorldDef::parse(&text) {
                Ok(definition) => {
                    for portal in &definition.portals {
                        // Portal types are object types, which are sixteen bits. A definition with
                        // a nonsense value should not shadow a real portal.
                        if let Ok(object_type) = u16::try_from(*portal) {
                            destinations.insert(object_type, definition.name.clone());
                        }
                    }
                    let stem = path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    definitions.insert(definition.name.clone(), (definition, stem));
                }
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "skipping world definition")
                }
            }
        }

        tracing::info!(
            worlds = definitions.len(),
            portals = destinations.len(),
            "world definitions loaded"
        );

        Worlds {
            catalog,
            behaviours,
            loadout,
            directory: directory.to_path_buf(),
            definitions,
            destinations,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Which world a portal object leads to, if any.
    pub fn destination_of(&self, portal_type: u16) -> Option<&str> {
        self.destinations.get(&portal_type).map(String::as_str)
    }

    /// Which portal leads to a world, which is the table read the other way round.
    ///
    /// A definition lists the portals that lead *to* it, so the answer is the first one that names
    /// this world: any of them opens the same door.
    pub fn portal_to(&self, world: &str) -> Option<u16> {
        self.destinations
            .iter()
            .find(|(_, destination)| destination.as_str() == world)
            .map(|(portal, _)| *portal)
    }

    /// The worlds a nexus should show a portal to, and how busy each is.
    ///
    /// Only what has a portal object and is worth walking into: a world nothing leads to cannot be
    /// shown, and one nobody is in is still worth showing because somebody has to be first.
    pub fn signposts(
        &self,
        counts: &std::collections::HashMap<String, usize>,
    ) -> Vec<crate::world_task::PortalSign> {
        let Ok(running) = self.running.lock() else {
            return Vec::new();
        };

        let mut signs: Vec<crate::world_task::PortalSign> = running
            .iter()
            .filter(|(_, handle)| !handle.inbox.is_closed())
            .filter_map(|(key, _)| {
                // The instance key is the world's name for everything shared, which is everything
                // a nexus shows: a personal world is not somewhere anybody else can be sent.
                let name = key.split('#').next()?;
                if self.is_personal(name) {
                    return None;
                }

                Some(crate::world_task::PortalSign {
                    portal: hendra_content::ObjectType(self.portal_to(name)?),
                    players: counts.get(key).copied().unwrap_or(0),
                    world: name.to_string(),
                })
            })
            .collect();

        // Busiest first, so the portal somebody wants is the one they see.
        signs.sort_by(|a, b| b.players.cmp(&a.players).then(a.world.cmp(&b.world)));
        signs
    }

    /// Which worlds are ticking, by their instance key.
    pub fn running(&self) -> Vec<String> {
        let Ok(running) = self.running.lock() else {
            return Vec::new();
        };

        let mut keys: Vec<String> = running
            .iter()
            .filter(|(_, handle)| !handle.inbox.is_closed())
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    }

    pub fn known(&self) -> usize {
        self.definitions.len()
    }

    /// Whether a world gets one instance per account.
    pub fn is_personal(&self, name: &str) -> bool {
        self.definitions
            .get(name)
            .is_some_and(|(definition, _)| definition.is_limbo)
    }

    /// A handle to a world, starting it if it is not already running.
    ///
    /// `account_id` only matters for a personal world, where it selects which instance.
    ///
    /// # Why there is no access check
    ///
    /// The original checks, on entry, that the vault you are opening is yours and the hall is your
    /// guild's. Here the instance key carries the account or the guild, and the only caller is the
    /// session acting for that account, so there is no way to name somebody else's room. A check
    /// would be a second answer to a question the key has already answered, and a second answer is
    /// somewhere the two can disagree.
    pub fn get_or_start_for(&self, name: &str, account_id: i64) -> Option<WorldHandle> {
        if self.is_personal(name) {
            // The instance key includes the account, so two players asking for the vault get two
            // rooms. The world's own name stays as it was, so the client is told "Vault".
            self.start(name, &format!("{name}#{account_id}"), 0)
        } else {
            self.start(name, name, 0)
        }
    }

    /// A guild's hall, which is one room per guild rather than one per player.
    ///
    /// Per guild because that is the point of it: a hall nobody else could walk into would be a
    /// second vault. The level chooses which of the shipped maps is loaded, so a guild that has paid
    /// for a larger hall gets one.
    pub fn get_or_start_hall(&self, name: &str, guild_id: i64, level: i16) -> Option<WorldHandle> {
        self.start(
            name,
            &format!("{name}#guild{guild_id}"),
            level.clamp(0, hendra_store::MAX_GUILD_LEVEL) as usize,
        )
    }

    fn start(&self, name: &str, key: &str, map: usize) -> Option<WorldHandle> {
        // Held across the build, so that two players stepping into the same unopened dungeon
        // in the same tick must get the same world, not two of them.
        let mut running = self.running.lock().ok()?;

        if let Some(handle) = running.get(key) {
            // A world whose task has ended leaves a closed channel behind; replace it rather than
            // hand out something nothing is listening to.
            if !handle.inbox.is_closed() {
                return Some(handle.clone());
            }
            running.remove(key);
        }

        let (definition, stem) = self.definitions.get(name)?;
        let map = self.load_map(definition, stem, map)?;

        let terrain = Terrain::build(map, &self.catalog);
        tracing::info!(
            world = %name,
            width = terrain.width(),
            height = terrain.height(),
            walkable = terrain.walkable_count(),
            "starting world"
        );

        let mut world = World::new(name.to_string(), terrain, &self.catalog);

        // Without this every enemy in the world stands where it was placed and waits to be shot.
        world.set_behaviours(&self.catalog, (*self.behaviours).clone());

        // The nexus and the shops forbid it, in their own definitions. Without this a player could
        // teleport into a room the map author meant to be walked into.
        world.set_allows_teleport(!definition.restrict_tp);

        // What a wall hides, which the world's own definition decides and which defaults to
        // nothing. A dungeon asks for rooms; the realm asks for nothing and is meant to be seen
        // across.
        world.sight = hendra_sim::Sight::from_blocking(definition.blocking);

        report_portals(&world, self);

        // Only the world the original calls `Realm` fills itself and closes on a clock. Every other
        // definition is the map it was drawn as.
        let loadout = Loadout {
            is_realm: name == REALM,
            ..self.loadout.clone()
        };
        let handle = world_task::spawn(world, Arc::clone(&self.catalog), loadout);

        // Worlds that have closed since the last start are dropped here rather than by a sweeper,
        // because this is the only moment the registry is already locked and already being read.
        running.retain(|_, held| !held.inbox.is_closed());

        running.insert(key.to_string(), handle.clone());
        tracing::info!(running = running.len(), "worlds now ticking");
        Some(handle)
    }

    fn load_map(&self, definition: &WorldDef, stem: &str, which: usize) -> Option<Map> {
        // Ten of the game's forty-two definitions have no `maps` field, including the vault. The
        // convention -- undocumented, and enforced only by the loader that happened to implement it
        // -- is that such a world takes the sibling file of the same name. Without this they are
        // all silently unreachable, which is exactly how it presented: a portal that did nothing.
        let fallback = format!("{stem}.jm");

        // A definition listing several maps is offering a choice: the realm picks one at random in
        // the original and the guild hall picks by level. Asking past the end takes the last, so a
        // guild whose level outruns the maps gets the largest rather than nothing.
        let file = definition
            .maps
            .get(which)
            .or_else(|| definition.maps.last())
            .map(String::as_str)
            .unwrap_or(&fallback);

        let path = self.directory.join(file);

        let raw = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cannot read map");
                return None;
            }
        };

        load_map_bytes(&raw, file, &self.catalog)
    }
}

/// Says what a world's portals lead to, or that they lead nowhere.
fn report_portals(world: &World, worlds: &Worlds) {
    let portals = world_task::portals_in(world);
    if portals.is_empty() {
        return;
    }

    for (handle, object_type) in &portals {
        match worlds.destination_of(*object_type) {
            Some(destination) => tracing::info!(
                world = %world.name, ?handle, object_type = format!("0x{object_type:04x}"),
                %destination, "portal"
            ),
            None => tracing::warn!(
                world = %world.name, object_type = format!("0x{object_type:04x}"),
                "portal leads nowhere: no world definition claims this type"
            ),
        }
    }
}

/// Reads a map in whichever format it happens to be in.
///
/// Our own format when the file is one, and the legacy importers otherwise, so a partially
/// converted content directory still boots.
pub fn load_map_bytes(raw: &[u8], name: &str, catalog: &Catalog) -> Option<Map> {
    let outcome = if raw.starts_with(&hendra_content::map::MAGIC) {
        Map::read(raw).map_err(|err| err.to_string())
    } else if name.ends_with(".wmap") {
        legacy::from_wmap(raw, catalog)
            .map(|(map, _)| map)
            .map_err(|err| err.to_string())
    } else {
        std::str::from_utf8(raw)
            .map_err(|err| err.to_string())
            .and_then(|text| {
                legacy::from_jm(text, catalog)
                    .map(|(map, _)| map)
                    .map_err(|err| err.to_string())
            })
    };

    match outcome {
        Ok(map) => Some(map),
        Err(err) => {
            tracing::warn!(map = name, %err, "cannot load map");
            None
        }
    }
}
