//! The message set, and the id that tells the two directions apart.
//!
//! Messages decode by borrowing from the receive buffer rather than copying out of it. A `Hello`
//! hands back a `&str` pointing into the bytes that arrived, and the session layer copies only the
//! parts it keeps. For `Input`, which arrives twenty times a second per player, there is nothing to
//! copy at all.
//!
//! # Ids
//!
//! Client and server messages occupy separate numeric ranges. Nothing requires this, since the
//! reader always knows which direction it is decoding, but it means a message delivered to the wrong
//! handler fails immediately and obviously, instead of decoding as whatever unrelated message
//! shares its number.

use crate::codec::{CodecError, Reader, Writer};
use crate::entity::EntityId;
use crate::snapshot::{Acknowledgement, Tick};

/// The protocol revision. Both ends refuse a mismatch rather than guessing.
///
/// The legacy server had no version on the wire at all, so an outdated client failed by decoding
/// nonsense several packets later, far from the cause.
pub const PROTOCOL_VERSION: u32 = 6;

/// Messages travelling from client to server.
pub mod client_id {
    pub const HELLO: u16 = 0x0001;
    pub const INPUT: u16 = 0x0002;
    pub const CHAT: u16 = 0x0003;
    pub const USE_PORTAL: u16 = 0x0004;
    pub const USE_ITEM: u16 = 0x0009;
    pub const PONG: u16 = 0x0005;
    pub const SHOOT: u16 = 0x0006;
    pub const MOVE_ITEM: u16 = 0x0007;
    pub const REQUEST_TRADE: u16 = 0x000a;
    pub const CHANGE_TRADE: u16 = 0x000b;
    pub const ACCEPT_TRADE: u16 = 0x000c;
    pub const CANCEL_TRADE: u16 = 0x000d;
    pub const ESCAPE: u16 = 0x000e;
    pub const TELEPORT: u16 = 0x000f;
    pub const BUY: u16 = 0x0010;
    pub const GUILD: u16 = 0x0011;
    pub const MARKET: u16 = 0x0012;
    pub const EDIT_LIST: u16 = 0x0013;
    pub const PRESTIGE: u16 = 0x0014;
    pub const PRESTIGE_BUY: u16 = 0x0015;
    pub const VAULT_MOVE: u16 = 0x0016;
    pub const VAULT_BUY: u16 = 0x0017;
}

/// Messages travelling from server to client.
/// The most runs one terrain strip may claim.
///
/// A length prefix is attacker-controlled, so the capacity is bounded before anything is reserved.
pub const MAX_TERRAIN_RUNS: usize = 4096;

/// The most scenery one message may claim.
///
/// One message is one row of a map, and no row holds more objects than it has squares. A length
/// prefix is attacker-controlled, so the capacity is bounded before anything is reserved.
pub const MAX_SCENERY: usize = 4096;

/// The most ground changes one message may claim.
///
/// A length prefix is attacker-controlled, so the capacity is bounded before anything is reserved.
pub const MAX_GROUND_CHANGES: usize = 8192;

pub mod server_id {
    pub const WELCOME: u16 = 0x8001;
    pub const REJECTED: u16 = 0x8002;
    pub const SNAPSHOT: u16 = 0x8003;
    pub const CHAT: u16 = 0x8004;
    pub const PING: u16 = 0x8005;
    pub const SHOT: u16 = 0x8006;
    pub const CONTAINER: u16 = 0x8007;
    pub const REFUSED: u16 = 0x8008;
    pub const GROUND: u16 = 0x8009;
    pub const TERRAIN: u16 = 0x800a;
    pub const SCENERY: u16 = 0x800b;
    pub const TRADE_REQUESTED: u16 = 0x800c;
    pub const TRADE_START: u16 = 0x800d;
    pub const TRADE_CHANGED: u16 = 0x800e;
    pub const TRADE_ACCEPTED: u16 = 0x800f;
    pub const TRADE_DONE: u16 = 0x8010;
    pub const NOTICE: u16 = 0x8011;
    pub const DIED: u16 = 0x8012;
    pub const STACKS: u16 = 0x8013;
    pub const QUEUED: u16 = 0x8014;
    pub const DAMAGE: u16 = 0x8015;
    pub const VAULT_UPDATE: u16 = 0x8016;
    pub const GOTO: u16 = 0x8017;
    pub const SHOW_EFFECT: u16 = 0x8018;
    pub const NOTIFICATION: u16 = 0x8019;
    pub const STATUS_TEXT: u16 = 0x801a;
    pub const AOE: u16 = 0x801b;
    pub const INVITED_TO_GUILD: u16 = 0x801c;
    pub const SWITCH_MUSIC: u16 = 0x8020;
    pub const QUEST_TARGET: u16 = 0x8021;
    pub const SET_FOCUS: u16 = 0x8022;
    pub const ACCOUNT_LIST: u16 = 0x8023;
}

/// The kinds of thing [`ServerMessage::ShowEffect`] can ask for.
///
/// `EffectType` (`Structures.cs:133-151`), by its numbers. Only the ones the server names are here;
/// the client draws nineteen, and the three above sixteen have never had a server-side name in any
/// version of this game.
///
/// The names are the server's own, which disagree with the client's for eleven of the sixteen while
/// the numbers agree throughout: what the server calls `Trap` the client draws as a ring, and what
/// it calls `Earthquake` the client implements as camera jitter. The numbers are the contract.
pub mod effect {
    /// Motes circling a body and rising. The client calls it a heal.
    pub const POTION: u8 = 1;

    /// A column of blue motes where somebody arrived or left. The colour is ignored: the client
    /// hard-codes blue (`TeleportEffect.as`).
    pub const TELEPORT: u8 = 2;

    /// A lobbed ball arcing from the thrower to `pos1`, trailing sparks for its 1.5 seconds —
    /// which is exactly how long every telegraph in the original waits before it lands.
    pub const THROW: u8 = 4;

    /// A starburst expanding to `pos1.x` tiles. The client calls it a nova.
    pub const AREA_BLAST: u8 = 5;

    /// A momentary beam of static sparks from the target to `pos1`. The client calls it a line.
    pub const TRAIL: u8 = 7;

    /// A burst filling the circle from `pos1` out to `pos2`, which is how its radius is given. The
    /// client calls it a burst. What a vampire blast paints over the ground it drained
    /// (`Player.UseItem.cs:875-881`).
    pub const DIFFUSE: u8 = 8;

    /// A stream of motes drawn from `pos1` into the target, which is the direction that matters:
    /// this is what shows health being pulled out of a monster and into a player
    /// (`Player.UseItem.cs:911-917`).
    pub const FLOW: u8 = 9;

    /// A jagged bolt of sparks between the target and `pos1`, `pos2.x` units thick.
    ///
    /// The particles nearest the target live ten times longer than the ones at `pos1`, so the bolt
    /// appears to retract towards whatever it is anchored to.
    pub const LIGHTNING: u8 = 11;

    /// A ring drawn inward from `pos2` onto `pos1`, its radius the distance between the two. The
    /// client calls it a collapse. What a stasis blast telegraphs itself with before it freezes
    /// anything (`Player.UseItem.cs:802-810`).
    pub const CONCENTRATE: u8 = 12;

    /// Shakes the camera. Names no target and carries no colour or position.
    pub const EARTHQUAKE: u8 = 14;
}

/// Why a connection was refused.
///
/// A closed set rather than a free-text string: the client shows a different screen for each of
/// these, and a typo in a message should not be able to change which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RejectReason {
    /// The client speaks a different [`PROTOCOL_VERSION`].
    VersionMismatch = 0,

    /// The session token was absent, malformed, or expired.
    BadToken = 1,

    /// The account is already playing this character elsewhere.
    AlreadyPlaying = 2,

    /// The server has no room.
    Full = 3,

    /// The account is banned.
    Banned = 4,

    /// The character id does not belong to this account, or does not exist.
    NoSuchCharacter = 5,
}

impl RejectReason {
    pub fn from_code(code: u8) -> Option<RejectReason> {
        use RejectReason::*;
        Some(match code {
            0 => VersionMismatch,
            1 => BadToken,
            2 => AlreadyPlaying,
            3 => Full,
            4 => Banned,
            5 => NoSuchCharacter,
            _ => return None,
        })
    }
}

/// Which container a slot belongs to.
///
/// Sent as a small tag rather than an entity id for the player's own containers, because a client
/// naming its own inventory by entity id could name someone else's. Only a bag has to be addressed
/// by entity, since bags belong to the world rather than to a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotLocation {
    /// The player's own backpack.
    Inventory { slot: u8 },

    /// What the player is wearing.
    Equipment { slot: u8 },

    /// The account's vault.
    Vault { slot: u16 },

    /// The account's gift chest.
    ///
    /// Its own place rather than a bag, because what is in it is durable: taking a gift has to
    /// remove the row, and a bag that forgot to would hand the same gift out on every visit.
    Gift { slot: u16 },

    /// A bag or chest in the world.
    Bag { entity: EntityId, slot: u8 },

    /// The ground at the player's feet.
    ///
    /// Distinct from a bag rather than a bag with a reserved id, because "drop this" and "put this
    /// in that bag" are different requests and a magic entity number would make them look like the
    /// same one.
    Ground,
}

impl SlotLocation {
    fn tag(&self) -> u8 {
        match self {
            SlotLocation::Inventory { .. } => 0,
            SlotLocation::Equipment { .. } => 1,
            SlotLocation::Vault { .. } => 2,
            SlotLocation::Bag { .. } => 3,
            SlotLocation::Ground => 4,
            SlotLocation::Gift { .. } => 5,
        }
    }

