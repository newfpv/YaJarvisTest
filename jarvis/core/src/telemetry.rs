//! Local system readings and a bounded 24-hour history. No network uploads.
use serde::Serialize;
use std::{collections::VecDeque, path::{Path,PathBuf}, sync::{Arc,Mutex}, time::{Duration,Instant}};

pub const EVERY:u64=15;
const KEEP:usize=24*60*60/EVERY as usize;
#[derive(Clone,Default,Serialize)]
pub struct System {
    pub temp_c:f64, pub mem_total_mb:u64, pub mem_avail_mb:u64, pub load1:f64,
    pub wifi_dbm:i32, pub uptime_s:u64, pub disk_free_mb:u64, pub disk_total_mb:u64,
}
#[derive(Clone,Default,Serialize)]
pub struct Point { pub t:i64, pub cpu:f32, pub temp:f32, pub mem:u32, pub wifi:i16, pub disk:u32, pub load:f32, pub me:f32 }
#[derive(Clone,Copy,Default)]
struct Cpu {busy:u64,total:u64,own:u64}
#[derive(Default)]
struct State {
    sys: System,
    points: VecDeque<Point>,
    cpu: Option<Cpu>,
    sampled: Option<Instant>,
    data: Option<PathBuf>,
    live_at: Option<Instant>,
    wifi_at: Option<Instant>,
    wifi_val: i32,
}
#[derive(Default)]
pub struct Telemetry {state:Mutex<State>}

