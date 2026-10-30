//! Relay TLS (D3): a self-signed certificate on the device's own ed25519 key,
//! and a client verifier that pins the server's SPKI to its certified devicePub.
//! No CA, no hostname check, no new `node.addr` field.

use std::sync::{Arc, RwLock};

use anyhow::{Result, anyhow};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use zeroize::Zeroizing;

use crate::identity::{DeviceIdentity, IrohKey as _};

/// DER SubjectPublicKeyInfo of an Ed25519 key: `SEQ{ SEQ{ OID 1.3.101.112 } BITSTRING pub }`.
pub fn ed25519_spki(pubkey: &[u8; 32]) -> Vec<u8> {
    let mut v = hex::decode("302a300506032b6570032100").expect("const");
    v.extend_from_slice(pubkey);
    v
}

/// PKCS#8 v1 wrapping of an Ed25519 seed (RFC 8410).
fn ed25519_pkcs8(seed: &[u8; 32]) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(hex::decode("302e020100300506032b657004220420").expect("const"));
    v.extend_from_slice(seed);
    v
}

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A self-signed cert whose key is the device key, plus the rustls server
/// config the relay's HTTPS and QUIC (QAD) servers share. TLS 1.3 only, which
/// both iroh's relay client and QAD speak.
pub fn relay_server_config(
    id: &DeviceIdentity,
    names: Vec<String>,
) -> Result<rustls::ServerConfig> {
    let seed = Zeroizing::new(id.secret_key().to_bytes());
    server_config_on_seed(
        &seed,
        names,
        format!("irori relay {}", id.endpoint_id().fmt_short()),
    )
}

/// The one relay TLS construction (D3), for the spike relay above and the
/// home server's embedded relay (`irori_home::relay::server_tls`): a
/// self-signed certificate for `names` named `common_name`, whose key is the
/// Ed25519 device seed, TLS 1.3 only. Clients pin the key
/// ([`PinnedDeviceKey`]), never the names.
pub fn server_config_on_seed(
    seed: &[u8; 32],
    names: Vec<String>,
    common_name: String,
) -> Result<rustls::ServerConfig> {
    let pkcs8 = ed25519_pkcs8(seed);
    let der = PrivatePkcs8KeyDer::from(pkcs8.to_vec());
    let kp = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_ED25519)
        .map_err(|e| anyhow!("device key as cert key: {e}"))?;
    let mut params =
        rcgen::CertificateParams::new(names).map_err(|e| anyhow!("cert params: {e}"))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, common_name);
    let cert = params
        .self_signed(&kp)
        .map_err(|e| anyhow!("self-sign: {e}"))?;
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], PrivateKeyDer::Pkcs8(der))?;
    Ok(cfg)
}

/// Accepts only pinned server keys: the end-entity certificate's SPKI must be
/// one of the pinned Ed25519 devicePubs (the home servers' certified device
/// keys), and the handshake signature must verify with it (rustls checks that
/// with the same cert). Names and dates are not checked: it is a pin, not a CA.
#[derive(Debug)]
pub struct PinnedDeviceKey {
    pins: PinSet,
    provider: Arc<CryptoProvider>,
}

impl PinnedDeviceKey {
    pub fn new(pubkeys: &[[u8; 32]], provider: Arc<CryptoProvider>) -> Self {
        Self::shared(PinSet::new(pubkeys.iter().copied()), provider)
    }

    /// A verifier that reads `pins` on every handshake, so home servers
    /// learned after bind (new `node.addr` or cert rows) are trusted without
    /// rebuilding the endpoint (mesh.md §8.1).
    pub fn shared(pins: PinSet, provider: Arc<CryptoProvider>) -> Self {
        Self { pins, provider }
    }
}

/// The relay server keys a client trusts, shared and updatable.
#[derive(Debug, Clone, Default)]
pub struct PinSet(Arc<RwLock<Vec<Vec<u8>>>>);

