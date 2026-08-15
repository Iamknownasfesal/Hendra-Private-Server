//! The Godot side of the protocol.
//!
//! This is the whole reason the protocol lives in a shared crate. The extension links the same
//! `hendra-net` the server does, so the client never decodes a byte of the wire format itself.
//! what crosses into Godot is decoded state.
//!
//! # Threads
//!
//! Godot is single-threaded and calls [`HendraConnection::poll`] once per frame; it must never
//! block. A worker thread runs a tokio runtime that owns the connection, decodes everything that
//! arrives, and publishes the result. The two communicate only through the shared state below:
//!
//! ```text
//!   Godot frame ──▶ poll()  ──reads──▶  events queue, world view
//!                     │
//!                     └──sends──▶  command channel  ──▶  worker ──▶ QUIC
//! ```
//!
//! Nothing in `_process` waits on the network, and nothing on the worker touches a Godot object.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use godot::classes::Node;
use godot::prelude::*;

use hendra_net::message::{
    ClientMessage, Input, PROTOCOL_VERSION, RejectReason, ServerMessage, SlotLocation,
};
use hendra_net::snapshot::{Acknowledgement, BaselineRing};
use hendra_net::{
    Delivery, EntityId, Reader, Tick, WorldSnapshot, Writer, decode_body, read_header,
};
use hendra_transport::{Received, Trust, connect};

struct HendraExtension;

#[gdextension]
unsafe impl ExtensionLibrary for HendraExtension {}

/// Where a connection currently stands, as a value Godot can read without locking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Idle = 0,
    Connecting = 1,
    Connected = 2,
    Failed = 3,
    Closed = 4,
}

impl Status {
    fn from_code(code: u8) -> Status {
        match code {
            1 => Status::Connecting,
            2 => Status::Connected,
            3 => Status::Failed,
            4 => Status::Closed,
            _ => Status::Idle,
        }
    }
}

/// Something worth telling the game about. Snapshots are not here; they land in the world view,
/// because the client wants the current state rather than a history of changes to it.
enum Event {
    Connected,
    Welcome {
        player: u32,
        tick: u32,
        world: String,

        /// How large the map is, in tiles, so a client can size itself before the first row lands.
        width: u16,
        height: u16,

        /// What the world says about itself: the sky, the marks on the loading screen, whether the
        /// menu over another player should offer a teleport, and what to play.
        background: i32,
        difficulty: i32,
        allow_teleport: bool,
        show_displays: bool,
        music: String,
    },
    Rejected(RejectReason),

    /// The world this player is standing in now sounds different.
    SwitchMusic(String),

    /// What this player's quest arrow points at.
    QuestTarget(u32),

    /// Whose body this player's camera is on. Their own id gives the camera back.
    SetFocus(u32),

    /// Who is on one of this account's lists, by name.
    AccountList {
        /// `0` ignored, `1` locked out, as [`hendra_net::AccountList`] numbers them.
        list: u8,
        names: Vec<String>,
    },
    Chat {
        /// The body that said it, or zero for a line the server spoke in its own voice.
        speaker: u32,
        from: String,
        text: String,
    },
    Disconnected(String),

    /// The whole contents of one of the player's containers.
    Container {
        container: u8,
        slots: Vec<(u16, u16)>,
    },

    /// The whole vault, as the server has it: the panel is drawn from nothing else.
    VaultUpdate {
        version: u32,
        chest_count: u32,
        max_chests: u32,
        next_chest_price: u32,

        /// Item types, eight per chest, flat. `0xffff` where a slot is empty.
        slots: Vec<u16>,

        /// Gifts waiting to be claimed, dense.
        gifts: Vec<u16>,
    },

    /// Something the player asked for was refused, with a line to show them.
    Refused(String),

    /// Squares whose ground changed, as `(x, y, tile)`.
    Ground(Vec<(u16, u16, u16)>),

    /// Something the world wants shown rather than said.
    Notice(String),

    /// A word the client acts on rather than reads: whether a gift is waiting, a key colour, the
    /// key panel. `GlobalNotification` in the original.
    Notification(String),

    /// The server is full, and this is where you stand in the line.
    Queued {
        place: u32,
        waiting: u32,
    },

    /// How many of each stacking potion the character holds.
    Stacks {
        health: u16,
        magic: u16,
    },

    /// This character has died, and the session is over.
    Died {
        character: u32,
        killed_by: String,
        fame: i32,
    },

    /// Somebody asked to trade.
    TradeRequested(String),

    /// A trade began. Each slot is `(item, slot_type, included, tradeable)`, with the item absent
    /// where the slot is empty.
    TradeStart {
        mine: Vec<hendra_net::TradeSlot>,
        their_name: String,
        theirs: Vec<hendra_net::TradeSlot>,
    },

    /// The other side changed what they are offering.
    TradeChanged(Vec<bool>),

    /// The other side agreed, and to what.
    TradeAccepted {
        mine: Vec<bool>,
        theirs: Vec<bool>,
    },

    /// The trade ended. Zero means it went through.
    TradeDone {
        code: u32,
        message: String,
    },

    /// The scenery standing in one row of the map, as `(x, object, size)`.
    ///
    /// Not entities: scenery never moves and never acts, so it arrives once with the ground rather
    /// than in every snapshot.
    Scenery {
        y: u16,
        objects: Vec<(u16, u16, u16)>,
    },

    /// One row of the map, expanded from the runs it arrived as.
    Terrain {
        x: u16,
        y: u16,
        tiles: Vec<u16>,
    },

    /// A projectile was fired. Its whole flight follows from these fields, so this arrives once and
    /// the client animates the rest itself.
    Shot {
        projectile: u32,
        owner: u32,
        object_type: u16,
        x: f32,
        y: f32,
        angle: f32,
        speed: f32,
        lifetime_ms: u32,

        /// What it takes off whatever it hits, before that body's defence.
        damage: u16,
    },

    /// A body is somewhere it did not walk to.
    ///
    /// The one thing the snapshot cannot say. A client owns the position of its own player and
    /// glides every other body towards the position the snapshot gives it, so neither can express
    /// "stop, you are here now".
    Goto {
        object_id: u32,
        x: f32,
        y: f32,
    },

    /// Something to draw that is neither a body, a bullet nor a number.
    ///
    /// Passed through as the numbers it arrived as. Every one of them is overloaded per effect —
    /// a radius, a particle size, a flash period — and the renderer is the only thing that knows
    /// which, so nothing is interpreted on the way past.
    ShowEffect {
        effect: u8,
        target: u32,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: u32,
    },

    /// A line of text to float off a body: what healed it, what it just became, what it earned.
    StatusText {
        object_id: u32,
        text: String,
        color: u32,
    },

    /// A blast at a place, and what standing in it costs.
    Aoe {
        x: f32,
        y: f32,
        radius: f32,
        damage: u16,
        effect: u8,
        duration: f32,
        orig_type: u16,
    },

    /// Somebody has asked this player into their guild.
    InvitedToGuild {
        name: String,
        guild: String,
    },

    /// Something took a hit, and whether the hit ended it.
    ///
    /// The snapshot carries health, so this is not where the number comes from. It is the only
    /// thing on the wire that tells a body that died from one that walked out of sight, and it is
    /// what puts a damage number over anything the local player did not shoot themselves.
    Damage {
        target: u32,
        effects: u128,
        amount: u16,
        kill: bool,
        bullet: u8,
        owner: u32,
    },
}

