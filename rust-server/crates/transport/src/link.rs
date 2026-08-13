//! One connection, and the two ways a payload can travel along it.
//!
//! # Why reader tasks rather than a select
//!
//! The obvious shape is a `select!` over "next stream frame" and "next datagram". It is wrong.
//! Reading a length-prefixed frame means reading four bytes and then a body, and `select!` drops
//! the losing future — so a datagram arriving mid-frame would discard a read that had already
//! consumed bytes from the stream. QUIC streams are byte streams with no resynchronisation point,
//! so those bytes are simply gone and every frame after them is garbage.
//!
//! Instead each source is drained by its own task and both feed one queue. Taking from a queue is
//! cancel-safe, the framing never sits inside a `select!`, and the receiving end becomes a channel
//! read — which is also what the Godot client wants, since it polls once per frame rather than
//! awaiting.

use std::sync::Arc;

use bytes::Bytes;
use hendra_net::Delivery;
use tokio::sync::mpsc;

use crate::TransportError;

/// The largest reliable frame the transport will read.
///
/// Reliable messages are inventory operations, chat and world joins; none approaches this. The
/// limit exists so a hostile length prefix cannot make us reserve memory on demand.
pub const MAX_FRAME: usize = 256 * 1024;

/// How many reliable payloads may queue before a sender has to wait.
///
/// Deep enough to absorb a legitimate burst — joining a world sends a run of messages back to back
/// — and shallow enough that a peer which has genuinely stopped reading is noticed in well under a
/// second rather than after megabytes have piled up in memory.
const SEND_QUEUE: usize = 64;

/// How many received payloads may queue before the reader tasks stall.
const RECEIVE_QUEUE: usize = 256;

/// Something the peer sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    /// Arrived on the stream: ordered with respect to other reliable payloads, and guaranteed.
    Reliable(Vec<u8>),

    /// Arrived as a datagram: possibly out of order, possibly never.
    Unreliable(Vec<u8>),
}

impl Received {
    pub fn payload(&self) -> &[u8] {
        match self {
            Received::Reliable(bytes) | Received::Unreliable(bytes) => bytes,
        }
    }

    pub fn into_payload(self) -> Vec<u8> {
        match self {
            Received::Reliable(bytes) | Received::Unreliable(bytes) => bytes,
        }
    }
}

/// The sending half of a connection, which can be cloned and moved elsewhere.
///
/// A world owns its players' senders directly and writes snapshots to them from the tick, rather
/// than passing bytes through another queue to whichever task owns the connection. That saves an
/// allocation and a hop per player per tick, which at two hundred players and twenty ticks a second
/// is four thousand of each per second.
#[derive(Clone)]
pub struct LinkSender {
    connection: quinn::Connection,
    outbound: mpsc::Sender<Vec<u8>>,
}

impl LinkSender {
    /// Sends a payload by the route its [`Delivery`] names, waiting for room if there is none.
    pub async fn send(&self, delivery: Delivery, payload: &[u8]) -> Result<(), TransportError> {
        match delivery {
            Delivery::Datagram => send_datagram(&self.connection, payload),
            Delivery::Stream => self
                .outbound
                .send(payload.to_vec())
                .await
                .map_err(|_| TransportError::Closed),
        }
    }

    /// Sends without waiting, reporting [`TransportError::Backlogged`] if the queue is full.
    pub fn try_send(&self, delivery: Delivery, payload: &[u8]) -> Result<(), TransportError> {
        match delivery {
            Delivery::Datagram => send_datagram(&self.connection, payload),
            Delivery::Stream => {
                self.outbound
                    .try_send(payload.to_vec())
                    .map_err(|source| match source {
                        mpsc::error::TrySendError::Full(_) => TransportError::Backlogged,
                        mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
                    })
            }
        }
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.connection.max_datagram_size()
    }

    pub fn rtt(&self) -> std::time::Duration {
        self.connection.rtt()
    }

    pub fn remote_address(&self) -> std::net::SocketAddr {
        self.connection.remote_address()
    }

    /// Whether the peer has gone.
    pub fn is_closed(&self) -> bool {
        self.connection.close_reason().is_some()
    }

    pub fn close(&self, reason: &str) {
        self.connection.close(0u32.into(), reason.as_bytes());
    }
}

/// Datagrams never queue, so every send path shares this.
fn send_datagram(connection: &quinn::Connection, payload: &[u8]) -> Result<(), TransportError> {
    if let Some(limit) = connection.max_datagram_size()
        && payload.len() > limit
    {
        return Err(TransportError::DatagramTooLarge {
            len: payload.len(),
            limit,
        });
    }
    connection
        .send_datagram(Bytes::copy_from_slice(payload))
        .map_err(|source| TransportError::Send(source.to_string()))
}

/// An open connection to a peer.
pub struct Link {
    connection: quinn::Connection,
    outbound: mpsc::Sender<Vec<u8>>,
    inbound: mpsc::Receiver<Received>,
}

impl std::fmt::Debug for Link {
    /// Shows who is on the other end and how far away they are — the two things worth seeing in a
    /// log line about a connection.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("peer", &self.connection.remote_address())
            .field("rtt", &self.connection.rtt())
            .field("queued", &(SEND_QUEUE - self.outbound.capacity()))
            .finish()
    }
}