impl Telemetry {
    pub fn start(data:PathBuf)->Arc<Self> {
        let result=Arc::new(Self::default());
        result.state.lock().unwrap().data=Some(data.clone());
        let weak=Arc::downgrade(&result);
        tokio::spawn(async move {
            let mut timer=tokio::time::interval(Duration::from_secs(EVERY));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;let Some(metrics)=weak.upgrade()else{break;};
                let root=data.clone();let Ok((mut sys,cpu))=tokio::task::spawn_blocking(move||read(&root)).await else{continue;};
                if sys.wifi_dbm==0 {
                    sys.wifi_dbm=metrics.resolve_wifi().await;
                }
                metrics.record(sys,cpu,unix(),Instant::now());
            }
        });result
    }
    async fn resolve_wifi(&self)->i32 {
        let cached={
            let state=self.state.lock().unwrap();
            state.wifi_at.is_some_and(|at|at.elapsed()<Duration::from_secs(EVERY)).then_some(state.wifi_val)
        };
        if let Some(v)=cached {return v;}
        let v=wifi_fallback().await;
        let mut state=self.state.lock().unwrap();
        state.wifi_at=Some(Instant::now());
        state.wifi_val=v;
        v
    }
    pub async fn live_system(&self)->System {
        let data={
            let state=self.state.lock().unwrap();
            if state.live_at.is_some_and(|at|at.elapsed()<Duration::from_secs(2)) {
                return state.sys.clone();
            }
            state.data.clone()
        };
        let Some(root)=data else{return self.system();};
        let Ok((mut sys,_))=tokio::task::spawn_blocking(move||read(&root)).await else{return self.system();};
        if sys.wifi_dbm==0 {
            sys.wifi_dbm=self.resolve_wifi().await;
        }
        let mut state=self.state.lock().unwrap();
        state.sys=sys.clone();
        state.live_at=Some(Instant::now());
        sys
    }
    fn record(&self,sys:System,cpu:Option<Cpu>,now:i64,at:Instant) {
        let mut state=self.state.lock().unwrap();state.sys=sys.clone();state.live_at=Some(at);
        if state.sampled.is_some_and(|last|at.duration_since(last)<Duration::from_secs(EVERY)){return;}
        let (usage,own)=cpu.zip(state.cpu).map(|(current,previous)|cpu_delta(current,previous)).unwrap_or_default();
        let point=Point {
            t:now,
            cpu:usage as f32,
            me:own as f32,
            temp:sys.temp_c as f32,
            mem:sys.mem_total_mb.saturating_sub(sys.mem_avail_mb).min(u32::MAX as u64) as u32,
            wifi:sys.wifi_dbm.clamp(i16::MIN as i32,i16::MAX as i32) as i16,
            disk:sys.disk_free_mb.min(u32::MAX as u64) as u32,
            load:sys.load1 as f32,
        };
        state.cpu=cpu;state.sampled=Some(at);state.points.push_back(point);
        while state.points.len()>KEEP{state.points.pop_front();}
    }
    pub fn system(&self)->System{self.state.lock().unwrap().sys.clone()}
    pub fn since(&self,seconds:i64)->Vec<Point>{
        let seconds=if (1..=86400).contains(&seconds){seconds}else{900};let from=unix().saturating_sub(seconds);
        self.state.lock().unwrap().points.iter().filter(|p|p.t>=from).cloned().collect()
    }
}
fn unix()->i64{std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) as i64}
fn cpu_delta(current:Cpu,previous:Cpu)->(f64,f64){
    let Some(total)=current.total.checked_sub(previous.total).filter(|n|*n>0)else{return(0.0,0.0);};
    let pct=|n:u64|((n as f64)*100.0/(total as f64)).clamp(0.0,100.0);
    (pct(current.busy.saturating_sub(previous.busy)),pct(current.own.saturating_sub(previous.own)))
}
fn cpu_times(text:&str,process:&str)->Option<Cpu>{
    let mut fields=text.lines().next()?.split_whitespace();if fields.next()?!="cpu"{return None;}
    // guest and guest_nice are already included in user/nice. Count only fields 1..8.
    let ticks=fields.take(8).map(str::parse::<u64>).collect::<Result<Vec<_>,_>>().ok()?;if ticks.len()<4{return None;}
    let total=ticks.iter().try_fold(0u64,|sum,n|sum.checked_add(*n))?;
    let idle=ticks[3].saturating_add(ticks.get(4).copied().unwrap_or(0));
    let after=process.rsplit_once(')').map(|(_,rest)|rest).unwrap_or("");let stat:Vec<_>=after.split_whitespace().collect();
    let own=stat.get(11).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0).saturating_add(stat.get(12).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0));
    Some(Cpu{busy:total.saturating_sub(idle),total,own})
}
fn memory(text:&str)->(u64,u64){
    let field=|key:&str|text.lines().find_map(|line|line.strip_prefix(key)?.split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0);
    let total=field("MemTotal:");let available=text.lines().any(|l|l.starts_with("MemAvailable:"));
    let free=if available{field("MemAvailable:")}else{field("MemFree:").saturating_add(field("Buffers:")).saturating_add(field("Cached:")).saturating_add(field("SReclaimable:")).saturating_sub(field("Shmem:"))};
    (total/1024,free.min(total)/1024)
}
fn wireless(text:&str)->i32{
    text.lines().find_map(|line|{let f:Vec<_>=line.split_whitespace().collect();if f.len()<4 || !f[0].ends_with(':'){return None;}
        let mut dbm=f[3].trim_end_matches('.').parse::<i32>().ok()?;if (129..=255).contains(&dbm){dbm-=256;}
        (-120..0).contains(&dbm).then_some(dbm)
    }).unwrap_or(0)
}
fn rssi(text:&str)->i32{text.lines().find_map(|line|{let n=line.trim().strip_prefix("RSSI=")?.parse::<i32>().ok()?;(-119..0).contains(&n).then_some(n)}).unwrap_or(0)}
async fn wifi_fallback()->i32{
    #[cfg(target_os="linux")]
    if let Ok(Ok(out))=tokio::time::timeout(Duration::from_secs(2),tokio::process::Command::new("wpa_cli").args(["-i","wlan0","signal_poll"]).kill_on_drop(true).output()).await {
        if out.status.success(){return rssi(&String::from_utf8_lossy(&out.stdout));}
    }
    0
}
fn read(data:&Path)->(System,Option<Cpu>){
    let file=|path:&str|std::fs::read_to_string(path).unwrap_or_default();
    let (total,available)=memory(&file("/proc/meminfo"));let (free,disk)=disk(data);
    let first=|text:String|text.split_whitespace().next().and_then(|v|v.parse::<f64>().ok()).filter(|n|n.is_finite()&&*n>=0.0).unwrap_or(0.0);
    let temperature=file("/sys/class/thermal/thermal_zone0/temp").trim().parse::<f64>().ok().filter(|n|n.is_finite()&&*n>0.0).unwrap_or(0.0)/1000.0;
    let sys=System{temp_c:temperature,mem_total_mb:total,mem_avail_mb:available,load1:first(file("/proc/loadavg")),wifi_dbm:wireless(&file("/proc/net/wireless")),uptime_s:first(file("/proc/uptime")) as u64,disk_free_mb:free,disk_total_mb:disk};
    (sys,cpu_times(&file("/proc/stat"),&file("/proc/self/stat")))
}
fn disk(path:&Path)->(u64,u64){
    #[cfg(unix)] {use std::os::unix::ffi::OsStrExt;
        if let Ok(path)=std::ffi::CString::new(path.as_os_str().as_bytes()){
            let mut s:libc::statvfs=unsafe{std::mem::zeroed()};if unsafe{libc::statvfs(path.as_ptr(),&mut s)}==0{return((s.f_bavail as u64).saturating_mul(s.f_frsize as u64)>>20,(s.f_blocks as u64).saturating_mul(s.f_frsize as u64)>>20);}
        }
    }
    #[cfg(not(unix))] let _=path;
    (0,0)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn linux_readings_handle_old_kernels_and_guest_ticks(){
        assert_eq!(memory("MemTotal: 262144 kB\nMemAvailable: 32768 kB\nMemFree: 100 kB\n"),(256,32));
        assert_eq!(memory("MemTotal: 262144 kB\nMemFree: 16384 kB\nBuffers: 1024 kB\nCached: 16384 kB\nSReclaimable: 2048 kB\nShmem: 3072 kB\n"),(256,32));
        assert_eq!(wireless("wlan0: 0000 60. -50. -256\n"),-50);assert_eq!(wireless("wlan0: 0000 60. 207. -256\n"),-49);assert_eq!(rssi("FAIL\nRSSI=-61\n"),-61);assert_eq!(rssi("RSSI=-999"),0);
        let own="1 (odd ) worker) R 0 0 0 0 0 0 0 0 0 0 12 8";
        let first=cpu_times("cpu 20 2 3 70 5 0 0 0 9 1\n",own).unwrap();assert_eq!((first.busy,first.total,first.own),(25,100,20));
        assert_eq!(cpu_delta(Cpu{busy:75,total:200,own:30},first),(50.0,10.0));assert_eq!(cpu_delta(Cpu{busy:0,total:1,own:0},first),(0.0,0.0));
    }
    #[test]fn history_is_bounded_and_sampling_does_not_depend_on_http_polling(){
        let stats=Telemetry::default();let start=Instant::now();let now=unix();
        for i in 0..KEEP+2{stats.record(System::default(),None,now+i as i64*EVERY as i64,start+Duration::from_secs(i as u64*EVERY));}
        assert_eq!(stats.state.lock().unwrap().points.len(),KEEP);let count=stats.since(86400).len();assert_eq!(count,KEEP);
        stats.record(System{temp_c:42.0,..Default::default()},None,now,start+Duration::from_secs((KEEP as u64+1)*EVERY+1));
        assert_eq!(stats.system().temp_c,42.0);assert_eq!(stats.state.lock().unwrap().points.len(),KEEP);
    }
}