/// The decoded world, laid out for the client to read cheaply.
///
/// Parallel arrays rather than a list of dictionaries: building 120 Godot dictionaries every frame
/// would cost more than the networking does, and the client wants to walk these column-wise anyway.
#[derive(Default)]
struct WorldView {
    ids: Vec<i32>,
    types: Vec<i32>,
    /// Interleaved x, y.
    positions: Vec<f32>,
    hp: Vec<i32>,
    max_hp: Vec<i32>,
    mp: Vec<i32>,
    max_mp: Vec<i32>,

    /// Condition masks, split because the engine has no 128-bit integer: low 64 bits then high.
    conditions_low: Vec<i64>,
    conditions_high: Vec<i64>,

    /// Rendered size in percent, and which sprite to draw for something that changes appearance
    /// without changing type.
    sizes: Vec<i32>,
    textures: Vec<i32>,

    /// The skin each player is wearing, as the skin object's own type. Zero for the class's own
    /// sprite, which is what everything that is not a player wears.
    skins: Vec<i32>,

    /// Names, in entity order, empty where there is none.
    names: Vec<GString>,

    /// The eleven stats per entity, laid end to end: entity `n` occupies `n * 11 .. n * 11 + 11`.
    /// Meaningful only for the player's own entity, and zeroes everywhere else.
    stats: Vec<i32>,

    /// What equipment and running boosts add to each of those eleven, laid out the same way.
    ///
    /// The green figures on the character sheet: a total on its own cannot say how much of an
    /// attack is the player and how much is the ring.
    boosts: Vec<i32>,

    /// Each player's guild, in entity order, and their rank in it. Empty and zero for nobody's.
    guilds: Vec<GString>,
    guild_ranks: Vec<i32>,

    stars: Vec<i32>,
    oxygen: Vec<i32>,

    /// Levelling, meaningful only for the player's own entity: the level reached, the experience
    /// earned since that level began, what the level needs before the next, and the fame banked.
    levels: Vec<i32>,
    experience: Vec<i32>,
    experience_goals: Vec<i32>,
    fame: Vec<i32>,

    /// What the account can spend, three per entity laid end to end: entity `n` occupies
    /// `n * 3 .. n * 3 + 3`, in the order gold, fame, prestige. Meaningful only for the player's
    /// own entity.
    purse: Vec<i32>,

    /// The eight container slots per entity, laid end to end: entity `n` occupies `n * 8 .. n * 8 + 8`.
    /// Meaningful only for a bag, a chest or a vault; `-1` everywhere else, and `-1` for an empty
    /// slot, because zero is a real object type.
    contents: Vec<i32>,

    /// The five merchandise stats per entity, laid end to end: entity `n` occupies
    /// `n * 5 .. n * 5 + 5`, in the order item type, price, currency, count, rank requirement.
    ///
    /// The item type is `-1` for everything that is not a vendor, which is what the client reads as
    /// "sells nothing": it draws a merchant as the item rather than as itself, and the panel that
    /// buys opens on the type alone.
    merchandise: Vec<i32>,

    /// The halo colour per entity, as a packed `0xRRGGBB`. Zero is no halo, which is nearly
    /// everybody: it is what `/glow` sets, and the client paints a ring of it around the sprite.
    glow: Vec<i32>,

    /// The rarely-changing booleans, packed one integer per entity: bit zero an administrator, bit
    /// one a character that owns a backpack, bit two an account that chose its own name, bit three
    /// a portal that refuses to be entered.
    marks: Vec<i32>,

    /// The two dyes per entity laid end to end, cloth then accessory: entity `n` occupies
    /// `n * 2 .. n * 2 + 2`. Zero for anybody wearing none, which is nearly everybody.
    dyes: Vec<i32>,

    /// Which neighbours each piece of scenery joins onto, as `ConnectionInfo.Bits`. Zero for
    /// everything that is not a connected wall or a fence.
    connection: Vec<i32>,

    /// What is left of the three boost clocks per entity, in seconds, laid end to end: experience,
    /// loot drop, loot tier. `n * 3 .. n * 3 + 3`, and zeroes for anybody without a boost.
    boost_time: Vec<i32>,

    /// The fame the next class quest asks for, per entity. Zero once every star has been earned,
    /// and for everything that is not a player.
    fame_goal: Vec<i32>,

    /// Bumped whenever the contents change, so the client can skip rebuilding when nothing has.
    revision: u64,
}

/// How many numbers one entity's merchandise takes in [`WorldView::merchandise`].
const MERCHANDISE_FIELDS: usize = 5;

impl WorldView {
    /// Empties the view, for a player who has left the world it described.
    fn clear(&mut self) {
        self.empty();
        self.revision += 1;
    }

    fn empty(&mut self) {
        self.ids.clear();
        self.types.clear();
        self.positions.clear();
        self.hp.clear();
        self.max_hp.clear();
        self.mp.clear();
        self.max_mp.clear();
        self.conditions_low.clear();
        self.conditions_high.clear();
        self.sizes.clear();
        self.textures.clear();
        self.skins.clear();
        self.names.clear();
        self.stats.clear();
        self.boosts.clear();
        self.guilds.clear();
        self.guild_ranks.clear();
        self.stars.clear();
        self.oxygen.clear();
        self.levels.clear();
        self.experience.clear();
        self.experience_goals.clear();
        self.fame.clear();
        self.purse.clear();
        self.contents.clear();
        self.merchandise.clear();
        self.glow.clear();
        self.marks.clear();
        self.dyes.clear();
        self.connection.clear();
        self.boost_time.clear();
        self.fame_goal.clear();
    }

    fn replace_with(&mut self, world: &WorldSnapshot) {
        self.empty();

        for (id, state) in world.iter() {
            self.ids.push(id.0 as i32);
            self.types.push(state.object_type as i32);
            self.positions.push(state.x);
            self.positions.push(state.y);
            self.hp.push(state.hp);
            self.max_hp.push(state.max_hp);
            self.mp.push(state.mp);
            self.max_mp.push(state.max_mp);

            // Split rather than truncated: the effects above bit 63 are real ones, and an engine
            // that cannot hold a 128-bit integer must still be told about them.
            self.conditions_low.push(state.conditions as u64 as i64);
            self.conditions_high
                .push((state.conditions >> 64) as u64 as i64);

            self.sizes.push(state.size as i32);
            self.textures.push(state.texture as i32);
            self.skins.push(state.skin as i32);
            self.names
                .push(state.name.as_deref().map(GString::from).unwrap_or_default());
            self.stats.extend_from_slice(&state.stats);
            self.boosts.extend_from_slice(&state.boosts);
            self.guilds.push(
                state
                    .guild
                    .as_deref()
                    .map(GString::from)
                    .unwrap_or_default(),
            );
            self.guild_ranks.push(state.guild_rank as i32);
            self.stars.push(state.stars as i32);
            self.oxygen.push(state.oxygen as i32);
            self.levels.push(state.level as i32);
            self.experience.push(state.experience);
            self.experience_goals.push(state.experience_goal);
            self.fame.push(state.fame);
            self.purse
                .extend_from_slice(&[state.credits, state.current_fame, state.prestige]);

            match &state.contents {
                Some(slots) => self.contents.extend(slots.iter().map(|item| {
                    if *item == hendra_net::NO_ITEM {
                        -1
                    } else {
                        *item as i32
                    }
                })),
                None => self.contents.extend(std::iter::repeat_n(-1, 8)),
            }

            match &state.merchandise {
                Some(stall) => self.merchandise.extend_from_slice(&[
                    stall.item as i32,
                    stall.price,
                    stall.currency as i32,
                    stall.count,
                    stall.rank as i32,
                ]),

                // An item type of -1 is how the client's `Entity` starts, and what its merchant
                // panel reads as nothing for sale.
                None => self
                    .merchandise
                    .extend_from_slice(&[-1, 0, 0, -1, 0][..MERCHANDISE_FIELDS]),
            }

            self.glow.push(state.glow);
            self.marks.push(
                i32::from(state.admin)
                    | i32::from(state.has_backpack) << 1
                    | i32::from(state.name_chosen) << 2
                    | i32::from(state.portal_unusable) << 3,
            );
            self.dyes.extend_from_slice(&[state.tex1, state.tex2]);
            self.connection.push(state.connection as i32);
            self.boost_time.extend_from_slice(&[
                state.experience_boost_seconds,
                state.loot_drop_boost_seconds,
                state.loot_tier_boost_seconds,
            ]);
            self.fame_goal.push(state.fame_goal);
        }
        self.revision += 1;
    }
}

