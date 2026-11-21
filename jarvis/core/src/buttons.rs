//! Сброс пароля веб-настроек кнопками колонки — если пароль забыли.
//! Последовательность: «громче», «тише» поочерёдно 8 раз подряд (+ − + − + − + −), между нажатиями
//! не дольше 1,5 с. Случайно так не нажать, а громкость в итоге остаётся прежней.
//! Кнопки читаем из /dev/input без захвата — Алиса их тоже видит, как обычно.

use std::time::{Duration, Instant};

#[cfg(not(target_os = "linux"))]
use std::{
    io::Read,
    sync::mpsc::{SyncSender, sync_channel},
};

const KEY_VOLUME_DOWN: u16 = 114;
const KEY_VOLUME_UP: u16 = 115;
const RESET_PRESSES: usize = 8;

/// Распознавание последовательности сброса по нажатиям.
#[derive(Default)]
pub struct Seq {
    keys: Vec<u16>,
    last: Option<Instant>,
}

impl Seq {
    /// Нажатие в момент now; true — набрана последовательность сброса.
    pub fn press(&mut self, k: u16, now: Instant) -> bool {
        if k != KEY_VOLUME_UP && k != KEY_VOLUME_DOWN {
            self.keys.clear();
            return false;
        }
        if self.last.is_none_or(|t| now.duration_since(t) > Duration::from_millis(1500)) || self.keys.last() == Some(&k) {
            self.keys.clear();
        }
        self.last = Some(now);
        if self.keys.is_empty() && k != KEY_VOLUME_UP {
            return false; // начинается с «громче»
        }
        self.keys.push(k);
        if self.keys.len() >= RESET_PRESSES {
            self.keys.clear();
            return true;
        }
        false
    }
}

/// Следить за кнопками устройств ввода в одном потоке; on_reset — набрана последовательность сброса.
pub fn watch(on_reset: impl Fn() + Send + 'static) {
    let devs: Vec<_> = std::fs::read_dir("/dev/input")
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event"))).collect())
        .unwrap_or_default();
    if devs.is_empty() {
        return;
    }
    watch_inner(devs, Box::new(on_reset));
}

#[cfg(target_os = "linux")]
fn watch_inner(devs: Vec<std::path::PathBuf>, on_reset: Box<dyn Fn() + Send + 'static>) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};

    const fn eviocgbit_key(len: usize) -> u32 {
        (2 << 30) | ((len as u32) << 16) | ((b'E' as u32) << 8) | 0x21
    }
    fn has_bit(bits: &[u8], code: u16) -> bool {
        let i = code as usize / 8;
        i < bits.len() && (bits[i] & (1 << (code as usize % 8))) != 0
    }

    let mut fds: Vec<OwnedFd> = Vec::new();
    let req = eviocgbit_key(96);
    for d in devs {
        let Ok(cpath) = CString::new(d.as_os_str().as_bytes()) else { continue };
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            continue;
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut bits = [0u8; 96];
        let rc = unsafe { libc::ioctl(owned.as_raw_fd(), req as _, bits.as_mut_ptr()) };
        if rc < 0 || has_bit(&bits, KEY_VOLUME_DOWN) || has_bit(&bits, KEY_VOLUME_UP) {
            fds.push(owned);
        }
    }
    if fds.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("buttons".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let size = 2 * std::mem::size_of::<std::ffi::c_long>() + 8;
            let mut buf = vec![0u8; size * 16];
            let mut pfds: Vec<libc::pollfd> = fds
                .iter()
                .map(|f| libc::pollfd {
                    fd: f.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            let mut seq = Seq::default();
            while !pfds.is_empty() {
                for p in &mut pfds {
                    p.revents = 0;
                }
                let rc = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as _, -1) };
                if rc < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                let mut remove = Vec::new();
                for (i, p) in pfds.iter().enumerate() {
                    if p.revents == 0 {
                        continue;
                    }
                    if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 && p.revents & libc::POLLIN == 0 {
                        remove.push(i);
                        continue;
                    }
                    loop {
                        let n = unsafe { libc::read(p.fd, buf.as_mut_ptr().cast(), buf.len()) };
                        if n < 0 {
                            let err = std::io::Error::last_os_error().kind();
                            if err == std::io::ErrorKind::WouldBlock || err == std::io::ErrorKind::Interrupted {
                                break;
                            }
                            remove.push(i);
                            break;
                        }
                        if n == 0 {
                            remove.push(i);
                            break;
                        }
                        let n = n as usize;
                        for ev in buf[..n - n % size].chunks_exact(size) {
                            let ev = &ev[size - 8..];
                            let (typ, code) = (u16::from_le_bytes([ev[0], ev[1]]), u16::from_le_bytes([ev[2], ev[3]]));
                            let val = i32::from_le_bytes([ev[4], ev[5], ev[6], ev[7]]);
                            if typ == 1 && val == 1 && seq.press(code, Instant::now()) {
                                on_reset();
                            }
                        }
                    }
                }
                for i in remove.into_iter().rev() {
                    pfds.remove(i);
                    fds.remove(i);
                }
            }
        })
        .ok();
}

