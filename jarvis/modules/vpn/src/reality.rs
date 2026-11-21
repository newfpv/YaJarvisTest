//! REALITY authentication using connection-scoped hooks in the pinned rustls fork.
//! Wire format follows XTLS/REALITY; see THIRD-PARTY.md for implementation references.
use crate::config::{Config, Fingerprint};
use reality_tls as tls;
use ring::{aead, hkdf, hmac, signature};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tls::{
    Error, NamedGroup, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ActiveKeyExchange, SharedSecret, SupportedKxGroup},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use x509_parser::prelude::{FromDer, X509Certificate};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

fn failure() -> Error {
    Error::InvalidCertificate(tls::CertificateError::ApplicationVerificationFailure)
}
struct Context {
    auth: Option<Zeroizing<[u8; 32]>>,
    public: Option<[u8; 32]>,
}
struct Hooks {
    fingerprint: Fingerprint,
    server: [u8; 32],
    short_id: [u8; 8],
    shared: Option<Zeroizing<[u8; 32]>>,
    context: Arc<Mutex<Context>>,
}
struct Verifier(Arc<Mutex<Context>>);
impl fmt::Debug for Hooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealityHooks")
    }
}
impl fmt::Debug for Verifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealityVerifier")
    }
}
struct Exchange {
    secret: StaticSecret,
    public: [u8; 32],
    p256: Option<Box<dyn ActiveKeyExchange>>,
}
impl ActiveKeyExchange for Exchange {
    fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        let peer: [u8; 32] = peer.try_into().map_err(|_| failure())?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(peer));
        if !shared.was_contributory() {
            return Err(failure());
        }
        Ok(SharedSecret::from(shared.as_bytes().as_slice()))
    }
    fn pub_key(&self) -> &[u8] {
        &self.public
    }
    fn group(&self) -> NamedGroup {
        NamedGroup::X25519
    }
    // rustls uses this hook to send and negotiate a second share alongside the primary one.
    fn hybrid_component(&self) -> Option<(NamedGroup, &[u8])> {
        self.p256.as_ref().map(|exchange| (exchange.group(), exchange.pub_key()))
    }
    fn complete_hybrid_component(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        self.p256.ok_or_else(failure)?.complete(peer)
    }
}
impl tls::reality::ClientHooks for Hooks {
    fn client_hello(&mut self, hello: &[u8]) -> Result<Vec<u8>, Error> {
        match self.fingerprint {
            Fingerprint::Chrome133 => crate::chrome::hello(hello),
            Fingerprint::Firefox120 => crate::firefox::hello(hello),
            Fingerprint::Rustls => Ok(hello.to_vec()),
        }
    }
    fn start_key_exchange(
        &mut self,
        group: &'static dyn SupportedKxGroup,
    ) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        if self.fingerprint == Fingerprint::Chrome133 && group.name() == NamedGroup::X25519MLKEM768 {
            let exchange = crate::chrome::Exchange::new()?;
            self.shared = Some(exchange.reality_secret(self.server)?);
            return Ok(Box::new(exchange));
        }
        if group.name() != NamedGroup::X25519 {
            return Err(failure());
        }
        let mut bytes = Zeroizing::new([0; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| Error::FailedToGetRandomBytes)?;
        let secret = StaticSecret::from(*bytes);
        let public = PublicKey::from(&secret).to_bytes();
        let shared = secret.diffie_hellman(&PublicKey::from(self.server));
        if !shared.was_contributory() {
            return Err(failure());
        }
        self.shared = Some(Zeroizing::new(shared.to_bytes()));
        let p256 = if self.fingerprint == Fingerprint::Firefox120 {
            Some(tls::crypto::ring::kx_group::SECP256R1.start()?)
        } else {
            None
        };
        Ok(Box::new(Exchange { secret, public, p256 }))
    }
    fn session_id(&mut self, hello: &[u8]) -> Result<[u8; 32], Error> {
        let shared = self.shared.take().ok_or_else(failure)?;
        let random = hello.get(6..38).ok_or_else(failure)?;
        let auth = derive(&shared, &random[..20])?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::FailedToGetCurrentTime)?
            .as_secs();
        let timestamp = u32::try_from(timestamp).map_err(|_| Error::FailedToGetCurrentTime)?;
        let mut plain = Zeroizing::new([0; 16]);
        plain[..3].copy_from_slice(&[1, 8, 0]);
        plain[4..8].copy_from_slice(&timestamp.to_be_bytes());
        plain[8..].copy_from_slice(&self.short_id);
        let mut aad = hello.to_vec();
        aad.get_mut(39..71).ok_or_else(failure)?.fill(0);
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &auth[..]).map_err(|_| failure())?,
        );
        let nonce = aead::Nonce::try_assume_unique_for_key(&random[20..]).map_err(|_| failure())?;
        let mut encrypted = Zeroizing::new(plain.to_vec());
        key.seal_in_place_append_tag(nonce, aead::Aad::from(aad), &mut *encrypted)
            .map_err(|_| failure())?;
        self.context.lock().unwrap().auth = Some(auth);
        encrypted.as_slice().try_into().map_err(|_| failure())
    }
    fn server_hello(&mut self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
}
fn derive(shared: &[u8; 32], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, Error> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, salt);
    let prk = salt.extract(shared);
    let mut auth = Zeroizing::new([0; 32]);
    prk.expand(&[b"REALITY"], hkdf::HKDF_SHA256)
        .map_err(|_| failure())?
        .fill(auth.as_mut())
        .map_err(|_| failure())?;
    Ok(auth)
}

