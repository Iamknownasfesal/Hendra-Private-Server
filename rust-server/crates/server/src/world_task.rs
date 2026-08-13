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

use std::sync::Arc;
use std::time::Instant;

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

    /// What a character has become, so it can be written down.
    ///
    /// Asked for rather than sent on leaving, because a session also checkpoints while playing and
    /// both paths want the same answer.
    Snapshot {
        handle: Handle,
        reply: tokio::sync::oneshot::Sender<Option<Vitals>>,
    },
}

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

    loop {
        tokio::select! {
            command = inbox.recv() => {
                let Some(command) = command else { break };
                handle(&mut world, &catalog, loadout, &mut players, command);
            }

            _ = ticker.tick() => {
                let started = Instant::now();

                world.advance(&catalog, elapsed_ms);
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

    tracing::info!(world = %world.name, "world stopped");
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
            reply,
        } => {
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

    if players.is_empty() {
        return;
    }

    for announcement in &said {
        // Named by what spoke, falling back to its kind, so a boss is quoted rather than a number.
        let from = world
            .get(announcement.from)
            .and_then(|entity| {
                entity
                    .name
                    .as_deref()
                    .map(str::to_owned)
                    .or_else(|| catalog.object(entity.object_type).map(|d| d.id.clone()))
            })
            .unwrap_or_else(|| "?".to_string());

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
pub fn spawn(mut world: World, catalog: Arc<Catalog>, loadout: Loadout) -> WorldHandle {
    world.set_bag_types(loadout.bag_types.clone());

    let name: Arc<str> = Arc::from(world.name.as_str());
    let (inbox, receiver) = mpsc::channel(1024);

    tokio::spawn(run(world, catalog, loadout, receiver));

    WorldHandle { name, inbox }
}