    fn encode(&self, w: &mut Writer<'_>) {
        w.u8(self.tag());
        match self {
            SlotLocation::Inventory { slot } | SlotLocation::Equipment { slot } => w.u8(*slot),
            SlotLocation::Vault { slot } | SlotLocation::Gift { slot } => w.varint(*slot as u64),
            SlotLocation::Bag { entity, slot } => {
                w.varint(entity.0 as u64);
                w.u8(*slot);
            }
            SlotLocation::Ground => {}
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<SlotLocation, CodecError> {
        let tag = r.u8()?;
        Ok(match tag {
            0 => SlotLocation::Inventory { slot: r.u8()? },
            1 => SlotLocation::Equipment { slot: r.u8()? },
            2 => SlotLocation::Vault {
                slot: r.varint_u32()? as u16,
            },
            3 => SlotLocation::Bag {
                entity: EntityId(r.varint_u32()?),
                slot: r.u8()?,
            },
            4 => SlotLocation::Ground,
            5 => SlotLocation::Gift {
                slot: r.varint_u32()? as u16,
            },
            other => {
                return Err(CodecError::InvalidValue {
                    what: "slot location",
                    value: other as u64,
                });
            }
        })
    }
}

/// One end of a vault move, when it is not a chest.
///
/// The vault panel addresses everything as a chest and a slot, because the operation is a swap and
/// a swap is symmetric. Three chest numbers are not chests, and they are negative so that no real
/// chest can ever collide with one.
///
/// Ours. The original names both ends of a vault swap by object id and slot, since the chest, the
/// gift chest and the player are all entities standing in one room
/// (`networking/handlers/InvSwapHandler.cs:29-33`); the three cases below are the same three ends it
/// distinguishes — `b == player` with a stack slot (`:56-58`), a `GiftChest` source (`:116-119`),
/// and the player's own container.
pub mod vault_chest {
    /// The player's own inventory, worn slots included, in the numbering `SlotLocation` uses.
    pub const PLAYER: i16 = -1;

    /// The gifts waiting to be claimed. A source and never a destination: claiming is what removes
    /// one from the account, so there is nothing to send back the other way.
    pub const GIFTS: i16 = -2;

    /// A potion stack, with the slot choosing which: nought for health, one for magic.
    ///
    /// A destination only. An item put here stops being an item and becomes a number, so nothing
    /// comes back out of it except by drinking.
    pub const STACKS: i16 = -3;
}

/// What a vault slot holds when it holds nothing.
///
/// `0xffff` rather than zero, because zero is a real object type. The same sentinel the original
/// stores for an empty vault slot: `DbVault`'s indexer and `RInventory.Items` both default to
/// `Enumerable.Repeat((ushort)0xffff, …)` (`common/DbModels.cs:857-858`, `:870`), and
/// `Vault.InitVault` pads a short gift chest with it (`realm/worlds/logic/Vault.cs:122`).
pub const VAULT_SLOT_EMPTY: u16 = 0xffff;

/// The most vault slots one update may claim.
///
/// A length prefix is attacker-controlled, so the capacity is bounded before anything is reserved.
/// Eighty chests of eight is the ceiling the original's map imposes, and this is well past it.
pub const MAX_VAULT_SLOTS: usize = 4096;

/// Writes a run of vault slots.
///
/// Shifted by one so that zero means empty, exactly as a trade slot is: an empty slot is one byte
/// instead of the three `0xffff` would cost, and most of a vault is empty.
fn write_vault_slots(w: &mut Writer<'_>, slots: &[u16]) {
    w.varint(slots.len() as u64);
    for slot in slots {
        w.varint(if *slot == VAULT_SLOT_EMPTY {
            0
        } else {
            *slot as u64 + 1
        });
    }
}

/// Reads one end of a vault move, refusing anything that is not a sixteen-bit index.
///
/// The panel's own numbering is sixteen bits wide, so a wider value is not a chest anybody could
/// have clicked. Refused rather than truncated: truncation turns an unreachable number into a
/// reachable one.
fn vault_index(r: &mut Reader<'_>) -> Result<i16, CodecError> {
    let value = r.varint_signed()?;
    i16::try_from(value).map_err(|_| CodecError::InvalidValue {
        what: "vault index",
        value: value as u64,
    })
}

fn read_vault_slots(r: &mut Reader<'_>) -> Result<Vec<u16>, CodecError> {
    let count = r.count(MAX_VAULT_SLOTS)?;

    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        let held = r.varint_u32()?;
        slots.push(if held == 0 {
            VAULT_SLOT_EMPTY
        } else {
            (held - 1) as u16
        });
    }
    Ok(slots)
}

/// Which of a player's containers a listing describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContainerId {
    Inventory = 0,
    Equipment = 1,
    Vault = 2,
}

impl ContainerId {
    pub fn from_code(code: u8) -> Option<ContainerId> {
        Some(match code {
            0 => ContainerId::Inventory,
            1 => ContainerId::Equipment,
            2 => ContainerId::Vault,
            _ => return None,
        })
    }
}

/// One tick of player intent.
///
/// Sent on every client tick, so it carries no strings and needs no allocation to decode. The
/// acknowledgement rides along here rather than in a message of its own precisely because this one
/// is already being sent at tick rate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Input {
    /// The newest snapshot the client holds. Everything the server sends back is encoded against
    /// this.
    pub ack: Acknowledgement,

    /// The client's own clock, used to measure round trip and to bound how far a move may claim to
    /// have travelled.
    pub client_time_ms: u32,

    /// Where the client believes it is. Advisory only: the server validates it against the tiles and
    /// the player's speed, and its own answer wins.
    pub x: f32,
    pub y: f32,
}

impl Input {
    fn encode(&self, w: &mut Writer<'_>) {
        self.ack.encode(w);
        w.varint(self.client_time_ms as u64);
        w.position(self.x);
        w.position(self.y);
    }

    fn decode(r: &mut Reader<'_>) -> Result<Input, CodecError> {
        Ok(Input {
            ack: Acknowledgement::decode(r)?,
            client_time_ms: r.varint_u32()?,
            x: r.position_value()?,
            y: r.position_value()?,
        })
    }
}

/// Anything the client can say.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage<'a> {
    /// The first message on a connection. The token comes from the app server over HTTPS; no
    /// password ever reaches the game socket.
    Hello {
        protocol: u32,
        token: &'a str,
        character: u32,
    },

    Input(Input),

    Chat {
        text: &'a str,
    },

    UsePortal {
        entity: EntityId,
    },

    /// Uses the item in a slot of some container, aimed at a point.
    ///
    /// The slot rather than the item, because the server knows what is in a slot and a client
    /// naming an item it does not hold is a claim rather than a fact.
    ///
    /// A container as well as a slot, because a slot alone cannot say whose. The original addresses
    /// both — `UseItem.SlotObject` is an object id and a slot id, and `UseItemHandler` passes both
    /// on (`networking/handlers/UseItemHandler.cs:22`), which `Player.UseItem` resolves to `Owner
    /// .GetEntity(objId)` and reads as an `IContainer` (`Player.UseItem.cs:112-125`). That is what
    /// lets a player drink a potion straight out of a bag on the ground rather than having to pick
    /// it up first. A zero, and the player's own id, both mean their own slots.
    UseItem {
        container: EntityId,
        slot: u16,
        x: f32,
        y: f32,
    },

    /// Answers a `Ping`, proving the client is still listening.
    ///
    /// Both fields are echoes of a clock: `serial` is the server's own uptime as it stamped the
    /// ping, so the server can subtract it from its uptime now and halve the difference for a
    /// one-way latency; `client_time_ms` is the client's own uptime as it answered, so the server
    /// can subtract the two clocks and learn the offset between them. The original carries exactly
    /// these two (`networking/packets/incoming/Pong.cs:7-8`, answered by the AS3 client at
    /// `messaging/impl/GameServerConnectionConcrete.as:1727-1731` with `getTimer()`), and both are
    /// milliseconds.
    Pong {
        serial: u32,
        client_time_ms: u32,
    },

    /// A request to fire, carrying only where the player is aiming.
    ///
    /// Aim is the one thing taken from the client as given, because where someone points is genuinely
    /// theirs to decide, and there is nothing to check it against. Everything downstream is the
    /// server's: whether the weapon is off cooldown, where the shot travels, what it strikes, and
    /// what that costs. Notably there is no hit report anywhere in this protocol.
    Shoot {
        angle: f32,
        client_time_ms: u32,
    },

    /// A request to move an item.
    ///
    /// Advisory in the same way movement is: the server decides whether the item is where the
    /// client thinks, whether the destination will take it, and whether the two are close enough to
    /// reach. A refusal comes back as [`ServerMessage::Refused`] and the client re-reads the
    /// container rather than assuming.
    MoveItem {
        from: SlotLocation,
        to: SlotLocation,
    },

    /// Asks a named player to trade.
    ///
    /// Asking somebody who has already asked you accepts theirs, which is how a trade begins: there
    /// is no separate accept, and both sides having asked is the agreement.
    RequestTrade {
        name: &'a str,
    },

    /// What this player is now offering, one flag per inventory slot.
    ChangeTrade {
        offer: Vec<bool>,
    },

    /// Agrees to a trade: what this player is offering and what they believe the other is.
    ///
    /// Both are sent because an offer can change between the moment it is shown and the moment it
    /// is agreed to. A player accepts what they were looking at, and an accept that names a stale
    /// offer is ignored rather than trusted.
    AcceptTrade {
        mine: Vec<bool>,
        theirs: Vec<bool>,
    },

    /// Ends the trade, from either side.
    CancelTrade,

    /// Leave for the nexus.
    ///
    /// Not a portal: there is no portal to step into, and a player who is stuck or in trouble has to
    /// be able to leave from wherever they are.
    Escape,

    /// Move to another player, by name.
    ///
    /// The server decides whether it is allowed: a world can forbid it, and a player who cannot be
    /// seen cannot be reached.
    Teleport {
        name: &'a str,
    },

    /// Buy what a merchant in the world is selling.
    ///
    /// Names the merchant rather than the item, because what it sells is the server's to know. A
    /// client that names an item is a client that can name a cheaper one.
    Buy {
        merchant: EntityId,
    },

    /// Something to do with a guild.
    Guild(GuildCommand<'a>),

    /// Something to do with the market.
    Market(MarketCommand),

    /// Give up this character's fame for prestige, and start it over.
    Prestige,

    /// Buy one of the things prestige buys.
    ///
    /// Names which of the shop's offers rather than the item, for the same reason buying from a
    /// merchant does: a client that names the item can name a cheaper one.
    PrestigeBuy {
        offer: u8,
    },

    /// Add or remove somebody from one of the account's lists.
    EditList {
        list: AccountList,
        name: &'a str,
        add: bool,
    },

    /// Moves an item within the vault, or between the vault and the player's own inventory.
    ///
    /// Both ends are named the same way — a chest and a slot — because the operation is a swap and
    /// is symmetric. See [`vault_chest`] for the three chest numbers that are not chests.
    ///
    /// The version is the vault as this client last saw it. A move quoting a version the server has
    /// moved past is refused and answered with the truth, which is the whole of the concurrency
    /// story. Ours: the original has no such packet and no such version — a vault move there is an
    /// `InvSwap` naming two entities (`networking/handlers/InvSwapHandler.cs:20-33`), and two
    /// clients on one account each get their own `Vault` world over the same redis fields
    /// (`realm/worlds/logic/Vault.cs:23-28`), a race it does not resolve.
    VaultMove {
        version: u32,
        from_chest: i16,
        from_slot: i16,
        to_chest: i16,
        to_slot: i16,
    },

    /// Buys one more vault chest.
    ///
    /// Carries only the count the client believes it owns, so a second click arriving while the
    /// first is still being paid for buys nothing. The price and the purse are the server's and are
    /// never sent by the client: the original reads `VaultChestCost` off the entity being clicked
    /// and the balance off the account (`realm/entities/vendors/ClosedVaultChest.cs:14-16`,
    /// `realm/entities/vendors/SellableObject.cs:100`), and its `Buy` packet carries only the object
    /// id of the chest.
    VaultBuy {
        chest_count: u32,
    },
}

