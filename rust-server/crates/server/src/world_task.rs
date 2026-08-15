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

/// The most walking one movement claim may be paid for out of, in milliseconds.
///
/// A player who has been standing still has a second of travel saved up and no more.
///
/// A second because that is the window the original's own speed check measured against: it compared
/// one packet's distance to `GetTilesPerSecSqr()`, a whole second's travel, so a claim was allowed a
/// second's worth however recently the last one arrived (`MoveHandler.cs:25-31` at `94615c4`).
///
/// That check was commented out, and the method it calls does not exist anywhere in the original --
/// so it could not have been uncommented and run. The original therefore validates a player's speed
/// not at all, and this cap is ours. The window is the only part of it with an ancestor.
const MOST_ALLOWANCE_MS: u32 = 1_000;

/// What a session asks the world to do.
pub enum ToWorld {
    /// A player is arriving. The world replies with the handle it was given.
    Join {
        name: String,
        arrival: Arrival,
        sender: LinkSender,

        /// What the welcome should say is playing.
        ///
        /// Carried rather than read off the world because `/music` writes the track onto the
        /// world's handle and not onto the world itself, so the world's own copy is the one the
        /// definition named and would welcome somebody with a track nobody else is hearing.
        music: String,

        /// How the world tells this session to do something only a session can do.
        orders: mpsc::Sender<Order>,

        /// How the world tells this session its character has died.
        died: mpsc::Sender<Departed>,

        reply: tokio::sync::oneshot::Sender<Handle>,
    },

    /// What a player wears has changed, so its stat layer is replaced.
    ///
    /// Everything `StatsManager.ReCalculateValues` recomputes on an inventory change, which is both
    /// halves of it: the equipped weapon's damage into the base layer (`BaseStatManager.cs:40-53`)
    /// and the worn bonuses into the boost layer (`BoostStatManager.cs:33-44`).
    Equipment {
        handle: Handle,

        /// Every bonus separately rather than summed, because the floor the original puts under a
        /// negative one is measured against the base and applied to each in turn
        /// (`BoostStatManager.cs:168-171`). A sum cannot be floored the same way.
        bonuses: Vec<(hendra_content::Stat, i32)>,

        /// The equipped weapon's first projectile, as `SetWeaponDamage` reads it.
        weapon_damage: (i32, i32),

        /// What is in the weapon slot, which is what the body now shoots with.
        ///
        /// The original never remembers this: `PlayerShootHandler` reads `Inventory[0]` afresh on
        /// every shot and refuses one naming anything else with `ITEM_MISMATCH`
        /// (`Player.AntiCheat.cs:88-90`), and `item.Projectiles[0]` is taken from that same item
        /// (`PlayerShootHandler.cs:46`). A body here holds its weapon between shots, so the slot is
        /// re-read into it on the same inventory change that recomputes the stats — otherwise a
        /// player who swaps wands keeps firing the old one's bullet for the rest of the session.
        weapon: Option<ObjectType>,

        /// The look a completed equipment set puts on, and `None` when nothing worn completes one.
        ///
        /// The other half of `ApplySetBonus`, which is not a stat at all: a set that is all there
        /// dresses the body in its own skin and size, and one that has just lost a piece puts the
        /// character's own back (`BoostStatManager.cs:71-112`). Carried on the same message as the
        /// bonuses because it is answered by the same read of the worn slots -- the original
        /// recomputes both from the inventory in one pass.
        set_skin: Option<hendra_content::SetSkin>,
    },

    /// A player has started or extended an experience boost, which doubles what a kill is worth
    /// while it runs.
    ///
    /// Sent rather than waited for, because `AEXPBoost` sets the clock on the body being played
    /// (`Player.UseItem.cs:531-539`): a boost drunk in a room takes effect on the next thing that
    /// dies in it, not on the next world.
    ExperienceBoost {
        handle: Handle,
        milliseconds: i32,
    },

    /// What the account can spend has changed, and the body being played has to be told.
    ///
    /// `Merchant.TransactionItemComplete` writes all three back onto the player after a purchase
    /// (`realm/entities/vendors/Merchant.cs:192-195`), and `RemoveItemFromMarketAsync` clamps the
    /// fame down after the five it charges (`Player.Market.cs:145-146`). Without this the client
    /// keeps showing money that has already been spent.
    Purse {
        handle: Handle,
        purse: hendra_sim::world::Purse,
    },

    /// A player reports where it believes it is, and what it has received.
    Input {
        handle: Handle,
        x: f32,
        y: f32,
        client_time_ms: u32,
        ack: Acknowledgement,
    },