#[derive(Default)]
struct Shared {
    events: Mutex<VecDeque<Event>>,
    world: Mutex<WorldView>,
    status: AtomicU8,
    /// Round-trip time in milliseconds, as QUIC estimates it.
    rtt_ms: AtomicU8,
}

impl Shared {
    fn push(&self, event: Event) {
        if let Ok(mut events) = self.events.lock() {
            // A game that stops polling should not grow this without bound.
            if events.len() < 512 {
                events.push_back(event);
            }
        }
    }

    fn set_status(&self, status: Status) {
        self.status.store(status as u8, Ordering::Release);
    }
}

/// What Godot asks the worker to do.
enum Command {
    Input { x: f32, y: f32, time_ms: u32 },
    Shoot { angle: f32 },
    MoveItem {
        from: SlotLocation,
        to: SlotLocation,
    },
    Drop {
        from: SlotLocation,
    },
    Chat(String),
    UsePortal(u32),
    UseItem {
        container: u32,
        slot: u16,
        x: f32,
        y: f32,
    },
    VaultMove {
        version: u32,
        from: (i16, i16),
        to: (i16, i16),
    },
    VaultBuy { chest_count: u32 },

    /// Buy what a merchant standing in the world is selling.
    Buy { merchant: u32 },
    Disconnect,
}

/// A connection to a Hendra server.
#[derive(GodotClass)]
#[class(base=Node)]
pub struct HendraConnection {
    base: Base<Node>,
    shared: Arc<Shared>,
    commands: Option<tokio::sync::mpsc::UnboundedSender<Command>>,
    runtime: Option<tokio::runtime::Runtime>,
}

#[godot_api]
impl INode for HendraConnection {
    fn init(base: Base<Node>) -> Self {
        HendraConnection {
            base,
            shared: Arc::new(Shared::default()),
            commands: None,
            runtime: None,
        }
    }
}

#[godot_api]
impl HendraConnection {
    /// The protocol version this build speaks. The server refuses anything else.
    #[func]
    fn protocol_version(&self) -> i64 {
        PROTOCOL_VERSION as i64
    }

