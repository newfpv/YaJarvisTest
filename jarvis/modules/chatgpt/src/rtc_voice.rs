use super::{Callback,ChatGpt,opus,rtc_runtime::RoutedRuntime};
use std::{sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},collections::BTreeMap,time::{Duration,Instant}};
use serde_json::{Value,json};
use base64::{Engine,engine::general_purpose::STANDARD as B64};
use webrtc::{peer_connection::{PeerConnection,PeerConnectionBuilder,PeerConnectionEventHandler,RTCIceGatheringState,RTCPeerConnectionState},data_channel::{DataChannel,DataChannelEvent},media_stream::{track_local::{TrackLocal,static_sample::TrackLocalStaticSample},track_remote::{TrackRemote,TrackRemoteEvent}}};
use rtc::{peer_connection::{configuration::{RTCConfigurationBuilder,media_engine::MediaEngine,interceptor_registry::register_default_interceptors,setting_engine::SettingEngineBuilder},sdp::RTCSessionDescription},rtp_transceiver::rtp_sender::{RTCRtpCodec,RTCRtpCodecParameters,RTCRtpEncodingParameters,RTCRtpCodingParameters,RtpCodecKind},media_stream::MediaStreamTrack};
pub const LIVE_11: &str = "gpt-live-1.1-codex";
const LIVE_1: &str = "gpt-live-1-codex";
const PLAYOUT_BUFFER: Duration = Duration::from_millis(100);
const LATE_PACKET_GRACE: Duration = Duration::from_millis(30);
const AUDIO_PAYLOAD: u8 = 111;
const CHATGPT_VOICES: &[&str] = &[
    "cove", "juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol",
    "aube", "rivage", "bubbie", "haetsal", "hanul", "himari", "rindo", "leher", "neer",
    "miko", "sonna", "rio", "viola", "tinta",
];
pub struct RtcVoice {pub id:String,pub requested_model:&'static str,pc:Arc<dyn PeerConnection>,dc:Arc<dyn DataChannel>,pending:Arc<Mutex<Vec<i16>>>,stop:tokio::sync::watch::Sender<bool>,tasks:Mutex<Vec<tokio::task::JoinHandle<()>>>}
struct Guard(Option<Arc<dyn PeerConnection>>);
impl Drop for Guard{fn drop(&mut self){if let Some(pc)=self.0.take(){tokio::spawn(async move{let _=pc.close().await;});}}}
async fn emit(services:&Callback,id:&str,kind:&str,value:Value){let _=services("voice.event".into(),json!({"id":id,"kind":kind,"value":value})).await;}
fn selected_voice(value: &Value) -> Result<&str, String> {
    let voice = value.as_str().filter(|voice| !voice.is_empty()).unwrap_or("cove");
    CHATGPT_VOICES
        .contains(&voice)
        .then_some(voice)
        .ok_or_else(|| "Этот голос недоступен для подписки ChatGPT".into())
}

async fn rejected_model(mut response: reqwest::Response) -> bool {
    if !matches!(response.status().as_u16(), 400 | 404 | 422) { return false; }
    let mut bytes = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => return false,
        };
        if bytes.len() + chunk.len() > 8192 { return false; }
        bytes.extend_from_slice(&chunk);
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { return false };
    let error = &payload["error"];
    matches!(error["code"].as_str(), Some("model_not_found" | "unsupported_model" | "model_not_available" | "invalid_model"))
        || error["param"] == "model"
}

fn audio_bitrate(sdp: &str) -> i32 {
    let mut in_audio = false;
    let mut bitrate = opus::SPEECH_BITRATE;
    let prefix = format!("a=fmtp:{AUDIO_PAYLOAD} ");
    for line in sdp.lines() {
        if let Some(media) = line.strip_prefix("m=") {
            in_audio = media.starts_with("audio ");
        } else if in_audio && let Some(params) = line.strip_prefix(&prefix) {
            for param in params.split(';') {
                if let Some((key, value)) = param.trim().split_once('=')
                    && key.trim().eq_ignore_ascii_case("maxaveragebitrate")
                    && let Ok(limit @ 6000..=510000) = value.trim().parse::<i32>()
                {
                    bitrate = bitrate.min(limit);
                }
            }
        }
    }
    bitrate
}