    /// A player is firing. The aim and the clock reading come from them.
    ///
    /// The clock reading is what the rate of fire is enforced against, which is where the original
    /// enforces it (`Player.AntiCheat.cs:93-95`). Believing it is what the ratio in [`TimeCop`] is
    /// for: it is worth nothing on its own, and worth everything alongside a check that it is not
    /// running faster than the server's own.
    Shoot {
        handle: Handle,
        angle: f32,
        client_time_ms: u32,
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

    /// Places the object the vault panel opens from, on the `Vault` square nearest the spawn.
    ///
    /// One object, where there used to be one per eight slots. What the account owns does not come
    /// with the request because the object holds nothing: it is not a container, and its class is
    /// what tells the client to open the vault panel rather than an eight-slot grid. The contents
    /// travel as their own message — see `crate::vault`.
    PlaceVaultAccess {
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
        reply: tokio::sync::oneshot::Sender<Option<Taken>>,
    },

    /// Puts an item into a bag, or makes a new one at the player's feet.
    PutInBag {
        player: Handle,
        bag: Option<hendra_net::EntityId>,
        item: u16,

        /// Whether this item may only be held by whoever is dropping it.
        ///
        /// It decides whether a named bag will take it: a soulbound item goes into a bag its owner
        /// solely owns or into a fresh one of their own, never into a bag anybody could reach.
        soulbound: bool,

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

    /// Whether using that item would do anything, asked before a consumable is spent.
    MayUseItem {
        handle: Handle,
        item: ObjectType,
        reply: tokio::sync::oneshot::Sender<bool>,
    },

    /// Uses an item out of a container standing in the world, aimed at a point.
    ///
    /// Whole rather than split into a read and a use, because what is in a bag lives in the world
    /// and nowhere else: taking it and using it in one visit is what stops two asks arriving
    /// together from drinking the same potion twice.
    UseFromContainer {
        handle: Handle,
        container: hendra_net::EntityId,
        slot: u8,
        aim: (f32, f32),
        reply: tokio::sync::oneshot::Sender<Vec<hendra_content::Effect>>,
    },

    /// Opens a portal where an entity is standing.
    OpenPortal {
        at: Handle,
        kind: ObjectType,
        duration_ms: u32,

        /// Who opened it, when a key did and the room should say so.
        ///
        /// `AECreate` (`Player.UseItem.cs:613-624`) follows the portal with a green line over the
        /// opener and one server line to every player in the world naming the dungeon and them.
        /// A `MysteryPortal` carries no name and announces nothing.
        opened_by: Option<Announced>,

        /// The number the portal was given, so that the world it leads to can be tied to this door.
        ///
        /// The original does not need one: it hangs the world off the `Portal` object itself
        /// (`Portal.cs:90`). Here the door is an entity in another task and the room is a key in a
        /// registry, and this is what joins them.
        reply: Option<tokio::sync::oneshot::Sender<Option<hendra_net::EntityId>>>,
    },

    /// Turns a locked door standing next to a player into the one it leads to.
    ///
    /// `AEUnlockPortal` (`Player.UseItem.cs:433-510`). The session resolves the two names against
    /// the catalog and the world registry; only the world knows where the doors are standing.
    UnlockPortal {
        at: Handle,

        /// The kind of door to look for near them.
        locked: ObjectType,

        /// The kind to stand in its place.
        unlocked: ObjectType,

        duration_ms: u32,

        /// Who unlocked it and what it leads to, for the two lines the room is told.
        announced: Announced,

        /// The number the new door was given, so the room behind it can be tied to it.
        reply: tokio::sync::oneshot::Sender<Option<hendra_net::EntityId>>,
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

    /// Whether this sends them home rather than ending the character.
    pub rekt: bool,

    /// What the body was at the moment it fell, which is what gets written down.
    pub vitals: Vitals,
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

/// A character's live state, as the durable side needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vitals {
    pub hp: i32,
    pub mp: i32,

    /// What the stats currently allow, equipment included. Reported rather than recomputed on the
    /// far side, because equipment lives on the body and the session only knows what was worn when
    /// it started.
    pub max_hp: i32,
    pub max_mp: i32,

    pub level: i16,
    pub experience: i32,
    pub fame: i32,

    /// The eight base stats as they stand, so what levelling produced survives leaving.
    pub stats: [i32; 8],

    /// What the character has done since the last snapshot, which is added to what it had done
    /// before. Taken from the body rather than copied off it, so two checkpoints in one session
    /// cannot count the same shot twice.
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

    /// How much walking this player has yet to be judged for, and when it was last added to.
    ///
    /// Movement is allowed by the time that has passed rather than by the number of claims made,
    /// or a client that sends its position ten times as often walks ten times as fast. What is
    /// left over is kept, so a handful of claims released together after a stall are all paid for
    /// out of the stall itself and nobody is dragged backwards for having a bad connection.
    walked_at: Instant,
    allowance_ms: u32,

    /// Whether this player's clock is running at the same speed as the server's.
    ///
    /// A rate of fire is enforced in the client's own time, so the clock it reports has to be worth
    /// something. This is what makes it worth something.
    shot_clock: TimeCop,

    /// What this player has been sent, so a snapshot can be a delta against what they confirm.
    history: BaselineRing<WorldSnapshot>,
    encoder: SnapshotEncoder,
    acknowledged: Acknowledgement,

    /// Ground and scenery this player has uncovered but the wire has had no room for yet.
    ///
    /// Encoded once, when the squares are revealed, and handed to the connection as it drains.
    /// The map is the largest thing on this wire, so a tick that finds the send queue full leaves
    /// the rest here instead of blocking the world on one slow client or throwing squares away --
    /// a lost square is a hole in the client's map that nothing would ever fill again.
    pending_map: std::collections::VecDeque<Vec<u8>>,

    /// Scratch, reused every tick.
    scratch: Vec<u8>,
}

/// How many shots the two clocks are compared over.
const TIME_COP_WINDOW: usize = 20;

/// The fastest and slowest a client's clock may run against the server's before its shots stop
/// counting. `Player.AntiCheat.cs:82-83`.
const FASTEST_CLOCK: f32 = 1.08;
const SLOWEST_CLOCK: f32 = 0.92;

/// Watches a player's clock against the server's, over the last [`TIME_COP_WINDOW`] shots.
///
/// The original's `TimeCop` (`Player.AntiCheat.cs:17-77`), and the thing that makes it safe to
/// enforce a rate of fire in a client's own time: the interval between two shots is measured in
/// time the client reports, so a client whose clock ran at twice the speed could fire twice as fast.
/// Comparing the two clocks over a window rather than per shot is what lets an honest client's
/// packets arrive bunched after a stall without any of them being thrown away.
struct TimeCop {
    /// What the server's clock is measured from, standing in for `Environment.TickCount`.
    since: Instant,

    client: [i32; TIME_COP_WINDOW],
    server: [i32; TIME_COP_WINDOW],
    index: usize,
    count: usize,
    client_elapsed: i32,
    server_elapsed: i32,
    last_client: i32,
    last_server: i32,
}

impl TimeCop {
    fn new() -> TimeCop {
        TimeCop {
            since: Instant::now(),
            client: [0; TIME_COP_WINDOW],
            server: [0; TIME_COP_WINDOW],
            index: 0,
            count: 0,
            client_elapsed: 0,
            server_elapsed: 0,
            last_client: 0,
            last_server: 0,
        }
    }

    /// Records a shot and answers how fast the client's clock is running.
    ///
    /// One means the two agree, less than one means the client is slow and more than one means it is
    /// fast. A window that has not filled yet answers one, so the first twenty shots of a session
    /// are taken on trust — as they are in the original, which is what stops a player being judged
    /// on the two samples they have made since walking through a door.
    fn observe(&mut self, client_time_ms: u32) -> f32 {
        let server_now = self.since.elapsed().as_millis() as i32;
        self.push(client_time_ms, server_now)
    }

    /// The same, against a stated server clock rather than the wall one.
    fn push(&mut self, client_time_ms: u32, server_now: i32) -> f32 {
        let client_now = client_time_ms as i32;

        let (client_step, server_step) = if self.count == 0 {
            (0, 0)
        } else {
            (
                client_now.wrapping_sub(self.last_client),
                server_now.wrapping_sub(self.last_server),
            )
        };

        self.count += 1;
        self.index = (self.index + 1) % TIME_COP_WINDOW;

        // The sample leaving the window is subtracted as the new one is added, so the two running
        // totals are always the last twenty steps and nothing is re-summed.
        self.client_elapsed += client_step - self.client[self.index];
        self.server_elapsed += server_step - self.server[self.index];
        self.client[self.index] = client_step;
        self.server[self.index] = server_step;
        self.last_client = client_now;
        self.last_server = server_now;

        if self.count < TIME_COP_WINDOW {
            return 1.0;
        }

        self.client_elapsed as f32 / self.server_elapsed as f32
    }
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

    // When this world was built, which is what decides whether an empty one goes.
    let born = Instant::now();

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
        init_realm(&mut world, &catalog, loadout);
    }

    // Only a map that marks a vault square has a chest to arc over, so the search for one is asked
    // once here rather than every couple of seconds in every world.
    let has_vault = world
        .terrain()
        .map()
        .regions()
        .any(|(_, _, region)| region == hendra_content::Region::Vault);
    let mut since_crackle_ms = 0u64;

    // The marketplace's merchants, for the one world whose map marks squares for them.
    // `Marketplace.Init` is the only caller of `InitMarketplace`
    // (`realm/worlds/logic/Marketplace.cs:17`); recognising it by the squares rather than by the
    // name means a map that marks a row gets a row.
    let mut board = Board::of_world(&world, &catalog);
    if let Some(board) = board.as_ref() {
        tracing::info!(
            world = %world.name,
            squares = board.market.pitches().len(),
            "marketplace squares marked"
        );
    }

    // The world's own clock, which is what decides whose turn it is to rotate:
    // `Merchant.Tick` measures against `time.TotalElapsedMs` (`Merchant.cs:99`).
    let mut total_ms: i64 = 0;

    // Anything said to the whole server, which one world hears the same way as any other.
    let mut server_wide = loadout.announcements.subscribe();

    // A portal opening, which every world hears differently: the line names the destination unless
    // the hearer is already in it, and points at the Nexus unless the hearer is standing in it.
    let mut portals_opened = loadout.portals_opened.subscribe();

    loop {
        tokio::select! {
            command = inbox.recv() => {
                let Some(command) = command else { break };

                match command {
                    // The board is the one command that needs state the handler does not carry, so
                    // it is answered here rather than there.
                    ToWorld::ShowListings { listings } => {
                        if let Some(board) = board.as_mut() {
                            board.market.restock(
                                &catalog,
                                &listings
                                    .iter()
                                    .map(|listed| hendra_sim::market::Listing {
                                        id: listed.listing,
                                        item: listed.item,
                                        price: listed.price,
                                    })
                                    .collect::<Vec<_>>(),
                            );
                            board.apply(&mut world);
                        }
                    }

                    command => handle(&mut world, &catalog, loadout, &mut players, command),
                }
            }

            said = server_wide.recv() => {
                // Lagging past the buffer drops the oldest lines rather than the channel, so a
                // world that stalled still hears whatever is said next.
                if let Ok(text) = said {
                    let mut buffer = Vec::new();
                    ServerMessage::Notice { text: format!("<ANNOUNCEMENT> {text}") }
                        .encode(&mut Writer::new(&mut buffer));

                    for player in players.iter() {
                        let _ = player.sender.try_send(Delivery::Stream, &buffer);
                    }
                }
            }

            opened = portals_opened.recv() => {
                if let Ok(destination) = opened {
                    let line = portal_opened_line(&world.name, &destination);
                    let mut buffer = Vec::new();
                    ServerMessage::Refused { message: &line }
                        .encode(&mut Writer::new(&mut buffer));

                    for player in players.iter() {
                        let _ = player.sender.try_send(Delivery::Stream, &buffer);
                    }
                }
            }

            _ = ticker.tick() => {
                let started = Instant::now();

                // A world nobody is in stops, so a night of dungeon-running does not leave a
                // hundred of them ticking. `World.Tick` (`World.cs:614`) does it on the world's own
                // age rather than on how long it has been empty: once a world is a minute old, the
                // moment the last player leaves is the moment it goes. The minute is what covers a
                // world built for a portal somebody has not walked through yet.
                //
                // A world its definition marks `persist` is exempt, which is the nexus, the realm
                // and the vault: each has to be there before anybody is in it.
                if players.is_empty() && !persistent && born.elapsed() >= WORLD_GRACE {
                    break;
                }

                world.advance(&catalog, elapsed_ms);

                // Every merchant moves on to a different item once a cycle, at its own moment in
                // it, and one with somebody standing at it waits until they leave.
                total_ms += elapsed_ms as i64;
                if let Some(board) = board.as_mut() {
                    // Copied out so the board can be borrowed for the turn while the world is
                    // still readable to answer who is standing where. Asked only of the squares
                    // whose turn has come round, which is one or two of two hundred and fifty-five.
                    let places: Vec<hendra_sim::market::Pitch> = board.market.pitches().to_vec();

                    board.market.tick(total_ms, elapsed_ms as i64, |pitch| {
                        Board::crowded(&world, places[pitch])
                    });
                    board.apply(&mut world);
                }

                if has_vault && !players.is_empty() {
                    since_crackle_ms += elapsed_ms as u64;
                    if since_crackle_ms >= CRACKLE_EVERY_MS {
                        since_crackle_ms -= CRACKLE_EVERY_MS;
                        crackle(&mut world, &catalog);
                    }
                }

                if is_realm {
                    tend_realm(&mut world, &catalog, loadout, elapsed_ms as u64, &players);
                    answer_kills(&mut world, &catalog, &loadout.maps);
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
                        rekt: death.rekt,

                        // Taken off the body before it was reaped, which is the only moment it can
                        // be: `Player.Death` calls `SaveToCharacter` from inside the entity
                        // (`Player.cs:1018`), so the fame a death is worth includes everything the
                        // last seconds of the life earned.
                        vitals: Vitals {
                            hp: death.body.hp,
                            mp: death.body.mp,
                            max_hp: death.body.max_hp,
                            max_mp: death.body.max_mp,
                            level: death.body.level,
                            experience: death.body.experience,
                            fame: death.body.fame,
                            stats: death.body.stats,
                            tally: counted(&death.body.tally),
                        },
                    });
                }

                for name in world.take_unknown_setpieces() {
                    tracing::warn!(
                        world = %world.name,
                        setpiece = %name,
                        "a behaviour asked for a setpiece that does not exist"
                    );
                }

                announce_shots(&mut world, &mut players).await;
                announce_damage(&mut world, &mut players).await;
                announce_teleports(&mut world, &mut players).await;
                announce_quests(&mut world, &mut players).await;
                announce_focus(&mut world, &mut players).await;
                announce_effects(&mut world, &mut players).await;
                announce_status_texts(&mut world, &mut players).await;
                announce_blasts(&mut world, &mut players).await;
                // Everyone's sight circle is brought up to date before anything is decided from it,
                // as `SendUpdate` takes one at the top of its pass (`Player.Update.cs:130`). A
                // player who has not moved keeps the circle they had, which is the whole of what
                // makes doing this every tick affordable.
                for player in players.iter() {
                    world.look_around(player.handle);
                }

                announce(&mut world, &catalog, &mut players).await;

                // The ground before the bodies standing on it, as `SendUpdate` puts `Tiles` ahead
                // of `NewObjs` in the one packet (`Player.Update.cs:179-184`): a client told about
                // an entity on a square it has never been given has nowhere to draw it.
                uncover(&mut world, &catalog, &mut players).await;

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
    /// The world's own definition decides, through its `persist` field, and the entry world is
    /// persistent whatever it says: it has to exist before anyone is in it. `World.Tick`
    /// (`World.cs:614`) deletes an empty world only when it is not persistent, which is why the
    /// realm -- `persist: true` in `Realm.jw` -- is recycled in place instead.
    pub persistent: bool,

    /// When the server started, which is the clock the realm's half-hour and Oryx's ten seconds are
    /// both measured against. `RealmTime.TotalElapsedMs` is server-wide in the original, so every
    /// realm shares one clock rather than keeping its own.
    pub started: Instant,

    /// How to build this world's ground again, for the one world that is rebuilt in place.
    ///
    /// Only the realm carries one. `Realm.Tick` (`Realm.cs:67`) calls `Init` again when a closed
    /// realm empties, which resets the map, redraws the set pieces and makes a new Oryx, and the
    /// world object itself is never deleted.
    pub blueprint: Option<Arc<Blueprint>>,

    /// How a world says something to the whole server rather than to the people in it.
    pub announcements: tokio::sync::broadcast::Sender<String>,

    /// A world a nexus portal has just opened onto, which everybody on the server is told about.
    ///
    /// The destination's name rather than a finished line, because the line depends on who hears
    /// it: `AddPortal` (`PortalMonitor.cs:86-91`) says "this land" to anybody already in the world
    /// the portal leads to, and leaves off " in Nexus" for anybody standing in the Nexus.
    pub portals_opened: tokio::sync::broadcast::Sender<String>,

    pub avatar: ObjectType,
    pub weapon: Option<ObjectType>,
}

/// The character that is arriving, as the world needs it.
///
/// Separate from the database row because the world has no business with experience, fame or the
/// account behind it. It needs a body, and this is the body.
#[derive(Debug, Clone)]
pub struct Arrival {
    pub avatar: ObjectType,

    /// How much likelier this account is to be given loot, from whatever boost it holds. One for
    /// everybody without one.
    pub loot_drop: f32,

    /// Milliseconds left on the account's experience boost, which the world counts down and which
    /// doubles what every kill is worth while it runs.
    pub experience_boost_ms: i32,

    /// How many stars the account has earned, for everybody else to see beside the name.
    pub stars: u8,

    /// What the account can spend. `Player`'s constructor seeds the three from the account
    /// (`Player.cs:399-404`), and without them a client shows an empty purse and every shop in the
    /// game refuses to sell.
    pub purse: hendra_sim::world::Purse,

    pub hp: i32,
    pub max_hp: i32,

    /// The magic the character has left. Carried like health, since a spell cast in the last room
    /// should still be missing from the pool in this one.
    pub mp: i32,

    pub weapon: Option<ObjectType>,

    /// The eight stats the character brings, before equipment.
    pub stats: hendra_sim::stats::Stats,

    /// The level, experience and fame the character brings.
    ///
    /// Carried rather than rebuilt, because a body is created fresh in every world: without this
    /// the character would arrive at level one on the far side of every portal, and a kill made in
    /// the last room would be worth nothing.
    pub progress: hendra_sim::leveling::Progress,

    /// What the character is wearing, one bonus at a time.
    ///
    /// Separate rather than summed for the same reason `ToWorld::Equipment` keeps them apart: the
    /// original floors each bonus against the base as it goes in.
    pub bonuses: Vec<(hendra_content::Stat, i32)>,

    /// The equipped weapon's first projectile, for base slots 8 and 9.
    pub weapon_damage: (i32, i32),

    /// Whether the account is under a mute that has not run out.
    ///
    /// Carried so the world can put the condition effect on the body. `Player.HandleEffects`
    /// (`Player.Effects.cs:23`) applies it from the account's own flag, and it is what draws the
    /// crossed-out speech bubble: the chat refusal on its own tells a muted player nothing until
    /// they try to speak.
    pub muted: bool,

    /// The skin the character is wearing, as the skin object's own type. Zero for the class's own
    /// sprite.
    ///
    /// Carried like the rest of the appearance, because a body is created fresh in every world:
    /// `Player.Init` (`Player.cs:431-436`) reads the skin off the character on every arrival and
    /// puts it on the body, so a skin that did not travel would be taken off at every portal.
    pub skin: u16,

    /// The look a completed equipment set puts on over that skin, or `None` when what the character
    /// is wearing completes none.
    ///
    /// Carried for the same reason the skin is: the body is new in every world, and `ApplySetBonus`
    /// runs the moment the stat manager is built (`StatsManager.cs:38`, `Player.cs:467`), so a set
    /// already worn dresses the player before anybody sees them rather than at the next slot change.
    pub set_skin: Option<hendra_content::SetSkin>,

    /// The guild this account belongs to and its rank in it, or `None` for nobody's.
    ///
    /// `Player`'s constructor takes both off the account (`Player.cs:405-406`) and `ExportStats`
    /// sends them on every update (`Player.cs:291-292`). Carried like the skin and the purse: the
    /// world holds no accounts, and reading a guild per snapshot would be a database read per
    /// player per tick.
    pub guild: Option<(String, i16)>,

    /// Whether the account is an administrator, which is what colours the star beside its name.
    ///
    /// `Player._admin` off `Account.Admin` (`Player.cs:359`).
    pub admin: bool,

    /// Whether this character owns the eight extra carried slots.
    ///
    /// `Player.HasBackpack` (`Player.cs:355`). Carried on arrival like the skin and the guild,
    /// because the world holds no characters and the client hides the row without it.
    pub has_backpack: bool,

    /// The two dyes this character is wearing, cloth then accessory.
    ///
    /// `Player`'s constructor seeds them from the character (`Player.cs:412-413`), which is what
    /// makes a dye survive a portal and a logout.
    pub dyes: (i32, i32),

    /// The fame the next class quest asks for, for this class.
    ///
    /// `GetFameGoal(FameCounter.ClassStats[ObjectType].BestFame)` (`Player.cs:537`), read off the
    /// best run this account has had with this class rather than off the character standing here.
    pub fame_goal: i32,

    /// What is left of the loot-drop and loot-tier boosts, in milliseconds.
    ///
    /// Measured at the door like the experience boost beside it, so a boost that lapsed while the
    /// player was in a room does not come back when they leave it.
    pub loot_drop_boost_ms: i32,
    pub loot_tier_boost_ms: i32,

    /// Whether this account picked its own name rather than being handed a reserved one.
    ///
    /// `Account.NameChosen` (`Player.cs:299-300`), which colours the name over the head.
    pub name_chosen: bool,
}

/// Draws a realm's temples, castles, groves and graveyards into it.
///
/// Positions come from the terrain, so a castle stands on high ground and an oasis in the sand, and
/// no two are drawn through each other.
/// Reads the saved map a setpiece is drawn from.
///
/// The extension depends on how far the content has been converted, so all three are tried rather
/// than one being assumed.
fn read_prefab(maps: &Path, name: &str, catalog: &Catalog) -> Option<hendra_content::Map> {
    ["hmap", "jm", "wmap"].iter().find_map(|extension| {
        let file = format!("{name}.{extension}");
        let raw = std::fs::read(maps.join(&file)).ok()?;
        crate::worlds::load_map_bytes(&raw, &file, catalog)
    })
}

/// Where the middle of a saved setpiece goes, given the corner it is placed by.
///
/// `ProjectOntoWorld` writes its square `(x, y)` at `pos + (x, y)` (`Wmap.cs:468-469`), so the point
/// a setpiece is placed by is its top left corner. [`World::stamp`] centres what it places, so the
/// corner is moved by half the saved map's own size — not by half the setpiece's declared size,
/// which is only how much room the scatter reserves for it and need not match.
fn prefab_middle(piece: &hendra_content::Map, x: u32, y: u32) -> (f32, f32) {
    (
        x as f32 + (piece.width() / 2) as f32,
        y as f32 + (piece.height() / 2) as f32,
    )
}

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

        // A setpiece that is a saved map is stamped rather than painted.
        if let Some(name) = drawing.prefab {
            let Some(map) = read_prefab(maps, name, catalog) else {
                tracing::warn!(setpiece = %name, path = %maps.display(), "cannot read a setpiece map");
                continue;
            };

            world.stamp(catalog, &map, prefab_middle(&map, place.x, place.y));
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

/// Everything a world needs to be built again from its map.
///
/// Kept so a realm can be rebuilt where it stands, without the registry that first assembled it:
/// the world task owns its world and has nobody to ask for a fresh one.
#[derive(Debug, Clone)]
pub struct Blueprint {
    pub map: hendra_content::Map,
    pub behaviours: hendra_behavior::Programs,
    pub sight: hendra_sim::Sight,
    pub allows_teleport: bool,
    pub drowns: bool,
}

/// Throws away everything a world holds and lays its ground out again.
///
/// The world's own state -- its entities, its painted set pieces, the ground they were painted
/// over -- all comes from the terrain, so the honest way to reset it is to build it again. Only
/// reached with nobody in it, which is why replacing the whole world cannot strand a player.
fn rebuild(world: &mut World, catalog: &Catalog, loadout: &Loadout) -> bool {
    let Some(blueprint) = &loadout.blueprint else {
        return false;
    };

    let terrain = hendra_sim::Terrain::build(blueprint.map.clone(), catalog);
    let mut fresh = World::new(world.name.clone(), terrain, catalog);

    fresh.set_behaviours(catalog, blueprint.behaviours.clone());
    fresh.set_allows_teleport(blueprint.allows_teleport);
    fresh.sight = blueprint.sight;
    fresh.drowns = blueprint.drowns;

    // The realm's own clock is carried across, because the original resets the map and the overseer
    // but not the world: any step of a closing sequence still on the world's timer list survives a
    // reset and goes off in the rebuilt realm.
    *fresh.realm_mut() = world.realm().clone();

    *world = fresh;
    true
}

/// Who Oryx is, when he speaks.
///
/// `ChatManager.Oryx` (`ChatManager.cs:193`) puts this in the `Name` of a `Text` packet, hash and
/// all: it is the client's cue to draw the line as the realm speaking rather than a player.
pub const ORYX: &str = "#Oryx the Mad God";

/// Broadcasts one of Oryx's lines to the realm, and writes it down.
///
/// Logged as well as said, as `ChatManager.Oryx` (`ChatManager.cs:201`) logs it: a taunt is how the
/// state of a realm is announced, and the record of what it said is the record of what it was.
fn oryx_says(world: &mut World, line: &str) {
    tracing::info!(world = %world.name, "<Oryx the Mad God> {line}");
    world.announce_as(ORYX, line);
}

/// Builds a realm from nothing: its set pieces, then its enemies.
///
/// `Realm.Init` (`Realm.cs:33`) in one function, and it is called twice in a realm's life -- once
/// when the world starts, and again each time a closed realm empties. That second call is what
/// makes the realm a world that is recycled rather than deleted.
fn init_realm(world: &mut World, catalog: &Catalog, loadout: &Loadout) {
    // Setpieces first, then enemies, as `Realm.Init` does. The other way round would bury
    // whatever had already spawned under a castle.
    build_setpieces(world, catalog, &loadout.maps);

    let census = world.terrain().terrain_census();
    world.realm_mut().measure(&census);

    // Twice, because the original fills it twice: `Realm.Init` constructs an `Oryx`, whose
    // constructor populates the realm, and then calls `Init` on it again. The second pass counts
    // from zero, so the realm opens at roughly twice its target and is thinned back on Oryx's first
    // population check.
    let mut placed = 0;
    for _ in 0..hendra_sim::realm::INITIAL_FILLS {
        let opening = world.realm().opening();
        let (added, _) = world.populate(catalog, &loadout.spawnable, &opening);
        placed += added;
    }

    tracing::info!(
        world = %world.name,
        target = world.realm().population(),
        placed,
        "realm populated"
    );
}

/// Sends everybody still in a closed realm to the castle.
///
/// The world can move a body but not a connection: which world somebody is in belongs to the
/// session that owns their link, so the world asks and the session does it. Asked of everybody,
/// including anybody whose session is too busy to hear right now, because leaving one player behind
/// in a realm that has stopped spawning is leaving them in an empty map.
///
/// Anybody paused goes to the nexus instead, as `QuakeToWorld` (`World.cs:572`) sends them: a
/// player who is not playing should not be dropped into a boss fight they cannot see coming. The
/// original exempts somebody paused because they are watching another player, which we cannot tell
/// apart -- spectating here is a camera the client moves and a pause the world holds, with no
/// target recorded on the body to check.
fn send_to_castle(world: &World, players: &[Player]) {
    for player in players {
        let paused = world
            .get(player.handle)
            .is_some_and(|entity| hendra_sim::effects::Rules::of(entity.conditions).paused);

        let destination = if paused {
            crate::commands::NEXUS
        } else {
            CASTLE
        };

        if player
            .orders
            .try_send(Order::GoTo(destination.to_string()))
            .is_err()
        {
            tracing::warn!(name = %player.name, destination, "could not move a player out of the realm");
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
    loadout: &Loadout,
    elapsed_ms: u64,
    players: &[Player],
) {
    use hendra_sim::realm::Event;

    let uptime_ms = loadout.started.elapsed().as_millis() as u64;
    let events = world
        .realm_mut()
        .advance(elapsed_ms, uptime_ms, players.len());

    for event in events {
        match event {
            Event::Ensure => {
                let alive = world.alive_by_terrain();
                let wanted = world.realm().adjustments(&alive);
                if wanted.is_empty() {
                    continue;
                }

                let (added, removed) = world.populate(catalog, &loadout.spawnable, &wanted);
                tracing::debug!(world = %world.name, added, removed, "realm population checked");
            }

            Event::Taunt => taunt(world, catalog),

            Event::Warned => {
                // Server-wide, as `Chat.Announce(text, local: true)` is: a realm closing is news
                // for the people deciding whether to go there, not only for the ones already in it.
                let _ = loadout
                    .announcements
                    .send("Realm closing in 1 minute.".to_string());
                tracing::info!(world = %world.name, "realm closing in 1 minute");
            }

            Event::Closed => {
                oryx_says(world, "I HAVE CLOSED THIS REALM!");
                oryx_says(world, "YOU WILL NOT LIVE TO SEE THE LIGHT OF DAY!");
                tracing::info!(world = %world.name, "realm closed");
            }

            Event::Castle => {
                oryx_says(world, "MY MINIONS HAVE FAILED ME!");
                oryx_says(world, "BUT NOW YOU SHALL FEEL MY WRATH!");
                oryx_says(world, "COME MEET YOUR DOOM AT THE WALLS OF MY CASTLE!");

                // The ground starts shaking here and the reconnect is eight seconds later, which is
                // what makes the wait mean something: `QuakeToWorld` (`World.cs:547-551`) broadcasts
                // the earthquake and only then arms the timer that moves anybody. Every field but
                // the kind is left at nothing, as the original leaves them — the client's camera
                // reads none of them.
                world.show_effect(hendra_sim::world::EffectEvent {
                    effect: hendra_net::message::effect::EARTHQUAKE,
                    target: None,
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                    color: 0,
                });

                if !players.is_empty() {
                    tracing::info!(
                        world = %world.name,
                        players = players.len(),
                        "the castle is up and the realm is shaking"
                    );
                }
            }

            Event::Quake => {
                tracing::info!(world = %world.name, players = players.len(), "quaking to the castle");
                send_to_castle(world, players);
            }

            Event::Reset => {
                tracing::info!(world = %world.name, "a closed realm has emptied; building it again");
                if rebuild(world, catalog, loadout) {
                    init_realm(world, catalog, loadout);
                }
            }
        }
    }
}

/// Says one of Oryx's lines about what still guards him.
///
/// `HandleAnnouncements` (`Oryx.cs:736`) picks one of the seventeen critical enemies at random
/// whether or not any are alive, counts them, and says nothing when none are. Nothing is said at
/// all while the realm is closed.
fn taunt(world: &mut World, catalog: &Catalog) {
    if !world.realm().admits() {
        return;
    }

    let roll = world.roll();
    let Some(taunts) = hendra_sim::realm::pick(hendra_sim::realm::CRITICAL, roll) else {
        return;
    };

    let Some(kind) = catalog.type_of(taunts.name) else {
        return;
    };

    let count = world.count_of_kind(kind);
    let roll = world.roll();

    if let Some(line) = hendra_sim::realm::announcement(taunts, count, roll) {
        oryx_says(world, &line);
    }
}

/// Answers the quest enemies that have been killed here.
///
/// `Oryx.OnEnemyKilled` (`Oryx.cs:800`): a taunt naming whoever did it, and then -- every time, not
/// one time in four, because the gate is commented out (`Oryx.cs:833`) -- a new event raised
/// somewhere in the realm and announced.
fn answer_kills(world: &mut World, catalog: &Catalog, maps: &Path) {
    for kill in world.take_quest_kills() {
        let Some(name) = catalog.object(kill.kind).map(|desc| desc.id.clone()) else {
            continue;
        };

        if let Some(taunts) = hendra_sim::realm::taunts_for(&name) {
            let roll = world.roll();
            if let Some(line) = hendra_sim::realm::eulogy(taunts, kill.killer.as_deref(), roll) {
                oryx_says(world, &line);
            }
        }

        raise_event(world, catalog, maps);
    }
}

/// Puts one of Oryx's events somewhere in the realm, and says what it is.
///
/// `SpawnEvent` (`Oryx.cs:783`) looks for a square whose terrain is in the band from the mountains
/// down to the middle forest, is passable with nothing occupying it, and has nobody nearby, then
/// draws the set piece with its middle on that square.
fn raise_event(world: &mut World, catalog: &Catalog, maps: &Path) {
    let roll = world.roll();
    let Some((index, name, kind)) = world.realm().choose_event(roll) else {
        return;
    };

    // A ceiling of one means this may happen once in a realm, so it comes off the list as it is
    // used. Anything else stays and may be drawn again.
    let once_only = catalog
        .type_of(name)
        .and_then(|kind| catalog.object(kind))
        .and_then(|desc| desc.per_realm_max)
        == Some(1);

    if once_only {
        world.realm_mut().spend_event(index);
    }

    let Some((x, y)) = world.event_ground(kind.size()) else {
        tracing::warn!(world = %world.name, event = name, "nowhere in the realm to raise an event");
        return;
    };

    let seed = (world.roll() * u32::MAX as f32) as u32;
    let mut dice = hendra_sim::setpiece::Dice::new(seed);
    let drawing = kind.draw(&mut dice);

    if let Some(prefab) = drawing.prefab {
        let Some(map) = read_prefab(maps, prefab, catalog) else {
            tracing::warn!(setpiece = %prefab, "cannot read the map an event is drawn from");
            return;
        };

        world.stamp(catalog, &map, prefab_middle(&map, x, y));
    } else {
        for missing in world.draw(catalog, &drawing, (x, y)) {
            tracing::warn!(name = %missing, event = name, "an event wants something the content does not have");
        }
    }

    tracing::info!(world = %world.name, event = name, x, y, "Oryx has raised an event");

    if let Some(taunts) = hendra_sim::realm::taunts_for(name) {
        let roll = world.roll();
        if let Some(line) = hendra_sim::realm::pick(taunts.spawn, roll) {
            oryx_says(world, line);
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
            music,
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
            entity.progress = arrival.progress;
            entity.loot_drop = arrival.loot_drop;
            entity.experience_boost_ms = arrival.experience_boost_ms;
            entity.stars = arrival.stars;
            entity.purse = arrival.purse;
            // Read off the account when the body is made, as `Player`'s constructor reads it
            // (`Player.cs:405-406`). A body is new in every world, so a guild that did not travel
            // would be taken off at every portal.
            if let Some((guild, rank)) = &arrival.guild {
                entity.guild = Some(guild.as_str().into());
                entity.guild_rank = *rank;
            }
            entity.admin = arrival.admin;
            entity.has_backpack = arrival.has_backpack;
            entity.name_chosen = arrival.name_chosen;
            entity.fame_goal = arrival.fame_goal;
            entity.loot_drop_boost_ms = arrival.loot_drop_boost_ms;
            entity.loot_tier_boost_ms = arrival.loot_tier_boost_ms;

            // What a dye put on, read off the character as `Player`'s constructor reads it
            // (`Player.cs:412-413`).
            entity.tex1 = arrival.dyes.0;
            entity.tex2 = arrival.dyes.1;
            entity
                .stats
                .arm(arrival.weapon_damage.0, arrival.weapon_damage.1);
            entity.stats.apply_equipment(&arrival.bonuses);

            // Only a skin the content knows and that belongs to this class, which is what
            // `ReskinHandler` checks before it puts one on (`ReskinHandler.cs:40-55`): a body
            // wearing a skin the client cannot resolve is a body with no sprite at all. A skin also
            // brings its own size (`Player.cs:434-435`), which is the whole point of the larger
            // ones.
            if arrival.skin != 0
                && let Some(skin) = catalog.skin(ObjectType(arrival.skin))
                && skin.class == avatar
            {
                entity.skin = arrival.skin;
                entity.default_skin = arrival.skin;
                if skin.size > 0 {
                    entity.size = skin.size.clamp(0, u16::MAX as i32) as u16;
                    entity.default_size = entity.size;
                }
            }

            // And what a completed set puts on over the top of it. The order is the original's:
            // `Player.Init` dresses the body in the character's own skin first (`Player.cs:431-436`)
            // and the stat manager it builds afterwards (`:467`) runs `ApplySetBonus`, which
            // overwrites both with the set's -- unchecked, so a set skin belonging to another class
            // is worn all the same.
            if let Some(look) = arrival.set_skin {
                entity.skin = look.skin;
                entity.size = look.size;
            }

            // The maxima are what the stats say, recomputed here rather than carried: health and
            // magic are stats like any other, and a body built without asking them keeps whatever
            // the constructor guessed. That is how a level-twenty wizard ends up with a hundred
            // magic and a spell it cannot pay for.
            entity.reseat_maxima();
            entity.hp = arrival.hp.clamp(1, entity.max_hp);
            entity.mp = arrival.mp.clamp(0, entity.max_mp);
            entity.name = Some(name.as_str().into());
            entity.weapon = arrival.weapon.or(loadout.weapon);

            // A moment before anything will attack them. Somebody who has just walked through a
            // portal has not seen what is in the room yet.
            entity.unseen_ms = hendra_sim::world::NEWCOMER_GRACE_MS;

            // And a moment before their movement is measured against a walking speed. Arriving in
            // a world is a jump from wherever the last one left off, and the claims a client makes
            // before it has been told where it now stands are measured from a position on a map it
            // is no longer standing on. `Player.Init` grants the same grace and for the same
            // reason (`Player.cs:532`).
            entity.move_grace_ms = hendra_sim::world::MOVE_GRACE_MS;

            // A player is a Character too, and the original gives one its class's immunities in the
            // same constructor it gives an enemy theirs.
            if let Some(desc) = catalog.object(avatar) {
                entity.take_immunities(desc);
            }

            // Held for as long as they are here, as the original holds it: `HandleEffects` reapplies
            // it every tick the account is muted, with no duration.
            if arrival.muted {
                let muted = hendra_content::ConditionEffect::Muted;
                entity.conditions.insert(muted);
                entity
                    .effects
                    .push((muted.index() as u8, hendra_sim::world::FOREVER));
            }

            let Some(handle) = world.spawn(entity) else {
                tracing::warn!(world = %world.name, "world is full; refusing a join");
                return;
            };

            // The welcome goes onto the wire before the player exists as far as the tick is
            // concerned, and that ordering is the point of doing it here rather than in the session.
            //
            // `Link::send` and `LinkSender::try_send` both push onto one queue drained by one
            // writer, so what is enqueued first arrives first. The instant this player is in
            // `players` the next tick may send them terrain -- and the client clears every tile it
            // holds when the welcome arrives (`RustSession.OnWelcomed`), so terrain that overtook
            // the welcome would be wiped and, because a square is only ever uncovered once, never
            // offered again. Writing the welcome first makes that impossible rather than unlikely.
            let mut greeting = Vec::new();
            ServerMessage::Welcome {
                player: handle.to_entity_id(),
                tick: hendra_net::Tick::ZERO,
                world: &world.name,

                // Square, both sides the longer of the two, as `ConnectManager` sizes it
                // (`ConnectManager.cs:337-341`).
                width: world.terrain().width().max(world.terrain().height()) as u16,
                height: world.terrain().width().max(world.terrain().height()) as u16,

                background: world.background,
                difficulty: world.difficulty,
                allow_teleport: world.allows_teleport(),
                show_displays: world.show_displays,
                music: &music,
            }
            .encode(&mut Writer::new(&mut greeting));

            if let Err(err) = sender.try_send(Delivery::Stream, &greeting) {
                tracing::warn!(world = %world.name, %name, %err, "could not welcome an arrival");
                world.despawn(handle);
                return;
            }

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
                walked_at: Instant::now(),
                allowance_ms: MOST_ALLOWANCE_MS,
                shot_clock: TimeCop::new(),
                history: BaselineRing::new(),
                encoder: SnapshotEncoder::with_budget(budget),
                acknowledged: Acknowledgement::NONE,
                pending_map: std::collections::VecDeque::new(),
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
            let mut allowance = tick_ms(client_time_ms);

            if let Some(player) = players.iter_mut().find(|player| player.handle == handle) {
                player.acknowledged = ack;

                // What the clock has handed this player since their last claim, plus whatever they
                // did not use then. Capped, or a player who stood still for a minute could cross a
                // map in one message. There is nothing to match here: the original never judged a
                // player's speed at all, so the cap is this server's own (see MOST_ALLOWANCE_MS).
                let now = Instant::now();
                let since = now.duration_since(player.walked_at);
                player.walked_at = now;
                player.allowance_ms = topped_up(player.allowance_ms, since);

                allowance = player.allowance_ms;
            }

            // The claim is advisory. The world decides where the player actually is.
            if let Some(outcome) = world.resolve_move(handle, catalog, x, y, allowance) {
                if let Some(player) = players.iter_mut().find(|player| player.handle == handle) {
                    player.allowance_ms = player.allowance_ms.saturating_sub(outcome.spent_ms);
                }

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
                        // Whether the square the claim named could have been stood on. A refusal
                        // with this true is a wall that was crossed rather than landed on: the
                        // claim began on open ground and ended on open ground, and only looking at
                        // what lay between them found anything wrong with it.
                        landing_clear = world.terrain().walkable_at(x, y),
                        "a move was clamped"
                    );
                }

                world.place(handle, outcome);
            }
        }

        ToWorld::Shoot {
            handle,
            angle,
            client_time_ms,
        } => {
            // The clock the shot claims to have been taken on is checked against the server's
            // before it is used for anything, which is the pair of tests `ValidatePlayerShoot` ends
            // with (`Player.AntiCheat.cs:114-119`). The original runs them after its interval test
            // rather than before; running them first only means a client that is also asking too
            // fast has both of its samples counted, and it keeps the judgement in the one place
            // that knows how long the packets really took to arrive.
            if let Some(player) = players.iter_mut().find(|player| player.handle == handle) {
                let running = player.shot_clock.observe(client_time_ms);
                if running < SLOWEST_CLOCK || running > FASTEST_CLOCK {
                    tracing::debug!(
                        world = %world.name,
                        name = %player.name,
                        running,
                        "a shot was refused: the client's clock is not keeping server time"
                    );
                    return;
                }
            }

            // The world decides whether the weapon is ready, where the shot goes and what it hits.
            // A client that asks faster than its rate of fire is refused, not believed. What was
            // fired is announced with everything else fired this tick, so a player's shot and the
            // enemy's answer travel by the same road.
            world.shoot_at(handle, catalog, angle, client_time_ms);
        }

        ToWorld::OpenPortal {
            at,
            kind,
            duration_ms,
            opened_by,
            reply,
        } => {
            let portal = world.open_portal(catalog, at, kind, duration_ms);

            if let Some(announced) = opened_by {
                announce_opened_dungeon(world, players, at, &announced);
            }

            if let Some(reply) = reply {
                let _ = reply.send(portal.map(Handle::to_entity_id));
            }
        }

        ToWorld::UnlockPortal {
            at,
            locked,
            unlocked,
            duration_ms,
            announced,
            reply,
        } => {
            let opened = world.swap_locked_portal(catalog, at, locked, unlocked, duration_ms);

            // Only when a door actually changed. A key used where its door is not standing says
            // nothing, because the original returns before it reaches the announcement
            // (`Player.UseItem.cs:441-446`).
            if opened.is_some() {
                announce_unlocked_dungeon(world, players, at, &announced);
            }

            let _ = reply.send(opened.map(Handle::to_entity_id));
        }

        ToWorld::Teleport { handle, to, reply } => {
            let answer = match world.player_named(&to) {
                Some(target) => world.teleport_to(handle, target).map(|why| why.to_string()),
                None => Some(format!("Unable to find player: {to}")),
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

        // Answered by the loop, which is where the board lives.
        ToWorld::ShowListings { .. } => {}

        ToWorld::ShowPortals { portals } => {
            show_portals(world, catalog, loadout, &portals);
        }

        ToWorld::SendEveryoneTo { world: destination } => {
            // Somebody paused goes to the Nexus instead of wherever everyone else is being sent.
            // `World.Quake` builds two `Reconnect` packets and picks between them per player
            // (`World.cs:547-573`), so a pause cannot be used to ride a quake into the next realm
            // without being at the keyboard for it.
            // The ground shakes first and nobody moves for eight seconds. `QuakeToWorld`
            // (`World.cs:547-552`) broadcasts the earthquake and only then arms the timer that
            // reconnects anybody, which is what makes the wait mean something rather than being a
            // world that swaps out from under a player mid-step.
            world.show_effect(hendra_sim::world::EffectEvent {
                effect: hendra_net::message::effect::EARTHQUAKE,
                target: None,
                x1: 0.0,
                y1: 0.0,
                x2: 0.0,
                y2: 0.0,
                color: 0,
            });

            for player in players.iter() {
                let paused = world
                    .get(player.handle)
                    .is_some_and(|entity| entity.conditions.contains(PAUSED));
                let going = if paused {
                    "Nexus".to_string()
                } else {
                    destination.clone()
                };

                // A task rather than anything the tick loop holds, which is what the original's
                // one-shot `WorldTimer` amounts to: nothing else in the world depends on it, and a
                // world that is torn down before it fires leaves a send nobody receives.
                let orders = player.orders.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        hendra_sim::realm::QUAKE_AFTER_MS,
                    ))
                    .await;
                    let _ = orders.try_send(Order::GoTo(going));
                });
            }
        }

        ToWorld::Wield {
            handle,
            what,
            reply,
        } => {
            let _ = reply.send(wield_in_world(world, catalog, loadout, handle, what));
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
                speaker: hendra_net::EntityId(0),
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
                speaker: handle.to_entity_id(),
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

        ToWorld::PlaceVaultAccess { reply } => {
            let _ = reply.send(place_vault_access(world, catalog));
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
            soulbound,
            reply,
        } => {
            let _ = reply.send(put_in_bag(world, catalog, player, bag, item, soulbound));
        }

        // The weapon's damage first, because the floor under each bonus is measured against the
        // base and one of the eleven base values is about to change. `ReCalculateValues` runs the
        // two managers in that order for the same reason (`StatsManager.cs:38-39`).
        ToWorld::Equipment {
            handle,
            bonuses,
            weapon_damage,
            weapon,
            set_skin,
        } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.stats.arm(weapon_damage.0, weapon_damage.1);
                entity.stats.apply_equipment(&bonuses);

                // And what the body shoots with, which is the same slot the damage above was read
                // from. The original has nothing to update because it never caches the weapon --
                // `PlayerShootHandler` reads `Inventory[0]` on every shot -- so following the slot
                // here is how a held weapon behaves as a re-read one. Falls back the same way an
                // arrival does, so swapping a weapon and walking through a portal agree.
                entity.weapon = weapon.or(loadout.weapon);

                // The health bar moves with the robe. `ReCalculateValues` is wired straight to the
                // inventory (`Player.cs:463`), and since the original reads `Stats[0]` rather than
                // carrying it, putting on a piece that grants maximum health raises the bar in the
                // same breath as the number under it.
                entity.reseat_maxima();

                // And what the player looks like, which a set changes and nothing else about a slot
                // does. Written from what is worn every time rather than only on the change that
                // completes or breaks the set: `ApplySetBonus` assigns the skin again on every
                // recalculation while the set is on (`BoostStatManager.cs:73-76`), and answering
                // from the slots as they stand is what stops a set skin outliving the set.
                match set_skin {
                    Some(look) => {
                        entity.skin = look.skin;
                        entity.size = look.size;
                    }

                    // `RestoreDefaultSkin` and `RestoreDefaultSize` (`BoostStatManager.cs:107-108`):
                    // back to the skin the wardrobe chose, and the size that skin brings with it.
                    None => {
                        entity.skin = entity.default_skin;
                        entity.size = entity.default_size;
                    }
                }
            }
        }

        // Added to whatever is left rather than replacing it, which is `XPBoostTime += eff.DurationMS`
        // (`Player.UseItem.cs:536`). A boost written as never lapsing is left alone, since nothing
        // can be added to forever.
        ToWorld::ExperienceBoost {
            handle,
            milliseconds,
        } => {
            if let Some(entity) = world.get_mut(handle)
                && entity.experience_boost_ms >= 0
            {
                entity.experience_boost_ms = entity.experience_boost_ms.saturating_add(milliseconds);
            }
        }

        ToWorld::Purse { handle, purse } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.purse = purse;
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

        ToWorld::MayUseItem {
            handle,
            item,
            reply,
        } => {
            let _ = reply.send(world.may_use_item(handle, catalog, item));
        }

        ToWorld::UseFromContainer {
            handle,
            container,
            slot,
            aim,
            reply,
        } => {
            let _ = reply.send(use_from_container(
                world, catalog, handle, container, slot, aim,
            ));
        }

        ToWorld::Snapshot { handle, reply } => {
            let vitals = world.get_mut(handle).map(|entity| {
                // The tally is moved out rather than read. Everything that asks for a snapshot goes
                // on to add the counts to the row, and a count left on the body would be added
                // again by the next checkpoint.
                let tally = std::mem::take(&mut entity.tally);

                Vitals {
                    hp: entity.hp,
                    mp: entity.mp,
                    max_hp: entity.max_hp,
                    max_mp: entity.max_mp,
                    level: entity.progress.level,
                    experience: entity.progress.experience,
                    fame: entity.progress.fame,
                    stats: entity.stats.to_base(),
                    tally: counted(&tally),
                }
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
}

/// The marketplace's merchants, and what each of them is holding.
///
/// The board of items and whose turn it is lives in [`hendra_sim::market`]; this is the part that
/// has to touch the world, which is standing an entity on each square and keeping what it shows in
/// agreement with the board.
struct Board {
    market: hendra_sim::market::Market,

    /// The entity standing on each pitch, parallel to the market's own pitches.
    stalls: Vec<Option<Handle>>,

    /// The object a merchant is. `Market.AddMerchants` builds every one of them as `0x01ca`
    /// (`realm/Market.cs:290`).
    merchant: ObjectType,
}

impl Board {
    /// Reads the squares a world marks for the marketplace, or `None` for a world that marks none.
    fn of_world(world: &World, catalog: &Catalog) -> Option<Board> {
        let market = hendra_sim::market::Market::of_map(world.terrain().map());
        if market.pitches().is_empty() {
            return None;
        }

        let merchant = catalog.type_of(MERCHANT)?;
        let stalls = vec![None; market.pitches().len()];

        Some(Board {
            market,
            stalls,
            merchant,
        })
    }

    /// Makes the world agree with the board.
    ///
    /// A merchant that is still holding the same thing is left alone rather than replaced, so a
    /// rotation reaches the client as a change of stats on an entity it already knows about. That is
    /// what the original does: the `PlayerMerchant` outlives every item it shows, and only
    /// `RemoveMerchant` takes one out of the world (`realm/Market.cs:338-344`).
    fn apply(&mut self, world: &mut World) {
        for pitch in 0..self.market.pitches().len() {
            let place = self.market.pitches()[pitch];
            let wanted = self.market.standing(pitch).map(|held| hendra_sim::shop::Stall {
                item: held.item,
                price: held.price,
                currency: hendra_sim::shop::Currency::Fame,

                // `PlayerMerchant`'s constructor asks for two stars (`PlayerMerchant.cs:17`),
                // which is what stops a fresh account buying up a marketplace.
                rank: MARKET_RANK,
                count: held.count,
                listing: Some(held.listing),
            });

            let standing = self.stalls[pitch].filter(|handle| world.get(*handle).is_some());

            match (wanted, standing) {
                (Some(stall), Some(handle)) => {
                    if let Some(entity) = world.get_mut(handle) {
                        entity.selling = Some(stall);
                    }
                }
                (Some(stall), None) => {
                    self.stalls[pitch] = world.open_stall(self.merchant, place.x, place.y, stall);
                }
                (None, Some(handle)) => {
                    world.despawn(handle);
                    self.stalls[pitch] = None;
                }
                (None, None) => self.stalls[pitch] = None,
            }
        }
    }

    /// Whether somebody is standing close enough to hold a merchant still.
    ///
    /// `this.AnyPlayerNearby(2)` (`realm/entities/vendors/Merchant.cs:106`), which is a circle
    /// rather than a square, is strict at the edge, and does not count anybody hidden
    /// (`realm/Utils.cs:25-37`).
    fn crowded(world: &World, place: hendra_sim::market::Pitch) -> bool {
        let (x, y) = (place.x as f32 + 0.5, place.y as f32 + 0.5);
        let reach = hendra_sim::market::HOLD_RADIUS;

        world.iter().any(|(_, entity)| {
            if entity.kind != hendra_sim::Kind::Player
                || entity
                    .conditions
                    .contains(hendra_content::ConditionEffect::Hidden)
            {
                return false;
            }

            let (dx, dy) = (entity.x - x, entity.y - y);
            dx * dx + dy * dy < reach * reach
        })
    }
}

/// The stars somebody needs before a player merchant will trade with them.
///
/// `PlayerMerchant` sets `RankReq = 2` (`realm/entities/vendors/PlayerMerchant.cs:17`), where the
/// shops the map stands ask for nothing.
const MARKET_RANK: i16 = 2;

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

/// How large the nexus draws a portal it stands itself.
///
/// `AddPortal` calls `SetDefaultSize(80)` on the portal it builds (`PortalMonitor.cs:79`). Nothing
/// else in the original scales a portal: `Entity` starts every entity at 100 and only `Character`
/// reads a size out of the object description (`Character.cs:35-39`), so a portal drawn from a map
/// stays at 100 however large its XML says it is.
const NEXUS_PORTAL_SIZE: u16 = 80;

/// Stands a portal for each named world on the squares the map marks for them.
///
/// Reconciled rather than rebuilt. `PortalMonitor` holds one portal per world for the life of that
/// world and only ever renames it: `Tick` (`PortalMonitor.cs:226-246`) rewrites the trailing count
/// with the regex ` \((\d+)\)$` and assigns to `Name`, which re-sends the stat and nothing else.
/// Standing new portals each refresh would give every one of them a new entity id and a new square
/// several times a minute, which the client draws as the door somebody was walking towards
/// disappearing and another appearing elsewhere.
fn show_portals(world: &mut World, catalog: &Catalog, loadout: &Loadout, portals: &[PortalSign]) {
    let pads: Vec<(u32, u32)> = world
        .terrain()
        .map()
        .regions()
        .filter(|(_, _, region)| *region == hendra_content::Region::RealmPortals)
        .map(|(x, y, _)| (x, y))
        .collect();

    if pads.is_empty() {
        return;
    }

    // What is already standing on the pads, and where each of them leads. The label carries the
    // world's name, which is what the original's `_portals` dictionary is keyed by.
    let standing: Vec<(Handle, String)> = world
        .iter()
        .filter(|(_, entity)| {
            entity.kind == hendra_sim::Kind::Portal
                && pads
                    .iter()
                    .any(|(x, y)| entity.x == *x as f32 + 0.5 && entity.y == *y as f32 + 0.5)
        })
        .filter_map(|(handle, entity)| {
            let labelled = entity.name.as_deref()?;
            Some((handle, labelled_world(labelled).to_string()))
        })
        .collect();

    // A portal whose world has gone is a door into nothing, and the original takes it down the
    // moment the world goes: `OnWorldRemoved` calls `Monitor.RemovePortal(world.Id)`
    // (`RealmManager.cs:383`).
    for (handle, destination) in &standing {
        if !portals.iter().any(|sign| &sign.world == destination) {
            world.despawn(*handle);
        }
    }

    for sign in portals {
        let wanted = format!("{} ({})", sign.world, sign.players);

        // Already standing: renamed where it is, and only when the count has actually moved, which
        // is the whole of `PortalMonitor.Tick`.
        if let Some((handle, _)) = standing.iter().find(|(_, held)| *held == sign.world) {
            if let Some(entity) = world.get_mut(*handle)
                && entity.name.as_deref() != Some(wanted.as_str())
            {
                entity.name = Some(wanted.into());
            }
            continue;
        }

        let Some(desc) = catalog.object(sign.portal) else {
            continue;
        };

        let Some((x, y)) = free_pad(world, &pads) else {
            continue;
        };

        let mut portal =
            hendra_sim::world::Entity::fixture(desc.object_type, x as f32 + 0.5, y as f32 + 0.5);
        portal.kind = hendra_sim::Kind::Portal;
        portal.size = NEXUS_PORTAL_SIZE;

        // Labelled with the count, which is what somebody choosing a realm is actually reading.
        portal.name = Some(wanted.into());
        world.spawn(portal);

        // Everybody on the server is told, wherever they are standing: `AddPortal` walks every
        // world's players and sends each of them a line (`PortalMonitor.cs:86-91`).
        let _ = loadout.portals_opened.send(sign.world.clone());

        tracing::info!(
            world = %sign.world,
            object_type = format!("0x{:04x}", sign.portal.0),
            x, y,
            players = sign.players,
            "nexus portal opened"
        );
    }
}

/// What a player standing in `here` is told when a portal to `destination` opens.
///
/// Word for word from `AddPortal` (`PortalMonitor.cs:86-91`), including both of its substitutions:
/// somebody already in the world the portal leads to is told about "this land" rather than about a
/// place by name, and somebody standing in the Nexus is not told where the portal is, because they
/// are looking at it.
fn portal_opened_line(here: &str, destination: &str) -> String {
    let subject = if here == destination {
        "this land"
    } else {
        destination
    };
    let suffix = if here == crate::commands::NEXUS {
        ""
    } else {
        " in Nexus"
    };
    format!("A portal to {subject} has opened up{suffix}.")
}

/// The world a portal's label names, which is everything before the trailing player count.
///
/// The counterpart of what `PortalMonitor.Tick` writes: it replaces ` \((\d+)\)$` and leaves the
/// rest of the name alone, so the rest of the name is the world.
fn labelled_world(label: &str) -> &str {
    let Some(open) = label.rfind(" (") else {
        return label;
    };
    let count = &label[open + 2..];
    let Some(digits) = count.strip_suffix(')') else {
        return label;
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return label;
    }
    &label[..open]
}

/// A pad nothing is standing on, drawn at random.
///
/// `GetRandPosition` (`PortalMonitor.cs:37-54`) draws a `Realm_Portals` square at random and draws
/// again while a portal is already at it, so a new portal never lands on an old one and the ones
/// already down never move. When there are more portals than squares it gives up and answers
/// `(0, 0)`, which puts the portal in the map's corner; that oddity is kept.
fn free_pad(world: &mut World, pads: &[(u32, u32)]) -> Option<(u32, u32)> {
    let taken: Vec<(u32, u32)> = world
        .iter()
        .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Portal)
        .filter_map(|(_, entity)| {
            pads.iter()
                .find(|(x, y)| entity.x == *x as f32 + 0.5 && entity.y == *y as f32 + 0.5)
                .copied()
        })
        .collect();

    if taken.len() >= pads.len() {
        return Some((0, 0));
    }

    // Drawn rather than scanned, so the realm's door is not always at the same corner of the pad.
    loop {
        let index = (world.roll() * pads.len() as f32) as usize;
        let pad = pads[index.min(pads.len() - 1)];
        if !taken.contains(&pad) {
            return Some(pad);
        }
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
    ///
    /// `summoned` is the original's `Spawned` flag, which `/spawn` sets and `/lootspawn` does not:
    /// an enemy carrying it awards no experience and drops no loot, and is invisible besides. That
    /// one flag is the whole difference between the two commands.
    Spawn {
        name: String,
        count: usize,
        summoned: bool,
    },

    /// Kill every enemy whose name contains this, or every enemy when it is empty.
    KillAll { name: String },

    /// Remove the gravestones standing near the caller.
    ClearGraves,

    /// Move a named player here.
    Summon { name: String },

    /// Watch a named player, if they are in this world.
    Spectate { name: String },

    /// Take control of the nearest enemy of this name.
    Warg { name: String },

    /// Go to where the caller's quest is.
    ToQuest,

    /// Draw them at this percentage of their natural size.
    Size { percent: u16 },

    /// Put a skin on them, and the size it brings with it.
    ///
    /// `Player.SetDefaultSkin` + `SetDefaultSize` (`Player.cs:1143-1152`), which is what the
    /// wardrobe calls once it has checked the skin belongs to the class (`ReskinHandler.cs:63-64`).
    /// The body already standing in the world is the one that has to change: a skin that only
    /// applied on the next arrival would be a wardrobe nobody could see themselves use.
    Skin { skin: u16, size: u16 },

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

    /// Take the caller to the godlands, which is a place in a realm rather than a world.
    Godlands,

    /// Take the caller to the top level, with the stats the skipped levels would have given.
    MaxLevel,

    /// Set every one of the caller's stats to the most their class allows.
    MaxStats,
}

/// Carries out an administrator's tool, and says what happened.
fn wield_in_world(
    world: &mut World,
    catalog: &Catalog,
    loadout: &Loadout,
    handle: Handle,
    what: Wielding,
) -> String {
    match what {
        Wielding::Spawn {
            name,
            count,
            summoned,
        } => {
            let Some(kind) = catalog.find_by_name(&name) else {
                return "Unknown entity!".to_string();
            };
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "Unknown entity!".to_string();
            };
            let id = catalog
                .object(kind)
                .map(|desc| desc.id.clone())
                .unwrap_or(name);

            // Bounded, because a typo in a number should not be the last thing a world does. The
            // original bounds it at five hundred in the timer that does the spawning.
            let count = count.clamp(1, MOST_SPAWNED_AT_ONCE);

            // Which handles are new is worked out rather than asked for, because the flag has to go
            // on the ones this command put down and on nothing else.
            let before: Vec<Handle> = world.iter().map(|(handle, _)| handle).collect();
            let made = world.spawn_at(catalog, kind, x, y, count);

            if summoned {
                let fresh: Vec<Handle> = world
                    .iter()
                    .map(|(handle, _)| handle)
                    .filter(|handle| !before.contains(handle))
                    .collect();

                for handle in fresh {
                    // The flag goes on everything the command put down, and the Invisible only on
                    // the enemies among them: `SpawnCommand` sets `entity.Spawned` before the cast
                    // and applies the effect inside `if (enemy != null)`
                    // (`RankedCommands.cs:561-583`).
                    let is_enemy = match world.get_mut(handle) {
                        Some(entity) => {
                            entity.spawned = true;
                            entity.kind == hendra_sim::Kind::Enemy
                        }
                        None => continue,
                    };

                    // Through the effect path rather than the condition bitmask, because that
                    // bitmask is rebuilt from the held effects on every tick that ages them: a
                    // condition written without its timer survives only on an entity that happens
                    // to hold no other effect, and every entity born with a descriptor immunity
                    // holds one. `DurationMS = -1` is a duration that is never counted down.
                    if is_enemy {
                        world.give_effect(
                            handle,
                            hendra_content::ConditionEffect::Invisible.index() as u8,
                            hendra_sim::world::FOREVER,
                        );
                    }
                }
            }

            // `/spawn` trails off and `/lootspawn` does not: the two copies of `NotifySpawn` differ
            // by those three characters, and the line is what a room sees.
            let trailing = if summoned { "..." } else { "" };
            let line = if made > 1 {
                format!("Spawning {made} {id}{trailing}")
            } else {
                format!("Spawning {id}{trailing}")
            };

            // Red, over the caller's own head, to everyone in the world: `NotifySpawn`
            // (`RankedCommands.cs:250`, `:510`) is a `BroadcastPacket` with no distance test. It is
            // the warning that something is about to be standing there, so a player who cannot see
            // the caller is the one who most needs it.
            world.float_text_everywhere(handle, &line, SPAWNING_TEXT_COLOUR);
            line
        }

        Wielding::KillAll { name } => {
            // A substring of the name rather than the whole of it, and an empty argument matches
            // every enemy in the room.
            //
            // Two tests, not one. `KillAllCommand` walks `Owner.Enemies.Values`
            // (`RankedCommands.cs:916`), which `World.EnterWorld` fills with `Enemy` instances alone
            // (`World.cs:338-345`) — breakable scenery is a `StaticObject` and goes elsewhere — and
            // it then asks the descriptor's own `<Enemy/>` flag on top of that. Since a
            // `Class=Character` object need not carry the flag, the two are different questions and
            // both are asked. The nexus crier is named and spared outright.
            let wanted = name.trim().to_lowercase();
            let doomed: Vec<Handle> = world
                .iter()
                .filter(|(_, entity)| {
                    entity.kind == hendra_sim::Kind::Enemy
                        && !entity.dead
                        && catalog.object(entity.object_type).is_some_and(|desc| {
                            desc.enemy
                                && desc.id != "Tradabad Nexus Crier"
                                && (wanted.is_empty() || desc.id.to_lowercase().contains(&wanted))
                        })
                })
                .map(|(handle, _)| handle)
                .collect();

            let killed = doomed.len();
            for handle in doomed {
                if let Some(entity) = world.get_mut(handle) {
                    // Marked as summoned first, exactly as `KillAllCommand` sets `Spawned` before
                    // killing: an administrator clearing a room hands out neither the experience
                    // nor the loot for it.
                    entity.spawned = true;
                    entity.dead = true;
                }
            }

            format!("{killed} enemy killed!")
        }

        Wielding::ClearGraves => {
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "0 gravestones removed!".to_string();
            };

            let doomed: Vec<Handle> = world
                .iter()
                .filter(|(_, entity)| {
                    let near = (entity.x - x).hypot(entity.y - y) < GRAVE_REACH;
                    near && catalog
                        .object(entity.object_type)
                        .is_some_and(|desc| desc.id.starts_with("Gravestone"))
                })
                .map(|(handle, _)| handle)
                .collect();

            let removed = doomed.len();
            for handle in doomed {
                world.despawn(handle);
            }

            format!("{removed} gravestones removed!")
        }

        Wielding::Summon { name } => {
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return format!("Player '{name}' could not be found!");
            };
            let Some(target) = world.player_named(name.trim()) else {
                return format!("Player '{name}' could not be found!");
            };

            // Somebody hidden is left where they are, as the original leaves them.
            if world
                .get(target)
                .is_some_and(|entity| entity.conditions.contains(HIDDEN))
            {
                return format!("Player '{name}' could not be found!");
            }

            if let Some(entity) = world.get_mut(target) {
                entity.x = x;
                entity.y = y;
                entity.move_grace_ms = hendra_sim::world::MOVE_GRACE_MS;
            }

            "Player summoned!".to_string()
        }

        Wielding::Spectate { name } => {
            let Some(target) = world.player_named(name.trim()) else {
                return "Player not found. Note: Target player must be on the same map."
                    .to_string();
            };

            let found = world
                .get(target)
                .and_then(|entity| entity.name.clone())
                .map(|name| name.to_string())
                .unwrap_or(name);

            // Two halves: the camera, which the client is told to move, and the pause, which the
            // world owns. Naming yourself ends it, which is what `/spectate <own name>` does —
            // `SetFocus` carries the target's id either way (`UnrankedCommands.cs:1420-1441`).
            if target != handle {
                world.give_effect(handle, PAUSED.index() as u8, hendra_sim::world::FOREVER);
            } else {
                // A duration of zero is how the original takes an effect off, and it is what
                // `ApplyConditionEffect(Paused, 0)` does on the way out of spectating.
                world.give_effect(handle, PAUSED.index() as u8, 0);
            }
            world.watch(handle, target);

            format!("Now spectating {found}. Use the /self command to exit.")
        }

        Wielding::Warg { name } => {
            // Anything with a name, not only a monster. `WargCommand` asks
            // `GetNearestEntityByName(2900, name)` (`RankedCommands.cs:2363`), and that walks
            // `EnemiesCollision` testing nothing but "has a descriptor" and "the name contains this"
            // (`Utils.cs:245-257`) — no `is Enemy`, no `<Enemy/>` flag. `World.EnterWorld` puts
            // every `StaticObject` that is not a decoy into that same map (`World.cs:352-361`), so a
            // wine barrel, a wall, a merchant and a portal are all wargable. The radius is 2900
            // tiles, which is wider than any map in the game and so is no filter at all.
            let wanted = name.trim().to_lowercase();
            let found = world.iter().any(|(_, entity)| {
                !matches!(
                    entity.kind,
                    hendra_sim::Kind::Player | hendra_sim::Kind::Decoy
                ) && !entity.dead
                    && catalog
                        .object(entity.object_type)
                        .is_some_and(|desc| desc.id.to_lowercase().contains(&wanted))
            });

            if !found {
                return "Mob not found.".to_string();
            }

            // Controlling one is the client's half, and there is no packet here that hands a body
            // over, so this answers what it found and does nothing to it.
            "Only one person can control a mob at a time.".to_string()
        }

        Wielding::ToQuest => {
            let Some((_, x, y)) = nearest_quest(world, catalog, handle) else {
                return "Player does not have a quest!".to_string();
            };

            world.teleport_to_square(handle, x as f32 + 0.5, y as f32 + 0.5);
            format!("Teleported to Quest Location: ({x}, {y})")
        }

        Wielding::Godlands => {
            // The fixed square GLandCommand.cs names, and only in a realm: the coordinates are a
            // point on the realm's map and mean nothing anywhere else.
            if !loadout.is_realm {
                return "This command requires you to be in realm first.".to_string();
            }

            match world.teleport_to_square(handle, GODLANDS_X, GODLANDS_Y) {
                Some(why) => why.to_string(),
                None => String::new(),
            }
        }

        Wielding::MaxLevel | Wielding::MaxStats => {
            let Some(entity) = world.get_mut(handle) else {
                return "you are nowhere".to_string();
            };
            let Some(class) = catalog.class(entity.object_type) else {
                return "your class is not in the content".to_string();
            };

            // Applied to the body standing here, so the stats appear at once rather than on the
            // next arrival, and the world's own save path carries them to the store.
            if what == Wielding::MaxStats {
                for stat in hendra_content::STATS {
                    let ceiling = class.stat(stat).maximum;
                    entity.stats.raise(class, stat, ceiling);
                }
                refill(entity);
                return "Your character stats have been maxed.".to_string();
            }

            // Silent either way: `Level20Command` answers with nothing at all, whether it raised
            // the character or found it already at twenty.
            if entity.progress.jump_to_max_level(class, &mut entity.stats) {
                refill(entity);
            }
            String::new()
        }

        Wielding::Size { percent } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.size = percent;
            }
            String::new()
        }

        Wielding::Skin { skin, size } => {
            if let Some(entity) = world.get_mut(handle) {
                // The default as well as what is worn now, because `ReskinHandler` chooses one
                // through `SetDefaultSkin` and `SetDefaultSize` (`ReskinHandler.cs:63-64`): this is
                // what a set skin is taken off back to when the set is broken.
                entity.skin = skin;
                entity.default_skin = skin;
                if size > 0 {
                    entity.size = size;
                    entity.default_size = size;
                }
            }
            String::new()
        }

        Wielding::Hide => {
            let hidden = world
                .get(handle)
                .is_some_and(|entity| entity.conditions.contains(HIDDEN));

            // Invincible alongside hidden, as `Player.HandleEffects` applies both to an
            // administrator, and both come off together. Held for ever rather than written as a
            // bare bit, because the bits are rebuilt from the held durations every tick.
            let duration = if hidden { 0 } else { hendra_sim::world::FOREVER };
            world.give_effect(handle, HIDDEN.index() as u8, duration);
            world.give_effect(handle, INVINCIBLE.index() as u8, duration);

            String::new()
        }

        Wielding::Pause => {
            // Kept exactly as `PauseCommand` behaves, dead code and all: a paused player is
            // released, and everybody else is told they cannot pause in an arena, because the guard
            // the original tests — `if (owner != null)` — is true for every player in every world.
            let paused = world
                .get(handle)
                .is_some_and(|entity| entity.conditions.contains(PAUSED));

            if !paused {
                return "Can't pause in arena.".to_string();
            }

            world.give_effect(handle, PAUSED.index() as u8, 0);
            "Game resumed.".to_string()
        }

        Wielding::Effect { name } => {
            let Some(effect) = hendra_content::ConditionEffect::from_name(name.trim()) else {
                return "Invalid effect!".to_string();
            };

            // A toggle rather than a switch, as `ToggleEffCommand` is: typing it twice puts things
            // back the way they were. Held for ever when it is on, which is the `DurationMS = -1`
            // the original applies, and cleared with the zero duration the original clears with.
            //
            // Through the world rather than by writing the bit, because the bits are rebuilt from
            // the held durations every tick: one written on its own lasts until the entity picks
            // up any timed effect at all and is then silently wiped.
            let held = world
                .get(handle)
                .is_some_and(|entity| entity.conditions.contains(effect));

            world.give_effect(
                handle,
                effect.index() as u8,
                if held { 0 } else { hendra_sim::world::FOREVER },
            );
            String::new()
        }

        Wielding::Glow { colour } => {
            if let Some(entity) = world.get_mut(handle) {
                entity.glow = colour;
            }
            String::new()
        }

        Wielding::KillPlayer { name } => {
            let Some(target) = world.player_named(name.trim()) else {
                return format!("Player '{name}' could not be found!");
            };

            // Blamed on whoever typed it, as `KillPlayerCommand` blames `player.Name`
            // (`RankedCommands.cs:1056`), and taken through the death path so the character is
            // written down as dead rather than quietly emptied of health.
            let killer = world
                .get(handle)
                .and_then(|caller| caller.name.as_deref())
                .unwrap_or("Unknown")
                .to_string();

            world.slay(target, killer);
            "Player killed!".to_string()
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

            for held in summoned {
                if let Some(entity) = world.get_mut(held) {
                    entity.x = x;
                    entity.y = y;
                    entity.move_grace_ms = hendra_sim::world::MOVE_GRACE_MS;
                }
            }

            "All players summoned!".to_string()
        }

        Wielding::Setpiece { name } => {
            let Some(kind) = hendra_sim::setpiece::Kind::named(name.trim()) else {
                return "Invalid SetPiece.".to_string();
            };
            let Some((x, y)) = world.get(handle).map(|entity| (entity.x, entity.y)) else {
                return "Invalid SetPiece.".to_string();
            };

            // By its corner, one square past where the caller is standing, which is the point
            // `SetpieceCommand` hands to `RenderSetPiece` (`RankedCommands.cs:857`). So the piece
            // appears south-east of whoever asked for it rather than around them.
            let at = (x as u32 + 1, y as u32 + 1);

            let mut dice = hendra_sim::setpiece::Dice::new(at.0 ^ at.1 ^ 0x5e7);
            let drawing = kind.draw(&mut dice);

            // A setpiece that is a saved map goes through `RenderFromProto` in the original and is
            // stamped here. Without this branch the four that are saved maps drew nothing at all.
            if let Some(prefab) = drawing.prefab {
                let Some(map) = read_prefab(&loadout.maps, prefab, catalog) else {
                    return "Invalid SetPiece.".to_string();
                };
                world.stamp(catalog, &map, prefab_middle(&map, at.0, at.1));
                return String::new();
            }

            let missing = world.draw(catalog, &drawing, at);

            if missing.is_empty() {
                String::new()
            } else {
                format!("{name} drawn, without {}", missing.join(", "))
            }
        }

        Wielding::ClearSpawn => {
            // Only what was summoned, which is what `ClearSpawnsCommand` clears: an enemy that
            // belongs to the room is left where it is.
            let doomed: Vec<Handle> = world
                .iter()
                .filter(|(_, entity)| {
                    entity.kind == hendra_sim::Kind::Enemy && !entity.dead && entity.spawned
                })
                .map(|(handle, _)| handle)
                .collect();

            let count = doomed.len();
            for handle in doomed {
                if let Some(entity) = world.get_mut(handle) {
                    entity.dead = true;
                }
            }

            format!("{count} spawned entities removed!")
        }

        Wielding::Debug => format!(
            "{}: {} entities, {} of them enemies, tick {}",
            world.name,
            world.len(),
            world.enemy_count(),
            world.tick_number().0
        ),

        Wielding::CloseRealm => {
            // The warning is server-wide, as it is when the half-hour rather than a command starts
            // the sequence: both go through `InitCloseRealm` (`Oryx.cs:869`).
            if world.realm_mut().close_now() {
                let _ = loadout
                    .announcements
                    .send("Realm closing in 1 minute.".to_string());
                tracing::info!(world = %world.name, "realm closing in 1 minute");
                String::new()
            } else {
                "Realm already closing.".to_string()
            }
        }
    }
}

