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
}

/// Messages travelling from server to client.
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
        }
    }

    fn encode(&self, w: &mut Writer<'_>) {
        w.u8(self.tag());
        match self {
            SlotLocation::Inventory { slot } | SlotLocation::Equipment { slot } => w.u8(*slot),
            SlotLocation::Vault { slot } => w.varint(*slot as u64),
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
}

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
        }
    }

    pub fn decode<'b>(r: &mut Reader<'b>) -> Result<ClientMessage<'b>, CodecError> {
        let id = r.u16()?;
        Ok(match id {
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