/// What a player wants done about a guild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuildCommand<'a> {
    Create { name: &'a str },
    Invite { name: &'a str },
    Join { name: &'a str },
    Remove { name: &'a str },
    SetRank { name: &'a str, rank: u8 },
    SetBoard { text: &'a str },
    Leave,
}

/// What a player wants done about the market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketCommand {
    /// Everything for sale.
    Browse,

    /// Offer what is in a slot at a price.
    List { slot: u8, price: i32 },

    /// Take back a listing.
    Cancel { listing: u64 },

    /// Buy one.
    Buy { listing: u64 },
}

/// One of the lists an account keeps about other people.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountList {
    /// People whose messages are not shown.
    Ignored,

    /// People who may not teleport to this player.
    Locked,
}

impl AccountList {
    /// Which list this is on the wire. Public so a client can pass it on without re-deriving it.
    pub fn number(self) -> u64 {
        match self {
            AccountList::Ignored => 0,
            AccountList::Locked => 1,
        }
    }

    fn from_number(number: u32) -> Option<AccountList> {
        Some(match number {
            0 => AccountList::Ignored,
            1 => AccountList::Locked,
            _ => return None,
        })
    }
}

impl GuildCommand<'_> {
    fn encode(&self, w: &mut Writer<'_>) {
        match self {
            GuildCommand::Create { name } => {
                w.u8(0);
                w.string(name);
            }
            GuildCommand::Invite { name } => {
                w.u8(1);
                w.string(name);
            }
            GuildCommand::Join { name } => {
                w.u8(2);
                w.string(name);
            }
            GuildCommand::Remove { name } => {
                w.u8(3);
                w.string(name);
            }
            GuildCommand::SetRank { name, rank } => {
                w.u8(4);
                w.string(name);
                w.u8(*rank);
            }
            GuildCommand::SetBoard { text } => {
                w.u8(5);
                w.string(text);
            }
            GuildCommand::Leave => w.u8(6),
        }
    }

    fn decode<'b>(r: &mut Reader<'b>) -> Result<GuildCommand<'b>, CodecError> {
        let kind = r.u8()?;
        Ok(match kind {
            0 => GuildCommand::Create { name: r.string()? },
            1 => GuildCommand::Invite { name: r.string()? },
            2 => GuildCommand::Join { name: r.string()? },
            3 => GuildCommand::Remove { name: r.string()? },
            4 => GuildCommand::SetRank {
                name: r.string()?,
                rank: r.u8()?,
            },
            5 => GuildCommand::SetBoard { text: r.string()? },
            6 => GuildCommand::Leave,
            _ => {
                return Err(CodecError::InvalidValue {
                    what: "guild command",
                    value: kind as u64,
                });
            }
        })
    }
}

impl MarketCommand {
    fn encode(&self, w: &mut Writer<'_>) {
        match self {
            MarketCommand::Browse => w.u8(0),
            MarketCommand::List { slot, price } => {
                w.u8(1);
                w.u8(*slot);
                w.varint(*price as u64);
            }
            MarketCommand::Cancel { listing } => {
                w.u8(2);
                w.varint(*listing);
            }
            MarketCommand::Buy { listing } => {
                w.u8(3);
                w.varint(*listing);
            }
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<MarketCommand, CodecError> {
        let kind = r.u8()?;
        Ok(match kind {
            0 => MarketCommand::Browse,
            1 => MarketCommand::List {
                slot: r.u8()?,
                // A price is never negative, so it travels unsigned and a client that wants to be
                // paid for taking something cannot say so.
                price: r.varint_u32()? as i32,
            },
            2 => MarketCommand::Cancel {
                listing: r.varint_u32()? as u64,
            },
            3 => MarketCommand::Buy {
                listing: r.varint_u32()? as u64,
            },
            _ => {
                return Err(CodecError::InvalidValue {
                    what: "market command",
                    value: kind as u64,
                });
            }
        })
    }
}

/// One inventory slot, as the other side of a trade sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeSlot {
    /// What is in it, or `None` for empty.
    pub item: Option<u16>,

    /// What the slot accepts, so a client can show why something will not go there.
    pub slot_type: i32,

    /// Whether it is part of the current offer.
    pub included: bool,

    /// Whether it could be offered at all. Worn slots and soulbound items cannot.
    pub tradeable: bool,
}

/// Writes what one side is holding.
fn write_slots(w: &mut Writer<'_>, slots: &[TradeSlot]) {
    w.varint(slots.len() as u64);
    for slot in slots {
        w.varint(slot.item.map_or(0, |item| item as u64 + 1));
        w.varint(slot.slot_type.max(0) as u64);
        w.u8(u8::from(slot.included) | (u8::from(slot.tradeable) << 1));
    }
}

/// Reads what one side is holding.
fn read_slots(r: &mut Reader<'_>) -> Result<Vec<TradeSlot>, CodecError> {
    let count = r.varint_u32()? as usize;

    let mut slots = Vec::with_capacity(count.min(MAX_TRADE_SLOTS));
    for _ in 0..count {
        // Shifted by one so that zero means empty, which is what keeps object type zero from
        // meaning both "nothing" and a real object.
        let item = r.varint_u32()?;
        let slot_type = r.varint_u32()? as i32;
        let flags = r.u8()?;

        slots.push(TradeSlot {
            item: (item > 0).then(|| (item - 1) as u16),
            slot_type,
            included: flags & 1 != 0,
            tradeable: flags & 2 != 0,
        });
    }
    Ok(slots)
}

/// Writes one side's offer: a flag per inventory slot.
fn write_offer(w: &mut Writer<'_>, offer: &[bool]) {
    w.varint(offer.len() as u64);
    for included in offer {
        w.u8(u8::from(*included));
    }
}

/// Reads one side's offer.
fn read_offer(r: &mut Reader<'_>) -> Result<Vec<bool>, CodecError> {
    let count = r.varint_u32()? as usize;

    let mut offer = Vec::with_capacity(count.min(MAX_TRADE_SLOTS));
    for _ in 0..count {
        offer.push(r.u8()? != 0);
    }
    Ok(offer)
}

/// The most inventory slots a trade message may claim.
///
/// A length prefix is attacker-controlled, so the capacity is bounded before anything is reserved.
pub const MAX_TRADE_SLOTS: usize = 64;

impl ClientMessage<'_> {
    pub fn id(&self) -> u16 {
        match self {
            ClientMessage::Hello { .. } => client_id::HELLO,
            ClientMessage::Input(_) => client_id::INPUT,
            ClientMessage::Chat { .. } => client_id::CHAT,
            ClientMessage::UsePortal { .. } => client_id::USE_PORTAL,
            ClientMessage::UseItem { .. } => client_id::USE_ITEM,
            ClientMessage::Pong { .. } => client_id::PONG,
            ClientMessage::Shoot { .. } => client_id::SHOOT,
            ClientMessage::MoveItem { .. } => client_id::MOVE_ITEM,
            ClientMessage::RequestTrade { .. } => client_id::REQUEST_TRADE,
            ClientMessage::ChangeTrade { .. } => client_id::CHANGE_TRADE,
            ClientMessage::AcceptTrade { .. } => client_id::ACCEPT_TRADE,
            ClientMessage::CancelTrade => client_id::CANCEL_TRADE,
            ClientMessage::Escape => client_id::ESCAPE,
            ClientMessage::Teleport { .. } => client_id::TELEPORT,
            ClientMessage::Buy { .. } => client_id::BUY,
            ClientMessage::Guild(_) => client_id::GUILD,
            ClientMessage::Market(_) => client_id::MARKET,
            ClientMessage::EditList { .. } => client_id::EDIT_LIST,
            ClientMessage::Prestige => client_id::PRESTIGE,
            ClientMessage::PrestigeBuy { .. } => client_id::PRESTIGE_BUY,
            ClientMessage::VaultMove { .. } => client_id::VAULT_MOVE,
            ClientMessage::VaultBuy { .. } => client_id::VAULT_BUY,
        }
    }

    pub fn encode(&self, w: &mut Writer<'_>) {
        w.u16(self.id());
        match self {
            ClientMessage::Hello {
                protocol,
                token,
                character,
            } => {
                w.varint(*protocol as u64);
                w.string(token);
                w.varint(*character as u64);
            }
            ClientMessage::Input(input) => input.encode(w),
            ClientMessage::Chat { text } => w.string(text),
            ClientMessage::UsePortal { entity } => w.varint(entity.0 as u64),
            ClientMessage::UseItem {
                container,
                slot,
                x,
                y,
            } => {
                w.varint(container.0 as u64);
                w.varint(*slot as u64);
                w.position(*x);
                w.position(*y);
            }
            ClientMessage::Pong {
                serial,
                client_time_ms,
            } => {
                w.varint(*serial as u64);
                w.varint(*client_time_ms as u64);
            }
            ClientMessage::Shoot {
                angle,
                client_time_ms,
            } => {
                w.f32(*angle);
                w.varint(*client_time_ms as u64);
            }
            ClientMessage::MoveItem { from, to } => {
                from.encode(w);
                to.encode(w);
            }
            ClientMessage::RequestTrade { name } => w.string(name),
            ClientMessage::ChangeTrade { offer } => write_offer(w, offer),
            ClientMessage::AcceptTrade { mine, theirs } => {
                write_offer(w, mine);
                write_offer(w, theirs);
            }
            ClientMessage::CancelTrade => {}

            ClientMessage::Escape => {}
            ClientMessage::Teleport { name } => w.string(name),
            ClientMessage::Buy { merchant } => w.varint(merchant.0 as u64),
            ClientMessage::Guild(command) => command.encode(w),
            ClientMessage::Market(command) => command.encode(w),
            ClientMessage::Prestige => {}
            ClientMessage::PrestigeBuy { offer } => w.u8(*offer),
            ClientMessage::EditList { list, name, add } => {
                w.varint(list.number());
                w.string(name);
                w.u8(u8::from(*add));
            }
            ClientMessage::VaultMove {
                version,
                from_chest,
                from_slot,
                to_chest,
                to_slot,
            } => {
                w.varint(*version as u64);
                // Signed, because three of the chest numbers are negative by design.
                w.varint_signed(*from_chest as i64);
                w.varint_signed(*from_slot as i64);
                w.varint_signed(*to_chest as i64);
                w.varint_signed(*to_slot as i64);
            }
            ClientMessage::VaultBuy { chest_count } => w.varint(*chest_count as u64),
        }
    }