/// How near a gravestone has to be for `/cleargraves` to reach it.
const GRAVE_REACH: f32 = 15.0;

/// What being untouchable is, which is what `/hide` gives alongside invisibility.
const INVINCIBLE: hendra_content::ConditionEffect = hendra_content::ConditionEffect::Invincible;

/// What being paused is, which is the same condition the game's own pause gives.
const PAUSED: hendra_content::ConditionEffect = hendra_content::ConditionEffect::Paused;

/// What being hidden is.
///
/// `Hidden`, not the `Invisible` a cloak gives. The two are different effects with different
/// consequences: `Player.HandleEffects` (`Player.Effects.cs:16-21`) applies `Hidden` to an
/// administrator, and it is `Hidden` that `AnyPlayerNearby` skips (`Utils.cs:42`), that the ocean
/// trench refuses to suffocate (`Player.Ground.cs:18`), and that `/summon` leaves where it is
/// (`RankedCommands.cs:1011`). Naming the wrong one leaves every one of those looking implemented
/// and testing a bit nobody sets.
const HIDDEN: hendra_content::ConditionEffect = hendra_content::ConditionEffect::Hidden;

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

/// The object the vault panel opens from.
///
/// `0x0504`, which is the type the original gives the `Vault Chest` it injects into its own world's
/// object XML (`git show 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs`, `:32`). There the
/// type names a real `Container` with eight slots; here it names the object the panel opens from,
/// which is not a container and holds nothing. The number is kept so nothing else in the data files
/// has to be renumbered.
pub const VAULT_ACCESS: hendra_content::ObjectType = hendra_content::ObjectType(0x0504);