    /// Opens a connection and sends the opening message.
    ///
    /// `token` comes from the app server over HTTPS; no password ever reaches this socket.
    /// `allow_any_certificate` exists for local development and turns off server verification.
    /// a released client must leave it false.
    #[func]
    fn connect_to_server(
        &mut self,
        host: GString,
        port: i32,
        token: GString,
        character: i32,
        allow_any_certificate: bool,
    ) -> bool {
        if self.runtime.is_some() {
            godot_warn!("hendra: already connected; disconnect first");
            return false;
        }

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                godot_error!("hendra: could not start the network runtime: {err}");
                return false;
            }
        };

        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let shared = Arc::clone(&self.shared);
        shared.set_status(Status::Connecting);

        let host = host.to_string();
        let token = token.to_string();
        let trust = if allow_any_certificate {
            Trust::AnyCertificate
        } else {
            Trust::Roots
        };

        runtime.spawn(async move {
            run(
                shared,
                receiver,
                host,
                port as u16,
                token,
                character as u32,
                trust,
            )
            .await;
        });

        self.commands = Some(sender);
        self.runtime = Some(runtime);
        true
    }

    /// Closes the connection and stops the worker.
    #[func]
    fn disconnect_from_server(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Command::Disconnect);
        }
        if let Some(runtime) = self.runtime.take() {
            // Background rather than blocking: shutting a runtime down waits for its tasks, and
            // this is called from a frame.
            runtime.shutdown_background();
        }
        self.shared.set_status(Status::Closed);
    }

    /// [`Status`] as an integer, matching the constants below.
    #[func]
    fn status(&self) -> i64 {
        self.shared.status.load(Ordering::Acquire) as i64
    }

    #[func]
    fn is_connected_to_server(&self) -> bool {
        Status::from_code(self.shared.status.load(Ordering::Acquire)) == Status::Connected
    }

    /// Round-trip time in milliseconds, saturating at 255.
    #[func]
    fn rtt_ms(&self) -> i64 {
        self.shared.rtt_ms.load(Ordering::Relaxed) as i64
    }

    /// Drains everything that has happened since the last frame.
    ///
    /// Each entry is a dictionary with a `kind` key. Returning an array rather than emitting
    /// signals keeps the whole frame's work in one place the client controls.
    #[func]
    fn poll(&mut self) -> Array<VarDictionary> {
        let mut out = Array::new();

        let Ok(mut events) = self.shared.events.lock() else {
            return out;
        };

        while let Some(event) = events.pop_front() {
            let mut entry = VarDictionary::new();
            match event {
                Event::Connected => {
                    entry.set("kind", "connected");
                }
                Event::Welcome {
                    player,
                    tick,
                    world,
                    width,
                    height,
                    background,
                    difficulty,
                    allow_teleport,
                    show_displays,
                    music,
                } => {
                    entry.set("kind", "welcome");
                    entry.set("width", width as i64);
                    entry.set("height", height as i64);
                    entry.set("player", player as i64);
                    entry.set("tick", tick as i64);
                    entry.set("world", world);
                    entry.set("background", background as i64);
                    entry.set("difficulty", difficulty as i64);
                    entry.set("allow_teleport", allow_teleport);
                    entry.set("show_displays", show_displays);
                    entry.set("music", music);
                }
                Event::SwitchMusic(music) => {
                    entry.set("kind", "switch_music");
                    entry.set("music", music);
                }
                Event::QuestTarget(target) => {
                    entry.set("kind", "quest_target");
                    entry.set("target", target as i64);
                }
                Event::SetFocus(target) => {
                    entry.set("kind", "set_focus");
                    entry.set("target", target as i64);
                }
                Event::AccountList { list, names } => {
                    entry.set("kind", "account_list");
                    entry.set("list", list as i64);

                    let mut out = PackedStringArray::new();
                    for name in &names {
                        out.push(name);
                    }
                    entry.set("names", &out);
                }
                Event::Rejected(reason) => {
                    entry.set("kind", "rejected");
                    entry.set("reason", reason as i64);
                    entry.set("reason_name", format!("{reason:?}"));
                }
                Event::Chat {
                    speaker,
                    from,
                    text,
                } => {
                    entry.set("kind", "chat");
                    entry.set("speaker", speaker as i64);
                    entry.set("from", from);
                    entry.set("text", text);
                }
                Event::Disconnected(why) => {
                    entry.set("kind", "disconnected");
                    entry.set("reason", why);
                }
                Event::Terrain { x, y, tiles } => {
                    entry.set("kind", "terrain");
                    entry.set("x", x as i64);
                    entry.set("y", y as i64);

                    let row: Vec<i32> = tiles.iter().map(|tile| *tile as i32).collect();
                    entry.set("tiles", &PackedInt32Array::from(row.as_slice()));
                }
                Event::Queued { place, waiting } => {
                    entry.set("kind", "queued");
                    entry.set("place", place as i64);
                    entry.set("waiting", waiting as i64);
                }
                Event::Stacks { health, magic } => {
                    entry.set("kind", "stacks");
                    entry.set("health", health as i64);
                    entry.set("magic", magic as i64);
                }
                Event::Died {
                    character,
                    killed_by,
                    fame,
                } => {
                    entry.set("kind", "died");
                    entry.set("character", character as i64);
                    entry.set("killed_by", killed_by.as_str());
                    entry.set("fame", fame as i64);
                }
                Event::Notice(text) => {
                    entry.set("kind", "notice");
                    entry.set("text", text.as_str());
                }
                Event::Notification(text) => {
                    entry.set("kind", "notification");
                    entry.set("text", text.as_str());
                }
                Event::TradeRequested(name) => {
                    entry.set("kind", "trade_requested");
                    entry.set("name", name.as_str());
                }
                Event::TradeStart {
                    mine,
                    their_name,
                    theirs,
                } => {
                    entry.set("kind", "trade_start");
                    entry.set("their_name", their_name.as_str());
                    entry.set("mine", &slots_of(&mine));
                    entry.set("theirs", &slots_of(&theirs));
                }
                Event::TradeChanged(offer) => {
                    entry.set("kind", "trade_changed");
                    entry.set("offer", &flags_of(&offer));
                }
                Event::TradeAccepted { mine, theirs } => {
                    entry.set("kind", "trade_accepted");
                    entry.set("mine", &flags_of(&mine));
                    entry.set("theirs", &flags_of(&theirs));
                }
                Event::TradeDone { code, message } => {
                    entry.set("kind", "trade_done");
                    entry.set("code", code as i64);
                    entry.set("message", message.as_str());
                }
                Event::Scenery { y, objects } => {
                    entry.set("kind", "scenery");
                    entry.set("y", y as i64);

                    // Parallel arrays, as the ground changes use: one marshalled block per field
                    // beats a dictionary per object.
                    let xs: Vec<i32> = objects.iter().map(|(x, _, _)| *x as i32).collect();
                    let types: Vec<i32> = objects.iter().map(|(_, kind, _)| *kind as i32).collect();
                    let sizes: Vec<i32> = objects.iter().map(|(_, _, size)| *size as i32).collect();

                    entry.set("x", &PackedInt32Array::from(xs.as_slice()));
                    entry.set("objects", &PackedInt32Array::from(types.as_slice()));
                    entry.set("sizes", &PackedInt32Array::from(sizes.as_slice()));
                }
                Event::Ground(changes) => {
                    entry.set("kind", "ground");

                    // Parallel arrays, as the containers and the world view use: one marshalled
                    // block per field beats a dictionary per square.
                    let xs: Vec<i32> = changes.iter().map(|(x, _, _)| *x as i32).collect();
                    let ys: Vec<i32> = changes.iter().map(|(_, y, _)| *y as i32).collect();
                    let tiles: Vec<i32> = changes.iter().map(|(_, _, t)| *t as i32).collect();

                    entry.set("x", &PackedInt32Array::from(xs.as_slice()));
                    entry.set("y", &PackedInt32Array::from(ys.as_slice()));
                    entry.set("tiles", &PackedInt32Array::from(tiles.as_slice()));
                }
                Event::Container { container, slots } => {
                    entry.set("kind", "container");
                    entry.set("container", container as i64);

                    // Parallel arrays for the same reason the world view uses them: one marshalled
                    // block per field beats a dictionary per slot.
                    let indices: Vec<i32> = slots.iter().map(|(slot, _)| *slot as i32).collect();
                    let items: Vec<i32> = slots.iter().map(|(_, item)| *item as i32).collect();
                    entry.set("slots", &PackedInt32Array::from(indices.as_slice()));
                    entry.set("items", &PackedInt32Array::from(items.as_slice()));
                }
                Event::VaultUpdate {
                    version,
                    chest_count,
                    max_chests,
                    next_chest_price,
                    slots,
                    gifts,
                } => {
                    entry.set("kind", "vault");
                    entry.set("version", version as i64);
                    entry.set("chest_count", chest_count as i64);
                    entry.set("max_chests", max_chests as i64);
                    entry.set("next_chest_price", next_chest_price as i64);

                    // Widened to i32 because that is what the engine's packed integer array holds.
                    // The empty sentinel widens with them, so `0xffff` stays `0xffff` rather than
                    // becoming minus one somewhere in the middle.
                    let held: Vec<i32> = slots.iter().map(|item| *item as i32).collect();
                    let waiting: Vec<i32> = gifts.iter().map(|item| *item as i32).collect();
                    entry.set("slots", &PackedInt32Array::from(held.as_slice()));
                    entry.set("gifts", &PackedInt32Array::from(waiting.as_slice()));
                }
                Event::Refused(message) => {
                    entry.set("kind", "refused");
                    entry.set("message", message);
                }
                Event::Shot {
                    projectile,
                    owner,
                    object_type,
                    x,
                    y,
                    angle,
                    speed,
                    lifetime_ms,
                    damage,
                } => {
                    entry.set("kind", "shot");
                    entry.set("projectile", projectile as i64);
                    entry.set("owner", owner as i64);
                    entry.set("object_type", object_type as i64);
                    entry.set("x", x);
                    entry.set("y", y);
                    entry.set("angle", angle);
                    entry.set("speed", speed);
                    entry.set("lifetime_ms", lifetime_ms as i64);
                    entry.set("damage", damage as i64);
                }
                Event::Goto { object_id, x, y } => {
                    entry.set("kind", "goto");
                    entry.set("object_id", object_id as i64);
                    entry.set("x", x);
                    entry.set("y", y);
                }
                Event::ShowEffect {
                    effect,
                    target,
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                } => {
                    entry.set("kind", "show_effect");
                    entry.set("effect", effect as i64);
                    entry.set("target", target as i64);
                    entry.set("x1", x1);
                    entry.set("y1", y1);
                    entry.set("x2", x2);
                    entry.set("y2", y2);
                    entry.set("color", color as i64);
                }
                Event::StatusText {
                    object_id,
                    text,
                    color,
                } => {
                    entry.set("kind", "status_text");
                    entry.set("object_id", object_id as i64);
                    entry.set("text", text.as_str());
                    entry.set("color", color as i64);
                }
                Event::Aoe {
                    x,
                    y,
                    radius,
                    damage,
                    effect,
                    duration,
                    orig_type,
                } => {
                    entry.set("kind", "aoe");
                    entry.set("x", x);
                    entry.set("y", y);
                    entry.set("radius", radius);
                    entry.set("damage", damage as i64);
                    entry.set("effect", effect as i64);
                    entry.set("duration", duration);
                    entry.set("orig_type", orig_type as i64);
                }
                Event::InvitedToGuild { name, guild } => {
                    entry.set("kind", "invited_to_guild");
                    entry.set("name", name.as_str());
                    entry.set("guild", guild.as_str());
                }
                Event::Damage {
                    target,
                    effects,
                    amount,
                    kill,
                    bullet,
                    owner,
                } => {
                    entry.set("kind", "damage");
                    entry.set("target", target as i64);

                    // Sent as the indices rather than the bitfield, which is what the packet the
                    // client already understands carries and what a Godot dictionary can hold: a
                    // hundred-and-twenty-eight-bit mask has no variant to travel in.
                    let mut indices = PackedInt32Array::new();
                    for index in 0..128u32 {
                        if effects & (1u128 << index) != 0 {
                            indices.push(index as i32);
                        }
                    }
                    entry.set("effects", &indices);

                    entry.set("amount", amount as i64);
                    entry.set("kill", kill);
                    entry.set("bullet", bullet as i64);
                    entry.set("owner", owner as i64);
                }
            }
            out.push(&entry);
        }

        out
    }

    /// How many times the world has changed. Unchanged between frames means nothing to rebuild.
    #[func]
    fn world_revision(&self) -> i64 {
        self.shared
            .world
            .lock()
            .map(|world| world.revision as i64)
            .unwrap_or(0)
    }

    #[func]
    fn entity_count(&self) -> i64 {
        self.shared
            .world
            .lock()
            .map(|world| world.ids.len() as i64)
            .unwrap_or(0)
    }

    #[func]
    fn entity_ids(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.ids.as_slice()))
    }

    #[func]
    fn entity_types(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.types.as_slice()))
    }

    /// Positions as interleaved x, y: two entries per entity, in the same order as the ids.
    #[func]
    fn entity_positions(&self) -> PackedFloat32Array {
        self.with_world(|world| PackedFloat32Array::from(world.positions.as_slice()))
    }

    #[func]
    fn entity_hp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.hp.as_slice()))
    }

    #[func]
    fn entity_max_hp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.max_hp.as_slice()))
    }

    #[func]
    fn entity_mp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.mp.as_slice()))
    }

    #[func]
    fn entity_max_mp(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.max_mp.as_slice()))
    }

    /// The low 64 bits of each entity's condition mask.
    ///
    /// Two arrays because the engine has no 128-bit integer and the game uses 51 effects today with
    /// room reserved above them. Truncating to 64 would work until it silently did not.
    #[func]
    fn entity_conditions_low(&self) -> PackedInt64Array {
        self.with_world(|world| PackedInt64Array::from(world.conditions_low.as_slice()))
    }

    #[func]
    fn entity_conditions_high(&self) -> PackedInt64Array {
        self.with_world(|world| PackedInt64Array::from(world.conditions_high.as_slice()))
    }

    /// Rendered size in percent, where 100 is the object's natural size.
    #[func]
    fn entity_sizes(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.sizes.as_slice()))
    }

    /// Which sprite to draw, for entities that change appearance without changing type.
    #[func]
    fn entity_textures(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.textures.as_slice()))
    }

    /// The skin each player is wearing, as the skin object's own type, and zero for no skin.
    ///
    /// A whole different animated sheet rather than a frame within the one the object type names,
    /// which is what `entity_textures` picks.
    #[func]
    fn entity_skins(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.skins.as_slice()))
    }

    /// Names in entity order, empty where an entity has none.
    #[func]
    fn entity_names(&self) -> PackedStringArray {
        self.with_world(|world| {
            let mut names = PackedStringArray::new();
            for name in &world.names {
                names.push(name);
            }
            names
        })
    }

    /// The eleven stats per entity, laid end to end: entity `n` occupies `n * 11 .. n * 11 + 11`.
    ///
    /// The order is the game's own stat numbering, so the last three are `DamageMin`, `DamageMax`
    /// and `Luck`. Only the player's own entity carries meaningful values; everything else is
    /// zeroes.
    #[func]
    fn entity_stats(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.stats.as_slice()))
    }

    /// What equipment and running boosts add to each of the eleven, laid out like the stats.
    ///
    /// The character sheet draws these in green beside the totals, and in red where an item takes a
    /// stat down. Only the player's own entity carries meaningful values.
    #[func]
    fn entity_boosts(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.boosts.as_slice()))
    }

    /// Each player's guild, in entity order, and empty for anybody in none.
    #[func]
    fn entity_guilds(&self) -> PackedStringArray {
        self.with_world(|world| {
            let mut out = PackedStringArray::new();
            for guild in &world.guilds {
                out.push(guild);
            }
            out
        })
    }

    /// Each player's rank within their guild: 0 initiate, 10 member, 20 officer, 40 founder.
    #[func]
    fn entity_guild_ranks(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.guild_ranks.as_slice()))
    }

    #[func]
    fn entity_stars(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.stars.as_slice()))
    }

    /// The eight container slots per entity, laid end to end. `-1` for an empty slot and for
    /// anything that is not a container, since zero is a real object type.
    #[func]
    fn entity_contents(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.contents.as_slice()))
    }

    /// The five merchandise stats per entity, laid end to end: entity `n` occupies
    /// `n * 5 .. n * 5 + 5`, in the order item type, price, currency, count, rank requirement.
    ///
    /// An item type of `-1` means the entity sells nothing, which is almost all of them. Currency
    /// is the game's own `CurrencyType`: zero gold, one fame. A count of `-1` is stock that never
    /// runs out.
    #[func]
    fn entity_merchandise(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.merchandise.as_slice()))
    }

    /// The halo colour per entity, as a packed `0xRRGGBB`. Zero is no halo.
    ///
    /// `StatsType.Glow` (`Player.cs:302`), which the original's client hands to
    /// `GameObject.setGlow` and paints as a ring around the sprite (`GlowRedrawer.as:19-46`).
    #[func]
    fn entity_glow(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.glow.as_slice()))
    }

    /// The marks a player carries, one integer per entity: bit zero an administrator, bit one a
    /// character that owns a backpack.
    ///
    /// `StatsType.Admin` and `StatsType.HasBackpack` (`Player.cs:355`, `:359`). Packed rather than
    /// given an array each because they are two bits that never move while somebody is playing.
    #[func]
    fn entity_marks(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.marks.as_slice()))
    }

    /// The two dyes per entity, cloth then accessory, laid end to end.
    ///
    /// `StatsType.Texture1`/`Texture2` (`Player.cs:301-302`). The top byte is a type and the low
    /// twenty-four its argument: `1` a solid `0xRRGGBB`, `4`/`5`/`9`/`10` an index into the
    /// `textile{n}x{n}` sheet (`TextureRedrawer.as:161-186`).
    #[func]
    fn entity_dyes(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.dyes.as_slice()))
    }

    /// Which neighbours each piece of scenery joins onto, as `ConnectionInfo.Bits`.
    ///
    /// `StatsType.ObjectConnection` (`ConnectedObject.cs:114-118`), four bytes of `1` or `2`. Zero
    /// for everything that is not a connected wall or a fence.
    #[func]
    fn entity_connection(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.connection.as_slice()))
    }

    /// What is left of the three boost clocks per entity, in seconds: experience, loot drop, loot
    /// tier, laid end to end.
    ///
    /// `XPBoostTime`, `LDBoostTime`, `LTBoostTime` (`Player.cs:356-358`).
    #[func]
    fn entity_boost_time(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.boost_time.as_slice()))
    }

    /// The fame the next class quest asks for, per entity. Zero once every star has been earned.
    #[func]
    fn entity_fame_goal(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.fame_goal.as_slice()))
    }

    /// What the account can spend, three per entity laid end to end: gold, fame, prestige.
    ///
    /// The fame here is the account's, which is what shops charge against; what the character has
    /// earned is in `entity_fame`.
    #[func]
    fn entity_purse(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.purse.as_slice()))
    }

    /// Air remaining, from 100 down to 0. Full everywhere but a drowning world.
    #[func]
    fn entity_oxygen(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.oxygen.as_slice()))
    }

    /// The level reached. Zero for anything that is not a player.
    #[func]
    fn entity_levels(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.levels.as_slice()))
    }

    /// Experience earned since the current level began, which is what the bar fills with.
    #[func]
    fn entity_experience(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.experience.as_slice()))
    }

    /// Experience the current level needs before the next, the bar's ceiling.
    #[func]
    fn entity_experience_goals(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.experience_goals.as_slice()))
    }

    /// Fame banked by this character, its lifetime experience divided by a thousand.
    #[func]
    fn entity_fame(&self) -> PackedInt32Array {
        self.with_world(|world| PackedInt32Array::from(world.fame.as_slice()))
    }

    /// Reports where the player believes it is, and acknowledges the newest snapshot held.
    #[func]
    fn send_input(&self, x: f32, y: f32, time_ms: i64) {
        self.send(Command::Input {
            x,
            y,
            time_ms: time_ms.max(0) as u32,
        });
    }

    /// Asks to move an item between two slots of the player's own pack.
    ///
    /// Both slots are flat numbers, as the game counts them: worn from nought with room for eight,
    /// carried from eight, the backpack from sixteen. [`hendra_net::slot`] turns each into the
    /// container and the index within it that the wire names, so a drag lands on the square the
    /// player dragged to and a worn slot is sent as a worn slot.
    ///
    /// The player's own containers are named by tag rather than by entity, so a client cannot
    /// address someone else's pack. Anything in a bag on the ground goes through
    /// [`Self::take_from_bag`] or [`Self::put_in_bag`], which name the bag.
    #[func]
    fn move_item(&self, from_slot: i64, to_slot: i64) {
        let (Some(from), Some(to)) = (flat_slot(from_slot), flat_slot(to_slot)) else {
            return;
        };

        self.send(Command::MoveItem { from, to });
    }

    /// Takes an item out of a bag on the ground and into one of the player's own slots.
    #[func]
    fn take_from_bag(&self, bag: i64, bag_slot: i64, into_slot: i64) {
        let Some(to) = flat_slot(into_slot) else {
            return;
        };

        self.send(Command::MoveItem {
            from: SlotLocation::Bag {
                entity: EntityId(bag.max(0) as u32),
                slot: bag_slot.clamp(0, u8::MAX as i64) as u8,
            },
            to,
        });
    }

    /// Puts one of the player's own items into a bag on the ground.
    ///
    /// The bag's own slot is named for the sake of saying where it was aimed; the server puts it
    /// wherever the bag has room, since a bag is shared and what is free in it is not the client's
    /// to know.
    #[func]
    fn put_in_bag(&self, from_slot: i64, bag: i64, bag_slot: i64) {
        let Some(from) = flat_slot(from_slot) else {
            return;
        };

        self.send(Command::MoveItem {
            from,
            to: SlotLocation::Bag {
                entity: EntityId(bag.max(0) as u32),
                slot: bag_slot.clamp(0, u8::MAX as i64) as u8,
            },
        });
    }

    /// Moves an item within the vault, or between the vault and the player's own inventory.
    ///
    /// Both ends are a chest and a slot, because the operation is a swap and is symmetric. Minus
    /// one is the player's own pack, minus two the gifts waiting, minus three a potion stack. The
    /// version is the vault as this client last saw it: the server refuses a move quoting a version
    /// it has moved past and answers with the truth.
    #[func]
    fn vault_move(
        &self,
        version: i64,
        from_chest: i64,
        from_slot: i64,
        to_chest: i64,
        to_slot: i64,
    ) {
        self.send(Command::VaultMove {
            version: version.max(0) as u32,
            from: (from_chest as i16, from_slot as i16),
            to: (to_chest as i16, to_slot as i16),
        });
    }

    /// Buys one more vault chest.
    ///
    /// Carries only the count this client believes it owns, so a second click arriving while the
    /// first is still being paid for buys nothing. The price and the purse are the server's.
    #[func]
    fn vault_buy(&self, chest_count: i64) {
        self.send(Command::VaultBuy {
            chest_count: chest_count.max(0) as u32,
        });
    }

    /// Buys what the merchant standing in front of the player is selling.
    ///
    /// Names the merchant and no more: what it sells and what it costs are the server's to know,
    /// and a client that named the item is a client that could name a cheaper one.
    #[func]
    fn buy(&self, merchant: i64) {
        self.send(Command::Buy {
            merchant: merchant.max(0) as u32,
        });
    }

    /// Drops what is in one of the player's own slots at their feet.
    ///
    /// The slot is a flat number, as a move's is.
    #[func]
    fn drop_item(&self, slot: i64) {
        let Some(from) = flat_slot(slot) else {
            return;
        };

        self.send(Command::Drop { from });
    }

    /// Fires in the given direction, in radians.
    ///
    /// Only the aim is sent. Whether the weapon is ready, where the shot travels and what it hits
    /// are all the server's, and there is no way for a client to report a hit at all.
    #[func]
    fn shoot(&self, angle: f32) {
        self.send(Command::Shoot { angle });
    }

    #[func]
    fn send_chat(&self, text: GString) {
        self.send(Command::Chat(text.to_string()));
    }

    #[func]
    fn use_portal(&self, entity: i64) {
        self.send(Command::UsePortal(entity.max(0) as u32));
    }

    /// Uses what is in a slot of some container, aimed at a point in the world.
    ///
    /// The slot rather than the item, because the server knows what is in a slot and a client
    /// naming an item it does not hold would be a claim rather than a fact. Where it is aimed is
    /// the one thing taken as given, as it is for a shot.
    ///
    /// A flat number, sent as it stands: the server turns it into the slot it holds, through the
    /// same numbering a move goes through. Two hundred and fifty-four and five are the potion
    /// stacks, which are not slots in the pack and are passed on as they are.
    ///
    /// The container is the entity whose slot is meant, which the original names alongside the slot
    /// (`UseItemHandler.cs:22`). Zero, and the player's own id, both mean their own slots; anything
    /// else is a bag or a chest standing in the world, and naming one is how a potion is drunk
    /// straight out of it.
    #[func]
    fn use_item(&self, container: i64, slot: i64, x: f32, y: f32) {
        self.send(Command::UseItem {
            container: container.max(0) as u32,
            slot: slot.max(0) as u16,
            x,
            y,
        });
    }

    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    fn with_world<T: Default>(&self, read: impl FnOnce(&WorldView) -> T) -> T {
        self.shared
            .world
            .lock()
            .map(|world| read(&world))
            .unwrap_or_default()
    }
}

