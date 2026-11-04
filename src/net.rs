//! TLS plumbing shared by IMAP (and anything else needing raw TLS streams).

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// Object-safe stream so IMAP code doesn't care about plain vs TLS.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug> Stream for T {}
pub type BoxStream = Box<dyn Stream>;

pub fn tls_config(accept_invalid: bool) -> Arc<ClientConfig> {
    let builder = ClientConfig::builder();
    let cfg = if accept_invalid {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_no_client_auth()
    } else {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Arc::new(cfg)
}

pub async fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    let s = tokio::time::timeout(std::time::Duration::from_secs(20), TcpStream::connect((host, port)))
        .await
        .context("connection timed out")?
        .with_context(|| format!("connecting to {host}:{port}"))?;
    s.set_nodelay(true).ok();
    Ok(s)
}

pub async fn wrap_tls<S>(stream: S, host: &str, accept_invalid: bool) -> Result<tokio_rustls::client::TlsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let name = ServerName::try_from(host.to_string()).context("invalid server name")?;
    TlsConnector::from(tls_config(accept_invalid))
        .connect(name, stream)
        .await
        .context("TLS handshake failed")
}

/// Accepts any certificate. Only used when the user explicitly enables it
/// (local Proton Bridge with its self-signed cert).
#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