/// How large the access object is drawn, as a percentage of its natural size.
///
/// Ours. The original's definition declares no `<Size>` (pristine `Vault.cs:31-41`), because there
/// are up to eighty chests in the room and none of them is the point of it. There is one object
/// here, and it should read as the reason the room exists.
const VAULT_ACCESS_SIZE: u16 = 200;

/// Puts the vault's access object on the marked square nearest the spawn.
///
/// Nearest-first is `Vault.InitVault`'s own ordering: it collects the `TileRegion.Vault` tiles and
/// sorts them by squared distance from the spawn tile before placing anything on them (pristine
/// `Vault.cs:70-90`). It then fills the nearest `VaultCount` of them with chests and the remaining
/// ones with buyable `ClosedVaultChest`s (`:96-113`) — eighty tiles in the 2020 `Vault.jm`, and the
/// map's own floor was the capacity limit. Capacity is a number on the account here, so only the
/// first square is used; the rest of the garden stays garden.
///
/// Returns how many were placed, which is one or none.
fn place_vault_access(world: &mut World, catalog: &Catalog) -> usize {
    let access_type = catalog.type_of("Vault Chest").unwrap_or(VAULT_ACCESS);

    // Anything left from a previous visit goes, so a second arrival does not stand two of them in
    // the same square.
    let existing: Vec<Handle> = world
        .iter()
        .filter(|(_, entity)| entity.object_type == access_type)
        .map(|(handle, _)| handle)
        .collect();
    for handle in existing {
        world.despawn(handle);
    }

    let mut spawn = (0u32, 0u32);
    let mut places: Vec<(u32, u32)> = Vec::new();

    for (x, y, region) in world.terrain().map().regions() {
        match region {
            hendra_content::Region::Spawn => spawn = (x, y),
            hendra_content::Region::Vault => places.push((x, y)),
            _ => {}
        }
    }

    let Some(&(x, y)) = places.iter().min_by_key(|(x, y)| {
        let dx = *x as i64 - spawn.0 as i64;
        let dy = *y as i64 - spawn.1 as i64;
        (dx * dx + dy * dy, *x, *y)
    }) else {
        tracing::warn!("this map marks no vault squares, so there is nowhere to open the vault");
        return 0;
    };

    let mut access =
        hendra_sim::world::Entity::fixture(access_type, x as f32 + 0.5, y as f32 + 0.5);
    access.size = VAULT_ACCESS_SIZE;

    usize::from(world.spawn(access).is_some())
}

/// How often the vault chest throws a spark.
///
/// Ours, and so is the whole effect. The 2020 vault has no lightning: its `Vault.cs` places chests
/// and gift chests and does nothing else (`git show
/// 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs`). `Vault.CrackleEveryMs` in the working
/// tree is this project's own C#, written in the same fortnight as this constant, so it is evidence
/// about nothing.
const CRACKLE_EVERY_MS: u64 = 2_600;

/// How far above the chest the arcs come down from, in tiles. Ours; see [`CRACKLE_EVERY_MS`].
const CRACKLE_HEIGHT: f32 = 3.5;

/// How far to either side the strike wanders, in tiles, so no two strikes land in the same place.
/// Ours; see [`CRACKLE_EVERY_MS`].
const CRACKLE_DRIFT: f32 = 1.6;

/// The colour of the arc: a warm pale yellow. Ours; see [`CRACKLE_EVERY_MS`].
const CRACKLE_COLOUR: u32 = 0xffff_e9a0;

/// How thick the bolt is drawn.
///
/// `Pos2.X` is a particle size rather than a coordinate (`Structures.cs:145`), and a particle's
/// size is `int((size / 100) * 5)` screen pixels (`Particle.as:60-62`). The working tree's
/// `Vault.Crackle` asks for five -- this project's own C#, not the original's -- which truncates to
/// a zero-pixel core; the bitmap that would
/// be drawn from it is filled with `fillRect(x, y, 0, 0)` and then blurred, which is to say it is
/// empty (`TextureRedrawer.as:114-116`). Five draws nothing, in the original's renderer as much as
/// in ours, so it cannot be what was meant by it.
///
/// Three hundred and fifty is the size every lightning bolt in the original is actually sent at
/// (`Player.UseItem.cs:721`, `:787`), and it is the only one there is.
const CRACKLE_PARTICLE_SIZE: f32 = 350.0;

/// Throws one arc down onto the vault chest.
///
/// Ours -- the 2020 vault has no such effect; see [`CRACKLE_EVERY_MS`]. The lightning is the server's
/// rather than the sprite's: the client already knows how to draw an arc between two points, so the
/// chest borrows it instead of the renderer growing an animation for one object.
fn crackle(world: &mut World, catalog: &Catalog) {
    let access_type = catalog.type_of("Vault Chest").unwrap_or(VAULT_ACCESS);

    let chests: Vec<(Handle, f32, f32)> = world
        .iter()
        .filter(|(_, entity)| entity.object_type == access_type)
        .map(|(handle, entity)| (handle, entity.x, entity.y))
        .collect();

    for (handle, x, y) in chests {
        let drift = world.roll_offset(CRACKLE_DRIFT);

        world.show_effect(hendra_sim::world::EffectEvent {
            effect: hendra_net::message::effect::LIGHTNING,
            target: Some(handle),

            // Down onto the chest from a point overhead. Absolute tiles, not an offset.
            x1: x + drift,
            y1: y - CRACKLE_HEIGHT,
            x2: CRACKLE_PARTICLE_SIZE,
            y2: 0.0,
            color: CRACKLE_COLOUR,
        });
    }
}

/// What came out of a container a player reached into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taken {
    /// The item type that was in the slot.
    pub item: u16,

    /// What kind of container it was in.
    ///
    /// A loot bag is the thing itself and has already given the item up by the time this is read. A
    /// gift chest is a drawing of rows in the database, so it still holds what it held and the
    /// caller has to take the row before the drawing means anything.
    pub kind: hendra_sim::ContainerKind,
}

/// How far a player may reach to use something out of a container.
///
/// Three tiles, which is the only distance `UseItem` checks: `if (this.Dist(entity) > 3)` refuses
/// the whole ask, and the original's own comment above it — "eheh no more clearing BBQ loot bags" —
/// says what it is for (`Player.UseItem.cs:127-132`). Wider than [`BAG_REACH`], because moving an
/// item and using one are different asks and the original measures them differently: a swap wants
/// `DistanceSquared <= 1` (`InvSwapHandler.cs:168`), which is one tile, where this is a plain
/// distance of three.
pub const USE_REACH: f32 = 3.0;

/// Uses an item out of a container standing in the world.
///
/// The container path of `Player.UseItem` (`Player.UseItem.cs:108-246`), which is what a slot alone
/// cannot express. Three refusals of the original's stand in front of it: the entity has to exist
/// and be reachable, the slot has to hold something, and the item has to be one that activates from
/// a container at all. That last is the tail of the method, `item.Consumable || item.SlotType ==
/// slotType` (`Player.UseItem.cs:245`): a bag's slots are untyped, so a sword lying in one is
/// refused and a potion is not.
///
/// Emptied before it is used, which is the order the original settles a consumable in and the same
/// order [`take_from_bag`] already takes one in. What is in a bag lives in the world, so there is no
/// durable write that could fail between the two.
fn use_from_container(
    world: &mut World,
    catalog: &Catalog,
    player: Handle,
    container: hendra_net::EntityId,
    slot: u8,
    aim: (f32, f32),
) -> Vec<hendra_content::Effect> {
    let nothing = Vec::new();
    let handle = Handle::from_entity_id(container);

    let (Some(standing), Some(target)) = (world.get(player), world.get(handle)) else {
        return nothing;
    };
    let (dx, dy) = (target.x - standing.x, target.y - standing.y);
    if dx * dx + dy * dy > USE_REACH * USE_REACH {
        return nothing;
    }
    if target.kind != hendra_sim::Kind::Container {
        return nothing;
    }

    // A bag that belongs to somebody belongs to them, the same rule taking from one goes by.
    if target.belongs_to.is_some_and(|owner| owner != player) {
        return nothing;
    }

    let Some(holding) = target.container.as_ref() else {
        return nothing;
    };

    // Only what is really in the world. A vault chest and a gift chest are both drawings of rows in
    // the database, and spending one here would leave the row behind to be drawn again.
    if holding.kind != hendra_sim::ContainerKind::Bag {
        return nothing;
    }

    let item = holding.item(slot as usize);
    if item.is_none() {
        return nothing;
    }

    let Some(desc) = catalog.object(item).and_then(|object| object.item.as_ref()) else {
        return nothing;
    };
    if !desc.consumable && desc.slot_type != 0 {
        return nothing;
    }

    if !world.may_use_item(player, catalog, item) {
        return nothing;
    }

    if desc.consumable {
        // Turned into its successor rather than removed outright, which is how an elixir with more
        // uses in it keeps them. `cInv[slot] = successor` sets the slot either way, and a successor
        // of nothing empties it (`Player.UseItem.cs:190-193`).
        let successor = catalog
            .successor_of(item)
            .and_then(|next| catalog.type_of_uuid(next))
            .unwrap_or(hendra_content::ObjectType::NONE);

        if let Some(entity) = world.get_mut(handle)
            && let Some(holding) = entity.container.as_mut()
        {
            holding.set(slot as usize, successor);
            if holding.occupied() == 0 {
                entity.dead = true;
            }
        }
    }

    world.use_item(player, catalog, item, aim)
}

/// Reads an item out of a container the player can reach, emptying it if it is a bag.
fn take_from_bag(
    world: &mut World,
    player: Handle,
    bag: hendra_net::EntityId,
    slot: u8,
) -> Option<Taken> {
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
    let kind = container.kind;

    // Nothing comes out of a vault chest this way. It is a drawing of what the account's rows say,
    // and handing over what the drawing shows would hand over an item whose row is still there --
    // a duplication, because the next visit draws the chest from those rows again. The vault has
    // slot numbers of its own and every real move goes through them.
    if kind == hendra_sim::ContainerKind::Vault {
        return None;
    }

    let item = container.item(slot as usize);
    if item.is_none() {
        return None;
    }

    // A bag gives the item up here; anything else is left as it stands, because what makes the
    // claim real is the row going, and that has not happened yet.
    if kind == hendra_sim::ContainerKind::Bag {
        container.set(slot as usize, hendra_content::ObjectType::NONE);

        // An emptied bag goes rather than sitting there inviting a second look. Only a bag: the
        // original's `Container.Tick` exempts the vault chest from expiring for the same reason a
        // chest that vanished when you emptied it would be wrong.
        if container.occupied() == 0 {
            entity.dead = true;
        }
    }

    Some(Taken {
        item: item.0,
        kind,
    })
}