/// The slot a flat number names, or nothing if it names no slot at all.
///
/// The game counts one run of squares and the wire names a container and an index within it, so
/// every slot the game hands over passes through here. Nothing is sent for a number that is not a
/// square — the four the layout leaves room for and this game does not have, or a stack, which is
/// not a slot and has no move of its own — rather than a request the server would have to refuse.
fn flat_slot(flat: i64) -> Option<SlotLocation> {
    u16::try_from(flat).ok().and_then(hendra_net::slot::located)
}

/// The worker: owns the connection, decodes everything, publishes the result.
async fn run(
    shared: Arc<Shared>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    host: String,
    port: u16,
    token: String,
    character: u32,
    trust: Trust,
) {
    let address = match resolve(&host, port) {
        Some(address) => address,
        None => {
            shared.set_status(Status::Failed);
            shared.push(Event::Disconnected(format!("cannot resolve {host}:{port}")));
            return;
        }
    };

    let mut link = match connect(address, &host, trust).await {
        Ok(link) => link,
        Err(err) => {
            shared.set_status(Status::Failed);
            shared.push(Event::Disconnected(err.to_string()));
            return;
        }
    };

    shared.set_status(Status::Connected);
    shared.push(Event::Connected);

    // The opening message. Reliable, because a lost hello is a connection that never starts.
    let mut scratch = Vec::new();
    ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        token: &token,
        character,
    }
    .encode(&mut Writer::new(&mut scratch));

    if let Err(err) = link.send(Delivery::Stream, &scratch).await {
        shared.set_status(Status::Failed);
        shared.push(Event::Disconnected(err.to_string()));
        return;
    }

    // What the client holds, so a snapshot naming a baseline can be resolved against the right one.
    let mut history: BaselineRing<WorldSnapshot> = BaselineRing::new();
    let mut newest: Option<Tick> = None;

    // What a pong reports. The AS3 client answers with `getTimer()`, its milliseconds since boot
    // (`messaging/impl/GameServerConnectionConcrete.as:1727-1731`). This is read here rather than on
    // the game thread deliberately: a client whose rendering has stalled is exactly the one the
    // server is asking about, and an answer that had to wait for a frame would make a slow frame
    // look like a disconnect.
    let clock = std::time::Instant::now();

    // How far ahead the game's clock reads, which is the time it spent booting before this
    // connection was made. Learned once from an input, and applied to every pong so that the two
    // client timestamps on the wire are readings of one clock rather than two. The original has no
    // choice about this -- `Move.time` and `Pong.time` are both `getTimer()` off the same stopwatch
    // -- and a server that measured its offset against one and applied it to the other would be out
    // by however long the client took to start. A pong sent before the first input reports this
    // connection's own clock instead, which is one noisy sample at the head of an average that runs
    // for the whole session.
    let mut booted_ms: u32 = 0;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };

                if let Command::Input { time_ms, .. } = &command {
                    booted_ms = time_ms.wrapping_sub(clock.elapsed().as_millis() as u32);
                }

                let now_ms = (clock.elapsed().as_millis() as u32).wrapping_add(booted_ms);

                if !handle(&mut link, command, newest, &mut scratch, now_ms).await {
                    break;
                }
            }

            received = link.recv() => {
                let Some(received) = received else {
                    shared.push(Event::Disconnected("the server closed the connection".into()));
                    break;
                };

                shared.rtt_ms.store(
                    link.rtt().as_millis().min(255) as u8,
                    Ordering::Relaxed,
                );

                // A ping is answered before anything else is done with the message, and answered
                // straight away. The server drops a connection that has not answered one for twelve
                // seconds, so this reply is what keeps the session alive.
                if let Some(serial) = apply(&shared, received, &mut history, &mut newest) {
                    scratch.clear();
                    ClientMessage::Pong {
                        serial,
                        client_time_ms: (clock.elapsed().as_millis() as u32)
                            .wrapping_add(booted_ms),
                    }
                    .encode(&mut Writer::new(&mut scratch));

                    if link.send(Delivery::Stream, &scratch).await.is_err() {
                        shared.push(Event::Disconnected("the connection dropped".into()));
                        break;
                    }
                }
            }
        }
    }

    shared.set_status(Status::Closed);
    link.close("client disconnecting");
}

