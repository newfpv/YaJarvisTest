//! RFC 1928 UDP associations, tied to their authenticated TCP control connection.
//! Standard VLESS uses length-prefixed UDP. Vision uses one XUDP session per destination.
use crate::{
    config::Config,
    protocol::{Address, Target},
    transport,
};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

// Xray's VLESS packet writer needs the payload plus u16 length in its 8192-byte buffer.
// Reject larger packets rather than silently turn one datagram into several.
const MAX_PACKET: usize = 8_190;
const IDLE: Duration = Duration::from_secs(60);
const XUDP_MAX_META: usize = 512;
const XUDP_NEW: u8 = 1;
const XUDP_KEEP: u8 = 2;
const XUDP_END: u8 = 3;
const XUDP_HEARTBEAT: u8 = 4;
const XUDP_DATA: u8 = 1;

pub(crate) fn decode(packet: &[u8]) -> Option<(Target, &[u8])> {
    if packet.get(..3)? != [0, 0, 0] {
        return None;
    } // reserved bytes and FRAG=0
    let (address, end) = match *packet.get(3)? {
        1 => (
            Address::Ip(Ipv4Addr::from(<[u8; 4]>::try_from(packet.get(4..8)?).ok()?).into()),
            8,
        ),
        4 => (
            Address::Ip(Ipv6Addr::from(<[u8; 16]>::try_from(packet.get(4..20)?).ok()?).into()),
            20,
        ),
        3 => {
            let end = 5 + *packet.get(4)? as usize;
            let domain = packet.get(5..end)?;
            if domain.is_empty() || domain.iter().any(|c| *c <= 32 || *c >= 127) {
                return None;
            }
            (Address::Domain(domain.to_vec()), end)
        }
        _ => return None,
    };
    let port = u16::from_be_bytes(packet.get(end..end + 2)?.try_into().ok()?);
    let payload = packet.get(end + 2..)?;
    if port == 0 || payload.len() > MAX_PACKET {
        return None;
    }
    Some((Target { address, port }, payload))
}

fn header(target: &Target) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0];
    match &target.address {
        Address::Ip(IpAddr::V4(ip)) => {
            bytes.push(1);
            bytes.extend(ip.octets());
        }
        Address::Ip(IpAddr::V6(ip)) => {
            bytes.push(4);
            bytes.extend(ip.octets());
        }
        Address::Domain(domain) => {
            bytes.extend([3, domain.len() as u8]);
            bytes.extend(domain);
        }
    }
    bytes.extend(target.port.to_be_bytes());
    bytes
}

pub(crate) async fn associate(
    mut control: TcpStream,
    source: Target,
    config: Arc<Config>,
    tls: Arc<reality_tls::ClientConfig>,
    limit: Arc<Semaphore>,
) -> io::Result<()> {
    let peer = control.peer_addr()?;
    let source_ip = match source.address {
        Address::Ip(ip) if ip.is_unspecified() || ip == peer.ip() => peer.ip(),
        _ => {
            crate::protocol::socks_reply(&mut control, 8).await?;
            return Ok(());
        }
    };
    let relay = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?);
    let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
    reply.extend(relay.local_addr()?.port().to_be_bytes());
    control.write_all(&reply).await?;
    let mut client = (source.port != 0).then_some(SocketAddr::new(source_ip, source.port));
    let mut routes: HashMap<Target, mpsc::Sender<Vec<u8>>> = HashMap::new();
    let mut tasks = JoinSet::new(); // dropping the association aborts every destination stream
    let mut packet = vec![0; 65_535];
    loop {
        tokio::select! {
            biased;
            _ = control.read_u8() => break, // EOF or unexpected data terminates the association
            Some(_) = tasks.join_next(), if !tasks.is_empty() => { routes.retain(|_, tx| !tx.is_closed()); }
            received = relay.recv_from(&mut packet) => {
                let (size, from) = received?;
                if from.ip() != source_ip || client.is_some_and(|c| c != from) { continue; }
                let Some((target, payload)) = decode(&packet[..size]) else { continue; };
                client.get_or_insert(from);
                if routes.get(&target).is_some_and(|tx| tx.is_closed()) { routes.remove(&target); }
                if !routes.contains_key(&target) {
                    if routes.len() >= 8 { continue; }
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue; };
                    let (tx, rx) = mpsc::channel(4);
                    routes.insert(target.clone(), tx);
                    let (config, tls, relay, target) = (config.clone(), tls.clone(), relay.clone(), target.clone());
                    tasks.spawn(async move {
                        let _permit = permit;
                        let _ = forward(config, tls, target, relay, from, rx).await;
                    });
                }
                // Bounded queues: a slow destination cannot stall other ICE candidates.
                let _ = routes[&target].try_send(payload.to_vec());
            }
        }
    }
    tasks.shutdown().await;
    Ok(())
}