/// The bag a dropped item lands in.
///
/// `soulBag` in `InvDropHandler.cs:31`, and the same object `InvSwapHandler.DropInSoulboundBag`
/// makes. Every drop uses it rather than the plain bag: the original picks between the two on
/// `item.Soulbound || Account.Admin || Account.Rank >= 0`, and a rank is never negative, so the
/// condition is always true and the plain-bag branch is unreachable.
const SOULBOUND_BAG: hendra_content::ObjectType = hendra_content::ObjectType(0x0503);

/// How large a dropped bag is drawn.
///
/// `SetDefaultSize(75)`, which is smaller than the eighty a loot bag gets — a dropped item reads as
/// something somebody put down rather than as something that fell.
const DROPPED_BAG_SIZE: u16 = 75;

/// Puts an item into a bag, creating one at the player's feet if none was named.
fn put_in_bag(
    world: &mut World,
    catalog: &Catalog,
    player: Handle,
    bag: Option<hendra_net::EntityId>,
    item: u16,
    soulbound: bool,
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

        // A soulbound item may only go into a bag this player solely owns. Into any other it is not
        // refused but diverted: `InvSwapHandler.cs:99-108` takes it out of the slot it came from and
        // `DropInSoulboundBag` puts it in a fresh bag of the player's own. Refusing instead would be
        // the safer-looking answer and the wrong one — the item would be back in the pack, and the
        // move the client already drew would have to be undone.
        let sole_owner = entity.belongs_to == Some(player);

        // And only into a loot bag. A vault or gift chest standing in a room is a drawing of what
        // the database holds, and an item put into the drawing is written nowhere: the next visit
        // redraws the chest from the rows and the item is gone. One aimed at a chest is diverted
        // into a bag of the player's own, the same way a soulbound one is.
        let into_a_bag = entity
            .container
            .as_ref()
            .is_some_and(|container| container.kind == hendra_sim::ContainerKind::Bag);

        if into_a_bag && (!soulbound || sole_owner) {
            let Some(container) = entity.container.as_mut() else {
                return false;
            };
            return container.insert(item, catalog).is_some();
        }
    }

    // Nothing named, or a soulbound item that has nowhere it is allowed to go: drop it where the
    // player stands.
    let player_handle = player;
    let Some(player) = world.get(player) else {
        return false;
    };
    let (x, y) = (player.x, player.y);

    // Eight slots like any other bag — `Container.BagSize` is a constant, so a bag made for one
    // item still has room for seven more and a player can pile a drop into it.
    let mut container =
        hendra_sim::Container::new(hendra_sim::ContainerKind::Bag, hendra_sim::world::BAG_SLOTS);
    if container.insert(item, catalog).is_none() {
        return false;
    }

    // Scattered half a tile on each axis, independently, so dropping several things in a row leaves
    // a pile that can be picked apart rather than one bag hiding the others.
    let (dx, dy) = (world.roll_offset(0.5), world.roll_offset(0.5));

    let mut dropped = hendra_sim::world::Entity::fixture(SOULBOUND_BAG, x + dx, y + dy);
    dropped.kind = hendra_sim::Kind::Container;
    dropped.container = Some(Box::new(container));
    dropped.expires_in_ms = Some(hendra_sim::world::BAG_LIFETIME_MS);
    dropped.size = DROPPED_BAG_SIZE;

    // Every dropped item goes into a bag only whoever dropped it can open. That is what makes
    // dropping one a way to move it rather than a way to give it away.
    dropped.belongs_to = Some(player_handle);

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
/// Who opened a dungeon, and what the door calls the place it leads to.
#[derive(Debug, Clone)]
pub struct Announced {
    /// The character that used the key.
    pub opener: String,

    /// The portal object's `<DungeonName>`, which is the name a player would recognise.
    pub dungeon: String,
}

/// The green a dungeon opening is announced in.
///
/// `ARGB(0xFF00FF00)`, written into the notification at `Player.UseItem.cs:617`.
const OPENED_TEXT_COLOUR: u32 = 0xFF00_FF00;

/// The red an administrator's spawn is announced in.
///
/// `ARGB(0xffff0000)` (`RankedCommands.cs:252`). Red rather than the green everything else uses,
/// because what it announces is something arriving that nobody chose to fight.
const SPAWNING_TEXT_COLOUR: u32 = 0xFFFF_0000;

/// Says that somebody opened a dungeon, in the two ways the original says it.
///
/// `AECreate` (`Player.UseItem.cs:613-624`) broadcasts a green notification anchored on the opener
/// reading "Opened by {name}", and then sends every player in the world one server line naming the
/// dungeon and the opener. Both are what tells a room that a key was used; without them a portal
/// appears out of nowhere and nobody knows whose it is.
fn announce_opened_dungeon(
    world: &mut World,
    players: &[Player],
    opener: Handle,
    announced: &Announced,
) {
    // To the whole world rather than to the twenty tiles around the opener: `AECreate` uses a bare
    // `Owner.BroadcastPacket` (`Player.UseItem.cs:616`), and the whole point of the line is to tell
    // the people who are not standing there that a portal has appeared.
    world.float_text_everywhere(
        opener,
        &format!("Opened by {}", announced.opener),
        OPENED_TEXT_COLOUR,
    );

    // A `LineBuilder` blob rather than a sentence: the client holds the wording per language and
    // this names the key and fills its two slots.
    let line = format!(
        "{{\"key\":\"{{server.dungeon_opened_by}}\",\"tokens\":{{\"dungeon\":\"{}\",\"name\":\"{}\"}}}}",
        announced.dungeon, announced.opener
    );

    let mut buffer = Vec::new();
    ServerMessage::Refused { message: &line }.encode(&mut Writer::new(&mut buffer));

    for player in players {
        let _ = player.sender.try_send(Delivery::Stream, &buffer);
    }
}

/// The two lines a room is told when a locked door is opened.
///
/// The same pair `AECreate` sends, worded differently: a green "Unlocked by X" over the player and
/// a `{server.dungeon_unlocked_by}` line to everybody in the world
/// (`Player.UseItem.cs:502-509`). Both use a bare `BroadcastPacket`, so both reach the whole world
/// rather than the tiles around the opener.
fn announce_unlocked_dungeon(
    world: &mut World,
    players: &[Player],
    opener: Handle,
    announced: &Announced,
) {
    world.float_text_everywhere(
        opener,
        &format!("Unlocked by {}", announced.opener),
        OPENED_TEXT_COLOUR,
    );

    let line = format!(
        "{{\"key\":\"{{server.dungeon_unlocked_by}}\",\"tokens\":{{\"dungeon\":\"{}\",\"name\":\"{}\"}}}}",
        announced.dungeon, announced.opener
    );

    let mut buffer = Vec::new();
    ServerMessage::Refused { message: &line }.encode(&mut Writer::new(&mut buffer));

    for player in players {
        let _ = player.sender.try_send(Delivery::Stream, &buffer);
    }
}

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

/// Where `/gland` puts somebody, from GLandCommand.cs.
///
/// A point on the realm's map, at the southern edge of the highlands where the god enemies walk.
/// The half is the original's: whole numbers name a corner and land you between two squares.
const GODLANDS_X: f32 = 1512.5;
const GODLANDS_Y: f32 = 1048.5;

/// Brings a body's health and magic up to what its stats now allow, and fills them.
///
/// Raising a stat is not enough on its own: the pools are held beside the stats rather than read
/// from them, so a ceiling that is never recomputed leaves a character with the magic of whatever
/// it was before and a spell it cannot afford.
fn refill(entity: &mut hendra_sim::Entity) {
    entity.reseat_maxima();
    entity.hp = entity.max_hp;
    entity.mp = entity.max_mp;
}

/// Tells everybody about every shot fired this tick.
///
/// A projectile is announced once and then stepped by both sides, rather than described in each
/// snapshot: a bullet is entirely predictable from where it started, so sending it every tick would
/// be paying per frame for something already known. This is what makes enemy fire visible — the
/// shot exists in the world and lands whether or not it was ever announced, so a world that skips
/// this kills players who saw nothing coming.
async fn announce_shots(world: &mut World, players: &mut [Player]) {
    let fired = world.take_fired();
    if fired.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for (handle, shot) in fired {
        buf.clear();
        ServerMessage::Shot {
            projectile: handle.to_entity_id(),
            owner: shot.owner.to_entity_id(),
            object_type: shot.container.0,
            x: shot.x,
            y: shot.y,
            angle: shot.angle,
            speed: shot.speed,
            lifetime_ms: shot.lifetime_ms,

            // What it will take off whatever it hits, before that body's defence. Whoever it hits
            // is the one client that is not told when it lands, so this is how the number over
            // their own head gets there: both of the original's shot packets carry it
            // (`outgoing/EnemyShoot.cs:30-39`, `outgoing/ServerPlayerShoot.cs:26-33`).
            damage: shot.damage.clamp(0, u16::MAX as i32) as u16,
        }
        .encode(&mut Writer::new(&mut buf));

        // Reliable: a shot nobody was told about is a bullet that appears to do damage from
        // nowhere. It is one message for the projectile's whole life, so the cost is small.
        //
        // The one it is not sent to is whoever fired it, and only when their own client already
        // drew it. `AllyShoot` goes to everybody but the shooter (`Player.UseItem.cs:1140`); a
        // nova's `ServerPlayerShoot` goes to the caster as well (`:1176-1180`), because it starts
        // at the cursor and no client could have predicted a bullet somewhere its player is not.
        for player in players.iter() {
            if shot.predicted_by_owner && player.handle == shot.owner {
                continue;
            }
            let _ = player.sender.try_send(Delivery::Stream, &buf);
        }
    }
}

/// Sends every hit that landed to whoever could see it land.
///
/// `BroadcastPacketNearby(pkt, entity, exclude, Low)` — everyone inside the sight radius of what was
/// hit, minus the one player the original leaves out. The snapshot already carries health, so this
/// is not how the number crosses; it is how a client learns that a particular hit landed for a
/// particular amount, and — through `kill` — how it tells a body that died from one that walked
/// out of sight. Without it a client can only diff snapshots, and a diff cannot tell those apart.
async fn announce_damage(world: &mut World, players: &mut [Player]) {
    let events = world.take_damage();
    if events.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for event in &events {
        buf.clear();
        ServerMessage::Damage {
            target: event.target.to_entity_id(),
            effects: event.effects.0,
            // Clamped where the original casts to `ushort`, which wraps. A hit past sixty-five
            // thousand only exists as a display, and showing the largest number the wire can hold
            // is closer to true than showing what it wrapped to.
            amount: event.amount.clamp(0, u16::MAX as i32) as u16,
            kill: event.kill,
            bullet: event.bullet,
            owner: event
                .owner
                .map(|owner| owner.to_entity_id())
                .unwrap_or(hendra_net::EntityId(0)),
        }
        .encode(&mut Writer::new(&mut buf));

        for player in players.iter() {
            if Some(player.handle) == event.except {
                continue;
            }

            let watching = world.get(player.handle).is_some_and(|viewer| {
                let (dx, dy) = (event.x - viewer.x, event.y - viewer.y);
                dx * dx + dy * dy <= SIGHT_RADIUS * SIGHT_RADIUS
            });

            // Reliable, like a shot: a kill that never arrives leaves a body standing on the
            // client's screen until it happens to notice the snapshot stopped mentioning it, which
            // is the very ambiguity this message exists to remove.
            if watching {
                let _ = player.sender.try_send(Delivery::Stream, &buf);
            }
        }
    }
}

/// Tells everybody where a body the world moved has ended up.
///
/// `TeleportPosition` sends a `Goto` to every player in the world and waits for each to say it
/// arrived (`Player.cs:700-704`). Sent to everybody rather than only to whoever moved, because a
/// teleport is the one movement no client can work out for itself: the mover's own client owns its
/// position and would otherwise walk on from where it believed it was, and every other client
/// glides bodies towards the position the snapshot gives them, which would draw a player sprinting
/// across a realm rather than vanishing from it.
///
/// Not restricted by sight. A player who teleports away from somebody is exactly the case where
/// where they went matters and where the destination is out of range of the person watching.
async fn announce_teleports(world: &mut World, players: &mut [Player]) {
    let events = world.take_teleports();
    if events.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for event in &events {
        buf.clear();
        ServerMessage::Goto {
            object_id: event.who.to_entity_id(),
            x: event.x,
            y: event.y,
        }
        .encode(&mut Writer::new(&mut buf));

        // Reliable. A dropped `Goto` is a player standing where the server says they are not, and
        // nothing later in the stream would correct it.
        for player in players.iter() {
            let _ = player.sender.try_send(Delivery::Stream, &buf);
        }
    }
}

/// Tells each player what their quest arrow has settled on.
///
/// To that player alone: two people standing together are pointed at different enemies, because the
/// score `FindQuest` weighs depends on each one's own level (`Player.Leveling.cs:186-204`). The
/// original sends it on a hundred-millisecond timer rather than immediately
/// (`Player.Leveling.cs:217-224`); here the arrow is chosen once every five hundred ticks anyway, so
/// the delay it was buying is already there.
///
/// Reliable. A dropped arrow leaves a player pointed at the enemy they killed a minute ago, and
/// nothing later in the stream would correct it — the next choice is not made until this one dies.
async fn announce_quests(world: &mut World, players: &mut [Player]) {
    let changes = world.take_quest_changes();
    if changes.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for (who, target) in &changes {
        let Some(player) = players.iter().find(|player| player.handle == *who) else {
            continue;
        };

        buf.clear();
        ServerMessage::QuestTarget {
            target: target.to_entity_id(),
        }
        .encode(&mut Writer::new(&mut buf));

        let _ = player.sender.try_send(Delivery::Stream, &buf);
    }
}

/// Moves a camera that has been put on a different body.
///
/// To the one player whose camera it is. `SetFocus` is sent to `player.Client` alone at all three of
/// its sites (`UnrankedCommands.cs:1437`, `RankedCommands.cs:2390`, `Player.cs:510`), because a
/// camera belongs to the person looking through it.
async fn announce_focus(world: &mut World, players: &mut [Player]) {
    let changes = world.take_focus_changes();
    if changes.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for (who, target) in &changes {
        let Some(player) = players.iter().find(|player| player.handle == *who) else {
            continue;
        };

        buf.clear();
        ServerMessage::SetFocus {
            target: target.to_entity_id(),
        }
        .encode(&mut Writer::new(&mut buf));

        // Reliable. A dropped focus leaves a paused player looking at their own motionless body
        // with no way to ask again.
        let _ = player.sender.try_send(Delivery::Stream, &buf);
    }
}

/// Sends everything the world asked to have drawn.
///
/// `ShowEffect` goes out by `BroadcastPacket` at most of its sites and `BroadcastPacketNearby` at
/// the rest, and the two effects this server sends are both of the first kind: the earthquake
/// shakes every camera in a closing realm (`World.cs:547-551`) and a vault holds one account alone
/// -- the original builds one `Vault` world per client and refuses anyone else
/// (`git show 94615c4:Server-Side/wServer/realm/worlds/logic/Vault.cs`, `:23-28`, `:47-50`) -- so
/// neither has anybody to leave out.
///
/// Unreliable, unlike a shot or a hit. An effect that never arrives costs a sparkle nobody knows
/// was owed; one that arrives late is drawn at a moment that has passed, which is worse.
async fn announce_effects(world: &mut World, players: &mut [Player]) {
    let effects = world.take_effects();
    if effects.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for effect in &effects {
        buf.clear();
        ServerMessage::ShowEffect {
            effect: effect.effect,
            target: effect
                .target
                .map(|handle| handle.to_entity_id())
                .unwrap_or(hendra_net::EntityId(0)),
            x1: effect.x1,
            y1: effect.y1,
            x2: effect.x2,
            y2: effect.y2,
            color: effect.color,
        }
        .encode(&mut Writer::new(&mut buf));

        for player in players.iter() {
            let _ = player.sender.try_send(Delivery::Datagram, &buf);
        }
    }
}

/// Sends everything the world wants said over a body.
///
/// Twenty tiles for almost all of them — `Player.Radius` (`Player.Update.cs:64`), which is what
/// both `BroadcastSync(pkt, p => this.DistSqr(p) < RadiusSqr)` and `BroadcastPacketNearby` measure.
/// The test here is the receiver's own distance to the body the text hangs over, which is what the
/// original's is: `this` at every one of those sites is the body the text is about.
///
/// The four that carry [`StatusTextEvent::everywhere`] skip the test, because the original's own
/// four skip it: a bare `BroadcastPacket` reaches the whole world.
///
/// Reliable, unlike a drawn effect. A dropped `+45` is the only sign a potion did anything, and
/// nothing later in the stream would say it again.
async fn announce_status_texts(world: &mut World, players: &mut [Player]) {
    let texts = world.take_status_texts();
    if texts.is_empty() || players.is_empty() {
        return;
    }
    let mut buf = Vec::new();
    for text in &texts {
        let Some((x, y)) = world.get(text.who).map(|entity| (entity.x, entity.y)) else {
            continue;
        };

        buf.clear();
        ServerMessage::StatusText {
            object_id: text.who.to_entity_id(),
            text: text.text.to_string(),
            color: text.color,
        }
        .encode(&mut Writer::new(&mut buf));

        for player in players.iter() {
            let heard = text.everywhere
                || world.get(player.handle).is_some_and(|viewer| {
                    let (dx, dy) = (x - viewer.x, y - viewer.y);
                    dx * dx + dy * dy <= SIGHT_RADIUS * SIGHT_RADIUS
                });
            if heard {
                let _ = player.sender.try_send(Delivery::Stream, &buf);
            }
        }
    }
}

