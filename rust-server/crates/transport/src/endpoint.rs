//! Binding a listener, and dialling one.
//!
//! Both ends agree on a convention: the client opens the single bidirectional stream immediately
//! after the handshake, and the server accepts it. Doing this during connection setup rather than
//! lazily means a [`Link`] is only ever handed out once both delivery paths are usable, so nothing
//! downstream has to handle a half-ready connection.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::link::{Link, transport_config};
use crate::tls::{ALPN, ServerIdentity, Trust};
use crate::TransportError;

/// Accepts incoming connections.
pub struct Listener {
    endpoint: quinn::Endpoint,
}

impl Listener {
    /// Binds to an address, presenting `identity` to anyone who connects.
    pub fn bind(address: SocketAddr, identity: ServerIdentity) -> Result<Listener, TransportError> {
        if identity.chain_len() == 1 {
            // Not fatal — a self-signed development certificate is legitimately one deep — but for
            // a real certificate it means the issuer was left out, and the failure it causes shows
            // up as an opaque handshake rejection on clients rather than anything pointing here.
            tracing::warn!(
                "certificate chain is one deep; if this is a CA-issued certificate, serve \
                 fullchain.pem rather than cert.pem"
            );
        }

        let mut tls = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(identity.chain, identity.key)
            .map_err(|source| TransportError::Identity {
                path: "<configured>".into(),
                detail: source.to_string(),
            })?;
        tls.alpn_protocols = vec![ALPN.to_vec()];

        let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|source| TransportError::Configuration(source.to_string()))?;

        let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        config.transport_config(transport_config());

        let endpoint = quinn::Endpoint::server(config, address)
            .map_err(|source| TransportError::Bind {
                address,
                detail: source.to_string(),
            })?;

        Ok(Listener { endpoint })
    }

    /// The address actually bound, which differs from the requested one when port 0 was asked for.
    pub fn local_address(&self) -> Result<SocketAddr, TransportError> {
        self.endpoint
            .local_addr()
            .map_err(|source| TransportError::Configuration(source.to_string()))
    }

    /// Waits for the next fully established connection.
    ///
    /// Returns `None` when the endpoint is closed. A connection that fails during setup is reported
    /// as an error and the listener stays open — one peer failing a handshake is not a reason to
    /// stop serving everyone else.
    pub async fn accept(&self) -> Option<Result<Link, TransportError>> {
        let incoming = self.endpoint.accept().await?;

        Some(async move {
            let connection = incoming
                .await
                .map_err(|source| TransportError::Handshake(source.to_string()))?;

            let (send, recv) = connection
                .accept_bi()
                .await
                .map_err(|source| TransportError::Handshake(source.to_string()))?;

            Ok(Link::start(connection, send, recv))
        }
        .await)
    }

    /// Stops accepting and lets existing connections drain.
    pub async fn shutdown(&self) {
        self.endpoint.close(0u32.into(), b"shutting down");
        self.endpoint.wait_idle().await;
    }
}

/// Connects to a server.
///
/// `server_name` is checked against the certificate under [`Trust::Roots`], so it must be the
/// hostname the certificate was issued for — not an address. Passing an IP literal here works only
/// with [`Trust::AnyCertificate`], and that is the intended asymmetry.
pub async fn connect(
    address: SocketAddr,
    server_name: &str,
    trust: Trust,
) -> Result<Link, TransportError> {
    let bind: SocketAddr = if address.is_ipv6() {
        "[::]:0".parse().expect("a valid wildcard address")
    } else {
        "0.0.0.0:0".parse().expect("a valid wildcard address")
    };

    let mut endpoint = quinn::Endpoint::client(bind).map_err(|source| TransportError::Bind {
        address: bind,
        detail: source.to_string(),
    })?;

    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(trust.client_config())
        .map_err(|source| TransportError::Configuration(source.to_string()))?;

    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    config.transport_config(transport_config());
    endpoint.set_default_client_config(config);

    let connection = endpoint
        .connect(address, server_name)
        .map_err(|source| TransportError::Configuration(source.to_string()))?
        .await
        .map_err(|source| TransportError::Handshake(source.to_string()))?;

    // Open the control stream now, and put a byte through it. QUIC creates a stream lazily — the
    // peer learns of it only when data arrives — so without this the server's `accept_bi` would
    // block until the first reliable message, which may be seconds away.
    let (mut send, recv) = connection
        .open_bi()
        .await
        .map_err(|source| TransportError::Handshake(source.to_string()))?;
    send.write_all(&0u32.to_le_bytes())
        .await
        .map_err(|source| TransportError::Handshake(source.to_string()))?;

    Ok(Link::start(connection, send, recv))
}
