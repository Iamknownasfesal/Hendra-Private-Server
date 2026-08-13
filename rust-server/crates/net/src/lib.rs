//! The Hendra network protocol.
//!
//! Both ends run this crate: the dedicated server links it directly, and the Godot client loads it
//! through a GDExtension. That is the reason it exists as its own crate. An encoder and its decoder
//! cannot disagree about a field's width or a coordinate's scale when there is exactly one of each,
//! and the failure this prevents, a protocol change applied to one side and forgotten on the
//! other, is silent, intermittent and expensive to find.
//!
//! The same property makes the test suite worth trusting. A round-trip test here exercises the code
//! that actually runs in production on both sides, rather than one implementation's idea of what the
//! other one does.
//!
//! # Snapshots are deltas against what the client confirms it holds
//!
//! [`codec`] is the byte-level encoding. [`snapshot`] is the rule that decides what a delta may be
//! measured from.
//!
//! Snapshots travel as unreliable datagrams, so "diff against what I sent last" is unsound: a
//! dropped packet would leave the two sides measuring from different baselines, and every
//! subsequent delta would compound the error. The client therefore reports the newest tick it
//! actually holds, and the server encodes against that. Packet loss costs one larger snapshot and
//! nothing else.

pub mod codec;
pub mod entity;
pub mod message;
pub mod snapshot;
pub mod world;

pub use codec::{
    CodecError, POSITION_SCALE, Reader, Writer, dequantize, quantize, unzigzag, zigzag,
};
pub use entity::{EntityId, EntityState, FieldMask};
pub use message::{
    ClientMessage, Input, PROTOCOL_VERSION, RejectReason, ServerMessage, begin_snapshot,
};
pub use snapshot::{Acknowledgement, Baseline, BaselineRing, SNAPSHOT_HISTORY, Tick};
pub use world::{
    DATAGRAM_BUDGET, Delivery, SnapshotEncoder, SnapshotHeader, WorldSnapshot, decode_body,
    decode_snapshot, read_header,
};
