//! The task that owns a world and drives it.
//!
//! Exactly one task owns one world, and it is the only thing that touches it. There is no lock
//! anywhere in here, and there does not need to be. Sessions do not reach into the world, they
//! send it messages, and the world writes back through each player's connection directly.
//!
//! ```text
//!   session task ──ToWorld──▶  world task  ──snapshot──▶  player's connection
//!   (one per player)           (one per world)            (LinkSender, no hop)
//! ```
//!
//! # The tick
//!
//! Fixed timestep, driven by an interval that skips rather than catches up. A world that falls
//! behind should drop the ticks it missed, not run them all back to back and fall further behind
//! while doing it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hendra_content::{Catalog, ObjectType};
use hendra_net::snapshot::{Acknowledgement, Baseline, BaselineRing};
use hendra_net::{Delivery, ServerMessage, SnapshotEncoder, WorldSnapshot, Writer};
use hendra_sim::world::{Entity, SIGHT_RADIUS};
use hendra_sim::{Handle, TickMetrics, World};
use hendra_transport::LinkSender;
use tokio::sync::mpsc;

/// How often the world advances.
pub const TICKS_PER_SECOND: u32 = 20;

/// How long a player's movement goes unjudged after they arrive somewhere.
///
/// `Player.Verify.MoveGraceMs`. Arriving through a portal, the first claims a client makes are
/// measured from where it last stood — which is a position in the world it just left — and judging
/// those disconnects everybody who uses a portal.
const MOVE_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// What a session asks the world to do.
pub enum ToWorld {
    /// A player is arriving. The world replies with the handle it was given.
    Join {
        name: String,
        arrival: Arrival,
        sender: LinkSender,

        /// How the world tells this session to do something only a session can do.
        orders: mpsc::Sender<Order>,

        /// How the world tells this session its character has died.
        died: mpsc::Sender<Departed>,

        reply: tokio::sync::oneshot::Sender<Handle>,
    },

    /// What a player wears has changed, so its stat layer is replaced.
    Equipment {
        handle: Handle,
        boosts: [i32; 8],
    },

    /// A player reports where it believes it is, and what it has received.
    Input {
        handle: Handle,
        x: f32,
        y: f32,
        client_time_ms: u32,
        ack: Acknowledgement,
    },

    /// A player is firing. Only the aim comes from them.
    Shoot {
        handle: Handle,
        angle: f32,
    },

    Chat {
        handle: Handle,
        text: String,
    },

    /// A player is stepping into a portal. The world answers with the portal's object type, or
    /// nothing if the player is not actually standing at one.
    UsePortal {
        handle: Handle,
        portal: hendra_net::EntityId,
        reply: tokio::sync::oneshot::Sender<Option<u16>>,
    },

    /// Places the account's gift chest, where the map marks one.
    ///
    /// A gift is not something the player put there, so it does not go in the vault: mixing them
    /// would let a full vault refuse a purchase that has already been paid for.
    PlaceGiftChest {
        slots: Vec<(u16, u16)>,
        reply: tokio::sync::oneshot::Sender<usize>,
    },

    /// Places the account's vault chests, one per `Vault` region the map marks.
    ///
    /// The world does not know about accounts, so the contents arrive with the request. Chests are
    /// rebuilt rather than updated: a vault changes when a player moves something, and rebuilding
    /// removes any question of the two drifting apart.
    PlaceVaultChests {
        /// Slot index to item type, for occupied slots only.
        slots: Vec<(u16, u16)>,
        /// How many chests the account has unlocked.
        unlocked: u16,
        reply: tokio::sync::oneshot::Sender<usize>,
    },

    /// Takes an item out of a bag, if it is still there and the player can reach it.
    ///
    /// The world answers with what it removed. Removing before the durable write is intentional:
    /// see the note on ordering in the session.
    TakeFromBag {
        player: Handle,
        bag: hendra_net::EntityId,
        slot: u8,
        reply: tokio::sync::oneshot::Sender<Option<u16>>,
    },

    /// Puts an item into a bag, or makes a new one at the player's feet.
    PutInBag {
        player: Handle,
        bag: Option<hendra_net::EntityId>,
        item: u16,

        /// Whether the bag this makes belongs to whoever dropped it.
        owned: bool,

        reply: tokio::sync::oneshot::Sender<bool>,
    },

    Leave {
        handle: Handle,
    },

    /// Uses an item, aimed at a point. The world answers with what it ran.
    UseItem {
        handle: Handle,
        item: ObjectType,
        aim: (f32, f32),
        reply: tokio::sync::oneshot::Sender<Vec<hendra_content::Effect>>,
    },

    /// Opens a portal where an entity is standing.
    OpenPortal {
        at: Handle,
        kind: ObjectType,
        duration_ms: u32,
    },

    /// The whole map, as one run-length encoded strip per row.
    Terrain {
        reply: tokio::sync::oneshot::Sender<Vec<TerrainStrip>>,
    },

    /// The map's scenery, as one list of `(x, object)` per row.
    ///
    /// Asked for separately from the ground because it is a different thing arriving over the same
    /// channel, and a map with no scenery should not pay for an empty list per row.
    Scenery {
        reply: tokio::sync::oneshot::Sender<Vec<SceneryRow>>,
    },

    /// A player wants to be where another player is. The world answers with why not, or nothing.
    Teleport {
        handle: Handle,
        to: String,
        reply: tokio::sync::oneshot::Sender<Option<String>>,
    },

    /// A player is buying a guild hall upgrade. The world answers with which one and its price.
    BuyHallUpgrade {
        handle: Handle,
        merchant: hendra_net::EntityId,
        reply: tokio::sync::oneshot::Sender<Option<HallUpgrade>>,
    },

    /// A player is buying from a merchant standing in the world. The world answers with what it is
    /// selling and for how much, since only the world knows which merchant that is.
    Buy {
        handle: Handle,
        merchant: hendra_net::EntityId,
        reply: tokio::sync::oneshot::Sender<Option<Sale>>,
    },

    /// Which of this world's keys have been found, for somebody who has just arrived.
    KeysFound {
        reply: tokio::sync::oneshot::Sender<Vec<String>>,
    },

    /// Stand a merchant for each of these listings, in the row its kind belongs to.
    ///
    /// The marketplace is the only world that asks. A listing nobody can walk up to is one that can
    /// only be found by typing, which is not what a marketplace is for.
    ShowListings {
        listings: Vec<Listed>,
    },

    /// Show a portal to each of these worlds, at the squares the map marks for them.
    ///
    /// The nexus is the only world that asks: it is the one place a player picks where to go, and
    /// the counts are what makes the choice mean anything.
    ShowPortals {
        portals: Vec<PortalSign>,
    },

    /// Send everybody here to another world, which is what a quake is.
    SendEveryoneTo {
        world: String,
    },

    /// An administrator's tool: do something to the world in front of them.
    ///
    /// One message rather than six, because each is a line of world state and the world is the only
    /// thing that can answer any of them.
    Wield {
        handle: Handle,
        what: Wielding,
        reply: tokio::sync::oneshot::Sender<String>,
    },

    /// The nearest quest, which is the most worthwhile enemy near a player.
    Quest {
        handle: Handle,
        reply: tokio::sync::oneshot::Sender<Option<(String, i32, i32)>>,
    },

    /// Puts a gravestone down where somebody died.
    Gravestone {
        at: (f32, f32),
        name: String,
        maxed: usize,
        level: i16,
        rekt: bool,
    },

    /// Says something to everybody in this world.
    Announce {
        text: String,
    },

    /// Where a player is standing.
    Where {
        handle: Handle,
        reply: tokio::sync::oneshot::Sender<Option<(i32, i32)>>,
    },

    /// A player's base stats, as the character has them.
    Stats {
        handle: Handle,
        reply: tokio::sync::oneshot::Sender<Option<[i32; 8]>>,
    },

    /// Who is in this world.
    Who {
        reply: tokio::sync::oneshot::Sender<Vec<String>>,
    },

    /// A line meant for one named player.
    Tell {
        to: String,
        from: String,
        text: String,
    },

    /// What a character has become, so it can be written down.
    ///
    /// Asked for rather than sent on leaving, because a session also checkpoints while playing and
    /// both paths want the same answer.
    Snapshot {
        handle: Handle,
        reply: tokio::sync::oneshot::Sender<Option<Vitals>>,
    },
}