impl PinSet {
    pub fn new(pubkeys: impl IntoIterator<Item = [u8; 32]>) -> Self {
        let s = Self::default();
        for k in pubkeys {
            s.add(k);
        }
        s
    }
    /// Trust `pubkey`'s relay; idempotent.
    pub fn add(&self, pubkey: [u8; 32]) {
        let spki = ed25519_spki(&pubkey);
        let mut v = self.0.write().expect("pins");
        if !v.contains(&spki) {
            v.push(spki);
        }
    }
    /// Stop trusting `pubkey`'s relay (a revoked server).
    pub fn remove(&self, pubkey: &[u8; 32]) {
        let spki = ed25519_spki(pubkey);
        self.0.write().expect("pins").retain(|s| *s != spki);
    }
    pub fn contains(&self, pubkey: &[u8; 32]) -> bool {
        self.matches(&ed25519_spki(pubkey))
    }
    fn matches(&self, spki: &[u8]) -> bool {
        self.0
            .read()
            .expect("pins")
            .iter()
            .any(|s| s.as_slice() == spki)
    }
}

impl ServerCertVerifier for PinnedDeviceKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ee = webpki::EndEntityCert::try_from(end_entity).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        let got = ee.subject_public_key_info();
        if self.pins.matches(got.as_ref()) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
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

/// The iroh `CaTlsConfig` that pins relay connections (HTTPS and QAD) to the
/// given server device keys.
pub fn pinned_ca_config(server_pubs: &[iroh::EndpointId]) -> iroh_relay::tls::CaTlsConfig {
    shared_ca_config(PinSet::new(server_pubs.iter().map(|p| *p.as_bytes())))
}

/// [`pinned_ca_config`] over a shared [`PinSet`] that may grow after bind.
pub fn shared_ca_config(pins: PinSet) -> iroh_relay::tls::CaTlsConfig {
    iroh_relay::tls::CaTlsConfig::custom_server_cert_verifier(Arc::new(move |provider| {
        Ok(Arc::new(PinnedDeviceKey::shared(pins.clone(), provider))
            as Arc<dyn ServerCertVerifier>)
    }))
}

/// Records the server's Ed25519 key from its end-entity certificate and
/// accepts it (trust on first use). Pairing away from home reads a typed
/// home server's device key this way: its relay certificate is on that key
/// (D3), and the words and typed key still decide who pairs.
#[derive(Debug)]
struct CaptureKey {
    got: Arc<RwLock<Option<[u8; 32]>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for CaptureKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ee = webpki::EndEntityCert::try_from(end_entity).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        let spki = ee.subject_public_key_info();
        let spki = spki.as_ref();
        let prefix = ed25519_spki(&[0u8; 32]);
        if spki.len() != prefix.len() || spki[..12] != prefix[..12] {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        let mut k = [0u8; 32];
        k.copy_from_slice(&spki[12..]);
        *self.got.write().expect("key") = Some(k);
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

/// The Ed25519 key a relay at `addr` serves its certificate on (a home
/// server's device key, D3): one TLS 1.3 handshake, then the connection goes.
pub async fn probe_server_key(addr: std::net::SocketAddr) -> Result<[u8; 32], String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || probe_blocking(addr)),
    )
    .await
    .map_err(|_| format!("{addr}: no answer"))?
    .map_err(|e| e.to_string())?
}

fn probe_blocking(addr: std::net::SocketAddr) -> Result<[u8; 32], String> {
    let got = Arc::new(RwLock::new(None));
    let provider = provider();
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CaptureKey {
            got: got.clone(),
            provider,
        }))
        .with_no_client_auth();
    let name = ServerName::IpAddress(addr.ip().into());
    let mut c = rustls::ClientConnection::new(Arc::new(cfg), name).map_err(|e| e.to_string())?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5))
        .map_err(|e| format!("{addr}: {e}"))?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    while c.is_handshaking() {
        c.complete_io(&mut s).map_err(|e| format!("{addr}: {e}"))?;
    }
    let k = *got.read().expect("key");
    k.ok_or_else(|| format!("{addr}: no device key in its certificate"))
}
