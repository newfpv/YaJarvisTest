use crate::{
    config::Config,
    protocol::{BoxIo, Target, VlessIo},
};
use reality_tls::{
    ClientConfig, ClientConnection, RootCertStore,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    io,
    sync::{Arc, LazyLock},
};
use tokio::{io::AsyncWriteExt, net::TcpStream};

static ROOTS: LazyLock<Arc<RootCertStore>> = LazyLock::new(|| {
    Arc::new(RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    ))
});

pub(crate) fn root_store() -> Arc<RootCertStore> {
    ROOTS.clone()
}

pub(crate) fn tls_config(config: &Config) -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(reality_tls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| "Не настроить TLS VPN")?;
    let mut tls = if config.insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Insecure(provider)))
            .with_no_client_auth()
    } else {
        builder
            .with_root_certificates(root_store())
            .with_no_client_auth()
    };
    if config.websocket {
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    }
    Ok(Arc::new(tls))
}

pub(crate) async fn dial(
    config: &Config,
    tls: Arc<ClientConfig>,
    target: &Target,
) -> io::Result<BoxIo> {
    dial_command(config, tls, target, false).await
}

pub(crate) async fn dial_command(
    config: &Config,
    tls: Arc<ClientConfig>,
    target: &Target,
    udp: bool,
) -> io::Result<BoxIo> {
    // Resolve only the VPN server locally. The target's domain is encoded into VLESS unchanged.
    let socket = TcpStream::connect((config.host.as_str(), config.port)).await?;
    socket.set_nodelay(true)?;
    if config.reality.is_some() || config.vision {
        let session = crate::reality::client(config).map_err(io::Error::other)?;
        let mut stream = crate::tls_io::TlsIo::new(Box::new(socket), session);
        stream.handshake().await?;
        let mut header = if udp && config.vision {
            // XUDP uses VLESS command Mux. Its first frame supplies the destination.
            let mut mux = vec![0];
            mux.extend(config.uuid);
            mux.extend([0, 3]);
            mux
        } else {
            target.vless_header_for(&config.uuid, udp)?
        };
        if config.vision {
            // Addons protobuf: field 1 (flow), using the canonical wire flow for both TCP aliases.
            let flow = b"xtls-rprx-vision";
            let mut addon = vec![(flow.len() + 2) as u8, 10, flow.len() as u8];
            addon.extend(flow);
            header.splice(17..18, addon);
        }
        stream.write_all(&header).await?;
        stream.flush().await?;
        return if config.vision {
            Ok(Box::new(crate::vision::VisionIo::new(stream, config.uuid)))
        } else {
            Ok(Box::new(VlessIo::new(Box::new(stream))))
        };
    }
    let mut io: BoxIo = if config.tls {
        let name = ServerName::try_from(config.server_name.clone())
            .map_err(|_| io::Error::other("invalid VPN SNI"))?;
        let conn = ClientConnection::new(tls, name).map_err(io::Error::other)?;
        let mut stream = crate::tls_io::TlsIo::new(Box::new(socket), conn);
        stream.handshake().await?;
        Box::new(stream)
    } else {
        Box::new(socket)
    };
    if config.websocket {
        io = Box::new(crate::websocket::connect(io, config).await?);
    }
    let header = if udp { target.vless_header_for(&config.uuid, true)? } else { target.vless_header(&config.uuid)? };
    io.write_all(&header).await?;
    io.flush().await?;
    // Do not await the VLESS response here: some servers send it only after the first payload.
    Ok(Box::new(VlessIo::new(io)))
}

#[derive(Debug)]
struct Insecure(Arc<CryptoProvider>);
impl ServerCertVerifier for Insecure {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, reality_tls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        signature: &reality_tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, reality_tls::Error> {
        reality_tls::crypto::verify_tls12_signature(
            msg,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        msg: &[u8],
        cert: &CertificateDer<'_>,
        signature: &reality_tls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, reality_tls::Error> {
        reality_tls::crypto::verify_tls13_signature(
            msg,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<reality_tls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