/// Where a closed realm sends everybody still in it.
pub const CASTLE: &str = "Castle";

/// The simulation's tally as the database holds it.
///
/// Two types on purpose: one crosses a database and is all i32 and a bitset, and the simulation has
/// no business knowing either.
fn counted(tally: &hendra_sim::fame::Tally) -> hendra_store::TallyRow {
    hendra_store::TallyRow {
        shots: tally.shots,
        shots_that_hit: tally.shots_that_hit,
        abilities_used: tally.abilities_used,
        tiles_seen: tally.tiles_seen,
        teleports: tally.teleports,
        potions_drunk: tally.potions_drunk,
        monster_kills: tally.monster_kills,
        god_kills: tally.god_kills,
        cube_kills: tally.cube_kills,
        oryx_kills: tally.oryx_kills,
        quests_completed: tally.quests_completed,
        level_up_assists: tally.level_up_assists,
        dungeons_completed: tally.dungeons_completed as i64,
    }
}

/// A character that has died, as the session needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Departed {
    /// What to name as the killer.
    pub killer: String,

    /// Where the body fell, which is where the gravestone goes.
    pub x: f32,
    pub y: f32,
}

/// Something a world asks of one session.
///
/// A world can move bodies but not connections: which world a player is in belongs to the session
/// that owns their link, so the world asks rather than does. There is one of these today, and it is
/// the end of a realm.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    /// Go to this world.
    GoTo(String),
}

/// Puts a merchant on every square a shop's region marks.
///
/// One merchant per square, each holding one thing, with the shop's stock dealt out around them and
/// wrapped. That is what makes a row of nexus stalls show different items.
fn open_shops(world: &mut World, catalog: &Catalog) -> usize {
    use hendra_sim::shop;

    let Some(kind) = catalog.type_of(MERCHANT) else {
        tracing::warn!(
            merchant = MERCHANT,
            "the content has no merchant to stand in a shop"
        );
        return 0;
    };

    let mut opened = 0;
    let mut missing: Vec<&'static str> = Vec::new();

    for shop in shop::SHOPS {
        let mut places: Vec<(u32, u32)> = world
            .terrain()
            .map()
            .regions()
            .filter(|(_, _, region)| *region == shop.region)
            .map(|(x, y, _)| (x, y))
            .collect();
        places.sort();

        if places.is_empty() {
            continue;
        }

        let (stalls, absent) = shop::deal(shop, places.len(), catalog);
        missing.extend(absent);

        for ((x, y), stall) in places.iter().zip(stalls) {
            if world.open_stall(kind, *x, *y, stall).is_some() {
                opened += 1;
            }
        }
    }

    missing.sort_unstable();
    missing.dedup();
    for name in &missing {
        tracing::warn!(item = %name, "a shop sells something the content does not have");
    }

    opened
}

/// The object a shop merchant is.
const MERCHANT: &str = "Merchant";

/// What a merchant standing in the world is selling.
///
/// Read from the world rather than told by the client, because a client that names the item is a
/// client that can name a cheaper one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sale {
    pub item: ObjectType,
    pub price: i32,
    pub currency: hendra_store::Currency,

    /// The account rank needed to buy here at all.
    pub rank: i16,

    /// The market listing this is, where it is one. A shop's stock is endless; a player's listing
    /// is one item somebody else owns until it is bought, and the two are paid for differently.
    pub listing: Option<i64>,
}

/// What the merchant a player is standing at is selling.
///
/// `None` when they are not at one, when it is out of reach, or when it sells nothing: each is a
/// refusal rather than a mistake, and telling them apart would tell a client where merchants are.
fn world_sale(world: &World, handle: Handle, merchant: hendra_net::EntityId) -> Option<Sale> {
    let stall = world.get(Handle(merchant.0))?;
    let player = world.get(handle)?;

    // Within reach, as a portal is. Buying from across a map is buying from a shop you are not in.
    let (dx, dy) = (stall.x - player.x, stall.y - player.y);
    if (dx * dx + dy * dy).sqrt() > MERCHANT_REACH {
        return None;
    }

    let selling = stall.selling?;

    Some(Sale {
        item: selling.item,
        price: selling.price,
        currency: match selling.currency {
            hendra_sim::shop::Currency::Fame => hendra_store::Currency::Fame,
            hendra_sim::shop::Currency::Gold => hendra_store::Currency::Gold,
        },
        rank: selling.rank,
        listing: selling.listing,
    })
}

/// How close a player must be to buy from a merchant.
pub const MERCHANT_REACH: f32 = 8.0;

/// One row of the map: which row, and its squares run-length encoded as `(count, tile)`.
pub type TerrainStrip = (u16, Vec<(u16, u16)>);

/// One row's scenery: which row, and the objects standing in it as `(x, object, size)`.
pub type SceneryRow = (u16, Vec<(u16, u16, u16)>);

/// A character's live state, as the durable side needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vitals {
    pub hp: i32,
    pub mp: i32,
    pub level: i16,
    pub experience: i32,
    pub fame: i32,

    /// What the character did while it was here, which is added to what it had done before. Sent
    /// with the rest because it is written at the same moment and for the same reason.
    pub tally: hendra_store::TallyRow,
}

/// How close a player must be to reach into a bag, in tiles.
///
/// One, from `InvSwapHandler.cs:168`, which refuses a swap when the squared distance is above one.
/// Two tiles is four times the area to grab from, and loot bags are contested.
pub const BAG_REACH: f32 = 1.0;

/// How close a player must be to a portal to use it, in tiles.
///
/// Checked because the client names the portal it wants, and one that names a portal across the map
/// should be refused rather than obliged.
pub const PORTAL_REACH: f32 = 1.5;

/// Everything the world keeps about one connected player.
struct Player {
    handle: Handle,
    name: String,
    sender: LinkSender,

    /// How the world asks this session to do something only a session can do.
    orders: mpsc::Sender<Order>,

    /// How the world tells this session its character has died.
    ///
    /// Apart from `orders` because a death is not a request: the world has already ended the life
    /// and the session's job is to write it down, not to decide whether to.
    died: mpsc::Sender<Departed>,

    /// What this connection has been refused lately.
    ///
    /// Per connection rather than per account, because it is a judgement about a link: the same
    /// person on a better line is not the same case.
    strikes: crate::strikes::Strikes,

    /// When this player's movement starts being judged.
    ///
    /// A player arriving in a world has not been told where they are yet, and the claims they make
    /// in the meantime are measured from wherever they last stood — which, coming out of a portal,
    /// is a different world entirely. `Player.Verify` gives three seconds before it judges anything
    /// for the same reason.
    judge_moves_from: std::time::Instant,

    /// What this player has been sent, so a snapshot can be a delta against what they confirm.
    history: BaselineRing<WorldSnapshot>,
    encoder: SnapshotEncoder,
    acknowledged: Acknowledgement,

    /// Scratch, reused every tick.
    scratch: Vec<u8>,
}