/// Sends one command. Returns false when the connection should end.
async fn handle(
    link: &mut hendra_transport::Link,
    command: Command,
    newest: Option<Tick>,
    scratch: &mut Vec<u8>,
    now_ms: u32,
) -> bool {
    scratch.clear();
    let ack = match newest {
        Some(tick) => Acknowledgement::of(tick),
        None => Acknowledgement::NONE,
    };

    let (delivery, message) = match command {
        Command::Disconnect => return false,
        Command::Input { x, y, time_ms } => (
            // Input is worthless once it is late, so it never waits for a retransmission.
            Delivery::Datagram,
            ClientMessage::Input(Input {
                ack,
                client_time_ms: time_ms,
                x,
                y,
            }),
        ),
        Command::Shoot { angle } => {
            let mut buf = Vec::new();
            ClientMessage::Shoot {
                angle,
                // The original's `PlayerShoot.Time` is `gs_.lastUpdate_`, the same client clock its
                // moves and its pongs are stamped with
                // (`messaging/impl/GameServerConnectionConcrete.as:757`), and the server's shot
                // validation reads it as one: `ValidatePlayerShoot` measures the cooldown from the
                // last shot's client time and feeds `TimeCop` the pair
                // (`realm/entities/player/Player.AntiCheat.cs:86-108`).
                client_time_ms: now_ms,
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::MoveItem { from, to } => {
            let mut buf = Vec::new();
            ClientMessage::MoveItem { from, to }.encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::Drop { from } => {
            let mut buf = Vec::new();
            ClientMessage::MoveItem {
                from,
                to: SlotLocation::Ground,
            }
            .encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }

        Command::VaultMove { version, from, to } => (
            Delivery::Stream,
            ClientMessage::VaultMove {
                version,
                from_chest: from.0,
                from_slot: from.1,
                to_chest: to.0,
                to_slot: to.1,
            },
        ),

        Command::Buy { merchant } => (
            Delivery::Stream,
            ClientMessage::Buy {
                merchant: EntityId(merchant),
            },
        ),
        Command::VaultBuy { chest_count } => {
            (Delivery::Stream, ClientMessage::VaultBuy { chest_count })
        }

        Command::Chat(text) => {
            let mut buf = Vec::new();
            ClientMessage::Chat { text: &text }.encode(&mut Writer::new(&mut buf));
            return link.send(Delivery::Stream, &buf).await.is_ok();
        }
        Command::UsePortal(entity) => (
            Delivery::Stream,
            ClientMessage::UsePortal {
                entity: EntityId(entity),
            },
        ),
        Command::UseItem {
            container,
            slot,
            x,
            y,
        } => (
            Delivery::Stream,
            ClientMessage::UseItem {
                container: hendra_net::EntityId(container),
                slot,
                x,
                y,
            },
        ),
    };

    message.encode(&mut Writer::new(scratch));
    link.send(delivery, scratch).await.is_ok()
}

/// Decodes one arriving payload and publishes whatever it means.
/// Applies one message from the server, and returns the serial of a ping that wants answering.
///
/// The answer is left to the caller because only it holds the link. Everything else here changes
/// what the game sees on its next poll and sends nothing.
fn apply(
    shared: &Arc<Shared>,
    received: Received,
    history: &mut BaselineRing<WorldSnapshot>,
    newest: &mut Option<Tick>,
) -> Option<u32> {
    let payload = received.into_payload();
    let mut reader = Reader::new(&payload);

    let message = match ServerMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, "undecodable message from the server");
            return None;
        }
    };

    match message {
        ServerMessage::Welcome {
            player,
            tick,
            world,
            width,
            height,
            background,
            difficulty,
            allow_teleport,
            show_displays,
            music,
        } => {
            // A second welcome means a different world, and a different world means everything
            // held about the last one is void. Its snapshots were measured against a history that
            // no longer applies, and its ticks started again from zero, so without this every
            // snapshot from the new world reads as older than what is already held and is
            // discarded. The symptom is a player who arrives somewhere and never appears.
            history.clear();
            *newest = None;
            if let Ok(mut view) = shared.world.lock() {
                view.clear();
            }

            shared.push(Event::Welcome {
                player: player.0,
                tick: tick.0,
                world: world.to_owned(),
                width,
                height,
                background,
                difficulty,
                allow_teleport,
                show_displays,
                music: music.to_owned(),
            });
        }

        ServerMessage::Rejected { reason } => shared.push(Event::Rejected(reason)),

        ServerMessage::SwitchMusic { music } => {
            shared.push(Event::SwitchMusic(music.to_owned()))
        }

        ServerMessage::QuestTarget { target } => shared.push(Event::QuestTarget(target.0)),

        ServerMessage::SetFocus { target } => shared.push(Event::SetFocus(target.0)),

        ServerMessage::AccountList { list, names } => shared.push(Event::AccountList {
            list: list.number() as u8,
            names,
        }),

        ServerMessage::Chat {
            speaker,
            from,
            text,
        } => shared.push(Event::Chat {
            speaker: speaker.0,
            from: from.to_owned(),
            text: text.to_owned(),
        }),

        ServerMessage::Ping { serial } => return Some(serial),

        ServerMessage::Container { container, slots } => shared.push(Event::Container {
            container: container as u8,
            slots,
        }),

        ServerMessage::VaultUpdate {
            version,
            chest_count,
            max_chests,
            next_chest_price,
            slots,
            gifts,
        } => shared.push(Event::VaultUpdate {
            version,
            chest_count,
            max_chests,
            next_chest_price,
            slots,
            gifts,
        }),

        ServerMessage::Refused { message } => shared.push(Event::Refused(message.to_owned())),

        ServerMessage::Ground { changes } => shared.push(Event::Ground(changes)),

        ServerMessage::Scenery { y, objects } => shared.push(Event::Scenery { y, objects }),

        ServerMessage::Stacks { health, magic } => shared.push(Event::Stacks { health, magic }),
        ServerMessage::Queued { place, waiting } => shared.push(Event::Queued { place, waiting }),
        ServerMessage::Notice { text } => shared.push(Event::Notice(text)),
        ServerMessage::Notification { text } => shared.push(Event::Notification(text)),
        ServerMessage::Died {
            character,
            killed_by,
            fame,
        } => shared.push(Event::Died {
            character,
            killed_by,
            fame,
        }),
        ServerMessage::TradeRequested { name } => shared.push(Event::TradeRequested(name)),
        ServerMessage::TradeStart {
            mine,
            their_name,
            theirs,
        } => shared.push(Event::TradeStart {
            mine,
            their_name,
            theirs,
        }),
        ServerMessage::TradeChanged { offer } => shared.push(Event::TradeChanged(offer)),
        ServerMessage::TradeAccepted { mine, theirs } => {
            shared.push(Event::TradeAccepted { mine, theirs })
        }
        ServerMessage::TradeDone { code, message } => {
            shared.push(Event::TradeDone { code, message })
        }

        ServerMessage::Terrain { x, y, runs } => {
            // Expanded here rather than in the game, so a script sees a row of squares rather than
            // an encoding it has to understand.
            let mut tiles = Vec::new();
            for (count, tile) in runs {
                tiles.extend(std::iter::repeat_n(tile, count as usize));
            }
            shared.push(Event::Terrain { x, y, tiles });
        }

        ServerMessage::Shot {
            projectile,
            owner,
            object_type,
            x,
            y,
            angle,
            speed,
            lifetime_ms,
            damage,
        } => shared.push(Event::Shot {
            projectile: projectile.0,
            owner: owner.0,
            object_type,
            x,
            y,
            angle,
            speed,
            lifetime_ms,
            damage,
        }),

        ServerMessage::Goto { object_id, x, y } => shared.push(Event::Goto {
            object_id: object_id.0,
            x,
            y,
        }),

        ServerMessage::ShowEffect {
            effect,
            target,
            x1,
            y1,
            x2,
            y2,
            color,
        } => shared.push(Event::ShowEffect {
            effect,
            target: target.0,
            x1,
            y1,
            x2,
            y2,
            color,
        }),

        ServerMessage::StatusText {
            object_id,
            text,
            color,
        } => shared.push(Event::StatusText {
            object_id: object_id.0,
            text,
            color,
        }),

        ServerMessage::Aoe {
            x,
            y,
            radius,
            damage,
            effect,
            duration,
            orig_type,
        } => shared.push(Event::Aoe {
            x,
            y,
            radius,
            damage,
            effect,
            duration,
            orig_type,
        }),

        ServerMessage::InvitedToGuild { name, guild } => {
            shared.push(Event::InvitedToGuild { name, guild })
        }

        ServerMessage::Damage {
            target,
            effects,
            amount,
            kill,
            bullet,
            owner,
        } => shared.push(Event::Damage {
            target: target.0,
            effects,
            amount,
            kill,
            bullet,
            owner: owner.0,
        }),

        ServerMessage::Snapshot { body } => {
            let mut body = Reader::new(body);
            let Ok(header) = read_header(&mut body) else {
                return None;
            };

            // Stale on arrival: datagrams reorder, and an older snapshot must not rewind the world.
            if newest.is_some_and(|held| !header.tick.is_newer_than(held)) {
                return None;
            }

            let world = {
                let baseline = match header.baseline {
                    Some(tick) => match history.get(tick) {
                        Some(world) => Some(world),
                        // Encoded against something that never arrived. The server notices when the
                        // acknowledgement stops advancing and sends a full snapshot.
                        None => return None,
                    },
                    None => None,
                };

                match decode_body(header, baseline, &mut body) {
                    Ok(world) => world,
                    Err(err) => {
                        tracing::warn!(%err, "undecodable snapshot");
                        return None;
                    }
                }
            };

            if let Ok(mut view) = shared.world.lock() {
                view.replace_with(&world);
            }

            history.store(header.tick, world);
            *newest = Some(header.tick);
        }
    }

    None
}