#[cfg(not(target_os = "linux"))]
fn watch_inner(devs: Vec<std::path::PathBuf>, on_reset: Box<dyn Fn() + Send + 'static>) {
    let (tx, rx) = sync_channel::<u16>(16);
    for d in devs {
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("buttons-in".into())
            .stack_size(128 * 1024)
            .spawn(move || read_keys(&d, tx))
            .ok();
    }
    std::thread::Builder::new()
        .name("buttons".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut seq = Seq::default();
            for k in rx {
                if seq.press(k, Instant::now()) {
                    on_reset();
                }
            }
        })
        .ok();
}

#[cfg(not(target_os = "linux"))]
/// Нажатия (EV_KEY, value=1) из одного /dev/input/eventN.
fn read_keys(path: &std::path::Path, out: SyncSender<u16>) {
    let Ok(mut f) = std::fs::File::open(path) else { return };
    // struct input_event: timeval (2×long) + type u16 + code u16 + value s32
    let size = 2 * std::mem::size_of::<std::ffi::c_long>() + 8;
    let mut buf = vec![0u8; size * 16];
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => {
                log::info!("кнопки: {}: {e}", path.display());
                return;
            }
        };
        for ev in buf[..n - n % size].chunks_exact(size) {
            let ev = &ev[size - 8..];
            let (typ, code) = (u16::from_le_bytes([ev[0], ev[1]]), u16::from_le_bytes([ev[2], ev[3]]));
            let val = i32::from_le_bytes([ev[4], ev[5], ev[6], ev[7]]);
            if typ == 1 && val == 1 {
                let _ = out.try_send(code);
            }
        }
    }
}

/// Сигнал «пароль сброшен»: три восходящие ноты дважды, не похоже на короткие сигналы быстрых команд.
pub fn reset_chime() -> Vec<i16> {
    let rate = 16000.0;
    let mut out = vec![];
    for _ in 0..2 {
        for f in [660.0, 880.0, 1320.0] {
            let n = (0.11 * rate) as usize;
            for i in 0..n {
                let env = (i.min(n - i) as f64 / (0.008 * rate)).min(1.0);
                out.push((9000.0 * env * (2.0 * std::f64::consts::PI * f * i as f64 / rate).sin()) as i16);
            }
            out.extend(std::iter::repeat_n(0, 16000 / 40));
        }
        out.extend(std::iter::repeat_n(0, 16000 / 6));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut s = Seq::default();
        let mut hit = false;
        for i in 0..8 {
            hit = s.press(if i % 2 == 0 { KEY_VOLUME_UP } else { KEY_VOLUME_DOWN }, at(i * 500));
        }
        assert!(hit);
        // пауза больше 1,5 с — всё сначала
        let mut s = Seq::default();
        let mut n = 0;
        for i in 0..8u64 {
            n += usize::from(s.press(if i % 2 == 0 { KEY_VOLUME_UP } else { KEY_VOLUME_DOWN }, at(i * 500 + if i >= 4 { 2000 } else { 0 })));
        }
        assert_eq!(n, 0);
        // два «громче» подряд — сначала
        let mut s = Seq::default();
        assert!(!(0..8).any(|i| s.press(KEY_VOLUME_UP, at(i * 300))));
    }
}