/// Runs a world until its command channel closes.
pub async fn run(
    mut world: World,
    catalog: Arc<Catalog>,
    loadout: Loadout,
    mut inbox: mpsc::Receiver<ToWorld>,
) {
    let mut players: Vec<Player> = Vec::new();
    let mut metrics = TickMetrics::for_rate(TICKS_PER_SECOND);
    let elapsed_ms = 1000 / TICKS_PER_SECOND;

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(elapsed_ms as u64));
    // Skip rather than burst: a world that falls behind should not try to run the ticks it missed.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut reported = Instant::now();

    let loadout = &loadout;
    let persistent = loadout.persistent;

    // When the world last became empty, or `None` while somebody is in it.
    let mut emptied: Option<Instant> = None;

    // Shops stand wherever a map marks them, which is the nexus and the donor shop rather than any
    // one world, so this is asked of every world rather than gated on a name.
    let opened = open_shops(&mut world, &catalog);
    if opened > 0 {
        tracing::info!(world = %world.name, merchants = opened, "shops opened");
    }

    // Only the realm populates itself and closes on a clock. Every other world is the map it was
    // drawn as, which is what makes a dungeon a fixed set of rooms. The original decides the same
    // way: a world definition named `Realm` gets an overseer and nothing else does.
    let is_realm = loadout.is_realm && !loadout.spawnable.is_empty();

    if is_realm {
        // Setpieces first, then enemies, as `Realm.Init` does. The other way round would bury
        // whatever had already spawned under a castle.
        build_setpieces(&mut world, &catalog, &loadout.maps);

        let census = world.terrain().terrain_census();
        world.realm_mut().measure(&census);

        let opening = world.realm().opening();
        let (placed, _) = world.populate(&catalog, &loadout.spawnable, &opening);

        tracing::info!(
            world = %world.name,
            target = world.realm().population(),
            placed,
            "realm populated"
        );
    }

    loop {
        tokio::select! {
            command = inbox.recv() => {
                let Some(command) = command else { break };
                handle(&mut world, &catalog, loadout, &mut players, command);
            }

            _ = ticker.tick() => {
                let started = Instant::now();

                // A world nobody is in stops, so a night of dungeon-running does not leave a
                // hundred of them ticking. The entry world is exempt: it is where players arrive,
                // and a world that has to exist before anyone is in it cannot wait for one.
                if players.is_empty() {
                    if !persistent && emptied.is_none_or(|at: Instant| at.elapsed() >= IDLE_TIMEOUT)
                    {
                        if emptied.is_some() {
                            break;
                        }
                        emptied = Some(Instant::now());
                    }
                } else {
                    emptied = None;
                }

                world.advance(&catalog, elapsed_ms);

                if is_realm {
                    tend_realm(
                        &mut world,
                        &catalog,
                        &loadout.spawnable,
                        elapsed_ms as u64,
                        &players,
                    );
                }

                // Deaths first, because a death ends a session and everything after it this tick
                // would be sent to somebody who is no longer playing.
                for death in world.take_deaths() {
                    let Some(index) = players.iter().position(|p| p.handle == death.who) else {
                        continue;
                    };
                    let player = players.remove(index);

                    tracing::info!(
                        world = %world.name,
                        name = %player.name,
                        killer = %death.killer,
                        "a player died"
                    );

                    let _ = player.died.try_send(Departed {
                        killer: death.killer,
                        x: death.x,
                        y: death.y,
                    });
                }

                for name in world.take_unknown_setpieces() {
                    tracing::warn!(
                        world = %world.name,
                        setpiece = %name,
                        "a behaviour asked for a setpiece that does not exist"
                    );
                }

                announce(&mut world, &catalog, &mut players).await;
                broadcast(&mut world, &mut players).await;

                metrics.record(started.elapsed());

                // A minute of silence is not evidence of health, so say so periodically.
                if reported.elapsed().as_secs() >= 30 {
                    tracing::info!(
                        world = %world.name,
                        players = players.len(),
                        entities = world.len(),
                        healthy = metrics.healthy(),
                        "{metrics}"
                    );
                    reported = Instant::now();
                }
            }
        }
    }

    tracing::info!(world = %world.name, players = players.len(), "world stopped");
}

/// What the server falls back to when a character says nothing useful.
///
/// A character carries its own class, health and weapon, so this is only reached when the catalog
/// has no class at all. A content directory too broken to name an avatar should still let someone
/// connect and see the problem.
#[derive(Debug, Clone)]
pub struct Loadout {
    /// The object to use for each loot colour, in the content's own order.
    pub bag_types: Vec<ObjectType>,

    /// What may live in a realm. The same list for every world; only a realm uses it.
    pub spawnable: Vec<hendra_sim::realm::Spawn>,

    /// Whether this world is the realm, and so fills itself and closes on a clock.
    pub is_realm: bool,

    /// Where the map files are, for the setpieces that are saved maps rather than drawings.
    pub maps: std::path::PathBuf,

    /// Whether this world keeps ticking with nobody in it.
    ///
    /// True for the world players arrive in, which has to exist before anyone is there. False for
    /// everything else, which closes when it empties.
    pub persistent: bool,

    pub avatar: ObjectType,
    pub weapon: Option<ObjectType>,
}

/// The character that is arriving, as the world needs it.
///
/// Separate from the database row because the world has no business with experience, fame or the
/// account behind it. It needs a body, and this is the body.
#[derive(Debug, Clone, Copy)]
pub struct Arrival {
    pub avatar: ObjectType,

    /// How much likelier this account is to be given loot, from whatever boost it holds. One for
    /// everybody without one.
    pub loot_drop: f32,

    /// How many stars the account has earned, for everybody else to see beside the name.
    pub stars: u8,

    pub hp: i32,
    pub max_hp: i32,
    pub weapon: Option<ObjectType>,

    /// The eight stats the character brings, before equipment.
    pub stats: hendra_sim::stats::Stats,

    /// What the character is wearing, as a stat layer.
    pub boosts: [i32; 8],
}

/// Draws a realm's temples, castles, groves and graveyards into it.
///
/// Positions come from the terrain, so a castle stands on high ground and an oasis in the sand, and
/// no two are drawn through each other.
fn build_setpieces(world: &mut World, catalog: &Catalog, maps: &Path) {
    use hendra_sim::setpiece;

    // Seeded from the clock, so two realms opened in one run are not the same realm. The drawing
    // itself is deterministic given the seed, which is what makes it testable.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() ^ since.as_secs() as u32)
        .unwrap_or(0x5eed);
    let mut dice = setpiece::Dice::new(seed);

    let (width, height) = (world.terrain().width(), world.terrain().height());
    let ground = |x: u32, y: u32| world.terrain().terrain_at(x, y);
    let places = setpiece::scatter(width, height, &ground, &mut dice);

    let mut missing: Vec<&'static str> = Vec::new();
    let mut drawn = 0;

    for place in &places {
        let drawing = place.kind.draw(&mut dice);

        // A setpiece that is a saved map is stamped rather than painted. `stamp` centres what it
        // places, and a setpiece is placed by its corner, so the point is moved to its middle.
        if let Some(name) = drawing.prefab {
            // The extension depends on how far the content has been converted, so all three are
            // tried rather than one being assumed.
            let found = ["hmap", "jm", "wmap"].iter().find_map(|extension| {
                let file = format!("{name}.{extension}");
                let raw = std::fs::read(maps.join(&file)).ok()?;
                crate::worlds::load_map_bytes(&raw, &file, catalog)
            });

            let Some(map) = found else {
                tracing::warn!(setpiece = %name, path = %maps.display(), "cannot read a setpiece map");
                continue;
            };

            let half = place.kind.size() as f32 / 2.0;
            world.stamp(
                catalog,
                &map,
                (place.x as f32 + half, place.y as f32 + half),
            );
            drawn += 1;
            continue;
        }

        missing.extend(world.draw(catalog, &drawing, (place.x, place.y)));
        drawn += 1;
    }

    missing.sort_unstable();
    missing.dedup();
    for name in &missing {
        tracing::warn!(name = %name, "a setpiece wants something the content does not have");
    }

    if world.refused_squares() > 0 {
        tracing::warn!(
            world = %world.name,
            squares = world.refused_squares(),
            "the map cannot describe any more kinds of square; setpieces have holes in them"
        );
    }

    tracing::info!(world = %world.name, setpieces = drawn, "setpieces drawn");
}

/// Sends everybody still in a closed realm to the castle.
///
/// The world can move a body but not a connection: which world somebody is in belongs to the
/// session that owns their link, so the world asks and the session does it. Asked of everybody,
/// including anybody whose session is too busy to hear right now, because leaving one player behind
/// in a realm that has stopped spawning is leaving them in an empty map.
fn send_to_castle(players: &[Player]) {
    for player in players {
        if player
            .orders
            .try_send(Order::GoTo(CASTLE.to_string()))
            .is_err()
        {
            tracing::warn!(name = %player.name, "could not send a player to the castle");
        }
    }
}

/// Keeps a realm's population up and runs its closing sequence.
///
/// Population is checked once a minute rather than every tick: placing an enemy is a search for a
/// square of the right terrain with nobody near it, and doing that for a whole realm every tick
/// would be the longest tick the world ever had.
fn tend_realm(
    world: &mut World,
    catalog: &Catalog,
    spawnable: &[hendra_sim::realm::Spawn],
    elapsed_ms: u64,
    players: &[Player],
) {
    use hendra_sim::realm::Event;

    let Some(event) = world.realm_mut().advance(elapsed_ms) else {
        return;
    };

    match event {
        Event::Ensure => {
            let alive = world.alive_by_terrain();
            let wanted = world.realm().adjustments(&alive);
            if wanted.is_empty() {
                return;
            }

            let (added, removed) = world.populate(catalog, spawnable, &wanted);
            tracing::debug!(world = %world.name, added, removed, "realm population checked");
        }

        Event::Warned => {
            world.announce("Realm closing in 1 minute.");
        }

        Event::Closed => {
            world.announce("I HAVE CLOSED THIS REALM!");
            world.announce("YOU WILL NOT LIVE TO SEE THE LIGHT OF DAY!");
        }

        Event::Castle => {
            world.announce("MY MINIONS HAVE FAILED ME!");
            world.announce("BUT NOW YOU SHALL FEEL MY WRATH!");
            world.announce("COME MEET YOUR DOOM AT THE WALLS OF MY CASTLE!");

            send_to_castle(players);
        }
    }
}