async fn forward_audio(
    services: Callback,
    id: String,
    mut stop: tokio::sync::watch::Receiver<bool>,
    mut audio: tokio::sync::mpsc::Receiver<Vec<i16>>,
) {
    let mut last_heard = None::<Instant>;
    while let Some(pcm) = audio.recv().await {
        let send = async {
            let now = Instant::now();
            if last_heard.is_none_or(|time| now.duration_since(time) >= Duration::from_millis(200))
                && pcm.iter().any(|v| v.unsigned_abs() > 400)
            {
                emit(&services, &id, "heard", Value::Null).await;
                last_heard = Some(now);
            }
            let data: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
            emit(&services, &id, "audio", B64.encode(data).into()).await;
        };
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = send => {}
        }
    }
}
struct Handler {gathered:Arc<tokio::sync::Notify>,services:Callback,id:String,stop:tokio::sync::watch::Receiver<bool>,track:AtomicBool}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self,state:RTCIceGatheringState){if state==RTCIceGatheringState::Complete{self.gathered.notify_one();}}
    async fn on_connection_state_change(&self,state:RTCPeerConnectionState){if matches!(state,RTCPeerConnectionState::Failed|RTCPeerConnectionState::Closed){emit(&self.services,&self.id,"closed",Value::Null).await;}}
    async fn on_track(&self,track:Arc<dyn TrackRemote>){
        if track.kind().await!=RtpCodecKind::Audio || self.track.swap(true,Ordering::SeqCst){return;}
        let services=self.services.clone();let id=self.id.clone();let mut stop=self.stop.clone();
        let (audio, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(forward_audio(services.clone(), id.clone(), stop.clone(), receiver));
        tokio::spawn(async move {
            let mut decoder=match opus::Decoder::new(){Ok(d)=>d,Err(e)=>{emit(&services,&id,"error",e.into()).await;return;}};
            let mut packets=BTreeMap::<u16,Vec<u8>>::new();let mut expected=None;let mut due=None;let mut last=Instant::now();let mut frame=960;
            let mut concealed = 0u32;
            let mut late = 0u32;
            let mut dropped = 0u32;
            let mut decoded = 0u32;
            let mut peak_gap = Duration::ZERO;
            loop {tokio::select!{
                biased;
                _=stop.changed()=>break,
                _=async {
                    if let Some(deadline) = due {
                        let grace = if expected.is_some_and(|next| !packets.contains_key(&next))
                            && last.elapsed() < PLAYOUT_BUFFER + LATE_PACKET_GRACE
                        {
                            LATE_PACKET_GRACE
                        } else {
                            Duration::ZERO
                        };
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline + grace)).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }=>{
                    let now=Instant::now();
                    if packets.is_empty() && now.duration_since(last)>Duration::from_millis(120){due=None;expected=None;continue;}
                    let next=expected.unwrap();let packet=packets.remove(&next);let fec=packet.is_none() && packets.contains_key(&next.wrapping_add(1));
                    if packet.is_some() { decoded += 1; }
                    else if !packets.is_empty() { concealed += 1; }
                    let input=packet.as_deref().or_else(||if fec{packets.get(&next.wrapping_add(1)).map(Vec::as_slice)}else{None});
                    let pcm=match decoder.decode(input,fec,if packet.is_some(){5760}else{frame}){Ok(p)=>p,Err(_)=>{emit(&services,&id,"error",json!("Повреждённый пакет Opus")).await;break;}};
                    frame=pcm.len().max(120);expected=Some(next.wrapping_add(1));
                    // Keep the RTP playout clock anchored. Scheduling from `now` adds
                    // every timer's lateness to all subsequent packets and builds seconds of lag.
                    let elapsed = Duration::from_secs_f64(frame as f64 / 48_000.0);
                    let next_due=due.unwrap()+elapsed;
                    due=Some(if now.saturating_duration_since(next_due)>Duration::from_millis(120){
                        now+elapsed
                    }else{next_due});
                    if audio.try_send(pcm).is_err() {
                        if audio.is_closed() { break; }
                        dropped += 1;
                    }
                },
                event=track.poll()=>match event{
                    Some(TrackRemoteEvent::OnRtpPacket(packet))=>{
                        let sequence=packet.header.sequence_number;if packet.payload.len()>65535{break;}
                        let next=*expected.get_or_insert(sequence);if sequence.wrapping_sub(next)>=32768{late+=1;continue;}
                        let now=Instant::now();
                        if due.is_some() && now.duration_since(last)<Duration::from_millis(500) {
                            peak_gap=peak_gap.max(now.duration_since(last));
                        }
                        packets.insert(sequence,packet.payload.to_vec());last=now;if due.is_none(){due=Some(last+PLAYOUT_BUFFER);}
                        if packets.len()>64{if let Some(farthest)=packets.keys().max_by_key(|k|k.wrapping_sub(next)).copied(){packets.remove(&farthest);}}
                    },Some(TrackRemoteEvent::OnEnded)|None=>break,_=>{},
                },
            }}
            if decoded > 0 {
                eprintln!("ChatGPT: звук RTP: {decoded} пакетов, опоздали {late}, замаскировано пропусков {concealed}, очередь PCM пропустила {dropped}, интервал пакетов до {} мс", peak_gap.as_millis());
            }
        });
    }
}
impl RtcVoice {
    pub async fn open(provider:&ChatGpt,params:&Value)->Result<Arc<Self>,String>{
        let id=params["id"].as_str().filter(|s|!s.is_empty()&&s.len()<=128).ok_or("Неверный ID звонка")?.to_string();
        let voice = selected_voice(&params["profile"]["voice"])?;
        let network=(provider.services)("network".into(),json!({})).await?;let runtime=Arc::new(RoutedRuntime::new(&network)?);let tcp_only=runtime.proxied() && !runtime.udp();
        let mut media=MediaEngine::default();let codec=RTCRtpCodec{mime_type:"audio/opus".into(),clock_rate:48000,channels:2,sdp_fmtp_line:"minptime=10;useinbandfec=1".into(),rtcp_feedback:vec![]};
        media.register_codec(RTCRtpCodecParameters{rtp_codec:codec.clone(),payload_type:AUDIO_PAYLOAD,..Default::default()},RtpCodecKind::Audio).map_err(|e|e.to_string())?;
        let registry=register_default_interceptors(rtc::interceptor::Registry::new(),&mut media).map_err(|e|e.to_string())?;
        let mut setting=SettingEngineBuilder::new().with_network_types(if tcp_only{vec![rtc::ice::network_type::NetworkType::Tcp4]}else{vec![rtc::ice::network_type::NetworkType::Udp4,rtc::ice::network_type::NetworkType::Tcp4]});
        if runtime.proxied() {
            // mDNS is local multicast and cannot follow the authenticated SOCKS route.
            setting = setting.with_multicast_dns_mode(rtc::ice::mdns::MulticastDnsMode::Disabled);
        }
        let setting = setting.build();
        let (stop,receiver)=tokio::sync::watch::channel(false);let gathered=Arc::new(tokio::sync::Notify::new());
        let handler=Arc::new(Handler{gathered:gathered.clone(),services:provider.services.clone(),id:id.clone(),stop:receiver.clone(),track:AtomicBool::new(false)});
        let pc:Arc<dyn PeerConnection>=Arc::new(PeerConnectionBuilder::new().with_configuration(RTCConfigurationBuilder::new().build()).with_setting_engine(setting).with_media_engine(media).with_interceptor_registry(registry).with_runtime(runtime).with_handler(handler).with_udp_addrs(if tcp_only{vec![]}else{vec!["0.0.0.0:0".to_string()]}).with_tcp_addrs(vec!["0.0.0.0:0".to_string()]).build().await.map_err(|e|e.to_string())?);
        let mut guard=Guard(Some(pc.clone()));
        let ssrc=0x4a415256;
        let track=Arc::new(TrackLocalStaticSample::new(Instant::now(),MediaStreamTrack::new("jarvis".into(),"audio".into(),"jarvis".into(),RtpCodecKind::Audio,vec![RTCRtpEncodingParameters{rtp_coding_parameters:RTCRtpCodingParameters{ssrc:Some(ssrc),..Default::default()},codec,..Default::default()}])).map_err(|e|e.to_string())?);
        pc.add_track(track.clone() as Arc<dyn TrackLocal>).await.map_err(|e|e.to_string())?;
        let dc=pc.create_data_channel("oai-events",None).await.map_err(|e|e.to_string())?;
        let (offer,auth)=tokio::try_join!(async {
            let offer=pc.create_offer(None).await.map_err(|e|e.to_string())?;
            pc.set_local_description(offer).await.map_err(|e|e.to_string())?;
            let _=tokio::time::timeout(Duration::from_secs(3),gathered.notified()).await;
            Ok::<_,String>(pc.local_description().await.ok_or("Нет предложения SDP")?.sdp)
        }, provider.token())?;
        let access=auth["access"].as_str().ok_or("Нет токена")?;let account=auth["account"].as_str().ok_or("Нет аккаунта")?;
        let instructions=params["instructions"].as_str().unwrap_or("");
        let thread=params["profile"]["thread"].as_str().filter(|s|!s.is_empty()&&s.len()<=128).unwrap_or(&id);
        let client = provider.client_for(&network)?;
        let send = |model| {
            let session=json!({"model":model,"instructions":instructions,"audio":{"output":{"voice":voice}},"delegation":{"type":"client"},"initial_items":[{"type":"message","role":"developer","content":[{"type":"input_text","text":instructions}]}]});
            client.post(&provider.call_url).bearer_auth(access).header("ChatGPT-Account-ID",account)
                .header("OpenAI-Alpha","quicksilver=v2").header("session-id",thread).header("thread-id",thread)
                .header("x-session-id",&id).header("originator","codex_cli_rs")
                .json(&json!({"sdp":offer,"session":session})).send()
        };
        let mut requested_model = LIVE_11;
        let mut response = send(requested_model).await.map_err(|_| "Не создать звонок ChatGPT")?;
        if matches!(response.status().as_u16(), 400 | 404 | 422) {
            let status = response.status();
            if !rejected_model(response).await {
                return Err(format!("ChatGPT отклонил голос {voice} или параметры звонка ({status})"));
            }
            requested_model = LIVE_1;
            response = send(requested_model).await.map_err(|_| "Не создать резервный звонок ChatGPT")?;
        }
        let status = response.status();
        if status.as_u16() == 429 {
            let retry = response.headers().get("Retry-After").and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok()).unwrap_or(60).clamp(1, 86400);
            emit(&provider.services, &id, "limit", json!(retry)).await;
            return Err(format!("ChatGPT ограничил голосовые звонки аккаунта (429). Повторите через {} мин.", retry.div_ceil(60)));
        }
        if !status.is_success() {
            return Err(format!("ChatGPT не открыл звонок с голосом {voice} на модели {requested_model} ({status})"));
        }
        let answer=response.text().await.map_err(|_|"Нет ответа SDP")?;if answer.len()>1024*1024{return Err("Слишком большой SDP".into());}
        if tcp_only && !answer.lines().any(|line|line.starts_with("a=candidate:") && line.to_ascii_lowercase().contains(" tcp ")){return Err("VPN не поддерживает UDP, а ChatGPT не предложил TCP ICE".into());}
        let bitrate = audio_bitrate(&answer);
        eprintln!("ChatGPT: Opus, микрофон 16 кГц моно, целевой битрейт {} кбит/с, ответ 48 кГц", bitrate / 1000);
        pc.set_remote_description(RTCSessionDescription::answer(answer).map_err(|e|e.to_string())?).await.map_err(|e|e.to_string())?;
        tokio::time::timeout(Duration::from_secs(12),async{loop{match dc.poll().await{Some(DataChannelEvent::OnOpen)=>return Ok(()),Some(DataChannelEvent::OnClose)|None=>return Err("Канал ChatGPT закрылся".to_string()),_=>{}}}}).await
            .map_err(|_|format!("Звонок ChatGPT с голосом {voice} не соединился за 12 секунд"))??;
        let pending=Arc::new(Mutex::new(Vec::<i16>::new()));let mut receiver_feed=receiver.clone();let feed=pending.clone();let services=provider.services.clone();let feed_id=id.clone();
        let feeder=tokio::spawn(async move{
            let mut encoder=match opus::Encoder::with_bitrate(bitrate){Ok(e)=>e,Err(e)=>{emit(&services,&feed_id,"error",e.into()).await;return;}};
            let mut mic = super::mic::MicClock::new();
            let mut tick=tokio::time::interval(Duration::from_millis(20));tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {tokio::select!{_=receiver_feed.changed()=>break,_=tick.tick()=>{
                let pcm = mic.take(&mut feed.lock().unwrap());
                let packet=match encoder.encode(&pcm){Ok(p)=>p,Err(_)=>break};
                if track.write_sample(ssrc,AUDIO_PAYLOAD,&rtc::media::Sample{data:packet.into(),duration:Duration::from_millis(20),..rtc::media::Sample::new(Instant::now())},&[]).await.is_err(){break;}
            }}}
        });
        let mut events_stop=receiver.clone();let channel=dc.clone();let services=provider.services.clone();let event_id=id.clone();
        let reader=tokio::spawn(async move{
            let mut work=tokio::task::JoinSet::new();let mut seen=std::collections::HashSet::new();
            loop{tokio::select!{
                biased;_=events_stop.changed()=>break,
                _=work.join_next(),if !work.is_empty()=>{},
                event=channel.poll()=>match event{
                    Some(DataChannelEvent::OnMessage(message))=>{
                        if message.data.len()>4*1024*1024{break;}let Ok(event)=serde_json::from_slice::<Value>(&message.data)else{continue;};let kind=event["type"].as_str().unwrap_or("");let text=event["item"]["text"].as_str().or_else(||event["delta"].as_str()).unwrap_or("");
                        match kind {
                            "session.started"|"session.created"|"session.updated"=>{
                                if let Some(model)=event["session"]["model"].as_str().filter(|model|model.starts_with("gpt-live-") && model.len()<=80 && model.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'.'|b'_'))){
                                    emit(&services,&event_id,"model",model.into()).await;
                                }
                            },
                            "session.input_transcript.delta"|"input_transcript.delta"|"input_transcript.added"|"session.input_transcript.added"=>emit(&services,&event_id,"user",text.into()).await,
                            "session.output_transcript.delta"|"output_transcript.delta"|"output_transcript.added"|"session.output_transcript.added"=>emit(&services,&event_id,"bot",text.into()).await,
                            "delegation.created"|"session.delegation.created"=>{
                                let item=event.get("item").unwrap_or(&event);let task=item["content"].as_array().into_iter().flatten().filter_map(|v|v["text"].as_str()).collect::<String>();let delegation=item["id"].as_str().unwrap_or("").to_string();
                                if task.is_empty() || delegation.is_empty() || !seen.insert(delegation.clone()){continue;}if seen.len()>1024 || work.len()>=8{break;}
                                let services=services.clone();let id=event_id.clone();let dc=channel.clone();emit(&services,&id,"working",json!(1)).await;
                                work.spawn(async move{let answer=tokio::time::timeout(Duration::from_secs(45),(services)("voice.brain".into(),json!({"id":id,"task":task}))).await.unwrap_or_else(|_|Err("Бэкенд не ответил вовремя".into())).unwrap_or_else(|e|json!(format!("Не получилось: {e}")));
                                    let text=answer.as_str().unwrap_or("");for chunk in chunks(text,500){if dc.send_text(&json!({"type":"delegation.context.append","delegation_item_id":delegation,"channel":"speakable","content":[{"type":"input_text","text":chunk}]}).to_string()).await.is_err(){break;}}emit(&services,&id,"working",json!(-1)).await;});
                            },
                            "session.closed"=>break,
                            "error"=>{
                                let detail=event["error"]["message"].as_str().or_else(||event["error"]["code"].as_str()).unwrap_or("Ошибка голосовой сессии ChatGPT");
                                emit(&services,&event_id,"error",json!(detail.chars().take(240).collect::<String>())).await;
                                let lower=detail.to_ascii_lowercase();
                                if lower.contains("quota") || lower.contains("limit"){emit(&services,&event_id,"limit",Value::Null).await;break;}
                            },_=>{},
                        }
                    },Some(DataChannelEvent::OnClose)|None=>break,_=>{},
                }
            }}emit(&services,&event_id,"closed",Value::Null).await;
        });
        guard.0.take();Ok(Arc::new(Self{id,requested_model,pc,dc,pending,stop,tasks:Mutex::new(vec![feeder,reader])}))
    }
    pub fn feed(&self,data:&str)->Result<(),String>{
        if data.len()>256*1024{return Err("Слишком большой кадр микрофона".into());}let bytes=B64.decode(data).map_err(|_|"Неверный PCM")?;if bytes.len()%2!=0{return Err("Неверный PCM".into());}
        let mut queue=self.pending.lock().unwrap();queue.extend(bytes.chunks_exact(2).map(|b|i16::from_le_bytes([b[0],b[1]])));if queue.len()>160000{let count=queue.len()-160000;queue.drain(..count);}Ok(())
    }
    pub async fn say(&self,text:&str)->Result<(),String>{
        for part in chunks(text,500){self.dc.send_text(&json!({"type":"session.context.append","channel":"speakable","content":[{"type":"input_text","text":part}]}).to_string()).await.map_err(|e|e.to_string())?;}
        Ok(())
    }
    pub async fn close(&self){self.stop.send_replace(true);let _=self.pc.close().await;let tasks=std::mem::take(&mut*self.tasks.lock().unwrap());for task in tasks{task.abort();let _=task.await;}}
}
impl Drop for RtcVoice{fn drop(&mut self){self.stop.send_replace(true);for task in self.tasks.lock().unwrap().drain(..){task.abort();}let pc=self.pc.clone();tokio::spawn(async move{let _=pc.close().await;});}}
fn chunks(text:&str,max:usize)->Vec<&str>{let mut result=vec![];let mut rest=text;while !rest.is_empty(){let mut end=rest.len().min(max);while !rest.is_char_boundary(end){end-=1;}if end==0{end=rest.chars().next().unwrap().len_utf8();}result.push(&rest[..end]);rest=&rest[end..];}result}
#[cfg(test)]mod tests;