/// Sends the ring of every blast that has gone off.
///
/// `BroadcastPacketNearby` (`Grenade.cs:85`), so whoever can see where it landed. Reliable, because
/// this is a warning: a ring that never arrives is damage out of nowhere, and one that arrives late
/// is a circle drawn around a player who has already been hit by it.
async fn announce_blasts(world: &mut World, players: &mut [Player]) {
    let blasts = world.take_blasts();
    if blasts.is_empty() || players.is_empty() {
        return;
    }

    let mut buf = Vec::new();
    for blast in &blasts {
        buf.clear();
        ServerMessage::Aoe {
            x: blast.x,
            y: blast.y,
            radius: blast.radius,
            damage: blast.damage,
            effect: blast.effect,
            duration: blast.duration,
            orig_type: blast.orig_type.0,
        }
        .encode(&mut Writer::new(&mut buf));

        for player in players.iter() {
            let close = world.get(player.handle).is_some_and(|viewer| {
                let (dx, dy) = (blast.x - viewer.x, blast.y - viewer.y);
                dx * dx + dy * dy <= SIGHT_RADIUS * SIGHT_RADIUS
            });
            if close {
                let _ = player.sender.try_send(Delivery::Stream, &buf);
            }
        }
    }
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
        // the client repeats -- unless it named one itself, which is how Oryx talks through a realm
        // he is not standing in.
        let from = announcement
            .speaker
            .as_deref()
            .map(str::to_owned)
            .or_else(|| {
                world.get(announcement.from).and_then(|entity| {
                    entity
                        .name
                        .as_deref()
                        .map(str::to_owned)
                        .or_else(|| catalog.object(entity.object_type).map(|d| d.id.clone()))
                })
            })
            .unwrap_or_else(|| world.name.to_string());

        let mut buffer = Vec::new();
        ServerMessage::Chat {
            speaker: hendra_net::EntityId(0),
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

    // Ground that has been repainted reaches only the players whose sight circle covers it. The
    // original re-sends a square from inside `SendUpdate`'s pass over that same circle
    // (`Player.Update.cs:133-151`), so a floor turning to lava on the far side of a dungeon paints
    // nothing on anybody's screen and nothing on their minimap. Anybody out of sight of it has
    // already forgotten the square in `take_ground_changes`, and is told when they walk back.
    //
    // Queued rather than sent outright, because a change dropped by a full connection would never
    // be offered again -- the square is already marked seen for everyone it was sent to.
    if !ground.is_empty() {
        for player in players.iter_mut() {
            let Some(circle) = world.sight_circle(player.handle) else {
                continue;
            };
            let mine: Vec<(u16, u16, u16)> = ground
                .iter()
                .copied()
                .filter(|(x, y, _)| circle.contains(*x as u32, *y as u32))
                .collect();
            if mine.is_empty() {
                continue;
            }

            // A connection already this far behind is not given more to hold. The squares are
            // forgotten instead, so the next look sends them afresh.
            if player.pending_map.len() >= MOST_PENDING_MAP {
                let squares: Vec<(u32, u32)> = mine
                    .iter()
                    .map(|(x, y, _)| (*x as u32, *y as u32))
                    .collect();
                world.forget_uncovered(player.handle, &squares);
                continue;
            }

            let mut buffer = Vec::new();
            ServerMessage::Ground { changes: mine }.encode(&mut Writer::new(&mut buffer));
            queue_map(player, buffer);
        }
    }

    // Scenery that has appeared goes out by row, in the same shape [`uncover`] sends the scenery a
    // player walks into sight of, so a client has one way of hearing about scenery rather than two.
    // Gated on the same circle and for the same reason: `GetNewStatics` only offers a client the
    // scenery standing on a square the circle covers (`Player.Update.cs:274-288`).
    if !scenery.is_empty() {
        for player in players.iter_mut() {
            let Some(circle) = world.sight_circle(player.handle) else {
                continue;
            };

            let mut rows: std::collections::BTreeMap<u16, Vec<(u16, u16, u16)>> =
                std::collections::BTreeMap::new();
            for (x, y, object, size) in scenery.iter().copied() {
                if !circle.contains(x as u32, y as u32) {
                    continue;
                }
                rows.entry(y).or_default().push((x, object, size));
            }

            // As with the ground: a connection at the cap is handed the squares back rather than
            // given a message that would be queued forever.
            if player.pending_map.len() >= MOST_PENDING_MAP {
                let squares: Vec<(u32, u32)> = rows
                    .iter()
                    .flat_map(|(y, objects)| {
                        objects.iter().map(move |(x, _, _)| (*x as u32, *y as u32))
                    })
                    .collect();
                world.forget_uncovered(player.handle, &squares);
                continue;
            }

            for (y, objects) in rows {
                for piece in objects.chunks(hendra_net::MAX_SCENERY) {
                    let mut buffer = Vec::new();
                    ServerMessage::Scenery {
                        y,
                        objects: piece.to_vec(),
                    }
                    .encode(&mut Writer::new(&mut buffer));

                    queue_map(player, buffer);
                }
            }
        }
    }
}

/// How many map messages one player may have waiting before the rest are given up on.
///
/// A sight circle is at most forty-one strips, so this is a hundred circles' worth: a client that
/// has fallen this far behind on the one thing it cannot play without is not draining its
/// connection at all, and QUIC will time it out shortly.
///
/// Reaching it is reported because the squares behind it are lost outright. The bit that says a
/// square was uncovered is set as the circle is walked, before the message it produced is queued,
/// and nothing sets it back -- so a message dropped here is a square that stays grey until the
/// player changes worlds. Walking away and back does not bring it back.
const MOST_PENDING_MAP: usize = 4_096;

/// Sends each player the ground and scenery they have just walked into sight of.
///
/// This is `SendUpdate`'s tile pass (`Player.Update.cs:133-151`) and its statics pass
/// (`Player.Update.cs:274-288`): the map reaches a client a sight circle at a time, as the player
/// uncovers it, and never twice. A realm is four million squares and a quarter of a million trees,
/// which is minutes of wire and more messages than a client will take in one breath; a circle is
/// thirteen hundred squares and arrives in the tick that reveals it.
///
/// A teleport is covered by the same rule rather than by a special case, because the circle is
/// taken from where the player is: landing a thousand squares away reveals a whole fresh circle and
/// sends it on that tick, which is what stops a jump arriving somewhere the client cannot draw.
async fn uncover(world: &mut World, catalog: &Catalog, players: &mut [Player]) {
    let revealed = world.take_revealed();

    if !revealed.is_empty() {
        let mut squares: std::collections::HashMap<Handle, Vec<(u32, u32)>> =
            std::collections::HashMap::new();
        for (who, x, y) in revealed {
            squares.entry(who).or_default().push((x, y));
        }

        for player in players.iter_mut() {
            let Some(mut uncovered) = squares.remove(&player.handle) else {
                continue;
            };

            // By row, then along it, so neighbouring squares fall next to each other and a strip
            // of ground costs one message and one run rather than one message per square.
            uncovered.sort_unstable_by_key(|(x, y)| (*y, *x));
            uncovered.dedup();

            let mut at = 0usize;
            while at < uncovered.len() {
                // A connection this far behind is given the rest of its circle back rather than
                // having it queued and dropped: the squares are forgotten, so the next look
                // uncovers them again. Without this the bit saying a square was uncovered outlives
                // the message that was supposed to carry it, and the square stays grey for the rest
                // of the world.
                if player.pending_map.len() >= MOST_PENDING_MAP {
                    tracing::warn!(
                        player = %player.name,
                        squares = uncovered.len() - at,
                        "a connection is too far behind to be told the map; holding the rest back"
                    );
                    world.forget_uncovered(player.handle, &uncovered[at..]);
                    break;
                }

                let y = uncovered[at].1;
                let ends = at + uncovered[at..].partition_point(|(_, other)| *other == y);
                let row = &uncovered[at..ends];
                at = ends;

                let mut scenery: Vec<(u16, u16, u16)> = Vec::new();
                let mut from = 0usize;

                while from < row.len() {
                    // As far along the row as the squares keep being neighbours. A circle uncovers
                    // one unbroken strip per row on arrival and a couple of squares per row as the
                    // player walks, so this is nearly always the whole of what is left.
                    let mut to = from + 1;
                    while to < row.len() && row[to].0 == row[to - 1].0 + 1 {
                        to += 1;
                    }

                    let from_x = row[from].0;
                    let width = row[to - 1].0 - from_x + 1;

                    let mut buffer = Vec::new();
                    ServerMessage::Terrain {
                        x: from_x as u16,
                        y: y as u16,
                        runs: world.terrain().row_runs(y, from_x, width),
                    }
                    .encode(&mut Writer::new(&mut buffer));
                    queue_map(player, buffer);

                    // Scenery standing on the same strip. The world holds none of it as entities,
                    // so the map is the only place it can be read from.
                    let map = world.terrain().map();
                    for x in from_x..from_x + width {
                        let Some(square) = map.at(x, y) else {
                            continue;
                        };
                        if !World::is_scenery(catalog, square) {
                            continue;
                        }
                        scenery.push((
                            x as u16,
                            square.object.0,
                            square.size().unwrap_or(0).clamp(0, u16::MAX as i32) as u16,
                        ));
                    }

                    from = to;
                }

                for piece in scenery.chunks(hendra_net::MAX_SCENERY) {
                    let mut buffer = Vec::new();
                    ServerMessage::Scenery {
                        y: y as u16,
                        objects: piece.to_vec(),
                    }
                    .encode(&mut Writer::new(&mut buffer));
                    queue_map(player, buffer);
                }
            }
        }
    }

    // Whatever the wire has room for, oldest first. Never awaits: the map is worth waiting a tick
    // for and is not worth holding up everyone else's tick for.
    for player in players.iter_mut() {
        while let Some(next) = player.pending_map.front() {
            match player.sender.try_send(Delivery::Stream, next) {
                Ok(()) => {
                    player.pending_map.pop_front();
                }
                Err(hendra_transport::TransportError::Backlogged) => break,
                Err(_) => {
                    player.pending_map.clear();
                    break;
                }
            }
        }
    }
}

/// Holds one map message for a player until the wire has room for it.
///
/// Unconditional, because the only caller that can produce an unbounded number of these checks the
/// depth a row at a time and hands the squares it did not queue back to the world instead. A queue
/// that refused a message here would lose the squares in it.
fn queue_map(player: &mut Player, buffer: Vec<u8>) {
    player.pending_map.push_back(buffer);
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

/// What a player's movement allowance becomes after a gap with no claim in it.
///
/// Adding the gap rather than replacing the allowance with it is what makes a burst of claims
/// released together after a stall cost the same as the same movement spread out, and capping it
/// is what stops standing still buying a jump.
fn topped_up(held: u32, since: Duration) -> u32 {
    held.saturating_add(since.as_millis().min(u32::MAX as u128) as u32)
        .min(MOST_ALLOWANCE_MS)
}

/// How long the world should believe elapsed for a movement claim it cannot time itself.
///
/// Deliberately not the client's own figure. A client that reports a large interval would be
/// granting itself a proportionally larger step, so the client's clock is diagnostic only. What an
/// ordinary claim is actually measured against is the time the server itself watched pass, which is
/// what the caller keeps in `Player.allowance_ms`; this is the fallback for a claim arriving from
/// somebody the world has no record of.
fn tick_ms(_client_time_ms: u32) -> u32 {
    1000 / TICKS_PER_SECOND
}

/// Somewhere walkable to put an arriving player.
///
/// `Player.Init` (`realm/entities/player/Player.cs:519-528`) takes every square the map marks
/// `Spawn` and picks one of them at random, so a nexus that marks sixty squares scatters its crowd
/// across all sixty instead of stacking it on one tile. A map that marks none puts everybody on the
/// origin there; here it walks outward from the middle instead, because dropping a player into a
/// wall is worse than putting them somewhere arbitrary.
///
/// The castle is the one world that narrows the choice. `Castle.GetSpawnPoints`
/// (`realm/worlds/logic/Castle.cs:24-43`) returns only the first marked square while fewer than
/// twenty players are arriving, and the count it reads is always zero: `DynamicWorld.TryGetWorld`
/// constructs with two arguments (`realm/worlds/DynamicWorld.cs:37`), which can only reach the
/// two-argument constructor, and that one sets it to zero (`Castle.cs:18-22`). So everyone arriving
/// from a closed realm lands on the same square, which is what puts the raid together.
fn spawn_point(world: &mut World) -> (f32, f32) {
    let mut places: Vec<(u32, u32)> = world
        .terrain()
        .map()
        .regions()
        .filter(|(_, _, region)| *region == hendra_content::Region::Spawn)
        .map(|(x, y, _)| (x, y))
        .collect();

    if world.name == CASTLE {
        places.truncate(1);
    }

    if !places.is_empty() {
        let pick = ((world.roll() * places.len() as f32) as usize).min(places.len() - 1);
        let (x, y) = places[pick];
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

    /// Which room this is, as against what it is called.
    ///
    /// Two parties in two Undead Lairs are in two worlds with one name, so the name cannot tell
    /// them apart and anything counting or comparing rooms has to use this. It is the registry's
    /// instance key, and defaults to the name for a world that has only ever had one.
    pub key: Arc<str>,

    /// The number `/daccept` names this world by.
    ///
    /// `World.Id`, handed out by `RealmManager.AddWorld` as `Interlocked.Increment(ref
    /// _nextWorldId)` (`RealmManager.cs:346`). It exists because an invitation has to travel to
    /// somebody in another world and name the room they are being asked into, and a name would name
    /// every copy of that dungeon at once.
    pub id: i32,

    /// What is playing here.
    ///
    /// Shared rather than copied, because it is the one piece of a world's identity that changes
    /// while people are standing in it: `/music` and the `ChangeMusic` behaviours write
    /// `World.Music` and then tell everyone (`RankedCommands.cs:1800-1815`,
    /// `logic/behaviors/ChangeMusic.cs:29-48`), and somebody arriving after that must be told the
    /// new track rather than the one the definition named.
    pub music: Arc<std::sync::RwLock<String>>,

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

/// How long a world is safe from being cleared away, however empty it is.
///
/// A minute from when it was built, as `World.Tick` (`World.cs:614`) measures it: `_elapsedTime >
/// 60000 && Players.Count <= 0`. The minute is not a grace period for an emptied world but for a
/// new one -- a dungeon built the moment somebody stepped into its portal has to survive until they
/// arrive. Past that, a world goes on the tick its last player leaves.
pub const WORLD_GRACE: Duration = Duration::from_secs(60);

pub fn spawn(mut world: World, catalog: Arc<Catalog>, loadout: Loadout) -> WorldHandle {
    world.set_bag_types(loadout.bag_types.clone());

    let name: Arc<str> = Arc::from(world.name.as_str());
    let music = Arc::new(std::sync::RwLock::new(world.music.clone()));
    let (inbox, receiver) = mpsc::channel(1024);

    tokio::spawn(run(world, catalog, loadout, receiver));

    WorldHandle {
        key: Arc::clone(&name),
        name,
        id: NEXT_WORLD_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        music,
        inbox,
    }
}

/// The next number a world will be known by.
///
/// `RealmManager._nextWorldId` (`RealmManager.cs:72`), incremented for every world the manager
/// takes (`:346`). Starts at one so that nothing carries the zero a missing world would read as.
static NEXT_WORLD_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::map::{Composition, Map};
    use hendra_content::{Region, TileType};
    use hendra_sim::world::Entity;

    #[test]
    fn a_clock_keeping_server_time_is_believed_and_one_running_fast_is_not() {
        // What makes it safe to enforce a rate of fire in time the client reports. The first
        // twenty shots are taken on trust, as `TimeCop.TimeDiff` takes them
        // (`Player.AntiCheat.cs:70-76`); after that the two clocks are compared over the window.
        let mut honest = TimeCop::new();
        for shot in 0..TIME_COP_WINDOW - 1 {
            let at = shot as i32 * 422;
            assert_eq!(
                honest.push(at as u32, at),
                1.0,
                "an unfilled window is answered with agreement"
            );
        }

        for shot in TIME_COP_WINDOW - 1..60 {
            let at = shot as i32 * 422;
            let running = honest.push(at as u32, at);
            assert!(
                (running - 1.0).abs() < 1e-6,
                "two clocks in step should read as one, not {running}"
            );
        }

        // A two-second stall in delivery, which the original counts against the player: the server
        // saw two seconds more pass than the client did, so the client reads as slow and its shots
        // are thrown away — for exactly as long as the stall is still inside the window, and no
        // longer. Kept because the original keeps it, and it is why a bad connection there costs
        // several seconds of shooting rather than one.
        let mut stalled = TimeCop::new();
        let mut server = 0;
        for shot in 0..60i32 {
            server += if shot == 30 { 2422 } else { 422 };
            let running = stalled.push((shot * 422) as u32, server);

            if shot < 30 || shot > 49 {
                assert!(
                    (running - 1.0).abs() < 1e-6,
                    "shot {shot} is outside the stall and should read as agreement, not {running}"
                );
            } else {
                assert!(
                    running < SLOWEST_CLOCK,
                    "shot {shot} is inside the stall and should read as slow, not {running}"
                );
            }
        }

        // A client claiming its own time is passing three times as fast, which is how a rate of
        // fire enforced in client time would otherwise be broken.
        let mut cheating = TimeCop::new();
        let mut running = 1.0;
        for shot in 0..60 {
            running = cheating.push((shot * 1266) as u32, shot as i32 * 422);
        }
        assert!(
            running > FASTEST_CLOCK,
            "a clock running triple should be caught, not read as {running}"
        );
    }

    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"/>
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
        <Object type="0x0500" id="Loot Bag"><Class>Container</Class></Object>
        <Object type="0x0503" id="Soulbound Loot Bag"><Class>Container</Class></Object>
        <Object type="0x900" id="Bolt"><Class>Projectile</Class></Object>
        <Object type="0x901" id="Practice Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>10</MinDamage><MaxDamage>10</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x902" id="Fine Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>90</MinDamage><MaxDamage>90</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x904" id="Blade">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType>
        </Object>
        <Object type="0x905" id="Heirloom">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType><Soulbound/>
        </Object>
        <Object type="0x908" id="Health Potion">
          <Class>Equipment</Class><Item/><SlotType>4</SlotType><Consumable/>
          <Activate amount="100">Heal</Activate>
        </Object>
        <Object type="0x90a" id="Spell of Mending">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <Activate amount="60">Heal</Activate>
        </Object>
        <Object type="0x909" id="Elixir of Health">
          <Class>Equipment</Class><Item/><SlotType>4</SlotType><Consumable/>
          <SuccessorId>Health Potion</SuccessorId>
          <Activate amount="50">Heal</Activate>
        </Object>
        <Object type="0x906" id="Warded Brute">
          <Class>Character</Class><Enemy/><StasisImmune/>
          <MaxHitPoints>500</MaxHitPoints><Defense>0</Defense>
        </Object>
        <Object type="0x907" id="Plain Brute">
          <Class>Character</Class><Enemy/>
          <MaxHitPoints>500</MaxHitPoints><Defense>0</Defense>
        </Object>
        <Object type="0x0712" id="Nexus Portal">
          <Class>Portal</Class><Size>80</Size>
        </Object>
        <Object type="0x90b" id="Brute Spawner">
          <Class>Character</Class>
          <MaxHitPoints>500</MaxHitPoints>
        </Object>
        <Object type="0x90c" id="Wine Barrel">
          <Class>GameObject</Class><Enemy/><Static/>
          <MaxHitPoints>100</MaxHitPoints>
        </Object>
      </Objects>"#;

    const BLADE: hendra_content::ObjectType = hendra_content::ObjectType(0x904);
    const HEIRLOOM: hendra_content::ObjectType = hendra_content::ObjectType(0x905);

    fn field() -> (Catalog, World) {
        let catalog = Catalog::load_str(&[FIXTURE]).0;
        let squares = (0..16 * 16).map(|_| Composition {
            tile: TileType(0x10),
            object: hendra_content::ObjectType::NONE,
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        });
        let map = Map::from_squares(16, 16, squares).unwrap();
        let world = World::new("Field", hendra_sim::Terrain::build(map, &catalog), &catalog);
        (catalog, world)
    }

    /// A nexus, for these purposes: a map with a run of squares marked as realm-portal pads.
    fn nexus(pads: usize) -> (Catalog, World) {
        let catalog = Catalog::load_str(&[FIXTURE]).0;
        let squares = (0..16 * 16).map(|index| Composition {
            tile: TileType(0x10),
            object: hendra_content::ObjectType::NONE,
            region: if index < pads {
                Region::RealmPortals
            } else {
                Region::None
            },
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        });
        let map = Map::from_squares(16, 16, squares).unwrap();
        let world = World::new(
            crate::commands::NEXUS,
            hendra_sim::Terrain::build(map, &catalog),
            &catalog,
        );
        (catalog, world)
    }

    /// What a nexus is being asked to show, for a world with this many people in it.
    fn realm_sign(players: usize) -> Vec<PortalSign> {
        vec![PortalSign {
            world: "Realm".to_string(),
            portal: hendra_content::ObjectType(0x0712),
            players,
        }]
    }

    /// The one portal standing, with the square it is standing on.
    fn one_portal(world: &World) -> (Handle, f32, f32, String) {
        let standing: Vec<(Handle, f32, f32, String)> = world
            .iter()
            .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Portal)
            .map(|(handle, entity)| {
                (
                    handle,
                    entity.x,
                    entity.y,
                    entity.name.as_deref().unwrap_or_default().to_string(),
                )
            })
            .collect();
        assert_eq!(standing.len(), 1, "exactly one portal should be standing");
        standing.into_iter().next().unwrap()
    }

    /// `PortalMonitor.Tick` (`PortalMonitor.cs:226-246`) rewrites the count in the portal's name
    /// and assigns it back, and does nothing else. The portal it renamed is the same entity on the
    /// same square, so a client keeps drawing the door it was walking towards. Standing a new one
    /// each refresh changes its id and its square several times a minute.
    #[test]
    fn a_refreshed_portal_is_renamed_where_it_stands() {
        let (catalog, mut world) = nexus(8);
        let loadout = bare_loadout();

        show_portals(&mut world, &catalog, &loadout, &realm_sign(0));
        let (handle, x, y, name) = one_portal(&world);
        assert_eq!(name, "Realm (0)");

        show_portals(&mut world, &catalog, &loadout, &realm_sign(7));
        let (again, ax, ay, renamed) = one_portal(&world);

        assert_eq!(handle, again, "the portal kept its entity, and so its id");
        assert_eq!((x, y), (ax, ay), "and stayed on the square it was stood on");
        assert_eq!(renamed, "Realm (7)");
    }

    /// `AddPortal` calls `SetDefaultSize(80)` on the portal it builds (`PortalMonitor.cs:79`).
    #[test]
    fn a_nexus_portal_is_drawn_at_four_fifths() {
        let (catalog, mut world) = nexus(8);
        show_portals(&mut world, &catalog, &bare_loadout(), &realm_sign(0));

        let (handle, ..) = one_portal(&world);
        assert_eq!(world.get(handle).unwrap().size, 80);
    }

    /// `OnWorldRemoved` calls `Monitor.RemovePortal(world.Id)` (`RealmManager.cs:383`): a portal
    /// whose world has gone is a door into nothing.
    #[test]
    fn a_portal_whose_world_has_gone_is_taken_down() {
        let (catalog, mut world) = nexus(8);
        let loadout = bare_loadout();

        show_portals(&mut world, &catalog, &loadout, &realm_sign(3));
        show_portals(&mut world, &catalog, &loadout, &[]);

        assert!(
            !world
                .iter()
                .any(|(_, entity)| entity.kind == hendra_sim::Kind::Portal),
            "nothing should be left standing"
        );
    }

    /// `GetRandPosition` (`PortalMonitor.cs:44-48`) draws again while a portal is already on the
    /// square it drew, so two worlds never share a pad.
    #[test]
    fn two_portals_never_share_a_pad() {
        let (catalog, mut world) = nexus(4);
        let mut signs = realm_sign(1);
        signs.push(PortalSign {
            world: "Realm2".to_string(),
            portal: hendra_content::ObjectType(0x0712),
            players: 2,
        });

        show_portals(&mut world, &catalog, &bare_loadout(), &signs);

        let places: Vec<(f32, f32)> = world
            .iter()
            .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Portal)
            .map(|(_, entity)| (entity.x, entity.y))
            .collect();
        assert_eq!(places.len(), 2);
        assert_ne!(places[0], places[1]);
    }

    /// The counterpart of the regex `PortalMonitor.Tick` replaces: everything before the trailing
    /// count is the world, and a name that has no count is the world entire.
    #[test]
    fn a_portals_label_names_the_world_it_leads_to() {
        assert_eq!(labelled_world("Realm (0)"), "Realm");
        assert_eq!(labelled_world("Cloth Bazaar (12)"), "Cloth Bazaar");
        assert_eq!(labelled_world("Realm"), "Realm");
        assert_eq!(labelled_world("Realm (many)"), "Realm (many)");
        assert_eq!(labelled_world("Realm ()"), "Realm ()");
    }

    /// Word for word from `AddPortal` (`PortalMonitor.cs:89-91`), whose two substitutions turn on
    /// where the hearer is standing rather than on where the portal is.
    #[test]
    fn a_portal_opening_is_told_differently_to_each_world() {
        assert_eq!(
            portal_opened_line(crate::commands::NEXUS, "Realm"),
            "A portal to Realm has opened up."
        );
        assert_eq!(
            portal_opened_line("Realm", "Realm"),
            "A portal to this land has opened up in Nexus."
        );
        assert_eq!(
            portal_opened_line("UndeadLair", "Realm"),
            "A portal to Realm has opened up in Nexus."
        );
    }

    #[test]
    fn walking_is_paid_for_by_the_clock_rather_than_by_the_message() {
        // A client that reports its position ten times as often must not be allowed to walk ten
        // times as far, which is what a per-message allowance grants. The gap since the last claim
        // is what is handed out, so ten claims inside one frame share one frame's worth.
        let none = Duration::from_millis(0);
        let frame = Duration::from_millis(16);

        let mut held = 0;
        for _ in 0..10 {
            held = topped_up(held, none);
        }
        assert_eq!(held, 0, "ten claims in no time at all bought nothing");

        held = topped_up(held, frame);
        assert_eq!(held, 16, "and a frame's gap buys a frame's walking");

        // What is not spent is kept, so claims held up by the network and released together are
        // paid for out of the stall that held them: five frames' worth arrives as five frames.
        let mut held = 0;
        held = topped_up(held, Duration::from_millis(80));
        assert_eq!(held, 80);
        for _ in 0..4 {
            held = topped_up(held, none);
        }
        assert_eq!(held, 80, "the backlog still has the whole stall to spend");

        // Standing still saves up a second and no more.
        assert_eq!(
            topped_up(0, Duration::from_secs(60)),
            MOST_ALLOWANCE_MS,
            "a minute of standing still is not a jump across the map"
        );
    }

    /// A loadout carrying nothing, for the world tools that take one and never read it.
    fn bare_loadout() -> Loadout {
        Loadout {
            bag_types: Vec::new(),
            spawnable: Vec::new(),
            is_realm: false,
            maps: std::path::PathBuf::new(),
            persistent: false,
            started: Instant::now(),
            blueprint: None,
            announcements: tokio::sync::broadcast::channel(1).0,
            portals_opened: tokio::sync::broadcast::channel(1).0,
            avatar: hendra_content::ObjectType(0x600),
            weapon: None,
        }
    }

    /// The one entity of a type, for the spawn tools that put down exactly one.
    fn only(world: &World, kind: u16) -> &Entity {
        world
            .iter()
            .find(|(_, entity)| entity.object_type == hendra_content::ObjectType(kind))
            .map(|(_, entity)| entity)
            .expect("the command should have put one down")
    }

    #[test]
    fn a_spawned_enemy_is_still_invisible_a_tick_later() {
        // `/spawn` applies Invisible with `DurationMS = -1` to the enemies it puts down
        // (`RankedCommands.cs:578-582`), and `/lootspawn` applies nothing (`:286-320`). The bit has
        // to be written with a timer behind it, because ageing the effects rebuilds each entity's
        // condition set from the effects still held: the Warded Brute is born carrying an immunity,
        // which is by itself enough to make that rebuild run over it and sweep away a bare bit.
        let (catalog, mut world) = field();
        let hero = standing(&mut world, 8.0, 8.0);
        let loadout = bare_loadout();

        for (name, summoned) in [("Warded Brute", true), ("Plain Brute", false)] {
            wield_in_world(
                &mut world,
                &catalog,
                &loadout,
                hero,
                Wielding::Spawn {
                    name: name.to_string(),
                    count: 1,
                    summoned,
                },
            );
        }

        world.advance(&catalog, 50);

        let summoned = only(&world, 0x906);
        assert!(summoned.spawned, "`/spawn` marks what it puts down");
        assert!(
            summoned
                .conditions
                .contains(hendra_content::ConditionEffect::Invisible),
            "the Invisible was swept away by the first tick that aged effects"
        );

        let asked_for = only(&world, 0x907);
        assert!(!asked_for.spawned, "`/lootspawn` marks nothing");
        assert!(
            !asked_for
                .conditions
                .contains(hendra_content::ConditionEffect::Invisible),
            "`/lootspawn` hides nothing"
        );
    }

    /// Puts down whatever a descriptor's class makes of it, as the world itself would.
    fn placed(world: &mut World, catalog: &Catalog, kind: u16, x: f32, y: f32) -> Handle {
        let object = hendra_content::ObjectType(kind);
        let desc = catalog.object(object).expect("in the fixture");
        let mut entity = Entity::fixture(object, x, y);
        entity.kind = hendra_sim::Kind::of_class(desc.class.as_str());
        entity.max_hp = desc.max_hp;
        entity.hp = desc.max_hp;
        world.spawn(entity).expect("room for it")
    }

    #[test]
    fn kill_all_reaches_only_what_the_descriptor_calls_an_enemy() {
        // `KillAllCommand` walks `Owner.Enemies.Values` (`RankedCommands.cs:916`), which
        // `World.EnterWorld` fills with `Enemy` instances alone (`World.cs:338-345`), and then asks
        // the descriptor's own `<Enemy/>` flag on top. The two are different questions: 162 shipped
        // `Class=Character` objects carry no flag — the Shatters portal spawner, the fountain
        // placers, every encounter manager — and an administrator clearing a room should not delete
        // the machinery running it. Breakable scenery is not reached either: a wine barrel is a
        // `StaticObject` and is not in `Enemies` at all.
        let (catalog, mut world) = field();
        let hero = standing(&mut world, 8.0, 8.0);

        let brute = placed(&mut world, &catalog, 0x907, 6.0, 6.0);
        let spawner = placed(&mut world, &catalog, 0x90b, 7.0, 6.0);
        let barrel = placed(&mut world, &catalog, 0x90c, 9.0, 6.0);

        let said = wield_in_world(
            &mut world,
            &catalog,
            &bare_loadout(),
            hero,
            Wielding::KillAll {
                name: String::new(),
            },
        );

        assert_eq!(said, "1 enemy killed!");
        assert!(world.get(brute).unwrap().dead, "a flagged enemy dies");
        assert!(
            !world.get(spawner).unwrap().dead,
            "a Character with no <Enemy/> is machinery and is spared"
        );
        assert!(
            !world.get(barrel).unwrap().dead,
            "and a StaticObject is not in Enemies at all"
        );
    }

    #[test]
    fn warg_finds_breakable_scenery_as_readily_as_a_monster() {
        // `WargCommand` asks `GetNearestEntityByName` (`RankedCommands.cs:2363`), which walks
        // `EnemiesCollision` testing only "has a descriptor" and "the name contains this"
        // (`Utils.cs:245-257`) — no `is Enemy`, no flag. `World.EnterWorld` puts every
        // `StaticObject` that is not a decoy into that same map (`World.cs:352-361`), so scenery is
        // exactly as findable as a monster.
        let (catalog, mut world) = field();
        let hero = standing(&mut world, 8.0, 8.0);
        placed(&mut world, &catalog, 0x90c, 9.0, 6.0);

        let found = wield_in_world(
            &mut world,
            &catalog,
            &bare_loadout(),
            hero,
            Wielding::Warg {
                name: "Wine Barrel".to_string(),
            },
        );
        assert_eq!(found, "Only one person can control a mob at a time.");

        let missing = wield_in_world(
            &mut world,
            &catalog,
            &bare_loadout(),
            hero,
            Wielding::Warg {
                name: "Ent Ancient".to_string(),
            },
        );
        assert_eq!(missing, "Mob not found.");
    }

    fn standing(world: &mut World, x: f32, y: f32) -> Handle {
        world
            .spawn(Entity::player(hendra_content::ObjectType(0x600), x, y, 100))
            .expect("room for a player")
    }

    /// The vault chest throws an arc down onto itself, and the arc is aimed.
    ///
    /// This project's own effect, not the original's -- see [`CRACKLE_EVERY_MS`]. The bolt runs
    /// from a point three and a half tiles above the chest down onto it, wandering up to 1.6 tiles
    /// to either side so no two strikes land in the same place, in a warm pale yellow. Every one of
    /// those numbers is visible: the height is how far the bolt reaches, `Pos2.X` is how thick it is
    /// drawn (`Structures.cs:145`), and a colour of nothing is a bolt drawn in black.
    #[test]
    fn the_vault_chest_arcs_onto_itself() {
        let (catalog, mut world) = field();

        let mut chest = Entity::fixture(VAULT_ACCESS, 8.5, 8.5);
        chest.size = VAULT_ACCESS_SIZE;
        let handle = world.spawn(chest).expect("room for the chest");

        // Twice, because the drift is drawn fresh each time and a fixed offset would pass a test
        // that only ever looked at one strike.
        crackle(&mut world, &catalog);
        crackle(&mut world, &catalog);

        let thrown = world.take_effects();
        assert_eq!(thrown.len(), 2, "one arc per crackle, and only for the chest");

        for effect in &thrown {
            assert_eq!(effect.effect, hendra_net::message::effect::LIGHTNING);
            assert_eq!(
                effect.target,
                Some(handle),
                "the bolt hangs off the chest, which is what anchors the end that lingers"
            );
            assert_eq!(effect.y1, 8.5 - CRACKLE_HEIGHT, "it comes down from overhead");
            assert!(
                (effect.x1 - 8.5).abs() <= CRACKLE_DRIFT,
                "the strike wanders, but only as far as the original lets it: {}",
                effect.x1
            );
            assert_eq!(effect.x2, CRACKLE_PARTICLE_SIZE, "`Pos2.X` is how thick it is");
            assert_eq!(effect.color, CRACKLE_COLOUR);
        }

        assert!(
            world.take_effects().is_empty(),
            "an effect is sent once, not on every tick after it"
        );
    }

    /// Nothing arcs in a world with no chest in it.
    #[test]
    fn a_world_without_a_vault_chest_throws_nothing() {
        let (catalog, mut world) = field();
        standing(&mut world, 8.0, 8.0);

        crackle(&mut world, &catalog);

        assert!(world.take_effects().is_empty());
    }

    /// A bag on the ground, owned by somebody or by nobody.
    fn bag_at(world: &mut World, x: f32, y: f32, owner: Option<Handle>) -> Handle {
        let mut bag = Entity::fixture(hendra_content::ObjectType(0x0500), x, y);
        bag.kind = hendra_sim::Kind::Container;
        bag.container = Some(Box::new(hendra_sim::Container::new(
            hendra_sim::ContainerKind::Bag,
            hendra_sim::world::BAG_SLOTS,
        )));
        bag.belongs_to = owner;
        world.spawn(bag).expect("room for a bag")
    }

    fn bags(world: &World) -> Vec<(hendra_content::ObjectType, Option<Handle>, u16, usize)> {
        world
            .iter()
            .filter(|(_, entity)| entity.kind == hendra_sim::Kind::Container)
            .map(|(_, entity)| {
                (
                    entity.object_type,
                    entity.belongs_to,
                    entity.size,
                    entity
                        .container
                        .as_ref()
                        .map(|held| held.occupied())
                        .unwrap_or(0),
                )
            })
            .collect()
    }

    #[test]
    fn a_dropped_item_lands_in_a_bag_of_the_dropper_s_own() {
        // `InvDropHandler.cs:82` picks the soul bag on `item.Soulbound || Account.Admin ||
        // Account.Rank >= 0`. A rank is never negative, so the condition is always true and the
        // plain-bag branch is dead: everything a player drops goes into a soulbound bag nobody else
        // can open. That is an oddity of the original rather than a design, and it is kept.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);

        assert!(put_in_bag(
            &mut world, &catalog, player, None, BLADE.0, false
        ));

        let dropped = bags(&world);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].0, SOULBOUND_BAG, "the soulbound bag");
        assert_eq!(dropped[0].1, Some(player), "and it is theirs");
        assert_eq!(dropped[0].2, DROPPED_BAG_SIZE);
        assert_eq!(dropped[0].3, 1);
    }

    /// A chest of a named kind standing where the player can reach it.
    fn chest_at(
        world: &mut World,
        x: f32,
        y: f32,
        kind: hendra_sim::ContainerKind,
        holding: hendra_content::ObjectType,
    ) -> Handle {
        let mut container = hendra_sim::Container::new(kind, crate::vault::SLOTS_PER_CHEST as usize);
        container.set(0, holding);

        let mut chest = Entity::fixture(hendra_content::ObjectType(0x0500), x, y);
        chest.kind = hendra_sim::Kind::Container;
        chest.container = Some(Box::new(container));
        world.spawn(chest).expect("room for a chest")
    }

    #[test]
    fn nothing_is_taken_out_of_a_vault_chest_by_reaching_into_it() {
        // The chest in the vault is a drawing of what the account's rows say. Handing over what the
        // drawing shows hands over an item whose row is still there, and the next visit draws the
        // chest from those rows again -- one item becomes two.
        let (_catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let chest = chest_at(
            &mut world,
            5.0,
            5.0,
            hendra_sim::ContainerKind::Vault,
            BLADE,
        );

        let taken = take_from_bag(&mut world, player, chest.to_entity_id(), 0);

        assert_eq!(taken, None, "the vault chest gave something up");
        assert_eq!(
            world
                .get(chest)
                .and_then(|entity| entity.container.as_ref())
                .map(|held| held.occupied()),
            Some(1),
            "and it should still be holding it"
        );
    }

    #[test]
    fn a_gift_chest_reports_what_it_holds_without_giving_it_up() {
        // The row is what makes the claim real, and it has not gone yet: the caller takes it and
        // redraws the chest. Emptying the drawing first would be a gift that could be claimed twice
        // if the row would not delete.
        let (_catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let chest = chest_at(
            &mut world,
            5.0,
            5.0,
            hendra_sim::ContainerKind::Merchant,
            BLADE,
        );

        let taken = take_from_bag(&mut world, player, chest.to_entity_id(), 0);

        assert_eq!(
            taken,
            Some(Taken {
                item: BLADE.0,
                kind: hendra_sim::ContainerKind::Merchant,
            })
        );
        assert_eq!(
            world
                .get(chest)
                .and_then(|entity| entity.container.as_ref())
                .map(|held| held.occupied()),
            Some(1),
            "the chest gave the gift up before the row did"
        );
        assert!(
            world.get(chest).is_some_and(|entity| !entity.dead),
            "and it should not have vanished"
        );
    }

    #[test]
    fn a_dropped_bag_lands_beside_the_player() {
        // `player.X + (Rand.NextDouble() * 2 - 1) * 0.5` per axis, so dropping several things in a
        // row leaves a pile that can be picked apart rather than one bag hiding the others.
        let mut places = Vec::new();
        for seed in 1..20u32 {
            let (catalog, mut world) = field();
            world.reseed(seed.wrapping_mul(2_654_435_761).wrapping_add(1));
            let player = standing(&mut world, 5.0, 5.0);

            assert!(put_in_bag(
                &mut world, &catalog, player, None, BLADE.0, false
            ));

            let placed = world
                .iter()
                .find(|(_, entity)| entity.kind == hendra_sim::Kind::Container)
                .map(|(_, entity)| (entity.x - 5.0, entity.y - 5.0))
                .expect("a bag");
            places.push(placed);
        }

        assert!(
            places
                .iter()
                .all(|(dx, dy)| dx.abs() <= 0.5 && dy.abs() <= 0.5),
            "within half a tile"
        );
        assert!(
            places.iter().any(|(dx, dy)| (dx - dy).abs() > 0.05),
            "and the two axes are rolled apart"
        );
    }

    #[test]
    fn a_soulbound_item_aimed_at_a_shared_bag_is_diverted_rather_than_given_away() {
        // `InvSwapHandler.cs:104-107`: the move succeeds, the item leaves the slot it came from, and
        // `DropInSoulboundBag` puts it in a fresh bag of the player's own. Letting it into the
        // shared bag is how a soulbound item reaches somebody else.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let shared = bag_at(&mut world, 5.0, 5.0, None);

        assert!(put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(shared.to_entity_id()),
            HEIRLOOM.0,
            true
        ));

        let shared_holds = world
            .get(shared)
            .and_then(|entity| entity.container.as_ref())
            .map(|held| held.occupied())
            .unwrap_or(0);
        assert_eq!(shared_holds, 0, "the shared bag did not take it");

        let mine: Vec<_> = bags(&world)
            .into_iter()
            .filter(|bag| bag.1 == Some(player))
            .collect();
        assert_eq!(mine.len(), 1, "a bag of the player's own was made");
        assert_eq!(mine[0].0, SOULBOUND_BAG);
        assert_eq!(mine[0].3, 1);
    }

    #[test]
    fn a_soulbound_item_may_go_into_a_bag_the_player_solely_owns() {
        // `IsSoleContainerOwner` (`InvSwapHandler.cs:190`). A bag only this player can open is not a
        // way to hand anything to anybody.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let mine = bag_at(&mut world, 5.0, 5.0, Some(player));

        assert!(put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(mine.to_entity_id()),
            HEIRLOOM.0,
            true
        ));

        assert_eq!(
            world
                .get(mine)
                .and_then(|entity| entity.container.as_ref())
                .map(|held| held.occupied()),
            Some(1),
            "it went straight in"
        );
        assert_eq!(bags(&world).len(), 1, "and no second bag was made");
    }

    #[test]
    fn an_ordinary_item_still_goes_into_a_shared_bag() {
        // The other side of `ValidateItemSwap`: only a soulbound item is diverted.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let shared = bag_at(&mut world, 5.0, 5.0, None);

        assert!(put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(shared.to_entity_id()),
            BLADE.0,
            false
        ));

        assert_eq!(
            world
                .get(shared)
                .and_then(|entity| entity.container.as_ref())
                .map(|held| held.occupied()),
            Some(1)
        );
        assert_eq!(bags(&world).len(), 1);
    }

    #[test]
    fn a_bag_out_of_reach_is_refused() {
        // `Vector2.DistanceSquared(aPos, bPos) > 1` (`InvSwapHandler.cs:168`) — a tile squared
        // between the two containers, which for a player reaching into a bag is a tile.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let far = bag_at(&mut world, 9.0, 5.0, None);

        assert!(!put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(far.to_entity_id()),
            BLADE.0,
            false
        ));
        assert!(!put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(far.to_entity_id()),
            HEIRLOOM.0,
            true
        ));
        assert_eq!(bags(&world).len(), 1, "and nothing new was made either");
    }

    #[test]
    fn somebody_else_s_bag_is_refused() {
        // `BagOwners.Contains(p.AccountId)` (`InvSwapHandler.cs:152-160`). A soulbound item aimed at
        // it is not diverted into a bag of the player's own: the whole move is refused, because the
        // player was never allowed to touch that container at all.
        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);
        let other = standing(&mut world, 5.0, 5.0);
        let theirs = bag_at(&mut world, 5.0, 5.0, Some(other));

        assert!(!put_in_bag(
            &mut world,
            &catalog,
            player,
            Some(theirs.to_entity_id()),
            BLADE.0,
            false
        ));
        assert_eq!(bags(&world).len(), 1);
    }

    /// Puts a set of bonuses on somebody, the way a successful move does.
    fn wear(
        world: &mut World,
        catalog: &Catalog,
        who: Handle,
        bonuses: Vec<(hendra_content::Stat, i32)>,
    ) {
        equip(world, catalog, who, bonuses, None);
    }

    /// The whole message a successful move sends: the bonuses, and what is in the weapon slot.
    fn equip(
        world: &mut World,
        catalog: &Catalog,
        who: Handle,
        bonuses: Vec<(hendra_content::Stat, i32)>,
        weapon: Option<hendra_content::ObjectType>,
    ) {
        let weapon_damage = weapon
            .and_then(|kind| catalog.object(kind))
            .and_then(|desc| desc.projectiles.first())
            .map(|shot| (shot.min_damage, shot.max_damage))
            .unwrap_or((0, 0));

        handle(
            world,
            catalog,
            &bare_loadout(),
            &mut Vec::new(),
            ToWorld::Equipment {
                handle: who,
                bonuses,
                weapon_damage,
                weapon,
                set_skin: None,
            },
        );
    }

    /// The same message, carrying only what a completed set puts on: the look, or nothing.
    fn dress(
        world: &mut World,
        catalog: &Catalog,
        who: Handle,
        set_skin: Option<hendra_content::SetSkin>,
    ) {
        handle(
            world,
            catalog,
            &bare_loadout(),
            &mut Vec::new(),
            ToWorld::Equipment {
                handle: who,
                bonuses: Vec::new(),
                weapon_damage: (0, 0),
                weapon: None,
                set_skin,
            },
        );
    }

    /// A finished set is worn on the body, and breaking it takes the look off again.
    ///
    /// `ApplySetBonus` answers `ChangeSkin` by assigning the set's skin and size to the player
    /// (`BoostStatManager.cs:73-76`) and answers the same set having just lost a piece with
    /// `RestoreDefaultSkin` and `RestoreDefaultSize` (`:104-112`). The second half is the one worth
    /// a test: a skin that stayed on after the fourth piece came off would be a player who could
    /// never get their own face back.
    #[test]
    fn a_completed_set_dresses_the_body_and_breaking_it_undresses_them() {
        const GEB: hendra_content::SetSkin = hendra_content::SetSkin {
            skin: 0x745A,
            size: 70,
        };

        let (catalog, mut world) = field();
        let player = standing(&mut world, 5.0, 5.0);

        // What the wardrobe chose, which is what breaking the set has to put back.
        if let Some(entity) = world.get_mut(player) {
            entity.skin = 0x0403;
            entity.default_skin = 0x0403;
            entity.size = 120;
            entity.default_size = 120;
        }

        dress(&mut world, &catalog, player, Some(GEB));
        let worn = world.get(player).expect("the player");
        assert_eq!(worn.skin, GEB.skin, "the set was completed and did nothing");
        assert_eq!(worn.size, GEB.size, "and it brings its own size with it");

        dress(&mut world, &catalog, player, None);
        let bare = world.get(player).expect("the player");
        assert_eq!(bare.skin, 0x0403, "the set skin outlived the set");
        assert_eq!(bare.size, 120, "and so did the size it brought");
    }

    /// Swapping a weapon changes the bullet, not only the number on the sheet.
    ///
    /// The original never holds a weapon: `PlayerShootHandler` reads `Inventory[0]` on every shot
    /// and refuses one that names anything else (`Player.AntiCheat.cs:88-90`), taking the
    /// descriptor straight off that item (`PlayerShootHandler.cs:46`). A body here carries its
    /// weapon between shots, and the only message that ever told it what to carry was the arrival.
    /// So a player who picked up a better wand went on firing the old one for the rest of the
    /// session -- and once the stats began following slot zero, the damage on the sheet and the
    /// damage the bullet actually did no longer agreed.
    #[test]
    fn swapping_the_weapon_changes_the_bullet_that_is_fired() {
        const PRACTICE: hendra_content::ObjectType = hendra_content::ObjectType(0x901);
        const FINE: hendra_content::ObjectType = hendra_content::ObjectType(0x902);

        let (catalog, mut world) = field();
        let who = standing(&mut world, 8.0, 8.0);
        world.reindex();

        equip(&mut world, &catalog, who, Vec::new(), Some(PRACTICE));

        let fired = world.shoot(who, &catalog, 0.0);
        let first = world
            .projectiles()
            .find(|(handle, _)| *handle == fired[0])
            .map(|(_, shot)| (shot.container, shot.damage))
            .expect("a bullet from the practice wand");
        assert_eq!(first.0, PRACTICE, "the practice wand's own bullet");

        // The swap, which is one inventory change and so one `Equipment` message.
        equip(&mut world, &catalog, who, Vec::new(), Some(FINE));

        // Off cooldown again, because the point is which weapon fires rather than how soon.
        world.advance(&catalog, 2000);

        let fired = world.shoot(who, &catalog, 0.0);
        let second = world
            .projectiles()
            .find(|(handle, _)| *handle == fired[0])
            .map(|(_, shot)| (shot.container, shot.damage))
            .expect("a bullet from the fine wand");
        assert_eq!(
            second.0, FINE,
            "the bullet follows the slot rather than the weapon the body arrived holding"
        );

        // And it hits for the new weapon's bounds, not the old one's: the same read feeds the
        // damage the character sheet shows, so the two cannot drift apart.
        assert!(
            second.1 > first.1,
            "the fine wand's nine-to-one damage, not the practice wand's ({} against {})",
            second.1,
            first.1
        );
    }

    /// And taking the weapon off leaves nothing to fire.
    ///
    /// An empty slot is `Inventory[0] == null`, which fails the `ITEM_MISMATCH` test against any
    /// item the client could name (`Player.AntiCheat.cs:88-90`): no shot is made at all.
    #[test]
    fn a_weapon_taken_out_of_the_slot_stops_being_fired() {
        const PRACTICE: hendra_content::ObjectType = hendra_content::ObjectType(0x901);

        let (catalog, mut world) = field();
        let who = standing(&mut world, 8.0, 8.0);
        world.reindex();

        equip(&mut world, &catalog, who, Vec::new(), Some(PRACTICE));
        assert_eq!(world.shoot(who, &catalog, 0.0).len(), 1);

        equip(&mut world, &catalog, who, Vec::new(), None);
        world.advance(&catalog, 2000);

        assert!(
            world.shoot(who, &catalog, 0.0).is_empty(),
            "an empty hand fires nothing"
        );
    }

    /// The bug the user could see: "my hp is not going up".
    ///
    /// `Player` wires `ReCalculateValues` straight to the inventory (`Player.cs:463`) and the
    /// original reads `Stats[0]` rather than carrying it (`StatsManager.cs:23`), so a piece that
    /// grants maximum health raises the bar the moment it goes on and lowers it the moment it comes
    /// off. Carrying the ceiling on the body without re-deriving it here left the number under the
    /// bar moving while the bar itself stood still.
    #[test]
    fn equipment_that_grants_health_moves_the_bar_and_not_only_the_number() {
        let (catalog, mut world) = field();
        let who = standing(&mut world, 8.0, 8.0);

        world.get_mut(who).unwrap().stats =
            hendra_sim::stats::Stats::from_base(&[100, 100, 0, 0, 0, 0, 0, 0]);
        world.get_mut(who).unwrap().reseat_maxima();
        assert_eq!(world.get(who).unwrap().max_hp, 100);

        wear(
            &mut world,
            &catalog,
            who,
            vec![
                (hendra_content::Stat::MaxHitPoints, 80),
                (hendra_content::Stat::MaxMagicPoints, 40),
            ],
        );

        let worn = world.get(who).unwrap();
        assert_eq!(worn.stats.max_hp(), 180);
        assert_eq!(worn.max_hp, 180, "the bar grew with the robe");
        assert_eq!(worn.max_mp, 140, "and so did the magic bar");

        // Taking it off again. `apply_equipment` replaces the layer wholesale, so an empty list is
        // an empty pair of hands.
        wear(&mut world, &catalog, who, Vec::new());

        let bare = world.get(who).unwrap();
        assert_eq!(bare.max_hp, 100, "and shrank again when it came off");
        assert_eq!(bare.max_mp, 100);
    }

    /// A boost held while the robe changes has to survive the change.
    ///
    /// The boost path moves the ceiling by what the boost was worth, and the equipment path works
    /// the whole thing out again from base plus equipment plus boost. The two agree only if the
    /// second counts the boost, which is the arithmetic that goes wrong when a maximum has two
    /// sources of truth.
    #[test]
    fn changing_equipment_under_a_boost_keeps_the_boost() {
        let (catalog, mut world) = field();
        let who = standing(&mut world, 8.0, 8.0);

        let entity = world.get_mut(who).unwrap();
        entity.stats = hendra_sim::stats::Stats::from_base(&[100, 100, 0, 0, 0, 0, 0, 0]);
        entity.stats.boost(hendra_content::Stat::MaxHitPoints, 50);
        entity.reseat_maxima();
        assert_eq!(world.get(who).unwrap().max_hp, 150);

        wear(
            &mut world,
            &catalog,
            who,
            vec![(hendra_content::Stat::MaxHitPoints, 80)],
        );
        assert_eq!(
            world.get(who).unwrap().max_hp,
            230,
            "the robe adds to the boost rather than replacing it"
        );

        wear(&mut world, &catalog, who, Vec::new());
        assert_eq!(
            world.get(who).unwrap().max_hp,
            150,
            "and taking the robe off leaves the boost standing"
        );
    }

    /// The original never pulls a pool down when its ceiling drops.
    ///
    /// `HandleRegen` only ever moves health by `Math.Min(Stats[0], HP + regen)`
    /// (`Player.cs:617-625`), which needs a regen tick that can regen at all: a player who takes off
    /// a health ring keeps the overflow, and keeps it for as long as something stops them
    /// regenerating.
    #[test]
    fn taking_off_a_health_ring_does_not_pull_health_down_on_the_spot() {
        let (catalog, mut world) = field();
        let who = standing(&mut world, 8.0, 8.0);

        world.get_mut(who).unwrap().stats =
            hendra_sim::stats::Stats::from_base(&[100, 100, 0, 0, 0, 0, 0, 0]);
        wear(
            &mut world,
            &catalog,
            who,
            vec![(hendra_content::Stat::MaxHitPoints, 80)],
        );
        assert_eq!(world.get(who).unwrap().max_hp, 180, "the ring went on");
        world.get_mut(who).unwrap().hp = 180;

        wear(&mut world, &catalog, who, Vec::new());

        let bare = world.get(who).unwrap();
        assert_eq!(bare.max_hp, 100, "the ceiling comes down at once");
        assert_eq!(bare.hp, 180, "and what is over it is left for the regen tick");
    }

    const POTION: hendra_content::ObjectType = hendra_content::ObjectType(0x908);
    const ELIXIR: hendra_content::ObjectType = hendra_content::ObjectType(0x909);
    const SPELL: hendra_content::ObjectType = hendra_content::ObjectType(0x90a);

    /// A player standing beside a bag, and the bag with one thing in its first slot.
    fn hurt_beside_a_bag(
        world: &mut World,
        item: hendra_content::ObjectType,
        away: f32,
    ) -> (Handle, Handle) {
        let who = standing(world, 8.0, 8.0);
        let hurt = world.get_mut(who).unwrap();
        hurt.max_hp = 500;
        hurt.hp = 20;

        let bag = bag_at(world, 8.0 + away, 8.0, None);
        world
            .get_mut(bag)
            .unwrap()
            .container
            .as_mut()
            .unwrap()
            .set(0, item);

        (who, bag)
    }

    /// The thing a slot on its own cannot say.
    ///
    /// `UseItem` is handed an object id as well as a slot id (`UseItemHandler.cs:22`) and reads the
    /// entity it names as an `IContainer` (`Player.UseItem.cs:106-128`), so a potion in a bag at the
    /// player's feet is drunk where it lies. A message carrying only a slot can express the ask at
    /// all only for the player's own pack.
    #[test]
    fn a_potion_is_drunk_out_of_a_bag_on_the_ground() {
        let (catalog, mut world) = field();
        let (who, bag) = hurt_beside_a_bag(&mut world, POTION, 1.0);

        let ran = use_from_container(
            &mut world,
            &catalog,
            who,
            bag.to_entity_id(),
            0,
            (8.0, 8.0),
        );

        assert_eq!(ran.len(), 1, "the heal ran");
        assert_eq!(world.get(who).unwrap().hp, 120, "and healed for a hundred");
        assert!(
            world.get(bag).is_none() || world.get(bag).unwrap().dead,
            "an emptied bag goes"
        );
    }

    /// Three tiles, not one. `UseItem` refuses on `this.Dist(entity) > 3`
    /// (`Player.UseItem.cs:126`), which is wider than the tile a swap is held to.
    #[test]
    fn a_bag_out_of_reach_is_refused_and_keeps_what_is_in_it() {
        let (catalog, mut world) = field();
        let (who, bag) = hurt_beside_a_bag(&mut world, POTION, 4.0);

        let ran = use_from_container(
            &mut world,
            &catalog,
            who,
            bag.to_entity_id(),
            0,
            (8.0, 8.0),
        );

        assert!(ran.is_empty(), "too far to reach");
        assert_eq!(world.get(who).unwrap().hp, 20, "and nobody was healed");
        assert_eq!(
            world.get(bag).unwrap().container.as_ref().unwrap().item(0),
            POTION,
            "the potion is still in the bag"
        );

        // And one tile inside the three works, which is what makes the number the number.
        let (who, bag) = hurt_beside_a_bag(&mut world, POTION, 2.5);
        assert_eq!(
            use_from_container(&mut world, &catalog, who, bag.to_entity_id(), 0, (8.0, 8.0)).len(),
            1
        );
    }

    /// An item that is not a consumable and does not fit an untyped slot is refused.
    ///
    /// The tail of the original's method: `item.Consumable || item.SlotType == slotType`
    /// (`Player.UseItem.cs:243`), where a bag's slot types are all zero
    /// (`EmbeddedData_ContainersCXML.xml`, Loot Bag). A sword lying in a bag is not an ability, and
    /// neither is a spell: an ability works where it is worn, and a bag is not where it is worn.
    #[test]
    fn a_sword_in_a_bag_is_not_something_to_use() {
        let (catalog, mut world) = field();

        for item in [BLADE, SPELL] {
            let (who, bag) = hurt_beside_a_bag(&mut world, item, 1.0);

            let ran =
                use_from_container(&mut world, &catalog, who, bag.to_entity_id(), 0, (8.0, 8.0));

            assert!(ran.is_empty(), "{item:?} activated out of a bag");
            assert_eq!(world.get(who).unwrap().hp, 20, "and healed nobody");
            assert_eq!(
                world.get(bag).unwrap().container.as_ref().unwrap().item(0),
                item,
                "and it is still there"
            );
        }
    }

    /// A consumable with more uses in it turns into the next one where it lies.
    ///
    /// `cInv[slot] = successor` sets the slot rather than emptying it (`Player.UseItem.cs:190-193`),
    /// so the bag is left holding a Health Potion and does not vanish.
    #[test]
    fn an_elixir_in_a_bag_becomes_its_successor_rather_than_going() {
        let (catalog, mut world) = field();
        let (who, bag) = hurt_beside_a_bag(&mut world, ELIXIR, 1.0);

        let ran = use_from_container(
            &mut world,
            &catalog,
            who,
            bag.to_entity_id(),
            0,
            (8.0, 8.0),
        );

        assert_eq!(ran.len(), 1);
        assert_eq!(world.get(who).unwrap().hp, 70, "healed for fifty");

        let left = world.get(bag).unwrap();
        assert!(!left.dead, "the bag still holds something");
        assert_eq!(left.container.as_ref().unwrap().item(0), POTION);
    }

    /// A bag somebody else's loot is in stays theirs.
    #[test]
    fn a_bag_that_belongs_to_somebody_else_gives_up_nothing() {
        let (catalog, mut world) = field();
        let (who, bag) = hurt_beside_a_bag(&mut world, POTION, 1.0);
        let other = standing(&mut world, 9.0, 9.0);
        world.get_mut(bag).unwrap().belongs_to = Some(other);

        assert!(
            use_from_container(&mut world, &catalog, who, bag.to_entity_id(), 0, (8.0, 8.0))
                .is_empty()
        );
        assert_eq!(world.get(who).unwrap().hp, 20);
    }

    /// The dead drink nothing, and the potion is not spent finding that out.
    #[test]
    fn a_dead_player_does_not_empty_the_bag_they_fell_beside() {
        let (catalog, mut world) = field();
        let (who, bag) = hurt_beside_a_bag(&mut world, POTION, 1.0);
        world.get_mut(who).unwrap().dead = true;

        assert!(
            use_from_container(&mut world, &catalog, who, bag.to_entity_id(), 0, (8.0, 8.0))
                .is_empty()
        );
        assert_eq!(
            world.get(bag).unwrap().container.as_ref().unwrap().item(0),
            POTION,
            "spending it on nothing is the bug the gate exists to stop"
        );
    }
}