fn handle(
    world: &mut World,
    catalog: &Catalog,
    loadout: &Loadout,
    players: &mut Vec<Player>,
    command: ToWorld,
) {
    match command {
        ToWorld::Join {
            name,
            arrival,
            sender,
            orders,
            died,
            reply,
        } => {
            // A closed realm is about to be emptied. Letting somebody in now would be putting them
            // straight into the quake back out of it. Dropping the reply is how the session hears
            // no, and it leaves the player where they were.
            if !world.realm().admits() {
                tracing::info!(world = %world.name, "refusing a join: the realm has closed");
                return;
            }

            let (x, y) = spawn_point(world);

            // The character decides all of this. The loadout is only what to do when it named a
            // class the catalog does not have.
            let avatar = if arrival.avatar == ObjectType::NONE {
                loadout.avatar
            } else {
                arrival.avatar
            };
            let max_hp = arrival.max_hp.max(1);

            let mut entity = Entity::player(avatar, x, y, max_hp);
            entity.stats = arrival.stats;
            entity.loot_drop = arrival.loot_drop;
            entity.stars = arrival.stars;
            entity.stats.set_equipment(arrival.boosts);
            entity.hp = arrival.hp.clamp(1, max_hp);
            entity.name = Some(name.as_str().into());
            entity.weapon = arrival.weapon.or(loadout.weapon);

            // A moment before anything will attack them. Somebody who has just walked through a
            // portal has not seen what is in the room yet.
            entity.unseen_ms = hendra_sim::world::NEWCOMER_GRACE_MS;

            let Some(handle) = world.spawn(entity) else {
                tracing::warn!(world = %world.name, "world is full; refusing a join");
                return;
            };

            // The ground around where they arrived counts as seen, without waiting for them to take
            // a step. A player who walks into a room and stands still has still looked at it.
            world.look_around(handle);

            // The encoder takes its budget from the connection rather than the default, because the
            // negotiated limit is what actually governs whether a datagram is accepted.
            let budget = sender
                .max_datagram_size()
                .unwrap_or(hendra_net::DATAGRAM_BUDGET);

            players.push(Player {
                handle,
                name: name.clone(),
                sender,
                orders,
                died,
                strikes: crate::strikes::Strikes::new(),
                judge_moves_from: std::time::Instant::now() + MOVE_GRACE,
                history: BaselineRing::new(),
                encoder: SnapshotEncoder::with_budget(budget),
                acknowledged: Acknowledgement::NONE,
                scratch: Vec::new(),
            });

            tracing::info!(world = %world.name, %name, ?handle, "player joined");
            let _ = reply.send(handle);
        }

        ToWorld::Input {
            handle,
            x,
            y,
            client_time_ms,
            ack,
        } => {
            if let Some(player) = players.iter_mut().find(|player| player.handle == handle) {
                player.acknowledged = ack;
            }

            // The claim is advisory. The world decides where the player actually is.
            if let Some(outcome) =
                world.resolve_move(handle, catalog, x, y, tick_ms(client_time_ms))
            {
                // Movement is not struck on at all, and the reason is in the line above: the
                // claim is advisory and the world decides. A clamped claim cannot cheat — the
                // player ends up where the server says whatever they asked for — so counting
                // refusals buys nothing and costs sessions.
                //
                // It cost three. Walking into a wall is refused every tick while the key is held.
                // Arriving in a world, the first claims are measured from the world just left. And
                // a claim that is merely *ahead* of the server is refused for every message it
                // takes the clamp to catch up, which is the ordinary way a client and a server
                // reconcile and looked exactly like cheating to a counter that only saw refusals.
                //
                // The original needed this check because its client reported its own position and
                // was believed. Ours is not believed, so the check is not load-bearing.
                if let Some(why) = outcome.refused {
                    tracing::debug!(
                        world = %world.name,
                        ?why,
                        claimed = ?(x, y),
                        server = ?(outcome.x, outcome.y),
                        "a move was clamped"
                    );
                }

                world.place(handle, outcome);
            }
        }

        ToWorld::Shoot { handle, angle } => {
            // The world decides whether the weapon is ready, where the shot goes and what it hits.
            // A client that asks faster than its rate of fire is refused, not believed.
            let fired = world.shoot(handle, catalog, angle);
            if fired.is_empty() {
                return;
            }

            let mut buf = Vec::new();
            for projectile in fired {
                let Some(shot) = world
                    .projectiles()
                    .find(|(handle, _)| *handle == projectile)
                else {
                    continue;
                };
                let (_, shot) = shot;

                buf.clear();
                ServerMessage::Shot {
                    projectile: projectile.to_entity_id(),
                    owner: handle.to_entity_id(),
                    object_type: shot.object_type.0,
                    x: shot.x,
                    y: shot.y,
                    angle: shot.angle,
                    speed: shot.speed,
                    lifetime_ms: shot.lifetime_ms,
                }
                .encode(&mut Writer::new(&mut buf));

                // Reliable: a shot nobody was told about is a bullet that appears to do damage from
                // nowhere. It is one message for the projectile's whole life, so the cost is small.
                for player in players.iter() {
                    let _ = player.sender.try_send(Delivery::Stream, &buf);
                }
            }
        }

        ToWorld::OpenPortal {
            at,
            kind,
            duration_ms,
        } => {
            world.open_portal(catalog, at, kind, duration_ms);
        }

        ToWorld::Scenery { reply } => {
            let map = world.terrain().map();

            let mut rows: Vec<SceneryRow> = Vec::new();
            let mut current: Option<SceneryRow> = None;

            // The map hands its objects back in row order, so the rows are gathered as they come
            // rather than by scanning the map once per row.
            for (x, y, square) in map.objects() {
                if !World::is_scenery(catalog, square) {
                    continue;
                }

                let placed = (
                    x as u16,
                    square.object.0,
                    square.size().unwrap_or(0).clamp(0, u16::MAX as i32) as u16,
                );

                match current.as_mut() {
                    Some((at, objects)) if *at == y as u16 => objects.push(placed),
                    _ => {
                        if let Some(row) = current.take() {
                            rows.push(row);
                        }
                        current = Some((y as u16, vec![placed]));
                    }
                }
            }
            rows.extend(current);

            let _ = reply.send(rows);
        }

        ToWorld::Terrain { reply } => {
            let terrain = world.terrain();
            let strips = (0..terrain.height())
                .map(|y| (y as u16, terrain.row_runs(y, 0, terrain.width())))
                .collect();
            let _ = reply.send(strips);
        }

        ToWorld::Teleport { handle, to, reply } => {
            let answer = match world.player_named(&to) {
                Some(target) => world.teleport_to(handle, target).map(|why| why.to_string()),
                None => Some(format!("{to} is not here.")),
            };

            let _ = reply.send(answer);
        }

        ToWorld::BuyHallUpgrade {
            handle,
            merchant,
            reply,
        } => {
            let _ = reply.send(hall_upgrade(world, catalog, handle, merchant));
        }

        ToWorld::Buy {
            handle,
            merchant,
            reply,
        } => {
            let _ = reply.send(world_sale(world, handle, merchant));
        }

        ToWorld::KeysFound { reply } => {
            let _ = reply.send(world.keys_found());
        }

        ToWorld::ShowListings { listings } => {
            show_listings(world, catalog, &listings);
        }

        ToWorld::ShowPortals { portals } => {
            show_portals(world, catalog, &portals);
        }

        ToWorld::SendEveryoneTo { world: destination } => {
            for player in players.iter() {
                let _ = player.orders.try_send(Order::GoTo(destination.clone()));
            }
        }

        ToWorld::Wield {
            handle,
            what,
            reply,
        } => {
            let _ = reply.send(wield_in_world(world, catalog, handle, what));
        }

        ToWorld::Quest { handle, reply } => {
            let _ = reply.send(nearest_quest(world, catalog, handle));
        }

        ToWorld::Gravestone {
            at,
            name,
            maxed,
            level,
            rekt,
        } => {
            world.place_gravestone(catalog, at, &name, maxed, level, rekt);
        }

        ToWorld::Announce { text } => world.announce(&text),

        ToWorld::Where { handle, reply } => {
            let at = world
                .get(handle)
                .map(|entity| (entity.x as i32, entity.y as i32));
            let _ = reply.send(at);
        }

        ToWorld::Stats { handle, reply } => {
            // Base stats rather than what equipment makes them, because "how far from maximum" is a
            // question about the character, and a ring can be taken off.
            let held = world.get(handle).map(|entity| {
                let mut base = [0i32; 8];
                for (index, stat) in hendra_content::STATS.iter().enumerate() {
                    base[index] = entity.stats.base(*stat);
                }
                base
            });
            let _ = reply.send(held);
        }

        ToWorld::Who { reply } => {
            let mut names: Vec<String> = players.iter().map(|player| player.name.clone()).collect();
            names.sort();
            let _ = reply.send(names);
        }

        ToWorld::Tell { to, from, text } => {
            let Some(target) = players
                .iter()
                .find(|player| player.name.eq_ignore_ascii_case(&to))
            else {
                // Answered to nobody: the sender is told by its own session, which is the only
                // side that knows whether the name exists anywhere else.
                return;
            };

            let mut buf = Vec::new();
            ServerMessage::Chat {
                from: &format!("{from} whispers"),
                text: &text,
            }
            .encode(&mut Writer::new(&mut buf));

            let _ = target.sender.try_send(Delivery::Stream, &buf);
        }

        ToWorld::Chat { handle, text } => {
            let Some(speaker) = players
                .iter()
                .find(|player| player.handle == handle)
                .map(|player| player.name.clone())
            else {
                return;
            };

            // Enemies hear it too. A dungeon whose door opens when you say the right word is built
            // out of this, and without it the door has no handle.
            if let Some(at) = world.get(handle).map(|entity| (entity.x, entity.y)) {
                world.heard(at, &text);
            }

            let mut buf = Vec::new();
            ServerMessage::Chat {
                from: &speaker,
                text: &text,
            }
            .encode(&mut Writer::new(&mut buf));

            for player in players.iter() {
                // Chat must arrive, so it goes reliably, and a full queue means that player is not
                // keeping up rather than that the message should be dropped for everyone.
                let _ = player.sender.try_send(Delivery::Stream, &buf);
            }
        }

        ToWorld::UsePortal {
            handle,
            portal,
            reply,
        } => {
            let _ = reply.send(resolve_portal(world, handle, portal));
        }

        ToWorld::PlaceGiftChest { slots, reply } => {
            let _ = reply.send(place_gift_chest(world, catalog, &slots));
        }

        ToWorld::PlaceVaultChests {
            slots,
            unlocked,
            reply,
        } => {
            let _ = reply.send(place_vault_chests(world, catalog, &slots, unlocked));
        }

        ToWorld::TakeFromBag {
            player,
            bag,
            slot,
            reply,
        } => {
            let _ = reply.send(take_from_bag(world, player, bag, slot));
        }

        ToWorld::PutInBag {
            player,
            bag,
            item,
            owned,
            reply,
        } => {
            let _ = reply.send(put_in_bag(world, catalog, player, bag, item, owned));
        }

        ToWorld::Equipment { handle, boosts } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.stats.set_equipment(boosts);
            }
        }

        ToWorld::UseItem {
            handle,
            item,
            aim,
            reply,
        } => {
            let ran = world.use_item(handle, catalog, item, aim);
            let _ = reply.send(ran);
        }

        ToWorld::Snapshot { handle, reply } => {
            let vitals = world.get(handle).map(|entity| Vitals {
                hp: entity.hp,
                mp: entity.mp,
                level: entity.progress.level,
                experience: entity.progress.experience,
                fame: entity.progress.fame,
                tally: counted(&entity.tally),
            });
            let _ = reply.send(vitals);
        }

        ToWorld::Leave { handle } => {
            players.retain(|player| player.handle != handle);
            world.despawn(handle);
            tracing::info!(world = %world.name, ?handle, "player left");
        }
    }
}

