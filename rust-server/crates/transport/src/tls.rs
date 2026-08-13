//! Server identity and client trust.
//!
//! The server is a public one, so this is ordinary TLS: a certificate issued by a real authority,
//! verified by the client against a root store. Nothing bespoke.
//!
//! That rules out certificate pinning, which is tempting for a game and wrong here. ACME
//! certificates renew roughly every ninety days and the key may rotate with them, so any client
//! holding a pinned fingerprint would stop connecting at the first renewal — a failure that arrives
//! months after the code that caused it, for every player at once.
//!
//! # Certificates on disk
//!
//! [`ServerIdentity::from_pem_files`] reads the pair certbot writes:
//!
//! ```text
//!   /etc/letsencrypt/live/<domain>/fullchain.pem   the certificate and its issuers
//!   /etc/letsencrypt/live/<domain>/privkey.pem     the private key
//! ```
//!
//! The chain matters. Serving only the leaf works in a browser, which will often fetch the missing
//! issuer itself, and fails in a QUIC client, which will not.

use std::path::Path;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::TransportError;

/// The protocol name negotiated during the TLS handshake.
///
/// QUIC requires ALPN, and it is worth more than a formality here: a client built for a different
/// protocol version is refused during the handshake rather than after it has been welcomed into a
/// world.
pub const ALPN: &[u8] = b"hendra/1";

/// The certificate chain and private key the server presents.
#[derive(Debug)]
pub struct ServerIdentity {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl ServerIdentity {
    /// Loads a certificate chain and key from PEM files.
    pub fn from_pem_files(
        chain: impl AsRef<Path>,
        key: impl AsRef<Path>,
    ) -> Result<ServerIdentity, TransportError> {
        let chain_path = chain.as_ref();
        let key_path = key.as_ref();

        let chain_bytes = std::fs::read(chain_path).map_err(|source| TransportError::Identity {
            path: chain_path.display().to_string(),
            detail: source.to_string(),
        })?;
        let key_bytes = std::fs::read(key_path).map_err(|source| TransportError::Identity {
            path: key_path.display().to_string(),
            detail: source.to_string(),
        })?;

        ServerIdentity::from_pem(&chain_bytes, &key_bytes).map_err(|detail| {
            TransportError::Identity {
                path: chain_path.display().to_string(),
                detail,
            }
        })
    }

    /// Parses a chain and key already in memory.
    pub fn from_pem(chain: &[u8], key: &[u8]) -> Result<ServerIdentity, String> {
        let chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &chain[..])
            .collect::<Result<_, _>>()
            .map_err(|source| format!("reading certificates: {source}"))?;

        if chain.is_empty() {
            return Err("no certificates found".into());
        }

        let key = rustls_pemfile::private_key(&mut &key[..])
            .map_err(|source| format!("reading the private key: {source}"))?
            .ok_or_else(|| "no private key found".to_string())?;

        Ok(ServerIdentity { chain, key })
    }

    /// Generates a throwaway self-signed identity for loopback testing and local development.
    ///
    /// No client verifying against a root store will accept this, which is the intended outcome:
    /// it pairs only with [`Trust::AnyCertificate`], and both carry the same warning.
    pub fn self_signed(names: &[&str]) -> Result<ServerIdentity, TransportError> {
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        let generated = rcgen::generate_simple_self_signed(names)
            .map_err(|source| TransportError::Identity {
                path: "<generated>".into(),
                detail: source.to_string(),
            })?;

        Ok(ServerIdentity {
            chain: vec![CertificateDer::from(generated.cert)],
            key: PrivateKeyDer::try_from(generated.signing_key.serialize_der())
                .map_err(|detail| TransportError::Identity {
                    path: "<generated>".into(),
                    detail: detail.to_string(),
                })?,
        })
    }

    /// How many certificates the chain holds.
    ///
    /// A chain of one is the common misconfiguration: it means the leaf was served without its
    /// issuer, which a browser hides and a QUIC client does not.
    pub fn chain_len(&self) -> usize {
        self.chain.len()
    }
}

/// How a client decides whether to accept the server it reached.
#[derive(Debug, Clone, Copy, Default)]
pub enum Trust {
    /// Verify against the bundled Mozilla root store, and require the certificate to match the
    /// hostname being connected to. This is what a released client uses.
    ///
    /// The roots are bundled rather than taken from the operating system so that verification
    /// behaves identically on every platform the client ships to, instead of inheriting whichever
    /// trust store an old Windows install happens to carry.
    #[default]
    Roots,