#[cfg(test)]
mod arriving {
    use super::*;
    use hendra_content::{Composition, Region, TileType};
    use hendra_sim::World;

    const CONTENT: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"/>
      </Objects>"#;

    /// A room whose named squares are all marked `Spawn`.
    fn room(name: &str, marked: &[(u32, u32)]) -> World {
        let catalog = Catalog::load_str(&[CONTENT]).0;

        let squares = (0..16 * 16).map(|index| {
            let (x, y) = (index as u32 % 16, index as u32 / 16);
            Composition {
                tile: TileType(0x10),
                object: hendra_content::ObjectType::NONE,
                region: if marked.contains(&(x, y)) {
                    Region::Spawn
                } else {
                    Region::None
                },
                terrain: hendra_content::Terrain::None,
                config: String::new(),
            }
        });

        let map = hendra_content::Map::from_squares(16, 16, squares).unwrap();
        World::new(name, hendra_sim::Terrain::build(map, &catalog), &catalog)
    }

    /// Where twenty arrivals in a row are put.
    fn arrivals(world: &mut World) -> std::collections::HashSet<(u32, u32)> {
        (0..20)
            .map(|_| {
                let (x, y) = spawn_point(world);
                (x as u32, y as u32)
            })
            .collect()
    }

    #[test]
    fn a_crowd_is_scattered_over_every_square_the_map_marks() {
        // `Player.Init` picks `spawnRegions.ElementAt(rand.Next(0, spawnRegions.Length))`
        // (`realm/entities/player/Player.cs:519-527`), so the sixty squares `nexustry.jm` marks are
        // sixty places to land and not one. Putting everybody on the first is a pile on one tile.
        let mut world = room("Nexus", &[(2, 3), (11, 4), (5, 12), (14, 14)]);

        let landed = arrivals(&mut world);
        assert!(
            landed.len() > 1,
            "twenty arrivals all landed on {landed:?}"
        );
        for (x, y) in &landed {
            assert!(
                [(2, 3), (11, 4), (5, 12), (14, 14)].contains(&(*x, *y)),
                "landed on ({x}, {y}), which the map does not mark"
            );
        }
    }