/// Rebuilds the gift chest from what the account has been sent.
///
/// One chest, wherever the map marks a gifting square, holding everything waiting. A prestige
/// purchase or an administrator's gift arrives in the store; without this there is nowhere to open
/// it from, which is a player owed something they cannot reach.
fn place_gift_chest(world: &mut World, catalog: &Catalog, slots: &[(u16, u16)]) -> usize {
    let chest_type = catalog
        .type_of(GIFT_CHEST)
        .or_else(|| catalog.type_of("Vault Chest"));

    let Some(chest_type) = chest_type else {
        tracing::warn!(chest = GIFT_CHEST, "the content has no gift chest");
        return 0;
    };

    let mut places: Vec<(u32, u32)> = world
        .terrain()
        .map()
        .regions()
        .filter(|(_, _, region)| *region == hendra_content::Region::GiftingChest)
        .map(|(x, y, _)| (x, y))
        .collect();
    places.sort();

    if places.is_empty() {
        return 0;
    }

    // Anything left from a previous visit goes first, so nothing survives that the store no longer
    // says is there. Recognised by where it stands, since a container is a container.
    let existing: Vec<Handle> = world
        .iter()
        .filter(|(_, entity)| {
            entity.kind == hendra_sim::Kind::Container
                && places
                    .iter()
                    .any(|(x, y)| entity.x == *x as f32 + 0.5 && entity.y == *y as f32 + 0.5)
        })
        .map(|(handle, _)| handle)
        .collect();
    for handle in existing {
        world.despawn(handle);
    }

    let mut placed = 0;
    for (x, y) in &places {
        // Take-only, which is what the original's `OneWayContainer` is for: a gift is something the
        // server put there, and a chest you could also put things into would be extra vault space
        // nobody paid for.
        let mut container = hendra_sim::Container::new(
            hendra_sim::ContainerKind::Merchant,
            hendra_store::GIFT_SLOTS.max(0) as usize,
        );
        for (slot, item) in slots {
            container.set(*slot as usize, hendra_content::ObjectType(*item));
        }

        let mut chest =
            hendra_sim::world::Entity::fixture(chest_type, *x as f32 + 0.5, *y as f32 + 0.5);
        chest.kind = hendra_sim::Kind::Container;
        chest.container = Some(Box::new(container));

        if world.spawn(chest).is_some() {
            placed += 1;
        }
    }

    placed
}

/// One thing a player has for sale, as the marketplace needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub listing: i64,
    pub item: ObjectType,
    pub price: i32,

    /// Which row it stands in, from what kind of thing it is.
    pub slot_type: i32,
}

/// Stands a merchant for each listing, in the row its kind belongs to.
///
/// Replaced outright each refresh, like the nexus portals: the set changes whenever anybody buys or
/// lists, and a merchant left standing for something already sold is a stall that refuses everybody
/// who walks up to it.
fn show_listings(world: &mut World, catalog: &Catalog, listings: &[Listed]) {
    use hendra_sim::shop;

    let Some(kind) = catalog.type_of(MERCHANT) else {
        return;
    };

    for (region, wanted) in shop::MARKET_ROWS {
        let mut places: Vec<(u32, u32)> = world
            .terrain()
            .map()
            .regions()
            .filter(|(_, _, held)| held == region)
            .map(|(x, y, _)| (x, y))
            .collect();
        places.sort();

        if places.is_empty() {
            continue;
        }

        let standing: Vec<Handle> = world
            .iter()
            .filter(|(_, entity)| {
                entity.selling.is_some()
                    && places
                        .iter()
                        .any(|(x, y)| entity.x == *x as f32 + 0.5 && entity.y == *y as f32 + 0.5)
            })
            .map(|(handle, _)| handle)
            .collect();
        for handle in standing {
            world.despawn(handle);
        }

        // Only what belongs in this row, so a marketplace reads as rows of a kind rather than one
        // heap somebody has to search.
        let mine = listings
            .iter()
            .filter(|listed| slot_row(listed.slot_type) == *wanted);

        for ((x, y), listed) in places.iter().zip(mine) {
            world.open_stall(
                kind,
                *x,
                *y,
                shop::Stall {
                    item: listed.item,
                    price: listed.price,
                    currency: hendra_sim::shop::Currency::Fame,
                    rank: 0,
                    listing: Some(listed.listing),
                },
            );
        }
    }
}