    pub fn decode<'b>(r: &mut Reader<'b>) -> Result<ClientMessage<'b>, CodecError> {
        let id = r.u16()?;
        Ok(match id {
            client_id::REQUEST_TRADE => ClientMessage::RequestTrade { name: r.string()? },
            client_id::CHANGE_TRADE => ClientMessage::ChangeTrade {
                offer: read_offer(r)?,
            },
            client_id::ACCEPT_TRADE => ClientMessage::AcceptTrade {
                mine: read_offer(r)?,
                theirs: read_offer(r)?,
            },
            client_id::CANCEL_TRADE => ClientMessage::CancelTrade,

            client_id::ESCAPE => ClientMessage::Escape,
            client_id::TELEPORT => ClientMessage::Teleport { name: r.string()? },
            client_id::BUY => ClientMessage::Buy {
                merchant: EntityId(r.varint_u32()?),
            },
            client_id::GUILD => ClientMessage::Guild(GuildCommand::decode(r)?),
            client_id::MARKET => ClientMessage::Market(MarketCommand::decode(r)?),
            client_id::PRESTIGE => ClientMessage::Prestige,
            client_id::PRESTIGE_BUY => ClientMessage::PrestigeBuy { offer: r.u8()? },
            client_id::EDIT_LIST => ClientMessage::EditList {
                list: {
                    let number = r.varint_u32()?;
                    AccountList::from_number(number).ok_or(CodecError::InvalidValue {
                        what: "account list",
                        value: number as u64,
                    })?
                },
                name: r.string()?,
                add: r.u8()? != 0,
            },

            client_id::HELLO => ClientMessage::Hello {
                protocol: r.varint_u32()?,
                token: r.string()?,
                character: r.varint_u32()?,
            },
            client_id::INPUT => ClientMessage::Input(Input::decode(r)?),
            client_id::CHAT => ClientMessage::Chat { text: r.string()? },
            client_id::USE_ITEM => ClientMessage::UseItem {
                container: EntityId(r.varint_u32()?),
                slot: r.varint_u32()? as u16,
                x: r.position_value()?,
                y: r.position_value()?,
            },
            client_id::USE_PORTAL => ClientMessage::UsePortal {
                entity: EntityId(r.varint_u32()?),
            },
            client_id::PONG => ClientMessage::Pong {
                serial: r.varint_u32()?,
                client_time_ms: r.varint_u32()?,
            },
            client_id::SHOOT => ClientMessage::Shoot {
                angle: r.f32()?,
                client_time_ms: r.varint_u32()?,
            },
            client_id::MOVE_ITEM => ClientMessage::MoveItem {
                from: SlotLocation::decode(r)?,
                to: SlotLocation::decode(r)?,
            },
            client_id::VAULT_MOVE => ClientMessage::VaultMove {
                version: r.varint_u32()?,
                from_chest: vault_index(r)?,
                from_slot: vault_index(r)?,
                to_chest: vault_index(r)?,
                to_slot: vault_index(r)?,
            },
            client_id::VAULT_BUY => ClientMessage::VaultBuy {
                chest_count: r.varint_u32()?,
            },
            unknown => {
                return Err(CodecError::InvalidValue {
                    what: "client message id",
                    value: unknown as u64,
                });
            }
        })
    }
}