fn certificate_key(der: &[u8]) -> Result<[u8; 32], Error> {
    let (remaining, cert) = X509Certificate::from_der(der).map_err(|_| failure())?;
    let spki = cert.public_key();
    if !remaining.is_empty()
        || spki.algorithm.algorithm.to_id_string() != "1.3.101.112"
        || spki.algorithm.parameters.is_some()
        || spki.subject_public_key.unused_bits != 0
    {
        return Err(failure());
    }
    spki.subject_public_key
        .data
        .as_ref()
        .try_into()
        .map_err(|_| failure())
}
impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let mut context = self.0.lock().unwrap();
        let auth = context.auth.take().ok_or_else(failure)?;
        if !intermediates.is_empty() {
            return Err(failure());
        }
        let public = certificate_key(cert)?;
        let (_, parsed) = X509Certificate::from_der(cert).map_err(|_| failure())?;
        if parsed.signature_value.unused_bits != 0 {
            return Err(failure());
        }
        let key = hmac::Key::new(hmac::HMAC_SHA512, &auth[..]);
        hmac::verify(&key, &public, parsed.signature_value.data.as_ref()).map_err(|_| failure())?;
        context.public = Some(public);
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(failure())
    }
    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        sig: &tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        let public = self.0.lock().unwrap().public.ok_or_else(failure)?;
        if sig.scheme != SignatureScheme::ED25519 || certificate_key(cert)? != public {
            return Err(failure());
        }
        signature::UnparsedPublicKey::new(&signature::ED25519, public)
            .verify(msg, sig.signature())
            .map_err(|_| failure())?;
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Include normal cover-site schemes in ClientHello; authenticated REALITY must use Ed25519.
        tls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn client(config: &Config) -> Result<tls::ClientConnection, String> {
    if config.reality.is_none() {
        return standard_client(config);
    }
    let reality = config.reality.as_ref().ok_or("Нет параметров REALITY")?;
    let context = Arc::new(Mutex::new(Context {
        auth: None,
        public: None,
    }));
    let mut provider = tls::crypto::ring::default_provider();
    use tls::crypto::ring::kx_group::{SECP256R1, SECP384R1, X25519};
    provider.kx_groups = match config.fingerprint {
        Fingerprint::Chrome133 => vec![&crate::chrome::HYBRID, X25519, SECP256R1, SECP384R1],
        Fingerprint::Firefox120 => vec![X25519, SECP256R1, SECP384R1],
        Fingerprint::Rustls => vec![X25519],
    };
    let mut settings = tls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&tls::version::TLS13])
        .map_err(|_| "Не настроить REALITY")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Verifier(context.clone())))
        .with_no_client_auth();
    settings.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    settings.resumption = tls::client::Resumption::disabled();
    if config.fingerprint != Fingerprint::Chrome133 {
        settings.cert_decompressors.clear();
    }
    let hooks = Hooks {
        fingerprint: config.fingerprint,
        server: reality.public_key,
        short_id: reality.short_id,
        shared: None,
        context,
    };
    tls::ClientConnection::new_reality(
        Arc::new(settings),
        config
            .server_name
            .clone()
            .try_into()
            .map_err(|_| "Неверный SNI REALITY")?,
        Box::new(hooks),
    )
    .map_err(|_| "Не создать REALITY-клиент".into())
}

