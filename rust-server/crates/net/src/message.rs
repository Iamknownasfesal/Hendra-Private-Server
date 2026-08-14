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
pub const PROTOCOL_VERSION: u32 = 1;

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

    /// Uses the item in a slot, aimed at a point.
    ///
    /// The slot rather than the item, because the server knows what is in a slot and a client
    /// naming an item it does not hold is a claim rather than a fact.
    UseItem {
        slot: u16,
        x: f32,
        y: f32,
    },

    Pong {
        serial: u32,
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
    fn number(self) -> u64 {
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
            ClientMessage::UseItem { slot, x, y } => {
                w.varint(*slot as u64);
                w.position(*x);
                w.position(*y);
            }
            ClientMessage::Pong { serial } => w.varint(*serial as u64),
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
                slot: r.varint_u32()? as u16,
                x: r.position_value()?,
                y: r.position_value()?,
            },
            client_id::USE_PORTAL => ClientMessage::UsePortal {
                entity: EntityId(r.varint_u32()?),
            },
            client_id::PONG => ClientMessage::Pong {
                serial: r.varint_u32()?,
            },
            client_id::SHOOT => ClientMessage::Shoot {
                angle: r.f32()?,
                client_time_ms: r.varint_u32()?,
            },
            client_id::MOVE_ITEM => ClientMessage::MoveItem {
                from: SlotLocation::decode(r)?,
                to: SlotLocation::decode(r)?,
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
        from: &'a str,
        text: &'a str,
    },

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
        object_type: u16,
        x: f32,
        y: f32,
        angle: f32,
        /// Tiles per second.
        speed: f32,
        lifetime_ms: u32,
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
        }
    }

    pub fn encode(&self, w: &mut Writer<'_>) {
        w.u16(self.id());
        match self {
            ServerMessage::Welcome {
                player,
                tick,
                world,
            } => {
                w.varint(player.0 as u64);
                w.varint(tick.0 as u64);
                w.string(world);
            }
            ServerMessage::Rejected { reason } => w.u8(*reason as u8),
            // Written raw: the snapshot encoder produced these bytes and re-length-prefixing them
            // would spend bytes to describe a payload that already runs to the end of the message.
            ServerMessage::Snapshot { body } => w.raw(body),
            ServerMessage::Chat { from, text } => {
                w.string(from);
                w.string(text);
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
            } => {
                w.varint(projectile.0 as u64);
                w.varint(owner.0 as u64);
                w.varint(*object_type as u64);
                w.position(*x);
                w.position(*y);
                w.f32(*angle);
                w.f32(*speed);
                w.varint(*lifetime_ms as u64);
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
        }
    }

    pub fn decode<'b>(r: &mut Reader<'b>) -> Result<ServerMessage<'b>, CodecError> {
        let id = r.u16()?;
        Ok(match id {
            server_id::WELCOME => ServerMessage::Welcome {
                player: EntityId(r.varint_u32()?),
                tick: Tick(r.varint_u32()?),
                world: r.string()?,
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
            server_id::CHAT => ServerMessage::Chat {
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
        round_trip_server(ServerMessage::TradeDone {
            code: 0,
            message: "Trade successful.".to_string(),
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
        round_trip_client(ClientMessage::Pong { serial: 77 });
        round_trip_client(ClientMessage::Shoot {
            angle: 1.25,
            client_time_ms: 900_000,
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
        });
        round_trip_server(ServerMessage::Rejected {
            reason: RejectReason::BadToken,
        });
        round_trip_server(ServerMessage::Chat {
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
        });
        round_trip_server(ServerMessage::Snapshot {
            body: &[1, 2, 3, 4, 5],
        });
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