    /// Accept any certificate, verifying nothing.
    ///
    /// Encrypted but unauthenticated: anything that can answer the address can pretend to be the
    /// server. Loopback tests and local development only.
    AnyCertificate,
}

impl Trust {
    /// Builds the rustls client configuration this trust setting implies.
    pub(crate) fn client_config(self) -> rustls::ClientConfig {
        let mut config = match self {
            Trust::Roots => {
                let roots = RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                };
                rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                    .with_root_certificates(roots)
                    .with_no_client_auth()
            }
            Trust::AnyCertificate => rustls::ClientConfig::builder_with_protocol_versions(&[
                &rustls::version::TLS13,
            ])
            .dangerous()
                .with_custom_certificate_verifier(AcceptAnyCertificate::new())
                .with_no_client_auth(),
        };

        config.alpn_protocols = vec![ALPN.to_vec()];
        config
    }
}

/// Verifies nothing. Paired only with [`Trust::AnyCertificate`].
#[derive(Debug)]
struct AcceptAnyCertificate {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyCertificate {
    fn new() -> Arc<AcceptAnyCertificate> {
        Arc::new(AcceptAnyCertificate {
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        })
    }
}

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_signed_identity_has_one_certificate_and_a_key() {
        let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
        assert_eq!(identity.chain_len(), 1);
    }

    #[test]
    fn a_pem_chain_round_trips_with_its_issuers_intact() {
        // Two concatenated certificates, as fullchain.pem holds: leaf then issuer.
        let leaf = ServerIdentity::self_signed(&["example.com"]).unwrap();
        let issuer = ServerIdentity::self_signed(&["issuer.example.com"]).unwrap();

        let mut pem = Vec::new();
        for certificate in leaf.chain.iter().chain(issuer.chain.iter()) {
            pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
            pem.extend_from_slice(base64_lines(certificate.as_ref()).as_bytes());
            pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
        }

        let key_pem = pem_key(&leaf);
        let loaded = ServerIdentity::from_pem(&pem, key_pem.as_bytes()).unwrap();

        assert_eq!(
            loaded.chain_len(),
            2,
            "both the leaf and its issuer should survive loading"
        );
    }

    #[test]
    fn an_empty_chain_is_refused_rather_than_serving_nothing() {
        let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
        let key = pem_key(&identity);
        assert_eq!(
            ServerIdentity::from_pem(b"", key.as_bytes()).unwrap_err(),
            "no certificates found"
        );
    }

    #[test]
    fn a_missing_key_is_refused() {
        let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
        let mut pem = Vec::new();
        pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
        pem.extend_from_slice(base64_lines(identity.chain[0].as_ref()).as_bytes());
        pem.extend_from_slice(b"-----END CERTIFICATE-----\n");

        assert!(ServerIdentity::from_pem(&pem, b"").is_err());
    }

    #[test]
    fn a_missing_file_names_the_path_it_could_not_read() {
        let error = ServerIdentity::from_pem_files("/nonexistent/fullchain.pem", "/nonexistent/privkey.pem")
            .unwrap_err();
        assert!(
            error.to_string().contains("fullchain.pem"),
            "the error should name the file: {error}"
        );
    }

    #[test]
    fn the_default_trust_verifies_against_roots() {
        assert!(matches!(Trust::default(), Trust::Roots));

        // The bundled store is non-empty, so a released client has something to verify against.
        assert!(!webpki_roots::TLS_SERVER_ROOTS.is_empty());
    }

    #[test]
    fn every_client_config_negotiates_our_protocol() {
        for trust in [Trust::Roots, Trust::AnyCertificate] {
            assert_eq!(trust.client_config().alpn_protocols, vec![ALPN.to_vec()]);
        }
    }

    // -- helpers -------------------------------------------------------------------------------

    fn pem_key(identity: &ServerIdentity) -> String {
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}-----END PRIVATE KEY-----\n",
            base64_lines(identity.key.secret_der())
        )
    }

    /// Minimal base64 with PEM line wrapping, so the tests do not need another dependency.
    fn base64_lines(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

        let mut encoded = String::new();
        for chunk in bytes.chunks(3) {
            let block = ((chunk[0] as u32) << 16)
                | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
                | (*chunk.get(2).unwrap_or(&0) as u32);

            for index in 0..4 {
                if index <= chunk.len() {
                    encoded.push(ALPHABET[((block >> (18 - index * 6)) & 0x3f) as usize] as char);
                } else {
                    encoded.push('=');
                }
            }
        }

        encoded
            .as_bytes()
            .chunks(64)
            .map(|line| format!("{}\n", std::str::from_utf8(line).expect("ascii")))
            .collect()
    }
}