/// Anything the server can say.
///
/// [`ServerMessage::Snapshot`] carries the remaining bytes rather than a decoded world, because
/// decoding one needs the baseline it was encoded against and only the session knows which that is.
/// Holding a borrow keeps that a hand-off rather than a copy.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage<'a> {
    Welcome {
        player: EntityId,
        tick: Tick,
        world: &'a str,

        /// How large the map is, in tiles.
        ///
        /// The terrain arrives as rows and the extent is implied by them, but a client has to size
        /// its map and its minimap before the first row lands. Sending it here costs four bytes
        /// once per world and saves the client guessing.
        width: u16,
        height: u16,

        /// Which sky the map is drawn against, by its number.
        ///
        /// `World.Background` out of the definition's `background` field (`World.cs:120`), carried
        /// to the client in `MapInfo.Background` (`ConnectManager.cs:346`).
        background: i32,

        /// How dangerous this place is said to be, as the marks on the loading screen.
        ///
        /// `World.Difficulty` (`World.cs:119`). Negative or zero means no marks at all, which is
        /// how every town and the vault describe themselves (`MapLoadingView.as:78-87`).
        difficulty: i32,

        /// Whether one player may teleport to another here.
        ///
        /// `AllowTeleport = !proto.restrictTp` (`World.cs:123`). The server refuses either way; this
        /// is what lets a client stop offering the option in the menu it draws over a player
        /// (`PlayerMenu.as:72`).
        allow_teleport: bool,

        /// Whether the scoreboards standing in this world show anything.
        ///
        /// `World.ShowDisplays` (`World.cs:124`), carried in `MapInfo.ShowDisplays`.
        show_displays: bool,

        /// What should be playing here.
        ///
        /// One name drawn from the definition's list when the world was built
        /// (`World.cs:127-132`), so two realms are not guaranteed the same track. Empty for a world
        /// whose definition names none, which leaves whatever was playing alone.
        music: &'a str,
    },

    Rejected {
        reason: RejectReason,
    },

    /// A world snapshot, still encoded. Pass `body` to `decode_snapshot` with the baseline the
    /// session holds.
    Snapshot {
        body: &'a [u8],
    },

    Chat {
        /// Who is speaking, so the client can draw the line over their head as well as in the log.
        /// Zero for anything the server says in its own voice, which belongs to no body.
        speaker: EntityId,
        from: &'a str,
        text: &'a str,
    },

    /// Asks the client to say it is still there.
    ///
    /// `serial` is the server's own uptime in milliseconds at the moment it stamped the ping, which
    /// is what the original puts there (`realm/entities/player/Player.KeepAlive.cs:92-96`). It is a
    /// token to be echoed rather than a counter, and echoing it is what lets the server measure the
    /// round trip without keeping a table of outstanding pings.
    Ping {
        serial: u32,
    },

    /// A projectile came into being.
    ///
    /// Sent so clients can draw the shot. Its flight is entirely predictable from these fields, so
    /// nothing further is sent per tick. A projectile costs one message for its whole life rather
    /// than a snapshot entry every tick.
    Shot {
        projectile: EntityId,
        owner: EntityId,

        /// What the bullet's descriptor is read off: the weapon, the ability item, or the enemy.
        ///
        /// Not the bullet's own object type, which names only a sprite. Everything a client needs
        /// to draw and step the shot — speed, lifetime, texture — lives in a `<Projectile>` element
        /// on this, which is why the original's two shot packets both carry `item.ObjectType`
        /// rather than the projectile's (`Player.UseItem.cs:1135`, `:1160`).
        object_type: u16,
        x: f32,
        y: f32,
        angle: f32,
        /// Tiles per second.
        speed: f32,
        lifetime_ms: u32,

        /// What it will take off whatever it hits, before that body's defence.
        ///
        /// Carried on the shot rather than reported when it lands, because whoever it lands on is
        /// the one client the server does not tell: both of the original's shot packets carry a
        /// `damage` field for exactly this (`incoming/EnemyShoot.as:23-35`,
        /// `incoming/ServerPlayerShoot.as:20-27`), and the client stamps it onto the projectile
        /// (`GameServerConnectionConcrete.as:1006`, `:1065`) so that the number over its own head
        /// appears the instant the bullet touches it (`Projectile.as:253`, `:264-265`).
        damage: u16,
    },

    /// The whole contents of one of the player's containers.
    ///
    /// Sent in full rather than as a delta. A container changes when a player moves something,
    /// which is rare next to movement, and sending the whole thing means a client that misses one
    /// update is corrected by the next rather than drifting.
    Container {
        container: ContainerId,

        /// Slot index and item type, for occupied slots only.
        ///
        /// Owned rather than borrowed like the rest of this enum, because decoding varints cannot
        /// hand back a slice of the input. That costs one allocation, which is fine here and would
        /// not be on the snapshot path. A container changes when a player moves something, not
        /// twenty times a second.
        slots: Vec<(u16, u16)>,
    },

    /// A request was refused, with something to show the player.
    Refused {
        message: &'a str,
    },

    /// One horizontal run of the map, so a client can draw the ground.
    ///
    /// Sent in strips rather than whole, because a map is four million squares and one message
    /// holding all of it would be larger than anything the transport will carry. A strip is a row
    /// of a rectangle, which is what the client fills in as it arrives.
    ///
    /// The tiles are run-length encoded as `(count, tile)`. A map is mostly the same square
    /// repeated, so this is the difference between a strip that fits and one that does not.
    Terrain {
        x: u16,
        y: u16,
        runs: Vec<(u16, u16)>,
    },

    /// The map's scenery: objects that never move and never act.
    ///
    /// Sent with the ground rather than as entities, because that is what they are. A realm map
    /// carries a quarter of a million trees and rocks; as entities they would fill the world four
    /// times over and leave no room for a single enemy, and every one of them would take a place in
    /// the snapshot for the life of the world.
    ///
    /// One message per row, as `(x, object, size)`, for the same reason the ground goes in strips.
    /// The size is a percentage of the object's natural size, and zero means natural: a realm map
    /// scales seventy thousand of its trees for variety, and a tree drawn at one size everywhere
    /// looks like a plantation.
    Scenery {
        y: u16,
        objects: Vec<(u16, u16, u16)>,
    },

    /// Somebody has asked to trade.
    TradeRequested {
        name: String,
    },

    /// A trade has begun, with what both sides are holding.
    TradeStart {
        mine: Vec<TradeSlot>,
        their_name: String,
        theirs: Vec<TradeSlot>,
    },

    /// The other side changed what they are offering.
    TradeChanged {
        offer: Vec<bool>,
    },

    /// The other side agreed to a trade, and to what.
    TradeAccepted {
        mine: Vec<bool>,
        theirs: Vec<bool>,
    },

    /// The trade ended, one way or the other.
    TradeDone {
        /// Zero when it went through, and anything else when it did not.
        code: u32,
        message: String,
    },

    /// The server is full, and this is where you stand in the line.
    ///
    /// Sent rather than closing the connection, because a refusal makes everybody retry and
    /// everybody retrying makes a busy server hardest to get into exactly when it is busiest. Sent
    /// again whenever the place changes, so waiting looks like waiting rather than a hang.
    Queued {
        place: u32,
        waiting: u32,
    },

    /// How many of each stacking potion the character holds.
    ///
    /// Its own message rather than a container, because a stack is one kind of thing many times
    /// over and a container says what is in a slot rather than how much of it. Squeezing a count
    /// into a slot number would be a number nobody reading the protocol could explain.
    Stacks {
        health: u16,
        magic: u16,
    },

    /// This character has died.
    ///
    /// Its own message rather than a notice, because it is the end of the session: what follows is
    /// the character select screen, and a client that treated it as text would keep playing a
    /// character the server has already written down as dead.
    Died {
        character: u32,
        killed_by: String,

        /// What the character finished with, which is what the death screen shows.
        fame: i32,
    },

    /// Something the world wants shown rather than said.
    ///
    /// Separate from chat because it is not somebody talking: a dungeon saying which keys have been
    /// found is state, and putting it in the chat log would bury it under conversation.
    Notice {
        text: String,
    },

    /// A word the client acts on rather than reads.
    ///
    /// `GlobalNotification` (`networking/packets/outgoing/GlobalNotification.cs`), which despite
    /// its name and its string field carries commands: `giftChestOccupied` and `giftChestEmpty`
    /// say whether anything is waiting in the chest, `showKeyUI` opens the key panel, and a colour
    /// names a key. Anything the client does not recognise it shows, which is why this is not
    /// simply [`Self::Notice`]: the five it does recognise are not sentences.
    Notification {
        text: String,
    },

    /// Squares whose ground has changed, as `(x, y, tile)`.
    ///
    /// Sent rather than folded into the snapshot because ground is not an entity: it has no id, it
    /// does not move, and a square that changed once should not be re-sent every tick for the rest
    /// of the world's life.
    Ground {
        /// Owned for the same reason a container's slots are: decoding varints cannot hand back a
        /// slice of the input. Ground changes are rare, so the allocation costs nothing that
        /// matters.
        changes: Vec<(u16, u16, u16)>,
    },

    /// Something took a hit, and whether the hit ended it.
    ///
    /// The snapshot already carries health, so this is not how a client learns a number changed. It
    /// is how a client learns *that* it changed and by how much, which is what a damage number
    /// over a body is, and it is the only thing on the wire that separates a death from a walk out
    /// of sight: a body that stops being mentioned by the snapshot could be either, and a client
    /// left to guess either leaves corpses standing or deletes anything that rounds a corner.
    Damage {
        /// What was hit.
        target: EntityId,

        /// The condition effects the hit carried, as the same bitfield the snapshot uses. Zero for
        /// anything but a projectile, which is the only hit that carries effects of its own.
        effects: u128,

        /// Health taken, after defence and after the target's own effects.
        amount: u16,

        /// Whether the target died of it.
        kill: bool,

        /// Which shot of the volley landed, so a client can retire the bullet it drew.
        bullet: u8,

        /// Who dealt it. Zero for the world itself, which is what ground damage is.
        owner: EntityId,
    },

    /// The whole vault, as the server has it.
    ///
    /// Sent whole rather than as a delta: on arrival in the vault, after every accepted move, and
    /// to the loser of a race. A vault is a few hundred item types at the very most, and a client
    /// that is told everything cannot drift out of step with the server — drifting out of step is
    /// what duplicates items. Ours: the original has nothing to snapshot, because its chests are
    /// entities and the client learns each one's eight slots from the ordinary object updates that
    /// carry a container's inventory (`realm/worlds/logic/Vault.cs:96-107`).
    ///
    /// Its own message rather than a [`ServerMessage::Container`] because the panel needs the
    /// version to quote back, the capacity to stop offering more chests, and the gifts, and a
    /// container says none of those.
    VaultUpdate {
        /// What the vault was at when this was written. Moves quote it back.
        version: u32,

        chest_count: u32,

        /// The most chests this account may ever own, so the panel can stop offering more.
        max_chests: u32,

        /// What the next chest costs, in fame.
        next_chest_price: u32,

        /// Item types, eight per chest, flat: chest `i` owns indices `i * 8` to `i * 8 + 7`.
        /// [`VAULT_SLOT_EMPTY`] where a slot is empty.
        slots: Vec<u16>,

        /// Gifts waiting to be claimed, which the same panel shows and one gesture takes.
        ///
        /// Dense and never empty in the middle, because a claimed gift leaves the list rather than
        /// leaving a hole. Not storage — nothing can be put into them — so they are sent apart from
        /// the slots rather than as more of them.
        gifts: Vec<u16>,
    },

    /// A body is at a place it did not walk to.
    ///
    /// The snapshot cannot say this. A client owns the position of its own player and glides every
    /// other body towards the position the snapshot gives it, so the one thing neither can express
    /// is "stop, you are here now" — which is what a teleport is. The original keeps this apart for
    /// the same reason: `Goto` (`networking/packets/outgoing/Goto.cs:5-11`) snaps and zeroes the
    /// body's velocity (`GameObject.as:678-687`), where an ordinary tick position is interpolated
    /// towards (`GameObject.as:689-698`).
    ///
    /// Sent to everyone who can see the body rather than only to whoever moved, as
    /// `Player.cs:700-704` sends it: a player who teleports out of a fight must stop being drawn
    /// mid-stride on every other screen too.
    Goto {
        /// Whose body moved. Not necessarily the receiver's own.
        object_id: EntityId,
        x: f32,
        y: f32,
    },

    /// Something for the client to draw that is not a body, a bullet or a number.
    ///
    /// The whole of the original's `ShowEffect` (`networking/packets/outgoing/ShowEffect.cs:5-34`)
    /// less its `Duration`, which no effect in the client reads: the two effects that are timed —
    /// `Flashing` and the shocked aura — take their duration out of `Pos1` instead, which is why
    /// the field is dead in the original too.
    ///
    /// Both positions are overloaded per effect and mean different things for each: a radius in
    /// `pos1.x` for an area blast, a particle count in `pos2.x` for lightning, a period and a cycle
    /// count in `pos1` for a flash. The enum in `Structures.cs:133-151` is the only description of
    /// which, and the client's `ShowEffectType` repeats it verbatim rather than reinterpreting it.
    ShowEffect {
        /// `EffectType` (`Structures.cs:133-151`), by its number. Not an enum here: the client
        /// draws nineteen of them and the server only ever names a few, so a value the server has
        /// no constant for is still one the client knows how to draw.
        effect: u8,

        /// The body the effect hangs off, which most of them need to know where to start. Zero for
        /// the effects that live at a place rather than on something.
        target: EntityId,

        /// First overloaded position. Tiles, or a bare number where the effect wants one.
        x1: f32,
        y1: f32,

        /// Second overloaded position, read by the effects that want two.
        x2: f32,
        y2: f32,

        /// Packed `0xAARRGGBB`, as `ARGB` (`Structures.cs:153-165`) is on the wire.
        color: u32,
    },

    /// A line of text that rises off a body and disappears.
    ///
    /// The original's `Notification` (`networking/packets/outgoing/Notification.cs:5-27`), which
    /// this cannot be called because [`Self::Notification`] already carries the original's
    /// `GlobalNotification` — a different packet with a confusingly similar name. The client's own
    /// name for what it draws is `CharacterStatusText`
    /// (`map/mapoverlay/CharacterStatusText.as`), so that is the name used here.
    ///
    /// Every `+N` over a healed player, every `+N Fame`, every "Stasis" and "Immune" over an enemy
    /// is one of these. It is the only channel in the game for saying something about one body
    /// rather than to one connection, which is why a heal that moves the bar and sends nothing
    /// reads as the bar drifting on its own.
    StatusText {
        /// The body the text hangs over. `CharacterStatusText` anchors to it and follows it, so a
        /// number stays over the monster it is about while the monster walks
        /// (`GameServerConnectionConcrete.as:1161-1163` looks it up and drops the packet when the
        /// client has never heard of the body).
        object_id: EntityId,

        /// Either a literal — `"+45"` — or a `LineBuilder` JSON blob naming a localisation key,
        /// which is how the two quest completions travel (`Player.Leveling.cs:250`, `:311`).
        text: String,

        /// Packed `0xAARRGGBB`. Green for health, purple for mana, orange for fame, red for a
        /// condition landing on an enemy.
        color: u32,
    },

    /// A blast ring at a place, and what standing in it costs.
    ///
    /// `Aoe` (`networking/packets/outgoing/Aoe.cs:6-37`). Sent by the two behaviours that throw
    /// something and detonate it after a delay: `Grenade.cs:85` and `Ported.cs:78`.
    ///
    /// The damage travels because the original's client is the one that applies it to its own
    /// player: `onAoe` measures its own distance, works the defence out itself and calls
    /// `player.damage` (`GameServerConnectionConcrete.as:1834-1861`). This server damages from the
    /// world instead, the same way it does for every other hit, so what this carries is the
    /// telegraph — the ring the player learns to step out of — and the numbers that let a client
    /// draw it truthfully.
    Aoe {
        /// Centre of the blast, in tiles.
        x: f32,
        y: f32,

        /// How far it reaches, in tiles.
        radius: f32,

        /// What it takes off an undefended body.
        damage: u16,

        /// `ConditionEffectIndex` by its number, or zero for none.
        effect: u8,

        /// How long that condition lasts, in seconds.
        duration: f32,

        /// What threw it, which the original's client names as the killer if the blast is fatal.
        orig_type: u16,
    },

    /// Somebody has asked this player into their guild.
    ///
    /// `InvitedToGuild` (`networking/packets/outgoing/InvitedToGuild.cs:6-27`), sent by
    /// `GuildInviteHandler.cs:49` to the invitee alone. The invitation is a standing offer rather
    /// than an act: the handler records it and the invitee accepts by answering with the guild's
    /// name (`JoinGuildHandler.cs:22-40`), so an invitation that is never shown is an invitation
    /// that can never be taken up.
    InvitedToGuild {
        /// Who is asking.
        name: String,

        /// Which guild, which is also what the invitee has to name to accept.
        guild: String,
    },

    /// Play something else from here on.
    ///
    /// `SwitchMusic` (`networking/packets/outgoing/SwitchMusic.cs:5-24`), sent when a world's track
    /// changes under the people already standing in it: the `/music` command
    /// (`RankedCommands.cs:1809`) and the two behaviours that rescore a dungeon as it turns,
    /// `ChangeMusic` (`logic/behaviors/ChangeMusic.cs:43`) and `ChangeMusicOnDeath.cs:39`.
    ///
    /// Apart from the welcome because the welcome is a world being entered, and this is the same
    /// world sounding different. The client does the same thing with both (`Music.load`,
    /// `GameServerConnectionConcrete.as:403-405`).
    SwitchMusic {
        music: &'a str,
    },

    /// What this player's quest arrow points at.
    ///
    /// `QuestObjId` (`networking/packets/outgoing/QuestObjId.cs:5-24`), sent by `HandleQuest`
    /// whenever the chosen enemy changes and never when it does not
    /// (`realm/entities/player/Player.Leveling.cs:213-229`). Which enemy is worth pointing at is
    /// already decided by the world; this is the only way a client learns of it, and without it
    /// there is no arrow and no marker on the map.
    ///
    /// Sent to the one player it belongs to. Two players standing together are pointed at different
    /// things, because the score depends on each one's level.
    QuestTarget {
        target: EntityId,
    },

    /// Look at this body instead of your own.
    ///
    /// `SetFocus` (`networking/packets/outgoing/SetFocus.cs:5-24`). The camera alone: the player is
    /// still where they were, and is held still by the `Paused` effect the same commands apply
    /// (`UnrankedCommands.cs:1421-1441`, `RankedCommands.cs:2384-2395`). Naming the player's own
    /// body is how the original gives the camera back (`Player.cs:510-513`).
    SetFocus {
        target: EntityId,
    },

    /// Who is on one of this account's lists.
    ///
    /// `AccountList` (`networking/packets/outgoing/AccountList.cs:5-40`), sent on arrival for both
    /// lists at once (`ConnectManager.cs:355-368`) and again whenever one is edited
    /// (`UnrankedCommands.cs:491`, `:537`, `:583`, `:630`). It is what draws the marker beside a
    /// name: enforcement happens on the server either way, and a player with no marker has no way
    /// to tell who they have already blocked.
    ///
    /// Names rather than the original's account ids, because a name is what this protocol already
    /// uses to say who somebody is — [`ClientMessage::EditList`] names one to add, and the snapshot
    /// carries no account id to match an id against.
    AccountList {
        list: AccountList,
        names: Vec<String>,
    },
}