/// Which marketplace row a slot type belongs in.
///
/// The content numbers slots and the marketplace groups them, so this is the one place that knows
/// a wand and a bow are both weapons.
fn slot_row(slot_type: i32) -> &'static str {
    match slot_type {
        1 | 2 | 3 | 8 | 17 | 24 => "Weapon",
        4 | 5 | 11 | 12 | 13 | 15 | 16 | 18 | 19 | 20 | 21 | 22 | 23 | 25 => "Ability",
        6 | 14 | 27 => "Armor",
        9 => "Ring",
        10 | 26 => "Potion",
        _ => "Other",
    }
}

/// One world a nexus portal leads to, and how busy it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalSign {
    /// The world's name, which is what the portal is labelled with.
    pub world: String,

    /// The portal object that leads there.
    pub portal: ObjectType,

    /// How many are in it, which is the whole reason the label is worth reading.
    pub players: usize,
}

/// Puts a portal for each named world on the squares the map marks for them.
///
/// Replaced outright each time rather than reconciled: the list is short, it changes every few
/// seconds anyway, and a portal that lingered after its world closed is a door into nothing.
fn show_portals(world: &mut World, catalog: &Catalog, portals: &[PortalSign]) {
    let mut places: Vec<(u32, u32)> = world
        .terrain()
        .map()
        .regions()
        .filter(|(_, _, region)| *region == hendra_content::Region::RealmPortals)
        .map(|(x, y, _)| (x, y))
        .collect();
    places.sort();

    if places.is_empty() {
        return;
    }

    let standing: Vec<Handle> = world
        .iter()
        .filter(|(_, entity)| {
            entity.kind == hendra_sim::Kind::Portal
                && places
                    .iter()
                    .any(|(x, y)| entity.x == *x as f32 + 0.5 && entity.y == *y as f32 + 0.5)
        })
        .map(|(handle, _)| handle)
        .collect();
    for handle in standing {
        world.despawn(handle);
    }

    for ((x, y), sign) in places.iter().zip(portals) {
        let Some(desc) = catalog.object(sign.portal) else {
            continue;
        };

        let mut portal =
            hendra_sim::world::Entity::fixture(desc.object_type, *x as f32 + 0.5, *y as f32 + 0.5);
        portal.kind = hendra_sim::Kind::Portal;

        // Labelled with the count, which is what somebody choosing a realm is actually reading.
        portal.name = Some(format!("{} ({})", sign.world, sign.players).into());
        world.spawn(portal);
    }
}

/// Where a player's quest arrow points, and what it is called.
///
/// Read from the world rather than worked out here, because the world is what remembers it: killing
/// the enemy the arrow pointed at counts toward a character's fame and is worth five times the usual
/// experience, and neither can be answered from a target recomputed after the fact.
fn nearest_quest(world: &World, catalog: &Catalog, handle: Handle) -> Option<(String, i32, i32)> {
    let target = world.quest_target(handle)?;
    let entity = world.get(target)?;
    let desc = catalog.object(entity.object_type)?;

    Some((desc.id.clone(), entity.x as i32, entity.y as i32))
}

/// What an administrator is doing to the world in front of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wielding {
    /// Put an enemy of this name in front of them, this many times.
    Spawn { name: String, count: usize },

    /// Kill every enemy of this name here.
    KillAll { name: String },

    /// Draw them at this percentage of their natural size.
    Size { percent: u16 },

    /// Become invisible, or stop being.
    Hide,

    /// Close this realm now, wherever its clock had got to.
    CloseRealm,

    /// Stop, and be left alone, until they move.
    Pause,

    /// Give the caller a condition by name.
    Effect { name: String },

    /// Set what colour the caller glows.
    Glow { colour: i32 },

    /// Kill a named player's character where it stands.
    KillPlayer { name: String },

    /// Bring everybody here to the caller.
    SummonAll,

    /// Draw a setpiece where the caller stands.
    Setpiece { name: String },

    /// Clear every enemy here.
    ClearSpawn,

    /// What the world is doing.
    Debug,
}

/// Carries out an administrator's tool, and says what happened.
fn wield_in_world(world: &mut World, catalog: &Catalog, handle: Handle, what: Wielding) -> String {
    match what {
        Wielding::Spawn { name, count } => {
            let Some(kind) = catalog.type_of(&name) else {
                return format!("there is no {name}");
            };
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "you are nowhere".to_string();
            };

            // Bounded, because a typo in a number should not be the last thing a world does.
            let count = count.clamp(1, MOST_SPAWNED_AT_ONCE);
            let made = world.spawn_at(catalog, kind, x, y, count);

            format!("{made} {name}")
        }

        Wielding::KillAll { name } => {
            let Some(kind) = catalog.type_of(&name) else {
                return format!("there is no {name}");
            };

            let doomed: Vec<Handle> = world
                .iter()
                .filter(|(_, entity)| {
                    entity.kind == hendra_sim::Kind::Enemy && entity.object_type == kind
                })
                .map(|(handle, _)| handle)
                .collect();

            let killed = doomed.len();
            for handle in doomed {
                if let Some(entity) = world.get_mut(handle) {
                    // Removed rather than killed, so an administrator clearing a room does not hand
                    // out the experience and the loot for it.
                    entity.dead = true;
                    entity.no_experience = true;
                }
            }

            format!("{killed} killed")
        }

        Wielding::Size { percent } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.size = percent.clamp(1, 1000);
            }
            format!("drawn at {percent}%")
        }

        Wielding::Hide => {
            let hidden = world
                .get(handle)
                .is_some_and(|entity| entity.conditions.contains(HIDDEN));

            if let Some(entity) = world.get_mut(handle) {
                if hidden {
                    entity.conditions.remove(HIDDEN);
                } else {
                    entity.conditions.insert(HIDDEN);
                }
            }

            if hidden { "seen again" } else { "hidden" }.to_string()
        }

        Wielding::Pause => {
            // The same condition a paused player has, which is what makes it mean something: a
            // paused player is not hit, does not hit, and is not somewhere anybody can teleport to.
            let paused = world
                .get(handle)
                .is_some_and(|entity| entity.conditions.contains(PAUSED));

            if let Some(entity) = world.get_mut(handle) {
                if paused {
                    entity.conditions.remove(PAUSED);
                } else {
                    entity.conditions.insert(PAUSED);
                }
            }

            if paused { "moving again" } else { "paused" }.to_string()
        }

        Wielding::Effect { name } => {
            let Some(effect) = hendra_content::ConditionEffect::from_name(&name) else {
                return format!("there is no {name}");
            };

            if let Some(entity) = world.get_mut(handle) {
                entity.conditions.insert(effect);
            }
            format!("{name} given")
        }

        Wielding::Glow { colour } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.glow = colour;
            }
            "glowing".to_string()
        }

        Wielding::KillPlayer { name } => {
            let Some(target) = world.player_named(&name) else {
                return format!("{name} is not here");
            };

            if let Some(entity) = world.get_mut(target) {
                entity.hp = 0;
                entity.dead = true;
            }
            format!("{name} killed")
        }

        Wielding::SummonAll => {
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "you are nowhere".to_string();
            };

            // Anybody hidden is left where they are, as the original leaves them: somebody who has
            // made themselves invisible has said where they want to be.
            let summoned: Vec<Handle> = world
                .iter()
                .filter(|(held, entity)| {
                    *held != handle
                        && entity.kind == hendra_sim::Kind::Player
                        && !entity.conditions.contains(HIDDEN)
                })
                .map(|(held, _)| held)
                .collect();

            let count = summoned.len();
            for held in summoned {
                if let Some(entity) = world.get_mut(held) {
                    entity.x = x;
                    entity.y = y;
                    entity.move_grace_ms = hendra_sim::world::MOVE_GRACE_MS;
                }
            }

            format!("{count} summoned")
        }

        Wielding::Setpiece { name } => {
            let Some(kind) = hendra_sim::setpiece::Kind::named(&name) else {
                return format!("there is no {name}");
            };
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "you are nowhere".to_string();
            };

            let half = kind.size() as f32 / 2.0;
            let at = ((x - half).max(0.0) as u32, (y - half).max(0.0) as u32);

            let mut dice = hendra_sim::setpiece::Dice::new(at.0 ^ at.1 ^ 0x5e7);
            let drawing = kind.draw(&mut dice);
            let missing = world.draw(catalog, &drawing, at);

            if missing.is_empty() {
                format!("{name} drawn")
            } else {
                format!("{name} drawn, without {}", missing.join(", "))
            }
        }

        Wielding::ClearSpawn => {
            let doomed: Vec<Handle> = world
                .iter()
                .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Enemy && !entity.dead)
                .map(|(handle, _)| handle)
                .collect();

            let count = doomed.len();
            for handle in doomed {
                if let Some(entity) = world.get_mut(handle) {
                    entity.dead = true;
                    entity.no_experience = true;
                }
            }

            format!("{count} cleared")
        }

        Wielding::Debug => format!(
            "{}: {} entities, {} of them enemies, tick {}",
            world.name,
            world.len(),
            world.enemy_count(),
            world.tick_number().0
        ),

        Wielding::CloseRealm => {
            if world.realm().phase() == hendra_sim::realm::Phase::Open {
                world.realm_mut().close_now();
                "closing".to_string()
            } else {
                "this realm is already closing".to_string()
            }
        }
    }
}

