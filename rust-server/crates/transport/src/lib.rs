//! QUIC transport for the Hendra protocol.
//!
//! The game has two kinds of traffic with opposite requirements, and QUIC is the reason they no
//! longer have to share a pipe:
//!
//! ```text
//!   stream     session, world join, inventory, vault, trade, chat
//!              ordered and retransmitted; losing one is a correctness bug
//!
//!   datagram   snapshots and player input
//!              congestion-controlled but never retransmitted, because a tick
//!              that arrives late is worse than one that never arrives
//! ```
//!
//! [`hendra_net::Delivery`] decides which path a payload takes, so the choice is made where the
//! payload is built and not guessed here. TLS 1.3 comes with the protocol rather than being layered
//! on, which is what lets the legacy RC4 disappear without anything replacing it.

pub mod endpoint;
pub mod link;
pub mod tls;

use std::net::SocketAddr;

pub use endpoint::{Listener, connect};
pub use link::{Link, MAX_FRAME, Received};
pub use tls::{ALPN, ServerIdentity, Trust};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("loading identity from {path}: {detail}")]
    Identity { path: String, detail: String },

    #[error("binding {address}: {detail}")]
    Bind { address: SocketAddr, detail: String },

    #[error("configuring the endpoint: {0}")]
    Configuration(String),

    #[error("handshake failed: {0}")]
    Handshake(String),

    #[error("sending: {0}")]
    Send(String),

    /// The payload exceeds what this path will carry in one datagram.
    ///
    /// The caller has usually got the routing wrong — anything this large belongs on the stream,
    /// which is why [`hendra_net::Delivery`] exists.
    #[error("datagram of {len} bytes exceeds the {limit}-byte path limit")]
    DatagramTooLarge { len: usize, limit: usize },

    /// The reliable queue is full: the peer has stopped keeping up.
    #[error("the peer is not keeping up with reliable traffic")]
    Backlogged,

    #[error("the connection is closed")]
    Closed,
}