/// Resolves a host and port, preferring IPv4 when both are offered.
/// One trade side's slots, as parallel arrays: one marshalled block per field beats a dictionary
/// per slot, which is how the containers and the world view are handed over too.
fn slots_of(slots: &[hendra_net::TradeSlot]) -> VarDictionary {
    let items: Vec<i32> = slots
        .iter()
        .map(|slot| slot.item.map_or(-1, |item| item as i32))
        .collect();
    let kinds: Vec<i32> = slots.iter().map(|slot| slot.slot_type).collect();
    let included: Vec<i32> = slots.iter().map(|slot| i32::from(slot.included)).collect();
    let tradeable: Vec<i32> = slots.iter().map(|slot| i32::from(slot.tradeable)).collect();

    let mut held = VarDictionary::new();
    held.set("items", &PackedInt32Array::from(items.as_slice()));
    held.set("slot_types", &PackedInt32Array::from(kinds.as_slice()));
    held.set("included", &PackedInt32Array::from(included.as_slice()));
    held.set("tradeable", &PackedInt32Array::from(tradeable.as_slice()));
    held
}

/// An offer, as one flag per slot.
fn flags_of(offer: &[bool]) -> PackedInt32Array {
    let flags: Vec<i32> = offer.iter().map(|on| i32::from(*on)).collect();
    PackedInt32Array::from(flags.as_slice())
}

fn resolve(host: &str, port: u16) -> Option<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;

    let candidates: Vec<std::net::SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    candidates
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| candidates.first())
        .copied()
}