fn xudp_packet(target: &Target, payload: &[u8], first: bool) -> io::Result<Vec<u8>> {
    let mut frame = Vec::with_capacity(payload.len() + 32);
    frame.extend([0, 0, 0, 0, if first { XUDP_NEW } else { XUDP_KEEP }, XUDP_DATA]);
    if first {
        frame.push(2); // UDP address family follows in the VLESS endpoint encoding.
        target.write_vless_endpoint(&mut frame)?;
    }
    let meta = ((frame.len() - 2) as u16).to_be_bytes();
    frame[..2].copy_from_slice(&meta);
    frame.extend((payload.len() as u16).to_be_bytes());
    frame.extend(payload);
    Ok(frame)
}

async fn xudp_read_meta(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<(u8, bool)> {
    let len = reader.read_u16().await? as usize;
    if !(4..=XUDP_MAX_META).contains(&len) {
        return Err(io::Error::other("invalid XUDP frame length"));
    }
    let mut meta = [0; XUDP_MAX_META];
    reader.read_exact(&mut meta[..len]).await?;
    if meta[..2] != [0, 0] {
        return Err(io::Error::other("unexpected XUDP session"));
    }
    match meta[2] {
        XUDP_KEEP | XUDP_HEARTBEAT => Ok((meta[2], meta[3] & XUDP_DATA != 0)),
        XUDP_END => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "XUDP session ended")),
        _ => Err(io::Error::other("invalid XUDP frame status")),
    }
}

async fn forward(
    config: Arc<Config>,
    tls: Arc<reality_tls::ClientConfig>,
    target: Target,
    relay: Arc<UdpSocket>,
    client: SocketAddr,
    mut packets: mpsc::Receiver<Vec<u8>>,
) -> io::Result<()> {
    let io = tokio::time::timeout(
        Duration::from_secs(15),
        transport::dial_command(&config, tls, &target, true),
    )
    .await??;
    let (mut read, mut write) = tokio::io::split(io);
    let vision = config.vision;
    let receive = async {
        let prefix = header(&target);
        loop {
            let skip = if vision {
                let (status, has_data) = tokio::time::timeout(IDLE, xudp_read_meta(&mut read)).await??;
                if !has_data {
                    continue;
                }
                status == XUDP_HEARTBEAT
            } else {
                false
            };
            let size = tokio::time::timeout(IDLE, read.read_u16()).await?? as usize;
            if size > MAX_PACKET {
                return Err(io::Error::other("VLESS UDP packet too large"));
            }
            let mut packet = prefix.clone();
            packet.resize(prefix.len() + size, 0);
            tokio::time::timeout(IDLE, read.read_exact(&mut packet[prefix.len()..])).await??;
            if !skip {
                relay.send_to(&packet, client).await?;
            }
        }
    };
    let send = async {
        let mut first = true;
        while let Some(packet) = tokio::time::timeout(IDLE, packets.recv()).await? {
            if vision {
                if packet.is_empty() { continue; }
                let frame = xudp_packet(&target, &packet, first)?;
                write.write_all(&frame).await?;
                first = false;
            } else {
                write.write_u16(packet.len() as u16).await?;
                write.write_all(&packet).await?;
            }
            write.flush().await?;
        }
        Ok::<(), io::Error>(())
    };
    tokio::select! { result = receive => result, result = send => result }
}