#[derive(Debug)]
struct Insecure;
impl ServerCertVerifier for Insecure {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(failure())
    }
    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        sig: &tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        tls::crypto::verify_tls13_signature(
            msg,
            cert,
            sig,
            &tls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        tls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
fn standard_client(config: &Config) -> Result<tls::ClientConnection, String> {
    let builder =
        tls::ClientConfig::builder_with_provider(Arc::new(tls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&tls::version::TLS13])
            .map_err(|_| "Не настроить TLS Vision")?;
    let settings = if config.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Insecure))
            .with_no_client_auth()
    } else {
        builder
            .with_root_certificates(crate::transport::root_store())
            .with_no_client_auth()
    };
    tls::ClientConnection::new(
        Arc::new(settings),
        config
            .server_name
            .clone()
            .try_into()
            .map_err(|_| "Неверный SNI Vision")?,
    )
    .map_err(|_| "Не создать TLS-клиент Vision".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    #[test]
    fn encoded_client_hello_authenticates_timestamp_short_id_and_ephemeral_key() {
        let server = StaticSecret::from([42; 32]);
        let public = URL_SAFE_NO_PAD.encode(PublicKey::from(&server).as_bytes());
        let config=Config::parse(&format!("vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:443?security=reality&sni=vpn.example.org&pbk={public}&sid=0123456789abcdef")).unwrap();
        let mut client = client(&config).unwrap();
        let mut wire = Vec::new();
        client.write_tls(&mut wire).unwrap();
        assert_eq!(wire[0], 22);
        let hello = &wire[5..];
        assert_eq!(hello[0], 1);
        assert_eq!(hello[38], 32);
        let suites_at = 71;
        let suites_len = u16::from_be_bytes([hello[suites_at], hello[suites_at + 1]]) as usize;
        let compression_at = suites_at + 2 + suites_len;
        let extensions_at = compression_at + 1 + hello[compression_at] as usize;
        let mut ext = &hello[extensions_at + 2..];
        let mut peer = None;
        while ext.len() >= 4 {
            let kind = u16::from_be_bytes([ext[0], ext[1]]);
            let n = u16::from_be_bytes([ext[2], ext[3]]) as usize;
            if kind == 51 {
                let value = &ext[4..4 + n];
                assert_eq!(&value[2..6], &[0, 29, 0, 32]);
                peer = Some(<[u8; 32]>::try_from(&value[6..38]).unwrap());
                break;
            }
            ext = &ext[4 + n..];
        }
        let shared = server.diffie_hellman(&PublicKey::from(peer.unwrap()));
        let auth = derive(shared.as_bytes(), &hello[6..26]).unwrap();
        let mut aad = hello.to_vec();
        aad[39..71].fill(0);
        let mut sealed = hello[39..71].to_vec();
        let key =
            aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &auth[..]).unwrap());
        let plaintext = key
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(&hello[26..38]).unwrap(),
                aead::Aad::from(aad),
                &mut sealed,
            )
            .unwrap();
        assert_eq!(&plaintext[..4], &[1, 8, 0, 0]);
        assert_eq!(&plaintext[8..], &hex::decode("0123456789abcdef").unwrap());
        let timestamp = u32::from_be_bytes(plaintext[4..8].try_into().unwrap()) as u64;
        assert!(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .abs_diff(timestamp)
                < 5
        );
    }
    #[test]
    fn certificate_authentication_is_one_use_and_never_accepts_a_cover_certificate() {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let cert = rcgen::CertificateParams::new(vec!["vpn.example.org".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let original = cert.der().to_vec();
        let public = certificate_key(&original).unwrap();
        let auth = [7; 32];
        let make = || {
            Verifier(Arc::new(Mutex::new(Context {
                auth: Some(Zeroizing::new(auth)),
                public: None,
            })))
        };
        let name = "vpn.example.org".try_into().unwrap();
        let time = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1));
        assert!(
            make()
                .verify_server_cert(
                    &CertificateDer::from(original.clone()),
                    &[],
                    &name,
                    &[],
                    time
                )
                .is_err()
        );
        let mut signed = original;
        let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA512, &auth), &public);
        let len = signed.len();
        signed[len - 64..].copy_from_slice(signature.as_ref());
        let verifier = make();
        let cert = CertificateDer::from(signed.clone());
        assert!(
            verifier
                .verify_server_cert(&cert, &[], &name, &[], time)
                .is_ok()
        );
        assert!(
            verifier
                .verify_server_cert(&cert, &[], &name, &[], time)
                .is_err()
        );
        signed[len - 1] ^= 1;
        assert!(
            make()
                .verify_server_cert(&CertificateDer::from(signed), &[], &name, &[], time)
                .is_err()
        );
    }
}