    #[test]
    fn the_castle_puts_everybody_on_the_same_square() {
        // `Castle.GetSpawnPoints` takes one marked square while fewer than twenty players are
        // arriving (`realm/worlds/logic/Castle.cs:26-29`), and the count is always zero because
        // `DynamicWorld` constructs with two arguments (`realm/worlds/DynamicWorld.cs:37`) and the
        // two-argument constructor sets it so (`Castle.cs:18-22`). A raid quaked out of a closed
        // realm arrives together.
        let mut world = room(CASTLE, &[(2, 3), (11, 4), (5, 12), (14, 14)]);

        let landed = arrivals(&mut world);
        assert_eq!(
            landed,
            std::collections::HashSet::from([(2, 3)]),
            "the castle takes the first marked square and only that one"
        );
    }

    #[test]
    fn one_marked_square_is_still_that_square() {
        // A vault, a guild hall and a shop all mark exactly one, and a random choice among one is
        // still it. Worth pinning: the scatter must not drift off a map that means one place.
        let mut world = room("Vault", &[(7, 9)]);

        assert_eq!(
            arrivals(&mut world),
            std::collections::HashSet::from([(7, 9)])
        );
    }

    #[test]
    fn a_map_that_marks_nothing_still_finds_ground() {
        // `Player.Init` leaves `x` and `y` at zero when nothing is marked and moves there. Walking
        // out from the middle instead is the one deliberate departure: the origin of a map is as
        // likely to be wall as anything else, and a player stood in a wall cannot move.
        let mut world = room("Nowhere", &[]);

        let (x, y) = spawn_point(&mut world);
        assert!(world.terrain().walkable(x as u32, y as u32));
    }
}

#[cfg(test)]
mod marketplace {
    use super::*;
    use hendra_content::{Composition, Region, TileType};
    use hendra_sim::World;

    const CONTENT: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"/>
        <Object type="0x01ca" id="Merchant"><Class>Merchant</Class></Object>

        <!-- Which row an item stands in is read off the slots a class declares, in the order
             weapon, ability, armour, ring (`common/resources/XmlData.cs:271-278`), so a board
             cannot be tested without one. -->
        <Object type="0x600" id="Hero">
          <Class>Player</Class><Player/>
          <SlotTypes>1, 4, 6, 9, 15, 24, 25, 26</SlotTypes>
        </Object>

        <Object type="0x904" id="Blade">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType>
        </Object>
        <Object type="0x905" id="Sabre">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType>
        </Object>
      </Objects>"#;

    const BLADE: ObjectType = ObjectType(0x904);
    const SABRE: ObjectType = ObjectType(0x905);

    /// A marketplace, for these purposes: a row of squares marked for weapons.
    fn row(squares: usize) -> (Catalog, World) {
        let catalog = Catalog::load_str(&[CONTENT]).0;

        let map = hendra_content::Map::from_squares(
            16,
            16,
            (0..16 * 16).map(|index| Composition {
                tile: TileType(0x10),
                object: ObjectType::NONE,
                region: if index < squares {
                    Region::Store9
                } else {
                    Region::None
                },
                terrain: hendra_content::Terrain::None,
                config: String::new(),
            }),
        )
        .unwrap();

        let world = World::new(
            "Marketplace",
            hendra_sim::Terrain::build(map, &catalog),
            &catalog,
        );
        (catalog, world)
    }

    fn listing(id: i64, item: ObjectType, price: i32) -> hendra_sim::market::Listing {
        hendra_sim::market::Listing { id, item, price }
    }

    fn stand(board: &mut Board, world: &mut World, catalog: &Catalog, listings: &[hendra_sim::market::Listing]) {
        board.market.restock(catalog, listings);
        board.apply(world);
    }

    /// What is standing, as `(item, price, count, listing)`, in the order the squares are marked.
    fn stalls(world: &World) -> Vec<(ObjectType, i32, i32, Option<i64>)> {
        let mut standing: Vec<((u32, u32), (ObjectType, i32, i32, Option<i64>))> = world
            .iter()
            .filter_map(|(_, entity)| {
                let selling = entity.selling?;
                Some((
                    (entity.x as u32, entity.y as u32),
                    (selling.item, selling.price, selling.count, selling.listing),
                ))
            })
            .collect();
        standing.sort_by_key(|(at, _)| (at.1, at.0));
        standing.into_iter().map(|(_, what)| what).collect()
    }

    #[test]
    fn a_world_that_marks_nothing_has_no_board() {
        let (catalog, world) = row(0);
        assert!(Board::of_world(&world, &catalog).is_none());
    }

    #[test]
    fn a_merchant_stands_on_a_marked_square_holding_the_cheapest_of_its_item() {
        // `AddMerchants` puts one on each marked square and gives it what the queue hands over
        // (`realm/Market.cs:270-305`); `Market.Reload` takes the price from `shop[0]` and the count
        // from the whole list (`:231-236`).
        let (catalog, mut world) = row(4);
        let mut board = Board::of_world(&world, &catalog).expect("a board");

        stand(
            &mut board,
            &mut world,
            &catalog,
            &[
                listing(1, BLADE, 500),
                listing(2, BLADE, 120),
                listing(3, SABRE, 90),
            ],
        );

        let standing = stalls(&world);
        assert_eq!(standing.len(), 2, "one merchant per item, not per listing");
        assert!(standing.contains(&(BLADE, 120, 2, Some(2))));
        assert!(standing.contains(&(SABRE, 90, 1, Some(3))));
    }

    #[test]
    fn a_market_merchant_asks_for_two_stars() {
        // `PlayerMerchant`'s constructor sets `RankReq = 2` (`PlayerMerchant.cs:17`), where the
        // shops the map stands ask for nothing.
        let (catalog, mut world) = row(2);
        let mut board = Board::of_world(&world, &catalog).expect("a board");
        stand(&mut board, &mut world, &catalog, &[listing(1, BLADE, 10)]);

        let ranks: Vec<i16> = world
            .iter()
            .filter_map(|(_, entity)| entity.selling.map(|stall| stall.rank))
            .collect();
        assert_eq!(ranks, vec![MARKET_RANK]);
    }

    #[test]
    fn a_rotation_keeps_the_entity_and_changes_only_what_it_holds() {
        // The original's `PlayerMerchant` outlives every item it shows: only `RemoveMerchant` takes
        // one out of the world (`realm/Market.cs:338-344`). A merchant that vanished and came back
        // would flicker in front of anybody standing at it.
        let (catalog, mut world) = row(1);
        let mut board = Board::of_world(&world, &catalog).expect("a board");
        stand(
            &mut board,
            &mut world,
            &catalog,
            &[listing(1, BLADE, 10), listing(2, SABRE, 20)],
        );

        let before: Vec<Handle> = world
            .iter()
            .filter(|(_, entity)| entity.selling.is_some())
            .map(|(handle, _)| handle)
            .collect();
        let held = stalls(&world);

        // One tick past this square's turn.
        board.market.tick(
            hendra_sim::market::OFFSET_STEP + 100,
            200,
            |_| false,
        );
        board.apply(&mut world);

        let after: Vec<Handle> = world
            .iter()
            .filter(|(_, entity)| entity.selling.is_some())
            .map(|(handle, _)| handle)
            .collect();

        assert_eq!(before, after, "the same merchant is standing there");
        assert_ne!(stalls(&world), held, "holding something else");
    }

    #[test]
    fn a_merchant_with_nothing_left_to_sell_leaves_the_world() {
        // `RemoveMerchant` (`realm/Market.cs:338-344`). A stall left standing for something already
        // sold refuses everybody who walks up to it.
        let (catalog, mut world) = row(3);
        let mut board = Board::of_world(&world, &catalog).expect("a board");
        stand(&mut board, &mut world, &catalog, &[listing(1, BLADE, 10)]);
        assert_eq!(stalls(&world).len(), 1);

        stand(&mut board, &mut world, &catalog, &[]);
        assert!(stalls(&world).is_empty());
    }

    #[test]
    fn a_purchase_leaves_the_merchant_showing_the_next_cheapest() {
        // `Market.Remove` reloads the merchant that was holding the listing (`realm/Market.cs:165`),
        // and with the item still for sale that keeps the same item and moves the price on.
        let (catalog, mut world) = row(2);
        let mut board = Board::of_world(&world, &catalog).expect("a board");
        stand(
            &mut board,
            &mut world,
            &catalog,
            &[listing(1, BLADE, 100), listing(2, BLADE, 300)],
        );
        assert_eq!(stalls(&world), vec![(BLADE, 100, 2, Some(1))]);

        // Somebody buys the cheap one.
        stand(&mut board, &mut world, &catalog, &[listing(2, BLADE, 300)]);
        assert_eq!(stalls(&world), vec![(BLADE, 300, 1, Some(2))]);
    }
}

#[cfg(test)]
mod vault_room {
    use super::*;
    use hendra_content::{Composition, Region, TileType};
    use hendra_sim::World;

    const CONTENT: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"/>
        <Object type="0x0504" id="Vault Chest"><Class>VaultAccess</Class></Object>
      </Objects>"#;

    /// A room with a spawn and several marked squares, at known distances from it.
    fn room(marked: &[(u32, u32)], spawn: (u32, u32)) -> (Catalog, World) {
        let catalog = Catalog::load_str(&[CONTENT]).0;

        let squares = (0..16 * 16).map(|index| {
            let (x, y) = (index as u32 % 16, index as u32 / 16);
            Composition {
                tile: TileType(0x10),
                object: hendra_content::ObjectType::NONE,
                region: if (x, y) == spawn {
                    Region::Spawn
                } else if marked.contains(&(x, y)) {
                    Region::Vault
                } else {
                    Region::None
                },
                terrain: hendra_content::Terrain::None,
                config: String::new(),
            }
        });

        let map = hendra_content::Map::from_squares(16, 16, squares).unwrap();
        let world = World::new("Vault", hendra_sim::Terrain::build(map, &catalog), &catalog);
        (catalog, world)
    }

    #[test]
    fn the_access_object_stands_on_the_marked_square_nearest_the_spawn() {
        // `Vault.InitVault` sorts the marked squares by their squared distance from the spawn
        // (pristine `realm/worlds/logic/Vault.cs:88-90`) and fills them from the nearest outwards
        // (`:96-107`). There were eighty of them ringing the garden; there is one object now,
        // wherever the map still marks.
        let (catalog, mut world) = room(&[(2, 2), (9, 10), (12, 3)], (10, 10));

        assert_eq!(place_vault_access(&mut world, &catalog), 1);

        let placed: Vec<(f32, f32)> = world
            .iter()
            .filter(|(_, entity)| entity.object_type == VAULT_ACCESS)
            .map(|(_, entity)| (entity.x, entity.y))
            .collect();

        assert_eq!(placed, vec![(9.5, 10.5)], "not the nearest marked square");
    }

    #[test]
    fn the_access_object_holds_nothing_and_cannot_be_reached_into() {
        // It is not a container: the vault travels as its own message, and a chest standing in the
        // room holding a copy of what the account owns is an item that exists in two places. It is
        // also why `Container.Tick` exempts object type 0x504 from the emptying that removes a loot
        // bag (`realm/entities/Container.cs:90-91`) -- nothing here can expire it either.
        let (catalog, mut world) = room(&[(4, 4)], (4, 5));

        place_vault_access(&mut world, &catalog);

        let (_, access) = world
            .iter()
            .find(|(_, entity)| entity.object_type == VAULT_ACCESS)
            .expect("the object should have been placed");

        assert!(access.container.is_none(), "the access object holds items");
        assert_eq!(access.kind, hendra_sim::Kind::Fixture);
        assert_eq!(access.expires_in_ms, None, "the vault can expire");
        assert_eq!(access.size, VAULT_ACCESS_SIZE);
    }

    #[test]
    fn arriving_twice_leaves_one_of_them() {
        // Placed on arrival, and a player can arrive in their vault more than once in a session.
        let (catalog, mut world) = room(&[(4, 4)], (4, 5));

        place_vault_access(&mut world, &catalog);
        place_vault_access(&mut world, &catalog);

        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.object_type == VAULT_ACCESS)
                .count(),
            1
        );
    }

    #[test]
    fn a_map_that_marks_nowhere_gets_nothing() {
        let (catalog, mut world) = room(&[], (4, 5));
        assert_eq!(place_vault_access(&mut world, &catalog), 0);
    }

}
