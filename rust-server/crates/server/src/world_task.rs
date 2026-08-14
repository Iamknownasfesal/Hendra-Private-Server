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

/// What a session asks the world to do.
pub enum ToWorld {
    /// A player is arriving. The world replies with the handle it was given.
    Join {
        name: String,
        arrival: Arrival,
        sender: LinkSender,

        /// How the world tells this session to do something only a session can do.
        orders: mpsc::Sender<Order>,

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
}

/// How close a player must be to reach into a bag, in tiles.
pub const BAG_REACH: f32 = 2.0;

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
            entity.stats.set_equipment(arrival.boosts);
            entity.hp = arrival.hp.clamp(1, max_hp);
            entity.name = Some(name.as_str().into());
            entity.weapon = arrival.weapon.or(loadout.weapon);

            let Some(handle) = world.spawn(entity) else {
                tracing::warn!(world = %world.name, "world is full; refusing a join");
                return;
            };

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
            reply,
        } => {
            let _ = reply.send(put_in_bag(world, catalog, player, bag, item));
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
        if entity.kind != hendra_sim::Kind::Container {
            return false;
        }
        let Some(container) = entity.container.as_mut() else {
            return false;
        };
        return container.insert(item, catalog).is_some();
    }

    // Nothing named: drop it where the player stands.
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
    let (inbox, receiver) = mpsc::channel(1024);

    tokio::spawn(run(world, catalog, loadout, receiver));

    WorldHandle { name, inbox }
}