impl ServerMessage<'_> {
    pub fn id(&self) -> u16 {
        match self {
            ServerMessage::Welcome { .. } => server_id::WELCOME,
            ServerMessage::Rejected { .. } => server_id::REJECTED,
            ServerMessage::Snapshot { .. } => server_id::SNAPSHOT,
            ServerMessage::Chat { .. } => server_id::CHAT,
            ServerMessage::Ping { .. } => server_id::PING,
            ServerMessage::Shot { .. } => server_id::SHOT,
            ServerMessage::Container { .. } => server_id::CONTAINER,
            ServerMessage::Refused { .. } => server_id::REFUSED,
            ServerMessage::Ground { .. } => server_id::GROUND,
            ServerMessage::Terrain { .. } => server_id::TERRAIN,
            ServerMessage::Scenery { .. } => server_id::SCENERY,
            ServerMessage::TradeRequested { .. } => server_id::TRADE_REQUESTED,
            ServerMessage::TradeStart { .. } => server_id::TRADE_START,
            ServerMessage::TradeChanged { .. } => server_id::TRADE_CHANGED,
            ServerMessage::TradeAccepted { .. } => server_id::TRADE_ACCEPTED,
            ServerMessage::TradeDone { .. } => server_id::TRADE_DONE,
            ServerMessage::Notice { .. } => server_id::NOTICE,
            ServerMessage::Notification { .. } => server_id::NOTIFICATION,
            ServerMessage::Died { .. } => server_id::DIED,
            ServerMessage::Stacks { .. } => server_id::STACKS,
            ServerMessage::Queued { .. } => server_id::QUEUED,
            ServerMessage::Damage { .. } => server_id::DAMAGE,
            ServerMessage::VaultUpdate { .. } => server_id::VAULT_UPDATE,
            ServerMessage::Goto { .. } => server_id::GOTO,
            ServerMessage::ShowEffect { .. } => server_id::SHOW_EFFECT,
            ServerMessage::StatusText { .. } => server_id::STATUS_TEXT,
            ServerMessage::Aoe { .. } => server_id::AOE,
            ServerMessage::InvitedToGuild { .. } => server_id::INVITED_TO_GUILD,
            ServerMessage::SwitchMusic { .. } => server_id::SWITCH_MUSIC,
            ServerMessage::QuestTarget { .. } => server_id::QUEST_TARGET,
            ServerMessage::SetFocus { .. } => server_id::SET_FOCUS,
            ServerMessage::AccountList { .. } => server_id::ACCOUNT_LIST,
        }
    }

    pub fn encode(&self, w: &mut Writer<'_>) {
        w.u16(self.id());
        match self {
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
                w.varint(player.0 as u64);
                w.varint(tick.0 as u64);
                w.string(world);
                w.varint(*width as u64);
                w.varint(*height as u64);
                w.varint_signed(*background as i64);
                w.varint_signed(*difficulty as i64);
                w.bool(*allow_teleport);
                w.bool(*show_displays);
                w.string(music);
            }
            ServerMessage::Rejected { reason } => w.u8(*reason as u8),
            // Written raw: the snapshot encoder produced these bytes and re-length-prefixing them
            // would spend bytes to describe a payload that already runs to the end of the message.
            ServerMessage::Snapshot { body } => w.raw(body),
            ServerMessage::Chat {
                speaker,
                from,
                text,
            } => {
                w.varint(speaker.0 as u64);
                w.string(from);
                w.string(text);
            }
            ServerMessage::Queued { place, waiting } => {
                w.varint(*place as u64);
                w.varint(*waiting as u64);
            }
            ServerMessage::Stacks { health, magic } => {
                w.varint(*health as u64);
                w.varint(*magic as u64);
            }
            ServerMessage::Notice { text } => w.string(text),
            ServerMessage::Notification { text } => w.string(text),
            ServerMessage::Died {
                character,
                killed_by,
                fame,
            } => {
                w.varint(*character as u64);
                w.string(killed_by);
                w.varint((*fame).max(0) as u64);
            }
            ServerMessage::TradeRequested { name } => w.string(name),
            ServerMessage::TradeStart {
                mine,
                their_name,
                theirs,
            } => {
                write_slots(w, mine);
                w.string(their_name);
                write_slots(w, theirs);
            }
            ServerMessage::TradeChanged { offer } => write_offer(w, offer),
            ServerMessage::TradeAccepted { mine, theirs } => {
                write_offer(w, mine);
                write_offer(w, theirs);
            }
            ServerMessage::TradeDone { code, message } => {
                w.varint(*code as u64);
                w.string(message);
            }

            ServerMessage::Scenery { y, objects } => {
                w.varint(*y as u64);
                w.varint(objects.len() as u64);
                for (x, object, size) in objects {
                    w.varint(*x as u64);
                    w.varint(*object as u64);
                    w.varint(*size as u64);
                }
            }
            ServerMessage::Terrain { x, y, runs } => {
                w.varint(*x as u64);
                w.varint(*y as u64);
                w.varint(runs.len() as u64);
                for (count, tile) in runs {
                    w.varint(*count as u64);
                    w.varint(*tile as u64);
                }
            }
            ServerMessage::Ground { changes } => {
                w.varint(changes.len() as u64);
                for (x, y, tile) in changes {
                    w.varint(*x as u64);
                    w.varint(*y as u64);
                    w.varint(*tile as u64);
                }
            }
            ServerMessage::Ping { serial } => w.varint(*serial as u64),
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
            } => {
                w.varint(projectile.0 as u64);
                w.varint(owner.0 as u64);
                w.varint(*object_type as u64);
                w.position(*x);
                w.position(*y);
                w.f32(*angle);
                w.f32(*speed);
                w.varint(*lifetime_ms as u64);
                w.varint(*damage as u64);
            }
            ServerMessage::Container { container, slots } => {
                w.u8(*container as u8);
                w.varint(slots.len() as u64);
                for (slot, item) in slots.iter() {
                    w.varint(*slot as u64);
                    w.varint(*item as u64);
                }
            }
            ServerMessage::Refused { message } => w.string(message),
            ServerMessage::Goto { object_id, x, y } => {
                w.varint(object_id.0 as u64);
                w.position(*x);
                w.position(*y);
            }
            ServerMessage::ShowEffect {
                effect,
                target,
                x1,
                y1,
                x2,
                y2,
                color,
            } => {
                w.u8(*effect);
                w.varint(target.0 as u64);

                // Plain floats rather than the quantised `position` the snapshot uses: half of
                // these are not positions at all. A radius, a particle size and a flash period all
                // travel here, and rounding a period of 0.4 seconds to the tile grid would lose it.
                w.f32(*x1);
                w.f32(*y1);
                w.f32(*x2);
                w.f32(*y2);
                w.varint(*color as u64);
            }
            ServerMessage::StatusText {
                object_id,
                text,
                color,
            } => {
                w.varint(object_id.0 as u64);
                w.string(text);
                w.varint(*color as u64);
            }
            ServerMessage::Aoe {
                x,
                y,
                radius,
                damage,
                effect,
                duration,
                orig_type,
            } => {
                w.position(*x);
                w.position(*y);
                w.f32(*radius);
                w.varint(*damage as u64);
                w.u8(*effect);
                w.f32(*duration);
                w.varint(*orig_type as u64);
            }
            ServerMessage::InvitedToGuild { name, guild } => {
                w.string(name);
                w.string(guild);
            }
            ServerMessage::SwitchMusic { music } => w.string(music),
            ServerMessage::QuestTarget { target } => w.varint(target.0 as u64),
            ServerMessage::SetFocus { target } => w.varint(target.0 as u64),
            ServerMessage::AccountList { list, names } => {
                w.u8(list.number() as u8);
                w.varint(names.len() as u64);
                for name in names {
                    w.string(name);
                }
            }
            ServerMessage::Damage {
                target,
                effects,
                amount,
                kill,
                bullet,
                owner,
            } => {
                w.varint(target.0 as u64);
                // Split in two because the effect set is a hundred and twenty-eight bits wide and
                // a varint carries sixty-four. Almost every hit carries none at all, so both halves
                // cost one byte each.
                w.varint(*effects as u64);
                w.varint((*effects >> 64) as u64);
                w.varint(*amount as u64);
                w.u8(*kill as u8);
                w.u8(*bullet);
                w.varint(owner.0 as u64);
            }
            ServerMessage::VaultUpdate {
                version,
                chest_count,
                max_chests,
                next_chest_price,
                slots,
                gifts,
            } => {
                w.varint(*version as u64);
                w.varint(*chest_count as u64);
                w.varint(*max_chests as u64);
                w.varint(*next_chest_price as u64);
                write_vault_slots(w, slots);
                write_vault_slots(w, gifts);
            }
        }
    }

    pub fn decode<'b>(r: &mut Reader<'b>) -> Result<ServerMessage<'b>, CodecError> {
        let id = r.u16()?;
        Ok(match id {
            server_id::WELCOME => ServerMessage::Welcome {
                player: EntityId(r.varint_u32()?),
                tick: Tick(r.varint_u32()?),
                world: r.string()?,
                width: r.varint_u32()? as u16,
                height: r.varint_u32()? as u16,
                background: r.varint_signed()? as i32,
                difficulty: r.varint_signed()? as i32,
                allow_teleport: r.bool()?,
                show_displays: r.bool()?,
                music: r.string()?,
            },
            server_id::REJECTED => {
                let code = r.u8()?;
                ServerMessage::Rejected {
                    reason: RejectReason::from_code(code).ok_or(CodecError::InvalidValue {
                        what: "reject reason",
                        value: code as u64,
                    })?,
                }
            }
            server_id::SNAPSHOT => ServerMessage::Snapshot { body: r.rest() },
            server_id::VAULT_UPDATE => ServerMessage::VaultUpdate {
                version: r.varint_u32()?,
                chest_count: r.varint_u32()?,
                max_chests: r.varint_u32()?,
                next_chest_price: r.varint_u32()?,
                slots: read_vault_slots(r)?,
                gifts: read_vault_slots(r)?,
            },
            server_id::CHAT => ServerMessage::Chat {
                speaker: EntityId(r.varint_u32()?),
                from: r.string()?,
                text: r.string()?,
            },
            server_id::PING => ServerMessage::Ping {
                serial: r.varint_u32()?,
            },
            server_id::SHOT => ServerMessage::Shot {
                projectile: EntityId(r.varint_u32()?),
                owner: EntityId(r.varint_u32()?),
                object_type: r.varint_u32()? as u16,
                x: r.position_value()?,
                y: r.position_value()?,
                angle: r.f32()?,
                speed: r.f32()?,
                lifetime_ms: r.varint_u32()?,
                damage: r.varint_u32()? as u16,
            },
            server_id::CONTAINER => {
                let code = r.u8()?;
                let container = ContainerId::from_code(code).ok_or(CodecError::InvalidValue {
                    what: "container",
                    value: code as u64,
                })?;

                // Decoded into the reader's own buffer would need an allocation, and this borrows
                // like everything else, so the caller reads the pairs itself.
                let count = r.count(crate::codec::MAX_SEQUENCE)?;
                let mut slots = Vec::with_capacity(count.min(256));
                for _ in 0..count {
                    slots.push((r.varint_u32()? as u16, r.varint_u32()? as u16));
                }
                ServerMessage::Container { container, slots }
            }
            server_id::REFUSED => ServerMessage::Refused {
                message: r.string()?,
            },
            server_id::TERRAIN => {
                let x = r.varint_u32()? as u16;
                let y = r.varint_u32()? as u16;
                let count = r.varint_u32()? as usize;

                let mut runs = Vec::with_capacity(count.min(MAX_TERRAIN_RUNS));
                for _ in 0..count {
                    runs.push((r.varint_u32()? as u16, r.varint_u32()? as u16));
                }
                ServerMessage::Terrain { x, y, runs }
            }
            server_id::QUEUED => ServerMessage::Queued {
                place: r.varint_u32()?,
                waiting: r.varint_u32()?,
            },
            server_id::STACKS => ServerMessage::Stacks {
                health: r.varint_u32()? as u16,
                magic: r.varint_u32()? as u16,
            },
            server_id::DIED => ServerMessage::Died {
                character: r.varint_u32()?,
                killed_by: r.string()?.to_string(),
                fame: r.varint_u32()? as i32,
            },
            server_id::NOTICE => ServerMessage::Notice {
                text: r.string()?.to_string(),
            },
            server_id::NOTIFICATION => ServerMessage::Notification {
                text: r.string()?.to_string(),
            },
            server_id::TRADE_REQUESTED => ServerMessage::TradeRequested {
                name: r.string()?.to_string(),
            },
            server_id::TRADE_START => ServerMessage::TradeStart {
                mine: read_slots(r)?,
                their_name: r.string()?.to_string(),
                theirs: read_slots(r)?,
            },
            server_id::TRADE_CHANGED => ServerMessage::TradeChanged {
                offer: read_offer(r)?,
            },
            server_id::TRADE_ACCEPTED => ServerMessage::TradeAccepted {
                mine: read_offer(r)?,
                theirs: read_offer(r)?,
            },
            server_id::TRADE_DONE => ServerMessage::TradeDone {
                code: r.varint_u32()?,
                message: r.string()?.to_string(),
            },

            server_id::SCENERY => {
                let y = r.varint_u32()? as u16;
                let count = r.varint_u32()? as usize;

                let mut objects = Vec::with_capacity(count.min(MAX_SCENERY));
                for _ in 0..count {
                    objects.push((
                        r.varint_u32()? as u16,
                        r.varint_u32()? as u16,
                        r.varint_u32()? as u16,
                    ));
                }
                ServerMessage::Scenery { y, objects }
            }
            server_id::GROUND => {
                let count = r.varint_u32()? as usize;
                let mut changes = Vec::with_capacity(count.min(MAX_GROUND_CHANGES));
                for _ in 0..count {
                    changes.push((
                        r.varint_u32()? as u16,
                        r.varint_u32()? as u16,
                        r.varint_u32()? as u16,
                    ));
                }
                ServerMessage::Ground { changes }
            }
            server_id::GOTO => ServerMessage::Goto {
                object_id: EntityId(r.varint_u32()?),
                x: r.position_value()?,
                y: r.position_value()?,
            },
            server_id::SHOW_EFFECT => ServerMessage::ShowEffect {
                effect: r.u8()?,
                target: EntityId(r.varint_u32()?),
                x1: r.f32()?,
                y1: r.f32()?,
                x2: r.f32()?,
                y2: r.f32()?,
                color: r.varint_u32()?,
            },
            server_id::DAMAGE => {
                let target = EntityId(r.varint_u32()?);
                let low = r.varint()?;
                let high = r.varint()?;
                ServerMessage::Damage {
                    target,
                    effects: (high as u128) << 64 | low as u128,
                    amount: r.varint_u32()? as u16,
                    kill: r.u8()? != 0,
                    bullet: r.u8()?,
                    owner: EntityId(r.varint_u32()?),
                }
            }
            server_id::STATUS_TEXT => ServerMessage::StatusText {
                object_id: EntityId(r.varint_u32()?),
                text: r.string()?.to_string(),
                color: r.varint_u32()?,
            },
            server_id::AOE => ServerMessage::Aoe {
                x: r.position_value()?,
                y: r.position_value()?,
                radius: r.f32()?,
                damage: r.varint_u32()? as u16,
                effect: r.u8()?,
                duration: r.f32()?,
                orig_type: r.varint_u32()? as u16,
            },
            server_id::INVITED_TO_GUILD => ServerMessage::InvitedToGuild {
                name: r.string()?.to_string(),
                guild: r.string()?.to_string(),
            },
            server_id::SWITCH_MUSIC => ServerMessage::SwitchMusic { music: r.string()? },
            server_id::QUEST_TARGET => ServerMessage::QuestTarget {
                target: EntityId(r.varint_u32()?),
            },
            server_id::SET_FOCUS => ServerMessage::SetFocus {
                target: EntityId(r.varint_u32()?),
            },
            server_id::ACCOUNT_LIST => {
                let code = r.u8()?;
                let list = AccountList::from_number(code as u32).ok_or(CodecError::InvalidValue {
                    what: "account list",
                    value: code as u64,
                })?;

                let count = r.count(crate::codec::MAX_SEQUENCE)?;
                let mut names = Vec::with_capacity(count.min(256));
                for _ in 0..count {
                    names.push(r.string()?.to_string());
                }
                ServerMessage::AccountList { list, names }
            }
            unknown => {
                return Err(CodecError::InvalidValue {
                    what: "server message id",
                    value: unknown as u64,
                });
            }
        })
    }
}

