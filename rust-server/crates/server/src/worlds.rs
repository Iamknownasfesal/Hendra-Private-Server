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

/// The one world where a player away from air drowns.
///
/// Named rather than flagged in the content, as the original names it in
/// `HandleOceanTrenchGround`.
const DROWNING_WORLD: &str = "OceanTrench";

/// The world definition that fills itself with enemies and closes half an hour later.
///
/// Named rather than flagged because that is how the original decides: `DynamicWorld` matches a
/// definition's name against a class, and only `Realm` gets an overseer.
pub const REALM: &str = "Realm";

/// The generic portal a nexus stands to send people to a realm.
///
/// The original does not look this up. `PortalMonitor.AddPortal` builds the entity itself with the
/// type written into the code -- `new Portal(_manager, 0x0712, null)` (`PortalMonitor.cs:74-77`) --
/// and hangs the destination off the portal rather than off its type, so no realm ever needed a
/// portal object of its own. Our table runs the other way, from type to world, so the realm claims
/// the same object the original hardcodes. `0x0712` is "Nexus Portal", which is a `Portal` with no
/// destination written into it, and no world definition claims it.
const NEXUS_PORTAL: u16 = 0x0712;

/// The portals that name no destination and mean "a realm, whichever one is open".
///
/// `UsePortalHandler._realmPortals` (`UsePortalHandler.cs:13`), verbatim and in its order: the
/// Realm Portal a dungeon boss leaves behind, its glowing twin, the Random Realm Portal, and the
/// three Portals of Cowardice. None of them is claimed by a world definition, in the original or
/// here, because the original resolves them in code: a portal with no `WorldInstance` and one of
/// these types is answered with `GetRandomGameWorld()` (`UsePortalHandler.cs:70-74`).
pub const REALM_PORTALS: [u16; 6] = [0x0704, 0x070e, 0x071c, 0x0703, 0x070d, 0x0d40];