/// What being paused is, which is the same condition the game's own pause gives.
const PAUSED: hendra_content::ConditionEffect = hendra_content::ConditionEffect::Paused;

/// What being hidden is, which is the same invisibility a cloak gives.
const HIDDEN: hendra_content::ConditionEffect = hendra_content::ConditionEffect::Invisible;

/// The most one `/spawn` may put down.
///
/// A typo in a number should not be the last thing a world does.
const MOST_SPAWNED_AT_ONCE: usize = 100;

/// What a guild-hall merchant standing in the world is selling.
///
/// Read from the object rather than told by the client, for the same reason a shop's stock is: a
/// client that names the upgrade can name a cheaper one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HallUpgrade {
    /// The level the hall becomes.
    pub level: i16,
    pub price: i32,
}

/// What the guild merchant a player is standing at is offering.
fn hall_upgrade(
    world: &World,
    catalog: &Catalog,
    handle: Handle,
    merchant: hendra_net::EntityId,
) -> Option<HallUpgrade> {
    let stall = world.get(Handle(merchant.0))?;
    let player = world.get(handle)?;

    let (dx, dy) = (stall.x - player.x, stall.y - player.y);
    if (dx * dx + dy * dy).sqrt() > MERCHANT_REACH {
        return None;
    }

    let desc = catalog.object(stall.object_type)?;
    if desc.guild_item.as_deref() != Some("GuildHallUpgrade") {
        return None;
    }

    // The content names the hall it becomes rather than the number, so the number is read out of
    // the name: "Guild Hall 2" is level one, since level zero is the hall a new guild starts with.
    let level = desc
        .guild_item_param
        .as_deref()?
        .rsplit(char::is_whitespace)
        .next()?
        .parse::<i16>()
        .ok()?
        - 1;

    Some(HallUpgrade {
        level,
        price: desc.price.unwrap_or(0),
    })
}

/// What the gift chest is called in the content.
const GIFT_CHEST: &str = "Gift Chest";

/// How many slots one vault chest holds.
pub const CHEST_SLOTS: u16 = 8;

/// Rebuilds the vault chests from an account's stored items.
///
/// The map marks where chests stand, and the vault map marks exactly one square. So one chest holds
/// the whole vault rather than eight slots of it, which is also how the vault reads to a player:
/// one place you go, with everything in it. When a map marks several the vault is split across them
/// in order, so a map that wants a row of chests gets one.
///
/// Returns how many chests were placed.
fn place_vault_chests(
    world: &mut World,
    catalog: &Catalog,
    slots: &[(u16, u16)],
    unlocked: u16,
) -> usize {
    // Anything left from a previous visit goes, so nothing survives that the vault no longer says.
    let existing: Vec<Handle> = world
        .iter()
        .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Container)
        .map(|(handle, _)| handle)
        .collect();
    for handle in existing {
        world.despawn(handle);
    }

    let chest_type = catalog
        .type_of("Vault Chest")
        .unwrap_or(hendra_content::ObjectType(0x0504));

    // Where the map says a chest stands. Sorted so the same square is always the same chest number,
    // which is what makes "my third chest" mean anything between visits.
    let mut places: Vec<(u32, u32)> = world
        .terrain()
        .map()
        .regions()
        .filter(|(_, _, region)| *region == hendra_content::Region::Vault)
        .map(|(x, y, _)| (x, y))
        .collect();
    places.sort();

    if places.is_empty() {
        tracing::warn!("this map marks no vault squares, so there is nowhere to put a chest");
        return 0;
    }

    // Every slot the account has paid for, shared out across however many squares the map offers.
    let total = (unlocked as usize) * (CHEST_SLOTS as usize);
    let per_chest = total.div_ceil(places.len()).max(CHEST_SLOTS as usize);

    let mut placed = 0usize;
    for (index, (x, y)) in places.iter().enumerate() {
        let first = (index * per_chest) as u16;
        let capacity = per_chest.min(total.saturating_sub(index * per_chest));
        if capacity == 0 {
            break;
        }

        let mut container = hendra_sim::Container::new(hendra_sim::ContainerKind::Vault, capacity);
        for (slot, item) in slots {
            if *slot >= first && (*slot as usize) < first as usize + capacity {
                container.set((slot - first) as usize, hendra_content::ObjectType(*item));
            }
        }

        let mut chest =
            hendra_sim::world::Entity::fixture(chest_type, *x as f32 + 0.5, *y as f32 + 0.5);
        chest.kind = hendra_sim::Kind::Container;
        chest.container = Some(Box::new(container));

        if world.spawn(chest).is_some() {
            placed += 1;
        }
    }

    placed
}

/// Removes an item from a bag the player can reach.
fn take_from_bag(
    world: &mut World,
    player: Handle,
    bag: hendra_net::EntityId,
    slot: u8,
) -> Option<u16> {
    let handle = Handle::from_entity_id(bag);
    if !within_reach(world, player, handle) {
        return None;
    }

    let entity = world.get_mut(handle)?;
    if entity.kind != hendra_sim::Kind::Container {
        return None;
    }

    // A bag that belongs to somebody belongs to them. Enforced here rather than only drawn
    // differently, because a bag anybody could take from would make the damage threshold decide who
    // the loot was rolled for and nothing at all about who ends up with it.
    if entity.belongs_to.is_some_and(|owner| owner != player) {
        return None;
    }

    let container = entity.container.as_mut()?;
    let item = container.item(slot as usize);
    if item.is_none() {
        return None;
    }

    container.set(slot as usize, hendra_content::ObjectType::NONE);

    // An emptied bag goes rather than sitting there inviting a second look.
    if container.occupied() == 0 {
        entity.dead = true;
    }

    Some(item.0)
}

/// Puts an item into a bag, creating one at the player's feet if none was named.
fn put_in_bag(
    world: &mut World,
    catalog: &Catalog,
    player: Handle,
    bag: Option<hendra_net::EntityId>,
    item: u16,
    owned: bool,
) -> bool {
    let item = hendra_content::ObjectType(item);

    if let Some(bag) = bag {
        let handle = Handle::from_entity_id(bag);
        if !within_reach(world, player, handle) {
            return false;
        }

        let Some(entity) = world.get_mut(handle) else {
            return false;
        };
        if entity.belongs_to.is_some_and(|owner| owner != player) {
            return false;
        }
        if entity.kind != hendra_sim::Kind::Container {
            return false;
        }
        let Some(container) = entity.container.as_mut() else {
            return false;
        };
        return container.insert(item, catalog).is_some();
    }

    // Nothing named: drop it where the player stands.
    let player_handle = player;
    let Some(player) = world.get(player) else {
        return false;
    };
    let (x, y) = (player.x, player.y);

    let mut container = hendra_sim::Container::new(hendra_sim::ContainerKind::Bag, 8);
    if container.insert(item, catalog).is_none() {
        return false;
    }

    let mut dropped = hendra_sim::world::Entity::fixture(hendra_content::ObjectType(0x0500), x, y);
    dropped.kind = hendra_sim::Kind::Container;
    dropped.container = Some(Box::new(container));
    dropped.expires_in_ms = Some(60_000);

    // A soulbound item goes into a bag only whoever dropped it can open, as the original does with
    // its soul bag. That is what makes dropping one a way to move it rather than to give it away.
    if owned {
        dropped.belongs_to = Some(player_handle);
    }

    world.spawn(dropped).is_some()
}