/// Writes the header a snapshot message needs, so the encoder can write its body straight after.
///
/// This exists so a snapshot never has to be encoded into a scratch buffer and copied into the
/// message: the tick's bytes are written once, into the buffer that goes to the transport.
pub fn begin_snapshot(w: &mut Writer<'_>) {
    w.u16(server_id::SNAPSHOT);
}

#[cfg(test)]
mod tests {
    #[test]
    fn ground_changes_survive_a_round_trip() {
        let changes = vec![(3u16, 4u16, 0x11u16), (5, 6, 0x12), (0, 0, 0)];

        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);
        let message = ServerMessage::Ground {
            changes: changes.clone(),
        };
        message.encode(&mut writer);

        let mut reader = Reader::new(&buffer);
        let read = ServerMessage::decode(&mut reader).unwrap();

        match read {
            ServerMessage::Ground { changes: back } => assert_eq!(back, changes),
            other => panic!("expected ground, got {other:?}"),
        }
    }

    #[test]
    fn a_ground_message_claiming_more_than_it_holds_is_refused() {
        // This checks the refusal, not the bound: `with_capacity` is capped separately so a
        // claimed million cannot reserve a million, and nothing in a test can observe that.
        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);
        writer.u16(server_id::GROUND);
        writer.varint(1_000_000);

        let mut reader = Reader::new(&buffer);
        assert!(ServerMessage::decode(&mut reader).is_err());
    }

    use super::*;

    fn round_trip_client(message: ClientMessage<'_>) -> Vec<u8> {
        let mut buf = Vec::new();
        message.encode(&mut Writer::new(&mut buf));

        let mut reader = Reader::new(&buf);
        let decoded = ClientMessage::decode(&mut reader).unwrap();
        assert_eq!(decoded, message);
        assert!(
            reader.is_empty(),
            "decoder left {} bytes",
            reader.remaining()
        );
        buf
    }

    fn round_trip_server(message: ServerMessage<'_>) {
        let mut buf = Vec::new();
        message.encode(&mut Writer::new(&mut buf));

        let mut reader = Reader::new(&buf);
        let decoded = ServerMessage::decode(&mut reader).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn the_vault_messages_round_trip() {
        // The three chest numbers that are not chests are negative, so they have to survive the
        // wire as negative: a truncating read turns "the gifts" into chest sixty-five thousand.
        round_trip_client(ClientMessage::VaultMove {
            version: 7,
            from_chest: vault_chest::GIFTS,
            from_slot: 1,
            to_chest: vault_chest::PLAYER,
            to_slot: 12,
        });
        round_trip_client(ClientMessage::VaultMove {
            version: 0,
            from_chest: 3,
            from_slot: 7,
            to_chest: vault_chest::STACKS,
            to_slot: 1,
        });
        round_trip_client(ClientMessage::VaultBuy { chest_count: 4 });

        round_trip_server(ServerMessage::VaultUpdate {
            version: 12,
            chest_count: 2,
            max_chests: 40,
            next_chest_price: 400,
            // Object type zero is a real type, so an empty slot has to be told apart from one
            // holding it by something other than the number.
            slots: vec![
                0,
                VAULT_SLOT_EMPTY,
                0x0dc2,
                VAULT_SLOT_EMPTY,
                VAULT_SLOT_EMPTY,
                VAULT_SLOT_EMPTY,
                VAULT_SLOT_EMPTY,
                VAULT_SLOT_EMPTY,
            ],
            gifts: vec![0x0a22],
        });
        round_trip_server(ServerMessage::VaultUpdate {
            version: 0,
            chest_count: 0,
            max_chests: 40,
            next_chest_price: 400,
            slots: Vec::new(),
            gifts: Vec::new(),
        });
    }

    #[test]
    fn a_vault_index_too_wide_to_be_a_chest_is_refused() {
        // Truncation would turn a number nobody can reach into one they can.
        let mut buf = Vec::new();
        let mut w = Writer::new(&mut buf);
        w.u16(client_id::VAULT_MOVE);
        w.varint(0);
        w.varint_signed(i32::MAX as i64);
        w.varint_signed(0);
        w.varint_signed(0);
        w.varint_signed(0);

        assert!(ClientMessage::decode(&mut Reader::new(&buf)).is_err());
    }

    #[test]
    fn the_trade_messages_round_trip() {
        round_trip_client(ClientMessage::RequestTrade { name: "Ana" });
        round_trip_client(ClientMessage::ChangeTrade {
            offer: vec![false, false, false, false, true, false, true, false],
        });
        round_trip_client(ClientMessage::AcceptTrade {
            mine: vec![false, true],
            theirs: vec![true, false, true],
        });
        round_trip_client(ClientMessage::CancelTrade);

        round_trip_server(ServerMessage::TradeRequested {
            name: "Bo".to_string(),
        });
        round_trip_server(ServerMessage::TradeStart {
            mine: vec![
                TradeSlot {
                    item: None,
                    slot_type: 0,
                    included: false,
                    tradeable: false,
                },
                // Object type zero is a real type, so an empty slot has to be told apart from one
                // holding it by something other than the number.
                TradeSlot {
                    item: Some(0),
                    slot_type: 3,
                    included: true,
                    tradeable: true,
                },
                TradeSlot {
                    item: Some(0x0dc2),
                    slot_type: 8,
                    included: false,
                    tradeable: true,
                },
            ],
            their_name: "Bo".to_string(),
            theirs: Vec::new(),
        });
        round_trip_server(ServerMessage::TradeChanged {
            offer: vec![true, false],
        });
        round_trip_server(ServerMessage::TradeAccepted {
            mine: vec![true],
            theirs: vec![false, true],
        });
        round_trip_server(ServerMessage::Stacks {
            health: 6,
            magic: 0,
        });
        round_trip_server(ServerMessage::Died {
            character: 7,
            killed_by: "Slime".to_string(),
            fame: 421,
        });
        round_trip_server(ServerMessage::Notice {
            text: "Purple Key has been found.".to_string(),
        });

        // A word the client acts on rather than reads, and the one the original follows every
        // market withdrawal and every gifted purchase with.
        round_trip_server(ServerMessage::Notification {
            text: "giftChestOccupied".to_string(),
        });
        round_trip_server(ServerMessage::TradeDone {
            code: 0,
            message: "Trade successful.".to_string(),
        });
        round_trip_server(ServerMessage::Damage {
            target: EntityId(4096),
            effects: 0,
            amount: 71,
            kill: false,
            bullet: 3,
            owner: EntityId(12),
        });

        // The high half of the effect set is the one a lazy encoder drops: `Curse` is bit 37 and
        // fits in a u64, while anything past sixty-three does not.
        round_trip_server(ServerMessage::Damage {
            target: EntityId(1),
            effects: (1u128 << 100) | (1u128 << 37),
            amount: 65_535,
            kill: true,
            bullet: 255,
            owner: EntityId(0),
        });
    }

    #[test]
    fn a_trade_message_claiming_more_slots_than_it_carries_is_refused() {
        // The length is attacker-controlled, so the capacity is bounded before anything is reserved
        // and a short body is an error rather than a silent truncation.
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.u16(client_id::CHANGE_TRADE);
            w.varint(1_000_000);
        }

        let mut reader = Reader::new(&buf);
        assert!(ClientMessage::decode(&mut reader).is_err());
    }

    #[test]
    fn every_client_message_round_trips() {
        round_trip_client(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            token: "eyJhbGciOiJIUzI1NiJ9.session",
            character: 4,
        });
        round_trip_client(ClientMessage::Chat {
            text: "hello from the vault",
        });
        round_trip_client(ClientMessage::UsePortal {
            entity: EntityId(90_210),
        });
        round_trip_client(ClientMessage::Pong {
            serial: 77,
            client_time_ms: 41_000,
        });
        round_trip_client(ClientMessage::Shoot {
            angle: 1.25,
            client_time_ms: 900_000,
        });

        // Out of the player's own pack, and out of a bag standing in the world. Both, because the
        // container is the half a slot number cannot carry: the original names an object id
        // alongside the slot id (`UseItemHandler.cs:22`) and that is what addresses the bag.
        round_trip_client(ClientMessage::UseItem {
            container: EntityId(0),
            slot: 9,
            x: 128.5,
            y: 64.25,
        });
        round_trip_client(ClientMessage::UseItem {
            container: EntityId(70_120),
            slot: 0,
            x: 1024.75,
            y: 8.0,
        });
        round_trip_client(ClientMessage::MoveItem {
            from: SlotLocation::Inventory { slot: 3 },
            to: SlotLocation::Vault { slot: 200 },
        });
        round_trip_client(ClientMessage::MoveItem {
            from: SlotLocation::Inventory { slot: 2 },
            to: SlotLocation::Ground,
        });
        round_trip_client(ClientMessage::MoveItem {
            from: SlotLocation::Bag {
                entity: EntityId(65_555),
                slot: 1,
            },
            to: SlotLocation::Equipment { slot: 0 },
        });
    }

    #[test]
    fn every_server_message_round_trips() {
        round_trip_server(ServerMessage::Welcome {
            player: EntityId(1234),
            tick: Tick(9),
            world: "Nexus",
            width: 64,
            height: 64,
            background: 3,
            difficulty: -1,
            allow_teleport: false,
            show_displays: true,
            music: "Nexus",
        });
        round_trip_server(ServerMessage::Rejected {
            reason: RejectReason::BadToken,
        });
        round_trip_server(ServerMessage::Chat {
            speaker: EntityId(7),
            from: "Fesal",
            text: "the chest is open",
        });
        round_trip_server(ServerMessage::Ping { serial: 3 });
        round_trip_server(ServerMessage::Container {
            container: ContainerId::Vault,
            slots: vec![(0, 0x900), (7, 0x901), (199, 0x902)],
        });
        round_trip_server(ServerMessage::Container {
            container: ContainerId::Inventory,
            slots: Vec::new(),
        });
        round_trip_server(ServerMessage::Refused {
            message: "that item is no longer where you left it",
        });
        round_trip_server(ServerMessage::Shot {
            projectile: EntityId(65_555),
            owner: EntityId(19),
            object_type: 0x0900,
            x: 103.5,
            y: 88.25,
            angle: -0.75,
            speed: 10.0,
            lifetime_ms: 2000,
            damage: 130,
        });
        round_trip_server(ServerMessage::Goto {
            object_id: EntityId(19),
            x: 965.5,
            y: 267.5,
        });
        round_trip_server(ServerMessage::Snapshot {
            body: &[1, 2, 3, 4, 5],
        });
        round_trip_server(ServerMessage::ShowEffect {
            effect: effect::LIGHTNING,
            target: EntityId(19),
            x1: 106.375,
            y1: 84.5,
            x2: 5.0,
            y2: 0.0,
            color: 0xffff_e9a0,
        });
    }

    /// An effect's positions are not always positions, so they may not be rounded to the tile grid
    /// the way a body's are.
    ///
    /// `Pos1` carries a flash period in seconds (`Structures.cs:149`) and `Pos2` a particle size
    /// (`:145`). A quarter-second period quantised to eighths of a tile is a different flash, and a
    /// vault chest's five-unit bolt rounded to zero is no bolt at all.
    #[test]
    fn an_effect_keeps_the_arguments_that_are_not_coordinates() {
        let message = ServerMessage::ShowEffect {
            effect: effect::EARTHQUAKE,
            target: EntityId(0),

            // A flashing period and a cycle count, which is what `Pos1` means for effect fifteen.
            x1: 0.4,
            y1: 3.0,
            x2: 350.0,
            y2: 0.0,
            color: 0xff00_0088,
        };

        let mut buf = Vec::new();
        message.encode(&mut Writer::new(&mut buf));
        let decoded = ServerMessage::decode(&mut Reader::new(&buf)).unwrap();

        let ServerMessage::ShowEffect { x1, y1, x2, color, .. } = decoded else {
            panic!("an effect decoded as something else");
        };

        assert_eq!(x1, 0.4, "a fractional period may not be rounded to the grid");
        assert_eq!(y1, 3.0);
        assert_eq!(x2, 350.0, "a particle size is not a coordinate");
        assert_eq!(color, 0xff00_0088, "the whole ARGB survives, alpha included");
    }

    /// The three things the player is told about themselves survive the wire.
    ///
    /// A status text's colour is what separates a heal from a mana refill from a fame gain
    /// (`Player.UseItem.cs:1257`, `:1281`, `Player.Leveling.cs:258`), so an alpha byte lost on the
    /// way would turn every float green. A blast's radius decides where the ring is drawn and its
    /// duration how long the condition it leaves lasts, and neither is a coordinate.
    #[test]
    fn what_is_said_about_one_player_survives_the_wire() {
        round_trip_server(ServerMessage::StatusText {
            object_id: EntityId(4321),
            text: "+45".to_string(),
            color: 0xff00_ff00,
        });
        round_trip_server(ServerMessage::StatusText {
            object_id: EntityId(1),
            text: "{\"key\":\"server.quest_complete\"}".to_string(),
            color: 0xff00_ff00,
        });
        round_trip_server(ServerMessage::InvitedToGuild {
            name: "Hendra".to_string(),
            guild: "The Deep".to_string(),
        });
        round_trip_server(ServerMessage::SwitchMusic { music: "Deep" });
        round_trip_server(ServerMessage::QuestTarget {
            target: EntityId(4211),
        });
        round_trip_server(ServerMessage::SetFocus {
            target: EntityId(9),
        });
        round_trip_server(ServerMessage::AccountList {
            list: AccountList::Locked,
            names: vec!["Hendra".to_string(), "Fesal".to_string()],
        });
        round_trip_server(ServerMessage::AccountList {
            list: AccountList::Ignored,
            names: Vec::new(),
        });

        let message = ServerMessage::Aoe {
            x: 103.5,
            y: 88.25,
            radius: 3.5,
            damage: 250,
            effect: 5,
            duration: 2.5,
            orig_type: 0x0d5b,
        };
        let mut buf = Vec::new();
        message.encode(&mut Writer::new(&mut buf));
        let ServerMessage::Aoe {
            radius, duration, ..
        } = ServerMessage::decode(&mut Reader::new(&buf)).unwrap()
        else {
            panic!("a blast decoded as something else");
        };
        assert_eq!(radius, 3.5, "a radius is not a coordinate");
        assert_eq!(duration, 2.5, "nor is a duration in seconds");
        round_trip_server(message);
    }

    #[test]
    fn an_input_message_is_small_and_allocation_free() {
        let input = Input {
            ack: Acknowledgement::of(Tick(4_000)),
            client_time_ms: 1_234_567,
            x: 103.5,
            y: 88.25,
        };

        let bytes = round_trip_client(ClientMessage::Input(input));

        // Two bytes of id, then the acknowledgement, clock and two coordinates. This travels
        // twenty times a second per player, so its size is the one that compounds.
        assert!(bytes.len() <= 16, "{} bytes for one input", bytes.len());
    }

    #[test]
    fn an_unknown_slot_location_is_refused() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.u16(client_id::MOVE_ITEM);
            w.u8(9); // no such container
            w.u8(0);
        }
        assert!(ClientMessage::decode(&mut Reader::new(&buf)).is_err());
    }

    #[test]
    fn a_player_addresses_its_own_containers_by_tag_not_by_entity() {
        // A client naming its own inventory by entity id could name someone else's; only bags,
        // which belong to the world rather than to a player, are addressed that way.
        let mut buf = Vec::new();
        ClientMessage::MoveItem {
            from: SlotLocation::Inventory { slot: 0 },
            to: SlotLocation::Vault { slot: 0 },
        }
        .encode(&mut Writer::new(&mut buf));

        // Two bytes of id, then a tag and a slot for each end.
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn an_unknown_message_id_is_an_error_not_a_misparse() {
        let mut buf = Vec::new();
        Writer::new(&mut buf).u16(0x7fff);

        assert!(matches!(
            ClientMessage::decode(&mut Reader::new(&buf)),
            Err(CodecError::InvalidValue {
                what: "client message id",
                ..
            })
        ));
    }

    #[test]
    fn a_server_message_cannot_be_decoded_as_a_client_one() {
        // The separate id ranges exist for this: a misrouted message fails at once.
        let mut buf = Vec::new();
        ServerMessage::Ping { serial: 1 }.encode(&mut Writer::new(&mut buf));

        assert!(ClientMessage::decode(&mut Reader::new(&buf)).is_err());
    }

    #[test]
    fn an_unknown_reject_reason_is_refused() {
        let mut buf = Vec::new();
        {
            let mut w = Writer::new(&mut buf);
            w.u16(server_id::REJECTED);
            w.u8(200);
        }
        assert!(ServerMessage::decode(&mut Reader::new(&buf)).is_err());
    }

    #[test]
    fn a_snapshot_message_hands_back_its_body_without_copying() {
        let body = [9u8, 8, 7, 6];
        let mut buf = Vec::new();
        begin_snapshot(&mut Writer::new(&mut buf));
        buf.extend_from_slice(&body);

        match ServerMessage::decode(&mut Reader::new(&buf)).unwrap() {
            ServerMessage::Snapshot { body: seen } => assert_eq!(seen, &body),
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_message_errors_rather_than_panicking() {
        let mut buf = Vec::new();
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            token: "a token of some length",
            character: 2,
        }
        .encode(&mut Writer::new(&mut buf));

        for cut in 0..buf.len() {
            let mut reader = Reader::new(&buf[..cut]);
            let _ = ClientMessage::decode(&mut reader);
        }
    }

    #[test]
    fn junk_never_panics_either_direction() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..2_000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = (seed % 40) as usize;
            let junk: Vec<u8> = (0..len).map(|i| (seed >> (i % 8 * 8)) as u8).collect();

            let _ = ClientMessage::decode(&mut Reader::new(&junk));
            let _ = ServerMessage::decode(&mut Reader::new(&junk));
        }
    }
}