/// Which of the realm's three maps is played.
///
/// The second, always. `Realm` sets `_mapId = 1` (`Realm.cs:24`) and `Init` loads `wmap[1]`
/// (`Realm.cs:35`), overriding the random choice the base `World.Init` would have made, so
/// `world1.wmap` and `world3.wmap` ship with the game and are never seen.
pub const REALM_MAP: usize = 1;

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

    /// Which of the running worlds a player opened, and who may come in.
    ///
    /// Beside the registry rather than inside a world because the two commands that read it are
    /// typed from other worlds, and because what a key names has to outlive the moment between a
    /// portal being stood and the room behind it being built.
    dungeons: crate::dungeons::Dungeons,
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

        claim_realm_portal(&definitions, &mut destinations);

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
            dungeons: crate::dungeons::Dungeons::new(),
        }
    }

    /// The dungeons a player opened, and who they asked in.
    pub fn dungeons(&self) -> &crate::dungeons::Dungeons {
        &self.dungeons
    }

    /// Which world a portal object leads to, if any.
    pub fn destination_of(&self, portal_type: u16) -> Option<&str> {
        self.destinations.get(&portal_type).map(String::as_str)
    }

    /// What a world calls itself on the scoreboard, which is the name the announcements use.
    ///
    /// `World.SBName` (`World.cs`), which every `.jw` carries as `sbName` and which is sometimes
    /// shorter than the world's own name and sometimes a translation key.
    pub fn scoreboard_name(&self, world: &str) -> Option<&str> {
        self.definitions
            .get(world)
            .map(|(definition, _)| definition.sb_name.as_str())
            .filter(|name| !name.is_empty())
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
    /// Realms, and nothing else. `Nexus.Init` (`Nexus.cs:20-62`) walks every world the manager
    /// holds and stands a portal for each `Realm`; the only other two it considers are the Cloth
    /// Bazaar and the Marketplace, and each of those is skipped unless the map marks a square for
    /// it (`Nexus.cs:39-41`, `:55-57`) — `nexustry.jm` marks neither, so neither is ever stood.
    /// Everything else in the nexus is a portal the map itself places: the vault, the guild hall
    /// and the marketplace all stand in `nexustry.jm` as objects.
    ///
    /// Nothing here is driven by which worlds happen to be running, which is what a dungeon is: the
    /// original never puts a door to somebody's Undead Lair in the nexus, and neither does this.
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
                let name = signposted(key)?;

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

    /// Takes a world that was built elsewhere into the registry, under its own name.
    ///
    /// The world players arrive in is built before the registry is asked for anything, and without
    /// this it would be a second nexus: everybody sent to one by a portal, by a death or by a quake
    /// would land in a room that nobody who logged in was standing in. The original has one world
    /// per id and hands the same one out to everybody, and so does this.
    pub fn adopt(&self, name: &str, mut handle: WorldHandle) {
        handle.key = Arc::from(name);
        if let Ok(mut running) = self.running.lock() {
            running.insert(name.to_string(), handle);
        }
    }

    /// The one running world with this number, which is how an invitation names a room.
    ///
    /// `RealmManager.GetWorld(id)` (`RealmManager.cs:290-297`), which `DungeonAccept` asks with the
    /// number typed after the command (`UnrankedCommands.cs:165`).
    pub fn by_id(&self, id: i32) -> Option<WorldHandle> {
        let running = self.running.lock().ok()?;
        running
            .values()
            .find(|handle| handle.id == id && !handle.inbox.is_closed())
            .cloned()
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
            self.start(name, name, self.which_map(name))
        }
    }

    /// Which of a world's maps it uses.
    ///
    /// The base `World.Init` (`World.cs:203`) picks at random among them, and the realm overrides
    /// that: `Realm` sets `_mapId = 1` and loads `wmap[1]` (`Realm.cs:23`, `:35`), so of the three
    /// realm maps the second is the only one ever played.
    fn which_map(&self, name: &str) -> usize {
        if name == REALM { REALM_MAP } else { 0 }
    }

    /// Whether a world is built fresh for each portal that leads into it.
    ///
    /// `Portal.CreateWorld` (`Portal.cs:69-80`) splits on the definition's own id: `p.id < 0` is
    /// answered with `GetWorld(p.id)`, the one world of that id the manager holds, and anything
    /// else is answered with `new World(p)` handed to `AddWorld`, which is a room nobody else has
    /// been in. That is every dungeon.
    ///
    /// The realm is the exception the original never has to write down. Its definition carries
    /// `id: 1`, but no realm portal ever reaches `CreateWorld`: the nexus builds that portal with
    /// its `WorldInstance` already set (`PortalMonitor.cs:74-78`), and the six portals that carry
    /// no instance are answered from `_realmPortals` with an existing realm
    /// (`UsePortalHandler.cs:70-74`). Instancing it here would give every doorway its own realm.
    pub fn is_instanced(&self, name: &str) -> bool {
        instanced(&self.definitions, name)
    }

    /// One particular instance of a world, started if it is not already running.
    ///
    /// The key is the caller's to choose and is what two callers must agree on to land in the same
    /// room. It stands in for the original's `Portal.WorldInstance`, which is one field on one
    /// entity and answers the same question: which of these rooms is *this* door's.
    pub fn get_or_start_instance(&self, name: &str, key: &str) -> Option<WorldHandle> {
        self.start(name, key, self.which_map(name))
    }

    /// A guild's hall, which is one room per guild rather than one per player.
    ///
    /// Per guild because that is the point of it: a hall nobody else could walk into would be a
    /// second vault. The level chooses which of the shipped maps is loaded, so a guild that has paid
    /// for a larger hall gets one.
    ///
    /// `occupied` says whether anybody is standing in the hall already, and an empty one is torn
    /// down and built again rather than reused. That is `GuildHall.GetInstance`
    /// (`GuildHall.cs:48-72`): a hall with players in it is handed back, a hall with none is
    /// `Delete()`d and the loop breaks to build a new one — and only the new one reads the guild's
    /// level and loads the map it has paid for. Reusing the running hall is why a bought upgrade
    /// never appeared: the hall's definition asks to persist, so nothing else would ever end it.
    pub fn get_or_start_hall(
        &self,
        name: &str,
        guild_id: i64,
        level: i16,
        occupied: bool,
    ) -> Option<WorldHandle> {
        let key = hall_key(name, guild_id);

        if !occupied {
            // Dropping the registry's handle closes the task's inbox once the last session lets go
            // of its own, which is what deleting a world amounts to here.
            if let Ok(mut running) = self.running.lock() {
                running.remove(&key);
            }
        }

        self.start(
            name,
            &key,
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

        let terrain = Terrain::build(map.clone(), &self.catalog);
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

        // What the client is told about the place it is standing in. None of it is read by the
        // simulation: the marks on the loading screen, the sky behind the map and the track that
        // plays are the world describing itself (`World.cs:119-132`).
        world.difficulty = definition.difficulty;
        world.background = definition.background;
        world.show_displays = definition.show_displays;
        world.music = chosen_music(&definition.music);

        // What a wall hides, which the world's own definition decides and which defaults to
        // nothing. A dungeon asks for rooms; the realm asks for nothing and is meant to be seen
        // across.
        world.sight = hendra_sim::Sight::from_blocking(definition.blocking);

        // Only the labelled mode needs the map walked, and it is walked once, here, exactly as
        // `World.FromWorldMap` does it (`realm/worlds/World.cs:302-303`). The Mad Lab, the Sewers
        // and the Shatters are the three worlds in the content that ask for it.
        if world.sight == hendra_sim::Sight::Region {
            world.label_sight_regions();
        }

        // One world takes the air away, and it is the whole shape of that dungeon: the vents are
        // where a fight can be held and everywhere else is a walk you can only make so far.
        world.drowns = name == DROWNING_WORLD;

        report_portals(&world, self);

        let is_realm = name == REALM;

        // Only the world the original calls `Realm` fills itself and closes on a clock. Every other
        // definition is the map it was drawn as.
        //
        // A world's own definition says whether it keeps ticking with nobody in it, through
        // `persist`: `World.Tick` (`World.cs:614`) deletes an empty non-persistent world once it is
        // a minute old, and leaves a persistent one alone. The nexus, the realm and the vault all
        // ask to stay.
        let loadout = Loadout {
            is_realm,
            persistent: definition.persist,

            // Only the realm is ever built again, so only the realm is given what it would take.
            // Holding the map of every world would be holding a second copy of every map.
            blueprint: is_realm.then(|| {
                Arc::new(world_task::Blueprint {
                    map,
                    behaviours: (*self.behaviours).clone(),
                    sight: world.sight,
                    allows_teleport: !definition.restrict_tp,
                    drowns: name == DROWNING_WORLD,
                })
            }),

            ..self.loadout.clone()
        };
        let mut handle = world_task::spawn(world, Arc::clone(&self.catalog), loadout);

        // The world knows what it is called; only the registry knows which of them this is.
        handle.key = Arc::from(key);

        // Worlds that have closed since the last start are dropped here rather than by a sweeper,
        // because this is the only moment the registry is already locked and already being read.
        running.retain(|_, held| !held.inbox.is_closed());

        running.insert(key.to_string(), handle.clone());

        // A room that has stopped takes its guest list with it, and the key it ran under can be
        // handed to a new one as soon as a portal entity number is reused.
        self.dungeons.retain(|held| running.contains_key(held));

        // The key and the number together, because an invitation names a room by its number and
        // nothing else on the wire ever says which room that is.
        tracing::info!(
            running = running.len(),
            key,
            id = handle.id,
            "worlds now ticking"
        );
        Some(handle)
    }

    /// What the world the server starts in is called, and which file its ground comes from.
    ///
    /// The argument names a world the way a person would — `Nexus`, or `Nexus.jm` — and a world's
    /// definition is what says which map draws it. The two are not the same file: the Nexus is
    /// defined in `Nexus.jw`, whose `maps` field names `nexustry.jm`, while `Nexus.jm` is a stub
    /// sitting beside it. Reading the argument as a filename gets the stub, which loads and ticks
    /// and serves perfectly — it is simply not the Nexus.
    ///
    /// A name no definition claims is still taken as a file, so a map that no world mentions can
    /// be served by naming it.
    pub fn entry_map(&self, requested: &str) -> (String, String) {
        entry_map_in(&self.definitions, requested)
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

/// The instance key a guild's hall runs under.
///
/// Shared with the caller because whoever asks for a hall also has to say whether anybody is in it,
/// and the roster it asks is keyed by exactly this.
pub fn hall_key(name: &str, guild_id: i64) -> String {
    format!("{name}#guild{guild_id}")
}

/// The world a name belongs to, read off an instance key.
///
/// A key is the world's name and then, for a room there can be more than one of, what tells that
/// room from the others. Only the part before the marker is anything a player would recognise, so
/// that is what is shown and what a portal is looked up by.
pub fn world_of_key(key: &str) -> &str {
    key.split('#').next().unwrap_or(key)
}

/// Which of a definition's tracks this world plays.
///
/// One of the list, drawn when the world is built and kept for its life: `World`'s constructor does
/// `Music = proto.music[rnd.Next(0, proto.music.Length)]` and falls back to `Test` for a definition
/// that names none (`World.cs:127-132`). Drawn per world rather than per definition, so two dungeons
/// opened from the same portal need not sound alike.
///
/// Seeded from the clock, since nothing else about a world's start is deterministic either.
fn chosen_music(tracks: &[String]) -> String {
    if tracks.is_empty() {
        return "Test".to_string();
    }

    let spin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as usize)
        .unwrap_or(0);

    tracks[spin % tracks.len()].clone()
}

/// Whether a definition is built fresh per portal, read off the definitions alone.
///
/// Separated from the registry so it can be exercised without a catalog and a directory of maps: it
/// is one field of one definition and one name.
fn instanced(definitions: &HashMap<String, (WorldDef, String)>, name: &str) -> bool {
    name != REALM
        && definitions
            .get(name)
            .is_some_and(|(definition, _)| definition.id >= 0)
}

/// The world a running instance earns a nexus portal for, if it earns one at all.
///
/// Only a realm does. `Nexus.Init` (`Nexus.cs:22-26`) stands a portal for each world that is a
/// `Realm` and for nothing else that is running; the two shops it also considers are placed by
/// region rather than by being open, and `nexustry.jm` marks neither region. A dungeon somebody
/// opened is not a realm and never gets a door in the nexus, however busy it is.
///
/// An instance key carrying a suffix is a personal or per-guild room, which no realm ever is.
fn signposted(key: &str) -> Option<&str> {
    if key != REALM { None } else { Some(key) }
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

/// Resolves an entry world's name and map file against the definitions that were loaded.
///
/// Separated from the registry so it can be exercised without a catalog, a behaviour set and a
/// directory of maps: it is a lookup over the definitions and nothing else.
fn entry_map_in(
    definitions: &HashMap<String, (WorldDef, String)>,
    requested: &str,
) -> (String, String) {
    let asked = requested.trim_end_matches(".jm").trim_end_matches(".wmap");

    let found = definitions.iter().find(|(name, (_, stem))| {
        name.eq_ignore_ascii_case(asked) || stem.eq_ignore_ascii_case(asked)
    });

    match found {
        Some((name, (definition, stem))) => {
            let file = definition
                .maps
                .first()
                .cloned()
                .unwrap_or_else(|| format!("{stem}.jm"));

            (name.clone(), file)
        }
        None => (asked.to_string(), requested.to_string()),
    }
}

/// Points the generic nexus portal at the realm, unless something already claims it.
///
/// `Realm.jw` lists no portals, because in the original nothing ever asked it to: the nexus builds
/// the realm's portal itself from a type written into the code and hangs the destination off the
/// entity (`PortalMonitor.cs:74-77`), so the realm never needed a portal object of its own. Our
/// table runs from type to world instead, so the claim has to be made somewhere, and this is the
/// same claim the original writes down — only in the direction we read it.
///
/// Left alone if a definition already claims the type, since a file saying so outranks a default.
fn claim_realm_portal(
    definitions: &HashMap<String, (WorldDef, String)>,
    destinations: &mut HashMap<u16, String>,
) {
    if !definitions.contains_key(REALM) {
        return;
    }

    destinations
        .entry(NEXUS_PORTAL)
        .or_insert_with(|| REALM.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definitions(held: &[(&str, &str, &[&str])]) -> HashMap<String, (WorldDef, String)> {
        held.iter()
            .map(|(name, stem, maps)| {
                let definition = WorldDef {
                    name: (*name).to_string(),
                    maps: maps.iter().map(|map| (*map).to_string()).collect(),
                    ..WorldDef::default()
                };
                (name.to_string(), (definition, stem.to_string()))
            })
            .collect()
    }

    /// The Nexus is defined in `Nexus.jw` and drawn by `nexustry.jm`, and `Nexus.jm` is a stub
    /// sitting beside it. Reading the argument as a filename serves the stub: a world that loads
    /// and ticks perfectly well and is not the Nexus.
    #[test]
    fn the_entry_world_takes_its_map_from_its_definition_rather_than_its_name() {
        let held = definitions(&[("Nexus", "Nexus", &["nexustry.jm"])]);

        assert_eq!(
            entry_map_in(&held, "Nexus.jm"),
            ("Nexus".to_string(), "nexustry.jm".to_string())
        );
        assert_eq!(
            entry_map_in(&held, "Nexus"),
            ("Nexus".to_string(), "nexustry.jm".to_string())
        );
    }

    /// A world is called what its definition calls it, not what its map file is called.
    #[test]
    fn the_entry_world_is_named_by_its_definition() {
        let held = definitions(&[("Nexus", "Nexus", &["nexustry.jm"])]);
        assert_eq!(entry_map_in(&held, "nexus.jm").0, "Nexus");
    }

    /// A definition with no `maps` takes the sibling file of its own name, as `load_map` does.
    #[test]
    fn a_definition_without_maps_falls_back_to_its_own_file() {
        let held = definitions(&[("Vault", "Vault", &[])]);
        assert_eq!(
            entry_map_in(&held, "Vault"),
            ("Vault".to_string(), "Vault.jm".to_string())
        );
    }

    /// Without this the realm is a world with no door: nothing claims a portal to it, so the nexus
    /// cannot show one and a player who has not learnt to type `/realm` never reaches the game.
    #[test]
    fn the_realm_is_reached_by_the_portal_the_original_hardcodes() {
        let held = definitions(&[("Realm", "Realm", &["world2.wmap"])]);
        let mut destinations = HashMap::new();

        claim_realm_portal(&held, &mut destinations);

        assert_eq!(
            destinations.get(&0x0712).map(String::as_str),
            Some("Realm"),
            "the nexus portal object leads to the realm"
        );
    }

    /// A definition that claims the type outranks the default, so content stays in charge.
    #[test]
    fn a_definition_that_claims_the_nexus_portal_keeps_it() {
        let held = definitions(&[("Realm", "Realm", &[])]);
        let mut destinations = HashMap::new();
        destinations.insert(NEXUS_PORTAL, "Somewhere".to_string());

        claim_realm_portal(&held, &mut destinations);

        assert_eq!(
            destinations.get(&NEXUS_PORTAL).map(String::as_str),
            Some("Somewhere")
        );
    }

    /// Nothing is claimed for a realm that is not there, so the type stays free.
    #[test]
    fn no_realm_means_no_claim() {
        let held = definitions(&[("Nexus", "Nexus", &["nexustry.jm"])]);
        let mut destinations = HashMap::new();

        claim_realm_portal(&held, &mut destinations);

        assert!(destinations.is_empty());
    }

    /// `Nexus.Init` stands a portal for each `Realm` and for nothing else that happens to be
    /// running. Showing a door to every open world puts one to somebody's dungeon, to a guild hall
    /// and to the marketplace on the realm pads, none of which the original ever stands there.
    #[test]
    fn only_a_realm_earns_a_nexus_portal() {
        assert_eq!(signposted(REALM), Some(REALM));
        assert_eq!(signposted("UndeadLair"), None);
        assert_eq!(signposted("Marketplace"), None);
        assert_eq!(signposted(crate::commands::GUILD_HALL), None);
        assert_eq!(signposted(crate::commands::NEXUS), None);
    }

    /// A per-account or per-guild room carries its instance in the key, and no realm ever does.
    #[test]
    fn a_personal_instance_is_never_signposted() {
        assert_eq!(signposted("Vault#7"), None);
        assert_eq!(signposted("GuildHall#guild3"), None);
    }

    /// The same definitions, with the id each `.jw` actually carries.
    fn identified(held: &[(&str, i32)]) -> HashMap<String, (WorldDef, String)> {
        held.iter()
            .map(|(name, id)| {
                let definition = WorldDef {
                    name: (*name).to_string(),
                    id: *id,
                    ..WorldDef::default()
                };
                (name.to_string(), (definition, (*name).to_string()))
            })
            .collect()
    }

    /// `Portal.CreateWorld` (`Portal.cs:72-78`) answers `p.id < 0` with the manager's one world of
    /// that id and everything else with `new World(p)`. Without the split, two parties walking into
    /// two Undead Lair portals arrive in the same Undead Lair.
    #[test]
    fn a_dungeon_is_built_fresh_for_each_portal_and_a_static_world_is_not() {
        let held = identified(&[
            ("UndeadLair", 0),
            ("Vault", -5),
            (crate::commands::GUILD_HALL, -8),
            (crate::commands::NEXUS, -2),
        ]);

        assert!(instanced(&held, "UndeadLair"));
        assert!(!instanced(&held, "Vault"));
        assert!(!instanced(&held, crate::commands::GUILD_HALL));
        assert!(!instanced(&held, crate::commands::NEXUS));
    }

    /// The realm's definition carries `id: 1`, but no realm portal ever reaches `CreateWorld`: the
    /// nexus stands its portal with the instance already set (`PortalMonitor.cs:74-78`) and the
    /// loose ones are answered from `_realmPortals` (`UsePortalHandler.cs:70-74`). Instancing it
    /// would give every doorway into the realm a realm of its own.
    #[test]
    fn the_realm_is_never_built_per_portal() {
        let held = identified(&[(REALM, 1)]);
        assert!(!instanced(&held, REALM));
    }

    /// A map no definition mentions is still served by naming it, which is what `--map` was for.
    #[test]
    fn a_map_no_definition_claims_is_still_taken_as_a_file() {
        let held = definitions(&[("Nexus", "Nexus", &["nexustry.jm"])]);
        assert_eq!(
            entry_map_in(&held, "scratch.jm"),
            ("scratch".to_string(), "scratch.jm".to_string())
        );
    }

    /// A key names a world and then which of them it is, and only the first half is anything a
    /// player would recognise.
    #[test]
    fn a_key_is_read_back_as_the_world_it_names() {
        assert_eq!(world_of_key("UndeadLair#Realm:11"), "UndeadLair");
        assert_eq!(world_of_key(&hall_key("GuildHall", 4)), "GuildHall");
        assert_eq!(world_of_key("Vault#7"), "Vault");

        // A world with only ever one copy runs under its own name, and reads back as itself.
        assert_eq!(world_of_key(REALM), REALM);
        assert_eq!(world_of_key(crate::commands::NEXUS), crate::commands::NEXUS);
    }

    /// Two guilds have two halls, and the same guild asking twice is asking for one room.
    #[test]
    fn a_hall_is_keyed_by_the_guild_that_owns_it() {
        assert_eq!(hall_key("GuildHall", 4), hall_key("GuildHall", 4));
        assert_ne!(hall_key("GuildHall", 4), hall_key("GuildHall", 5));
    }

    /// A registry that knows no definitions and has no maps, which is enough to watch what it does
    /// with the worlds it is already holding.
    fn registry() -> Worlds {
        let (announcements, _) = tokio::sync::broadcast::channel(4);
        let (portals_opened, _) = tokio::sync::broadcast::channel(4);

        Worlds::load(
            Path::new("/nonexistent-world-definitions"),
            Arc::new(Catalog::default()),
            Arc::new(hendra_behavior::Programs::default()),
            Loadout {
                bag_types: Vec::new(),
                spawnable: Vec::new(),
                is_realm: false,
                maps: PathBuf::new(),
                persistent: false,
                started: std::time::Instant::now(),
                blueprint: None,
                announcements,
                portals_opened,
                avatar: hendra_content::ObjectType::NONE,
                weapon: None,
            },
        )
    }

    /// A handle to nothing. What is being checked is which handles the registry keeps, not what the
    /// world behind one does.
    fn handle(key: &str) -> WorldHandle {
        let (inbox, receiver) = tokio::sync::mpsc::channel(1);

        // Held so the channel does not read as closed, which the registry takes for a world that
        // has stopped.
        std::mem::forget(receiver);

        WorldHandle {
            name: Arc::from(world_of_key(key)),
            key: Arc::from(key),
            id: 1,
            music: Arc::new(std::sync::RwLock::new(String::new())),
            inbox,
        }
    }

    /// A hall with somebody in it is handed back as it stands; an empty one is torn down so that the
    /// next open builds it again and reads the level the guild has paid for
    /// (`GuildHall.GetInstance`, `GuildHall.cs:53-66`). Ours kept the running hall alive forever,
    /// because the hall's definition asks to persist, so an upgrade never showed.
    #[test]
    fn an_empty_guild_hall_is_torn_down_and_a_busy_one_is_not() {
        let worlds = registry();
        let key = hall_key(crate::commands::GUILD_HALL, 4);

        worlds.adopt(&key, handle(&key));
        assert!(worlds.running().contains(&key));

        // Somebody is standing in it, so it survives being asked for.
        worlds.get_or_start_hall(crate::commands::GUILD_HALL, 4, 1, true);
        assert!(worlds.running().contains(&key));

        // Nobody is, so it goes -- and the registry has no definitions to build a new one from,
        // which is exactly what makes the tearing down visible.
        worlds.get_or_start_hall(crate::commands::GUILD_HALL, 4, 1, false);
        assert!(!worlds.running().contains(&key));
    }

    /// An invitation names a room by number, and two rooms never share one.
    #[test]
    fn a_world_is_found_by_the_number_an_invitation_names() {
        let worlds = registry();

        let mut first = handle("UndeadLair#Realm:11");
        first.id = 41;
        let mut second = handle("UndeadLair#Realm:12");
        second.id = 42;

        worlds.adopt("UndeadLair#Realm:11", first);
        worlds.adopt("UndeadLair#Realm:12", second);

        assert_eq!(
            worlds.by_id(42).map(|found| found.key.to_string()).as_deref(),
            Some("UndeadLair#Realm:12")
        );
        assert!(worlds.by_id(43).is_none());
    }
}
