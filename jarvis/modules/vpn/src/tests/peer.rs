//! Optional independent interoperability check. Xray is a test peer, never a runtime dependency.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::process::{Child, Command, Stdio};

struct Peer {
    child: Child,
    dir: std::path::PathBuf,
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !self.dir.as_os_str().is_empty() { let _ = std::fs::remove_dir_all(&self.dir); }
    }
}
async fn start_peer(cover: std::net::SocketAddr, vision: bool) -> Option<(Peer, u16, String)> {
    let binary = std::env::var_os("JARVIS_XRAY")?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut random = [0; 8];
    getrandom::fill(&mut random).unwrap();
    let dir = std::env::temp_dir().join(format!("jarvis-xray-peer-{}", hex::encode(random)));
    std::fs::create_dir_all(&dir).unwrap();
    let secret = x25519_dalek::StaticSecret::from([42; 32]);
    let public = x25519_dalek::PublicKey::from(&secret);
    let public = URL_SAFE_NO_PAD.encode(public.as_bytes());
    let flow = if vision { "xtls-rprx-vision" } else { "" };
    let config = json!({"log":{"loglevel":"none"},"inbounds":[{"listen":"127.0.0.1","port":port,"protocol":"vless",
        "settings":{"clients":[{"id":UUID,"flow":flow}],"decryption":"none"},
        "streamSettings":{"network":"tcp","security":"reality","realitySettings":{"show":false,"dest":cover.to_string(),"serverNames":["vpn.example.org"],"privateKey":URL_SAFE_NO_PAD.encode(secret.to_bytes()),"shortIds":["0123456789abcdef"]}}}],
        "outbounds":[{"protocol":"freedom","settings":{}}]});
    std::fs::write(
        dir.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let child = Command::new(binary)
        .args(["run", "-c"])
        .arg(dir.join("config.json"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut peer = Peer { child, dir };
    for _ in 0..100 {
        assert!(
            peer.child.try_wait().unwrap().is_none(),
            "Xray fixture exited before listening"
        );
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Some((peer, port, public));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Xray fixture did not start");
}

async fn cover() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let acceptor = test_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{let Ok((socket,_))=accepted else{break;};let acceptor=acceptor.clone();connections.spawn(async move {let _=acceptor.accept(socket).await;});},
                _=connections.join_next(),if !connections.is_empty()=>{},
            }
        }
    });
    (address, task)
}
struct AbortTask(tokio::task::JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn chrome_reality_uses_real_mlkem_with_independent_xray_and_go_tls_cover() {
    let (Some(cover_binary),Some(_))=(std::env::var_os("JARVIS_TLS_COVER"),std::env::var_os("JARVIS_XRAY")) else {return;};
    tokio::time::timeout(Duration::from_secs(15),async {
        use std::io::BufRead;
        let child=Command::new(cover_binary).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        // Peer owns cleanup even if the test fails.
        let mut cover=Peer{child,dir:std::path::PathBuf::new()};
        let mut address=String::new();std::io::BufReader::new(cover.child.stdout.take().unwrap()).read_line(&mut address).unwrap();
        let (_peer,port,key)=start_peer(address.trim().parse().unwrap(),false).await.unwrap();
        let echo=TcpListener::bind("127.0.0.1:0").await.unwrap();let echo_port=echo.local_addr().unwrap().port();
        let task=tokio::spawn(async move{let (mut io,_)=echo.accept().await.unwrap();let mut payload=[0;4];io.read_exact(&mut payload).await.unwrap();assert_eq!(&payload,b"ping");io.write_all(b"pong").await.unwrap();});
        let vpn=Vpn::new();let state=vpn.request(json!({"link":format!("vless://{UUID}@127.0.0.1:{port}?security=reality&pbk={key}&sid=0123456789abcdef&sni=vpn.example.org&fp=chrome")})).await.unwrap();
        let mut socket=socks(&state,"127.0.0.1",echo_port).await;socket.write_all(b"ping").await.unwrap();assert_eq!(socket.read_u32().await.unwrap(),u32::from_be_bytes(*b"pong"));
        task.await.unwrap();vpn.stop().await;
    }).await.unwrap();
}

#[tokio::test]
async fn reality_udp_interoperates_with_independent_xray() {
    if std::env::var_os("JARVIS_XRAY").is_none() { return; }
    tokio::time::timeout(Duration::from_secs(20),async {
      for vision in [false, true] {
        let (address,cover_task)=cover().await;let _cover=AbortTask(cover_task);
        let (_peer,port,key)=start_peer(address,vision).await.unwrap();
        let echo=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let echo_port=echo.local_addr().unwrap().port();
        let _echo=AbortTask(tokio::spawn(async move {let mut buffer=[0;65535];loop{let (n,from)=echo.recv_from(&mut buffer).await.unwrap();echo.send_to(&buffer[..n],from).await.unwrap();}}));
        let flow = if vision { "&flow=xtls-rprx-vision" } else { "" };
        let vpn=Vpn::new();let state=vpn.request(json!({"link":format!("vless://{UUID}@127.0.0.1:{port}?security=reality&pbk={key}&sid=0123456789abcdef&sni=vpn.example.org&fp=chrome{flow}")})).await.unwrap();
        let (mut control,udp,relay)=super::udp::association(&state).await;
        for size in [1,1200,4096,if vision { 7526 } else { 8190 }] {
            let payload:Vec<u8>=(0..size).map(|i|(i%251) as u8).collect();let packet=super::udp::packet("127.0.0.1",echo_port,&payload);
            udp.send_to(&packet,relay).await.unwrap();let mut response=vec![0;65535];let n=udp.recv(&mut response).await.unwrap();assert_eq!(n,packet.len(),"UDP payload size {size}");assert!(&response[..n]==packet.as_slice(),"UDP payload corrupted at size {size}");
        }
        vpn.stop().await;assert!(control.read_u8().await.is_err());
      }
    }).await.unwrap();
}

#[tokio::test]
async fn reality_interoperates_with_independent_xray_with_and_without_vision() {
    if std::env::var_os("JARVIS_XRAY").is_none() {
        eprintln!("JARVIS_XRAY unset: skipping independent REALITY peer");
        return;
    }
    let phase = std::sync::atomic::AtomicUsize::new(0);
    let sent = std::sync::atomic::AtomicBool::new(false);
    let received = std::sync::atomic::AtomicUsize::new(0);
    let result=tokio::time::timeout(Duration::from_secs(20),async {
        for (vision,chrome) in [(false,false),(true,false),(false,true),(true,true)] {
            phase.store(if vision {100}else{0},std::sync::atomic::Ordering::SeqCst);
            let (address,cover_task)=cover().await;let _cover=AbortTask(cover_task);
            let (_peer,port,key)=start_peer(address,vision).await.unwrap();
            let echo=TcpListener::bind("127.0.0.1:0").await.unwrap();let echo_port=echo.local_addr().unwrap().port();
            let echo_task=tokio::spawn(async move {let (mut socket,_)=echo.accept().await.unwrap();let (mut r,mut w)=socket.split();let _=tokio::io::copy(&mut r,&mut w).await;});
            let vpn=Vpn::new();let link=format!("vless://{UUID}@127.0.0.1:{port}?security=reality&pbk={key}&sid=0123456789abcdef&sni=vpn.example.org{}",if vision{"&flow=xtls-rprx-vision"}else{""});
            let link=if chrome {format!("{link}&fp=chrome")}else{link};
            let state=vpn.request(json!({"link":link})).await.unwrap();
            let mut socket=socks(&state,"127.0.0.1",echo_port).await;
            phase.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            // Full duplex exceeds both the Vision padding phase and TLS output buffers.
            let payload:Vec<u8>=(0..262144).map(|i|(i%251) as u8).collect();let expected=payload.clone();
            let (mut read,mut write)=socket.split();
            sent.store(false,std::sync::atomic::Ordering::SeqCst);received.store(0,std::sync::atomic::Ordering::SeqCst);
            let send=async {write.write_all(&payload).await.unwrap();write.flush().await.unwrap();sent.store(true,std::sync::atomic::Ordering::SeqCst);};
            let receive=async {let mut result=vec![0;expected.len()];read.read_exact(&mut result).await.unwrap();received.store(result.len(),std::sync::atomic::Ordering::SeqCst);assert_eq!(result,expected);};
            tokio::join!(send,receive);
            phase.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            drop(socket);vpn.stop().await;phase.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            // Xray may keep a freedom outbound alive after an inbound half-close until its idle
            // timeout. Retire the peer before awaiting the fixture echo server's EOF.
            drop(_peer);echo_task.await.unwrap();
        }
    }).await;
    assert!(
        result.is_ok(),
        "phase={} sent={} received={}",
        phase.load(std::sync::atomic::Ordering::SeqCst),
        sent.load(std::sync::atomic::Ordering::SeqCst),
        received.load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[tokio::test]
async fn reality_wrong_key_or_short_id_never_gets_socks_success() {
    if std::env::var_os("JARVIS_XRAY").is_none() {
        return;
    }
    tokio::time::timeout(Duration::from_secs(20),async {
        let (address,cover_task)=cover().await;let _cover=AbortTask(cover_task);
        let (_peer,port,key)=start_peer(address,false).await.unwrap();
        for (key,sid) in [(URL_SAFE_NO_PAD.encode(x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([9;32])).as_bytes()),"0123456789abcdef"),(key,"1111111111111111")] {
            let vpn=Vpn::new();let link=format!("vless://{UUID}@127.0.0.1:{port}?security=reality&pbk={key}&sid={sid}&sni=vpn.example.org&allowInsecure=1");
            let state=vpn.request(json!({"link":link})).await.unwrap();drop(socks_expect(&state,"127.0.0.1",1,5).await);vpn.stop().await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn vision_carries_inner_tls13_with_independent_xray() {
    if std::env::var_os("JARVIS_XRAY").is_none() {
        return;
    }
    tokio::time::timeout(Duration::from_secs(20),async {
        let (address,cover_task)=cover().await;let _cover=AbortTask(cover_task);
        let (_peer,port,key)=start_peer(address,true).await.unwrap();
        let target=TcpListener::bind("127.0.0.1:0").await.unwrap();let target_port=target.local_addr().unwrap().port();
        let target_task=tokio::spawn(async move {
            let (socket,_)=target.accept().await.unwrap();let mut socket=test_tls().accept(socket).await.unwrap();
            let (mut r,mut w)=tokio::io::split(&mut socket);let _=tokio::io::copy(&mut r,&mut w).await;
        });
        let _target=AbortTask(target_task);
        let vpn=Vpn::new();let state=vpn.request(json!({"link":format!("vless://{UUID}@127.0.0.1:{port}?security=reality&pbk={key}&sid=0123456789abcdef&sni=vpn.example.org&flow=xtls-rprx-vision-udp443")})).await.unwrap();
        let socket=socks(&state,"127.0.0.1",target_port).await;
        let inner_config=Config::parse(&format!("vless://{UUID}@127.0.0.1:443?security=tls&sni=vpn.example.org&allowInsecure=1")).unwrap();
        let conn=reality_tls::ClientConnection::new(transport::tls_config(&inner_config).unwrap(),"vpn.example.org".try_into().unwrap()).unwrap();
        let mut socket=crate::tls_io::TlsIo::new(Box::new(socket),conn);
        socket.handshake().await.unwrap();
        let payload:Vec<u8>=(0..262144).map(|i|(i%251) as u8).collect();
        let (mut r,mut w)=tokio::io::split(&mut socket);
        let send=async{w.write_all(&payload).await.unwrap();w.flush().await.unwrap();};
        let receive=async{let mut received=vec![0;payload.len()];r.read_exact(&mut received).await.unwrap();assert_eq!(received,payload);};
        tokio::join!(send,receive);drop(socket);vpn.stop().await;
    }).await.unwrap();
}