impl Link {
    /// Wraps an established QUIC connection and its control stream, starting the pump tasks.
    pub(crate) fn start(
        connection: quinn::Connection,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) -> Link {
        let (outbound, mut to_write) = mpsc::channel::<Vec<u8>>(SEND_QUEUE);
        let (delivered, inbound) = mpsc::channel::<Received>(RECEIVE_QUEUE);

        // Writer: serialises reliable payloads onto the stream, length-prefixed.
        tokio::spawn(async move {
            while let Some(payload) = to_write.recv().await {
                let header = (payload.len() as u32).to_le_bytes();
                if send.write_all(&header).await.is_err() || send.write_all(&payload).await.is_err()
                {
                    break;
                }
            }
            let _ = send.finish();
        });

        // Stream reader.
        let stream_side = delivered.clone();
        tokio::spawn(async move {
            loop {
                let mut header = [0u8; 4];
                if recv.read_exact(&mut header).await.is_err() {
                    break;
                }
                let len = u32::from_le_bytes(header) as usize;
                if len > MAX_FRAME {
                    tracing::warn!(len, "reliable frame exceeds the limit; closing the stream");
                    break;
                }

                // The client writes an empty frame to bring the control stream into existence, and
                // no real message is zero bytes. Dropping it here keeps that handshake detail out
                // of everything downstream.
                if len == 0 {
                    continue;
                }

                let mut body = vec![0u8; len];
                if recv.read_exact(&mut body).await.is_err() {
                    break;
                }
                if stream_side.send(Received::Reliable(body)).await.is_err() {
                    break;
                }
            }
        });

        // Datagram reader.
        let datagram_side = delivered;
        let datagram_connection = connection.clone();
        tokio::spawn(async move {
            while let Ok(datagram) = datagram_connection.read_datagram().await {
                if datagram_side
                    .send(Received::Unreliable(datagram.to_vec()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        Link {
            connection,
            outbound,
            inbound,
        }
    }

    /// Sends a payload by the route its [`Delivery`] names, waiting for room if there is none.
    ///
    /// Reliable payloads queue; unreliable ones go straight out, because queueing a snapshot only
    /// makes it staler. This is what the server uses: a client that has fallen behind should slow
    /// the sender down, not have its inventory updates thrown away.
    pub async fn send(&self, delivery: Delivery, payload: &[u8]) -> Result<(), TransportError> {
        match delivery {
            Delivery::Datagram => self.send_datagram(payload),
            Delivery::Stream => self
                .outbound
                .send(payload.to_vec())
                .await
                .map_err(|_| TransportError::Closed),
        }
    }

    /// Sends without waiting, reporting [`TransportError::Backlogged`] if the queue is full.
    ///
    /// This is what the Godot client uses. It runs inside a frame and cannot block, so a backlog
    /// has to come back as a value it can act on rather than a stall the player would feel.
    pub fn try_send(&self, delivery: Delivery, payload: &[u8]) -> Result<(), TransportError> {
        match delivery {
            Delivery::Datagram => self.send_datagram(payload),
            Delivery::Stream => {
                self.outbound
                    .try_send(payload.to_vec())
                    .map_err(|source| match source {
                        mpsc::error::TrySendError::Full(_) => TransportError::Backlogged,
                        mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
                    })
            }
        }
    }

    /// A cloneable handle for sending, so another task can write to this peer.
    ///
    /// The world holds one of these per player and writes snapshots from the tick itself.
    pub fn sender(&self) -> LinkSender {
        LinkSender {
            connection: self.connection.clone(),
            outbound: self.outbound.clone(),
        }
    }

    fn send_datagram(&self, payload: &[u8]) -> Result<(), TransportError> {
        send_datagram(&self.connection, payload)
    }

    /// Waits for the next payload from the peer, or `None` once the connection is finished.
    pub async fn recv(&mut self) -> Option<Received> {
        self.inbound.recv().await
    }

    /// Takes a payload if one is already waiting, without blocking.
    ///
    /// This is what the Godot client calls once per frame.
    pub fn try_recv(&mut self) -> Option<Received> {
        self.inbound.try_recv().ok()
    }

    /// The largest datagram this path currently accepts, if the peer allows datagrams at all.
    ///
    /// Worth checking rather than assuming: it is negotiated, and it shrinks if the path turns out
    /// to have a smaller MTU than the initial estimate.
    pub fn max_datagram_size(&self) -> Option<usize> {
        self.connection.max_datagram_size()
    }

    pub fn remote_address(&self) -> std::net::SocketAddr {
        self.connection.remote_address()
    }

    /// Round-trip time as QUIC currently estimates it.
    pub fn rtt(&self) -> std::time::Duration {
        self.connection.rtt()
    }

    /// Closes the connection, telling the peer why.
    pub fn close(&self, reason: &str) {
        self.connection.close(0u32.into(), reason.as_bytes());
    }
}

/// Shared configuration for both ends.
pub(crate) fn transport_config() -> Arc<quinn::TransportConfig> {
    let mut config = quinn::TransportConfig::default();

    // A player whose connection has genuinely gone should be released promptly, but not so promptly
    // that a slow mobile handover looks like a disconnect.
    config.max_idle_timeout(Some(
        std::time::Duration::from_secs(20)
            .try_into()
            .expect("20s is a valid idle timeout"),
    ));
    config.keep_alive_interval(Some(std::time::Duration::from_secs(5)));

    // The game opens exactly one bidirectional stream and never more, so anything beyond that is a
    // peer doing something it should not.
    config.max_concurrent_bidi_streams(1u8.into());
    config.max_concurrent_uni_streams(0u8.into());

    Arc::new(config)
}
