use jarvis_station::{CORE, FAILURE, NEXT, PENDING, PREVIOUS, Pending, atomic, sha256, sync_dir};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

static STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn main() -> io::Result<()> {
    let mut args = env::args_os().skip(1);
    let data = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| "/data/opt/jarvisd".into());
    let runtime = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/jarvisd".into());
    fs::create_dir_all(&runtime)?;
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, signal as *const () as libc::sighandler_t);
    }
    supervise(&data, &runtime)
}

fn supervise(data: &Path, runtime: &Path) -> io::Result<()> {
    let binary = data.join(CORE);
    let marker = data.join(PENDING);
    let next = data.join(NEXT);
    let previous = data.join(PREVIOUS);
    let ready = runtime.join("rust.ready");
    while !STOP.load(Ordering::Relaxed) && !data.join("disabled").exists() {
        let update = if marker.exists() {
            match prepare(data) {
                Ok(update) => Some(update),
                Err(error) => {
                    if !binary.exists() || !next.exists() {
                        restore(data)?;
                    }
                    record_failure(data, &error.to_string());
                    remove(&marker)?;
                    remove(&next)?;
                    None
                }
            }
        } else {
            None
        };
        remove(&ready)?;
        let log = runtime.join("jarvisd.log");
        if fs::metadata(&log).is_ok_and(|m| m.len() > 1 << 20) {
            let _ = fs::rename(&log, runtime.join("jarvisd.log.1"));
        }
        let output = fs::OpenOptions::new().create(true).append(true).open(log)?;
        let mut command = Command::new(&binary);
        command
            .arg("-data")
            .arg(data)
            .arg("-mic")
            .arg(runtime.join("mic.sock"))
            .env("JARVIS_READY_PATH", &ready)
            .env_remove("JARVIS_INSTALL_MODULE")
            .stdout(Stdio::from(output.try_clone()?))
            .stderr(Stdio::from(output));
        if let Some(module) = update.as_ref().and_then(|p| p.module.as_deref()) {
            command.env("JARVIS_INSTALL_MODULE", module);
        }
        match command.spawn() {
            Ok(mut child) => {
                if let Err(error) = fs::write(runtime.join("jarvisd.pid"), child.id().to_string()) {
                    terminate(&mut child);
                    return Err(error);
                }
                if let Some(pending) = update {
                    if wait_ready(&mut child, &ready, &pending.version)? {
                        remove(&marker)?;
                        sync_dir(data)?;
                    } else {
                        terminate(&mut child);
                        rollback(data, "Новая сборка не прошла проверку запуска")?;
                        remove(&runtime.join("jarvisd.pid"))?;
                        continue;
                    }
                }
                while child.try_wait()?.is_none() {
                    if STOP.load(Ordering::Relaxed) || data.join("disabled").exists() {
                        terminate(&mut child);
                        break;
                    }
                    thread::sleep(Duration::from_millis(1000));
                }
            }
            Err(error) if update.is_some() => {
                rollback(data, &format!("Новая сборка не запустилась: {error}"))?;
            }
            Err(error) => {
                if !binary.exists() && previous.exists() {
                    restore(data)?;
                }
                eprintln!("jarvis: {error}");
            }
        }
        remove(&runtime.join("jarvisd.pid"))?;
        for _ in 0..10 {
            if STOP.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn prepare(dir: &Path) -> io::Result<Pending> {
    let update: Pending =
        serde_json::from_slice(&fs::read(dir.join(PENDING))?).map_err(io::Error::other)?;
    let current = dir.join(CORE);
    let previous = dir.join(PREVIOUS);
    let next = dir.join(NEXT);
    let candidate = if next.exists() { &next } else { &current };
    if sha256(candidate)? != update.sha256 {
        return Err(io::Error::other("SHA-256 обновления не совпал"));
    }
    if candidate == &next {
        if current.exists() {
            if previous.exists() {
                return Err(io::Error::other("Занят файл отката"));
            }
            fs::rename(&current, &previous)?;
            sync_dir(dir)?;
        }
        fs::rename(&next, &current)?;
        sync_dir(dir)?;
    }
    Ok(update)
}

fn restore(dir: &Path) -> io::Result<()> {
    let backup = dir.join(PREVIOUS);
    if backup.is_file() {
        fs::rename(backup, dir.join(CORE))?;
        sync_dir(dir)?;
    }
    Ok(())
}

fn record_failure(dir: &Path, text: &str) {
    let _ = atomic(&dir.join(FAILURE), text.as_bytes());
}

fn rollback(dir: &Path, text: &str) -> io::Result<()> {
    restore(dir)?;
    record_failure(dir, text);
    remove(&dir.join(PENDING))?;
    sync_dir(dir)
}

fn wait_ready(child: &mut Child, ready: &Path, version: &str) -> io::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(25);
    let expected = format!("{} {version}", child.id());
    while Instant::now() < deadline && !STOP.load(Ordering::Relaxed) {
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if fs::read_to_string(ready).is_ok_and(|s| s == expected) {
            thread::sleep(Duration::from_secs(1));
            return Ok(child.try_wait()?.is_none());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(false)
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    for _ in 0..20 {
        if child.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}