/// Whether a player is close enough to reach something.
fn within_reach(world: &World, player: Handle, target: Handle) -> bool {
    let (Some(player), Some(target)) = (world.get(player), world.get(target)) else {
        return false;
    };
    let (dx, dy) = (target.x - player.x, target.y - player.y);
    dx * dx + dy * dy <= BAG_REACH * BAG_REACH
}

/// Checks that a player really is standing at the portal they named, and reports its type.
fn resolve_portal(world: &World, handle: Handle, portal: hendra_net::EntityId) -> Option<u16> {
    let player = world.get(handle)?;
    let target = world.get(Handle::from_entity_id(portal))?;

    if target.kind != hendra_sim::Kind::Portal {
        return None;
    }

    let (dx, dy) = (target.x - player.x, target.y - player.y);
    if dx * dx + dy * dy > PORTAL_REACH * PORTAL_REACH {
        tracing::debug!(?handle, "refused a portal the player is not standing at");
        return None;
    }

    Some(target.object_type.0)
}

/// Sends every player the world as they see it.
/// Sends what the world said and what its ground became.
///
/// Drained every tick whether or not anyone is listening, because a queue nobody empties is a slow
/// leak, and the world bounds its own but only after it has already grown.
async fn announce(world: &mut World, catalog: &Catalog, players: &mut [Player]) {
    let said = world.take_announcements();
    let ground = world.take_ground_changes();
    let scenery = world.take_scenery_changes();

    if players.is_empty() {
        return;
    }

    for announcement in &said {
        // Named by what spoke, falling back to its kind, so a boss is quoted rather than a number.
        // Something the world said has no speaker, and attributing it to an entity would be a lie
        // the client repeats.
        let from = world
            .get(announcement.from)
            .and_then(|entity| {
                entity
                    .name
                    .as_deref()
                    .map(str::to_owned)
                    .or_else(|| catalog.object(entity.object_type).map(|d| d.id.clone()))
            })
            .unwrap_or_else(|| world.name.to_string());

        let mut buffer = Vec::new();
        ServerMessage::Chat {
            from: &from,
            text: &announcement.text,
        }
        .encode(&mut Writer::new(&mut buffer));

        for player in players.iter() {
            // A broadcast reaches the world; anything else reaches whoever can see the speaker.
            let heard = announcement.broadcast
                || world
                    .get(announcement.from)
                    .zip(world.get(player.handle))
                    .is_some_and(|(speaker, listener)| {
                        let (dx, dy) = (speaker.x - listener.x, speaker.y - listener.y);
                        dx * dx + dy * dy <= SIGHT_RADIUS * SIGHT_RADIUS
                    });

            if heard {
                let _ = player.sender.try_send(Delivery::Stream, &buffer);
            }
        }
    }

    if !ground.is_empty() {
        let mut buffer = Vec::new();
        ServerMessage::Ground {
            changes: ground.clone(),
        }
        .encode(&mut Writer::new(&mut buffer));

        for player in players.iter() {
            let _ = player.sender.try_send(Delivery::Stream, &buffer);
        }
    }

    // Scenery that has appeared goes out by row, in the same shape a joining client is told the
    // map in, so a client has one way of hearing about scenery rather than two.
    if !scenery.is_empty() {
        let mut rows: std::collections::BTreeMap<u16, Vec<(u16, u16, u16)>> =
            std::collections::BTreeMap::new();
        for (x, y, object, size) in scenery {
            rows.entry(y).or_default().push((x, object, size));
        }

        for (y, objects) in rows {
            for piece in objects.chunks(hendra_net::MAX_SCENERY) {
                let mut buffer = Vec::new();
                ServerMessage::Scenery {
                    y,
                    objects: piece.to_vec(),
                }
                .encode(&mut Writer::new(&mut buffer));

                for player in players.iter() {
                    let _ = player.sender.try_send(Delivery::Stream, &buffer);
                }
            }
        }
    }
}

async fn broadcast(world: &mut World, players: &mut Vec<Player>) {
    // A connection that has gone should not be encoded for.
    players.retain(|player| !player.sender.is_closed());

    let tick = world.tick_number();

    for player in players.iter_mut() {
        let handle = player.handle;
        if world.get(handle).is_none() {
            continue;
        }

        let snapshot = world.snapshot_for(handle, SIGHT_RADIUS);

        player.scratch.clear();
        let mut writer = Writer::new(&mut player.scratch);
        hendra_net::begin_snapshot(&mut writer);

        let baseline = match player.history.baseline_for(player.acknowledged) {
            Baseline::Delta { tick, state } => Some((tick, state)),
            Baseline::Full => None,
        };
        let delivery = player
            .encoder
            .encode(tick, &snapshot, baseline, &mut writer);

        // Never awaits: a slow player must not hold up the tick for everyone else. A backlog is
        // reported and the next tick supersedes what was dropped.
        match player.sender.try_send(delivery, &player.scratch) {
            Ok(()) | Err(hendra_transport::TransportError::Backlogged) => {}
            Err(err) => tracing::debug!(%err, "dropping a snapshot"),
        }

        player.history.store(tick, snapshot);
    }
}

/// How long the world should believe elapsed for a movement claim.
///
/// Deliberately not the client's own figure. A client that reports a large interval would be
/// granting itself a proportionally larger step, so the server uses its own tick length and treats
/// the client's clock as diagnostic only.
fn tick_ms(_client_time_ms: u32) -> u32 {
    1000 / TICKS_PER_SECOND
}

/// Somewhere walkable to put an arriving player.
///
/// Prefers a spawn region when the map declares one, and otherwise walks outward from the middle
/// until it finds ground. Dropping a player into a wall is worse than putting them somewhere
/// arbitrary.
fn spawn_point(world: &World) -> (f32, f32) {
    if let Some((x, y, _)) = world
        .terrain()
        .map()
        .regions()
        .find(|(_, _, region)| *region == hendra_content::Region::Spawn)
    {
        return (x as f32 + 0.5, y as f32 + 0.5);
    }

    let terrain = world.terrain();
    let (centre_x, centre_y) = (terrain.width() / 2, terrain.height() / 2);

    for ring in 0..terrain.width().max(terrain.height()) {
        for dy in -(ring as i64)..=(ring as i64) {
            for dx in -(ring as i64)..=(ring as i64) {
                let x = centre_x as i64 + dx;
                let y = centre_y as i64 + dy;
                if x < 0 || y < 0 {
                    continue;
                }
                if terrain.walkable(x as u32, y as u32) {
                    return (x as f32 + 0.5, y as f32 + 0.5);
                }
            }
        }
    }

    (centre_x as f32, centre_y as f32)
}

/// A handle for sessions to talk to a world.
#[derive(Clone)]
pub struct WorldHandle {
    pub name: Arc<str>,

    /// How large the map is, in tiles.
    ///
    /// Kept beside the name because a joining client is told both before its first snapshot, and
    /// asking the world task for them would mean a round trip in the middle of a handshake.
    pub width: u16,
    pub height: u16,

    pub inbox: mpsc::Sender<ToWorld>,
}

impl WorldHandle {
    pub async fn send(&self, command: ToWorld) -> bool {
        self.inbox.send(command).await.is_ok()
    }
}

/// Every portal a world contains, as (entity, object type).
///
/// Logged when a world starts. A dungeon whose portals do not appear here leads nowhere, and that
/// is far easier to see at boot than to work out from a player reporting that a portal does nothing.
pub fn portals_in(world: &World) -> Vec<(Handle, u16)> {
    world
        .iter()
        .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Portal)
        .map(|(handle, entity)| (handle, entity.object_type.0))
        .collect()
}

/// Starts a world on its own task.
/// How long a world with nobody in it keeps ticking before it stops.
///
/// Long enough that walking out and back in returns you to the same room rather than a fresh one,
/// short enough that a night of dungeon-running does not leave a hundred empty worlds ticking.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn spawn(mut world: World, catalog: Arc<Catalog>, loadout: Loadout) -> WorldHandle {
    world.set_bag_types(loadout.bag_types.clone());

    let name: Arc<str> = Arc::from(world.name.as_str());
    let (width, height) = (world.terrain().width() as u16, world.terrain().height() as u16);
    let (inbox, receiver) = mpsc::channel(1024);

    tokio::spawn(run(world, catalog, loadout, receiver));

    WorldHandle {
        name,
        width,
        height,
        inbox,
    }
}
