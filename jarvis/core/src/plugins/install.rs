//! Установка плагина: архив zip с компьютера или из релиза GitHub. Проверяем всё: только обычные файлы,
//! без выхода за папку, разумный размер, правильный plugin.json. Данные плагина переживают обновление.

use super::{Host, Manifest, PluginState, valid_id};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

pub const ZIP_MAX: usize = 24 << 20;
pub const UNPACK_MAX: usize = 48 << 20;
pub const INSTALL_DEFERRED: &str = "Колонка занята. Установка отложена";

#[derive(serde::Deserialize)]
pub struct InstallRequest {
    pub id: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub expected_version: String,
    #[serde(default)]
    pub update: bool,
}

pub struct Installation {
    pub manifest: Manifest,
    pub changed: bool,
    pub reboot_from: Option<String>,
}

pub fn boot_id() -> Option<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(id.trim().to_owned()).filter(|id| !id.is_empty())
}

fn reboot_origin(data_dir: &Path) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    if data_dir == Path::new("/data/opt/jarvisd")
        && unsafe { libc::geteuid() } == 0
        && Path::new("/etc/init.d/S98jarvisd").is_file()
        && Path::new("/sbin/reboot").is_file()
        && Path::new("/system/vendor/quasar/launch.json").is_file()
        && let Some(id) = boot_id()
    {
        return Ok(id);
    }
    let _ = data_dir;
    Err("Для обновления плагина нужна перезагрузка колонки, здесь она недоступна".into())
}

struct ArchPlan {
    prefix: String,
    entries: Vec<(usize, String)>,
    manifest: Manifest,
    total_hint: u64,
}

fn inspect_archive<R: Read + std::io::Seek>(zr: &mut zip::ZipArchive<R>) -> Result<ArchPlan, String> {
    let mut raw_names: Vec<(usize, String, u64)> = Vec::new();
    let mut root_manifest: Option<usize> = None;
    let mut nested_manifests: Vec<(usize, String)> = Vec::new();
    for i in 0..zr.len() {
        let f = zr.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("");
        if f.is_dir() || base.starts_with("__MACOSX") || name.contains("/__MACOSX/") || name.starts_with("__MACOSX/") {
            continue;
        }
        if f.is_symlink() || !f.is_file() {
            return Err(format!("в архиве не обычный файл: {name}"));
        }
        let clean_ok = !name.is_empty()
            && !name.starts_with('/')
            && name.split('/').all(|p| !p.is_empty() && p != "." && p != "..");
        if !clean_ok {
            return Err(format!("в архиве недопустимый путь: {name}"));
        }
        if name == "plugin.json" {
            root_manifest = Some(i);
        } else if name.ends_with("/plugin.json") && name.matches('/').count() == 1 {
            nested_manifests.push((i, name.trim_end_matches("plugin.json").to_string()));
        }
        raw_names.push((i, name, f.size()));
    }
    let (manifest_idx, prefix) = if let Some(idx) = root_manifest {
        (idx, String::new())
    } else if let Some((idx, pfx)) = nested_manifests.into_iter().next() {
        (idx, pfx)
    } else {
        return Err("в архиве нет plugin.json".into());
    };
    let mut mf = zr.by_index(manifest_idx).map_err(|e| e.to_string())?;
    let mut mf_bytes = Vec::new();
    mf.by_ref()
        .take((256 * 1024 + 1) as u64)
        .read_to_end(&mut mf_bytes)
        .map_err(|e| e.to_string())?;
    if mf_bytes.len() > 256 * 1024 {
        return Err("plugin.json слишком большой".into());
    }
    drop(mf);
    let m: Manifest = serde_json::from_slice(&mf_bytes).map_err(|e| format!("plugin.json: {e}"))?;
    m.validate()?;
    let mut entries = Vec::new();
    let mut total_hint = 0u64;
    let mut has_main = false;
    for (idx, name, sz) in raw_names {
        let Some(rel) = name.strip_prefix(&prefix) else { continue };
        if rel.is_empty() {
            continue;
        }
        if rel == m.main {
            has_main = true;
        }
        total_hint = total_hint.saturating_add(sz);
        entries.push((idx, rel.to_string()));
    }
    if matches!(m.kind.as_str(), "lua" | "rust") && !has_main {
        return Err(format!("в архиве нет {}", m.main));
    }
    Ok(ArchPlan { prefix, entries, manifest: m, total_hint })
}

fn unpack_plan_to_dir<R: Read + std::io::Seek>(
    zr: &mut zip::ZipArchive<R>,
    plan: &ArchPlan,
    dir: &Path,
) -> Result<(), String> {
    let mut buf = [0u8; 32 * 1024];
    let mut total = 0usize;
    let mut elf_head = [0u8; 20];
    let mut elf_len = 0usize;
    for (idx, rel) in &plan.entries {
        let mut f = zr.by_index(*idx).map_err(|e| e.to_string())?;
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut opt = std::fs::OpenOptions::new();
        opt.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mode = if plan.manifest.kind == "rust" && rel == &plan.manifest.main { 0o700 } else { 0o644 };
            opt.mode(mode);
        }
        let mut out = opt.open(&p).map_err(|e| e.to_string())?;
        let is_main = plan.manifest.kind == "rust" && rel == &plan.manifest.main;
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total = total.checked_add(n).ok_or_else(|| format!("плагин больше {} МБ в распакованном виде", UNPACK_MAX >> 20))?;
            if total > UNPACK_MAX {
                return Err(format!("плагин больше {} МБ в распакованном виде", UNPACK_MAX >> 20));
            }
            if is_main && elf_len < 20 {
                let take = (20 - elf_len).min(n);
                elf_head[elf_len..elf_len + take].copy_from_slice(&buf[..take]);
                elf_len += take;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
    }
    if plan.manifest.kind == "rust" {
        super::process::check_binary(&elf_head[..elf_len], &plan.manifest.arch)?;
    }
    Ok(())
}

/// Разобрать и проверить архив. Архив может лежать как есть или в одной папке верхнего уровня (как его делает GitHub).
pub fn unpack(raw: &[u8]) -> Result<(BTreeMap<String, Vec<u8>>, Manifest), String> {
    if raw.len() > ZIP_MAX {
        return Err(format!("архив плагина больше {} МБ", ZIP_MAX >> 20));
    }
    let mut zr = zip::ZipArchive::new(std::io::Cursor::new(raw)).map_err(|_| "это не zip-архив плагина".to_string())?;
    let plan = inspect_archive(&mut zr)?;
    let _ = &plan.prefix;
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for (idx, rel) in &plan.entries {
        let mut f = zr.by_index(*idx).map_err(|e| e.to_string())?;
        let mut b = Vec::new();
        f.by_ref().take((UNPACK_MAX - total + 1) as u64).read_to_end(&mut b).map_err(|e| e.to_string())?;
        total += b.len();
        if total > UNPACK_MAX {
            return Err(format!("плагин больше {} МБ в распакованном виде", UNPACK_MAX >> 20));
        }
        files.insert(rel.clone(), b);
    }
    if plan.manifest.kind == "rust" {
        super::process::check_binary(&files[&plan.manifest.main], &plan.manifest.arch)?;
    }
    Ok((files, plan.manifest))
}

pub fn free_mb(dir: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut p = dir.as_os_str().as_bytes().to_vec();
        p.push(0);
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(p.as_ptr() as *const libc::c_char, &mut s) } == 0 {
            return (s.f_bavail as u64 * s.f_frsize as u64) >> 20;
        }
    }
    let _ = dir;
    0
}

struct TempFile(std::path::PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn download_to_temp(url: &str, max: usize, path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let mut resp = crate::net::direct()
        .get(url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| crate::net::short(&e))?;
    let code = resp.status().as_u16();
    if code != 200 {
        return Err(format!("скачивание: ответ {code}"));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let f = tokio::fs::File::create(path).await.map_err(|e| e.to_string())?;
    let mut wr = tokio::io::BufWriter::with_capacity(32 * 1024, f);
    let mut hasher = Sha256::new();
    let mut written = 0usize;
    while let Some(chunk) = resp.chunk().await.map_err(|e| crate::net::short(&e))? {
        written += chunk.len();
        if written > max {
            return Err(format!("файл больше {} МБ", max >> 20));
        }
        hasher.update(&chunk);
        wr.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    wr.flush().await.map_err(|e| e.to_string())?;
    Ok(hex::encode(hasher.finalize()))
}

impl Host {
    /// Поставить (или обновить) плагин из zip. source — owner/repo или file.
    pub async fn install(self: &Arc<Self>, raw: &[u8], source: &str, official: bool) -> Result<Installation, String> {
        if raw.len() > ZIP_MAX {
            return Err(format!("архив плагина больше {} МБ", ZIP_MAX >> 20));
        }
        self.install_reader(std::io::Cursor::new(raw), source, official, None, || true).await
    }

    async fn install_reader<R: Read + std::io::Seek + Send>(
        self: &Arc<Self>,
        rd: R,
        source: &str,
        official: bool,
        request: Option<&InstallRequest>,
        can_apply: impl Fn() -> bool + Send,
    ) -> Result<Installation, String> {
        let _g = self.install_lock.lock().await; // две установки сразу делили бы одну временную папку
        if !can_apply() {
            return Err(INSTALL_DEFERRED.into());
        }
        let mut zr = zip::ZipArchive::new(rd).map_err(|_| "это не zip-архив плагина".to_string())?;
        let plan = inspect_archive(&mut zr)?;
        let m = plan.manifest.clone();
        if plan.total_hint > UNPACK_MAX as u64 {
            return Err(format!("плагин больше {} МБ в распакованном виде", UNPACK_MAX >> 20));
        }
        if let Some(request) = request {
            if (official || request.update) && m.id != request.id {
                return Err("В архиве другой плагин".into());
            }
            if !request.expected_version.is_empty() && m.version != request.expected_version {
                return Err(format!("В доступном архиве версия {}, запрошена {}. Обновите каталог и попробуйте снова.", m.version, request.expected_version));
            }
        }
        if m.kind == "module" && !(official && source == "core") {
            return Err("Rust-модуль выбирается из официального каталога".into());
        }
        if m.kind != "module" && super::modules::built_in(&m.id)
            && !(m.kind == "rust" && official && source == crate::DEFAULT_REPO && m.provides == [m.id.clone()])
        {
            return Err("Этот идентификатор зарезервирован для официального плагина".into());
        }
        let later = m.needs_newer_core();
        if later {
            return Err(format!("плагину нужен Джарвис {} или новее — сначала обновите Джарвиса", m.min_core));
        }
        let (previous, previous_state) = {
            let state = self.inner.lock().unwrap();
            (state.plugins.get(&m.id).map(|p| (p.manifest.clone(), p.err.clone())), state.state.get(&m.id).cloned())
        };
        let had = previous.is_some();
        let updating = request.is_some_and(|r| r.update);
        if updating && (!had || !previous_state.as_ref().is_some_and(|s| s.source.eq_ignore_ascii_case(source))) {
            return Err("Исходный плагин для обновления не найден".into());
        }
        if let Some((prior, _)) = &previous {
            if (updating || official) && super::newer(&prior.version, &m.version) {
                if updating && !official && request.is_some_and(|r| r.expected_version.is_empty()) {
                    return Ok(Installation { manifest: prior.clone(), changed: false, reboot_from: None });
                }
                return Err(format!("На колонке уже стоит более новая версия {}", prior.version));
            }
            if updating && m.version == prior.version {
                return Ok(Installation { manifest: prior.clone(), changed: false, reboot_from: None });
            }
        }
        let reboot_from = if m.reboot_on_update && previous.as_ref().is_some_and(|(prior, _)| prior.version != m.version)
            && previous_state.as_ref().is_some_and(|state| state.enabled) {
            Some(reboot_origin(&self.data_dir)?)
        } else {
            None
        };
        let free = free_mb(&self.data_dir);
        if free > 0 && free < (plan.total_hint >> 20) + 3 {
            return Err(format!("мало места на колонке: свободно {free} МБ, плагину нужно {} МБ", (plan.total_hint >> 20) + 1));
        }
        let dst = self.dir().join(&m.id);
        let tmp = self.dir().join(format!("{}.tmp", m.id));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        if let Err(e) = unpack_plan_to_dir(&mut zr, &plan, &tmp) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        drop(zr);
        if !can_apply() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(INSTALL_DEFERRED.into());
        }
        let bak = self.dir().join(format!("{}.old", m.id));
        if bak.exists() {
            std::fs::remove_dir_all(&bak).map_err(|e| e.to_string())?;
        }
        if had {
            self.stop(&m.id).await;
        }
        if let Err(error) = replace_package(&tmp, &dst, &bak) {
            if had {
                self.start(&m.id).await;
            }
            return Err(error.to_string());
        }
        let enabled = {
            let mut g = self.inner.lock().unwrap();
            g.plugins.insert(
                m.id.clone(),
                super::Plugin {
                    manifest: m.clone(),
                    dir: dst.clone(),
                    rt: None,
                    err: if later { format!("заработает после обновления Джарвиса до {}", m.min_core) } else { String::new() },
                },
            );
            let st = g.state.entry(m.id.clone()).or_insert_with(PluginState::default);
            st.source = source.into();
            st.official = official;
            st.installed_at = super::now_rfc3339();
            if !had {
                st.enabled = true;
            }
            st.removed = false;
            let en = st.enabled;
            self.save(&g);
            self.sync_wake_training(&g);
            en
        };
        if enabled && reboot_from.is_none() {
            self.start(&m.id).await;
        }
        if m.kind == "rust" && enabled && reboot_from.is_none() {
            let error = self.inner.lock().unwrap().plugins.get(&m.id)
                .filter(|p| !p.rt.as_ref().is_some_and(super::Runtime::running))
                .map(|p| p.err.clone());
            if let Some(error) = error {
                self.stop(&m.id).await;
                if bak.exists() {
                    replace_package(&bak, &dst, &tmp).map_err(|e| e.to_string())?;
                    let _ = std::fs::remove_dir_all(&tmp);
                } else {
                    std::fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
                }
                let restart = previous_state.as_ref().is_some_and(|state| state.enabled);
                {
                    let mut state = self.inner.lock().unwrap();
                    state.plugins.remove(&m.id);
                    if let Some((manifest, err)) = previous {
                        state.plugins.insert(m.id.clone(), super::Plugin { manifest, dir: dst, rt: None, err });
                    }
                    if let Some(prior) = previous_state {
                        state.state.insert(m.id.clone(), prior);
                    } else {
                        state.state.remove(&m.id);
                    }
                    self.save(&state);
                    self.sync_wake_training(&state);
                }
                if restart {
                    self.start(&m.id).await;
                }
                return Err(if error.is_empty() { "Процесс плагина завершился".into() } else { error });
            }
        }
        let _ = std::fs::remove_dir_all(&bak);
        log::info!("плагины: установлен {} {} ({source})", m.id, m.version);
        Ok(Installation { manifest: m, changed: true, reboot_from })
    }

    /// Скачать релиз репозитория GitHub и поставить. tag "" — последний релиз.
    /// Ищется jarvis-plugin-<arch>.zip, иначе jarvis-plugin.zip; для официальных — plugin-<id>[-arch].zip
    /// в релизе самого Джарвиса.
    pub async fn install_repo(self: &Arc<Self>, request: &InstallRequest, can_apply: impl Fn() -> bool + Send) -> Result<Installation, String> {
        let repo = if request.repo.is_empty() { crate::DEFAULT_REPO } else { &request.repo };
        let id = request.id.as_str();
        if !super::catalog::valid_repo(repo) {
            return Err("репозиторий — вида owner/name".into());
        }
        let official = repo.eq_ignore_ascii_case(crate::DEFAULT_REPO);
        if official {
            if !valid_id(id) {
                return Err("не указан плагин".into());
            }
            let native = super::modules::built_in(id) || {
                let (items, _) = self.official_catalog(false).await;
                items.iter().any(|item| item["id"] == id && item["kind"] == "rust")
            };
            if native {
                let tmp_path = self.dir().join(format!("{id}.dl.zip"));
                let _guard = TempFile(tmp_path.clone());
                download_official_rust(id, &tmp_path).await?;
                let file = std::fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
                return self.install_reader(std::io::BufReader::new(file), crate::DEFAULT_REPO, true, Some(request), can_apply).await;
            }
        }
        #[cfg(feature = "lua")]
        if official {
            let raw = download_official_lua(id).await?;
            return self.install_reader(std::io::Cursor::new(&raw), crate::DEFAULT_REPO, true, Some(request), can_apply).await;
        }
        let names: Vec<String> = if official {
            if !valid_id(id) {
                return Err("не указан плагин".into());
            }
            vec![format!("plugin-{id}.zip")]
        } else {
            vec![format!("jarvis-plugin-{}.zip", super::arch()), "jarvis-plugin.zip".into()]
        };
        let api = super::catalog::github_api();
        let rel_url = format!("{api}/repos/{repo}/releases/latest");
        let rel = crate::net::get_json(&rel_url).await.map_err(|e| format!("релиз {repo}: {e}"))?;
        let assets = rel.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let link = names.iter().find_map(|want| {
            assets.iter().find_map(|a| {
                let ok = a.get("name").and_then(|v| v.as_str()) == Some(want.as_str())
                    && a.get("size").and_then(|v| v.as_u64()).unwrap_or(0) as usize <= ZIP_MAX;
                ok.then(|| a.get("browser_download_url").and_then(|v| v.as_str()).unwrap_or("").to_string())
            })
        });
        let Some(link) = link.filter(|l| !l.is_empty()) else {
            return Err(format!("в последнем релизе {repo} нет {}", names.join(" или ")));
        };
        let tmp_path = self.dir().join("repo.dl.zip");
        let _guard = TempFile(tmp_path.clone());
        download_to_temp(&link, ZIP_MAX, &tmp_path).await?;
        let file = std::fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
        self.install_reader(std::io::BufReader::new(file), repo, official, Some(request), can_apply).await
    }
}

async fn download_official_rust(id: &str, tmp_path: &Path) -> Result<(), String> {
    let asset = format!("plugin-{id}-{}.zip", super::arch());
    let base = if let Some(base) = option_env!("JARVIS_PLUGIN_ASSETS") {
        base.to_owned()
    } else {
        let tag = format!("v{}", crate::version());
        format!("https://github.com/{}/releases/download/{tag}", crate::DEFAULT_REPO)
    };
    let sums = crate::net::download(&format!("{base}/SHA256SUMS"), 64 << 10).await
        .map_err(|error| format!("Пакеты для этой версии недоступны: {error}"))?;
    let sums = std::str::from_utf8(&sums).map_err(|_| "Неверный список контрольных сумм")?;
    let expected = jarvis_station::checksum(sums, &asset).ok_or("Для этой архитектуры нет официального пакета")?;
    let actual = download_to_temp(&format!("{base}/{asset}"), ZIP_MAX, tmp_path).await?;
    if actual != expected {
        return Err("Контрольная сумма официального пакета не совпала".into());
    }
    let file = std::fs::File::open(tmp_path).map_err(|e| e.to_string())?;
    let mut zr = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|_| "это не zip-архив плагина".to_string())?;
    let plan = inspect_archive(&mut zr)?;
    if plan.manifest.id != id || plan.manifest.kind != "rust" {
        return Err("В архиве другой плагин".into());
    }
    Ok(())
}

fn replace_package(tmp: &Path, dst: &Path, backup: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        return std::fs::rename(tmp, dst);
    }
    std::fs::rename(dst, backup)?;
    let mut moved = Vec::new();
    let result = (|| {
        for name in ["data.json", "data"] {
            let old = backup.join(name);
            if !old.exists() {
                continue;
            }
            let staged = tmp.join(name);
            if staged.is_dir() {
                std::fs::remove_dir_all(&staged)?;
            } else if staged.exists() {
                std::fs::remove_file(&staged)?;
            }
            std::fs::rename(old, staged)?;
            moved.push(name);
        }
        std::fs::rename(tmp, dst)
    })();
    if result.is_err() {
        for name in moved {
            std::fs::rename(tmp.join(name), backup.join(name))?;
        }
        std::fs::rename(backup, dst)?;
        let _ = std::fs::remove_dir_all(tmp);
    }
    result
}

#[cfg(feature = "lua")]
async fn download_official_lua(id: &str) -> Result<Vec<u8>, String> {
    if !valid_id(id) { return Err("Не указан плагин".into()); }
    let manifest = crate::net::download(&super::catalog::official_url(&format!("{id}/plugin.json")), 64 << 10).await?;
    let m: Manifest = serde_json::from_slice(&manifest).map_err(|e| format!("plugin.json: {e}"))?;
    m.validate()?;
    if m.id != id || m.kind != "lua" || m.main != "main.lua" {
        return Err("Исходники не соответствуют запрошенному Lua-плагину".into());
    }
    let script = crate::net::download(&super::catalog::official_url(&format!("{id}/main.lua")), 512 << 10).await?;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in [("plugin.json", manifest), ("main.lua", script)] {
        archive.start_file(name, zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
        archive.write_all(&text).map_err(|e| e.to_string())?;
    }
    Ok(archive.finish().map_err(|e| e.to_string())?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in files {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacks_nested_and_rejects_bad() {
        let z = make_zip(&[("x/plugin.json", r#"{"id":"dice","name":"Кубик","kind":"lua","category":"fun","main":"main.lua"}"#), ("x/main.lua", "--")]);
        let (files, m) = unpack(&z).unwrap();
        assert_eq!(m.id, "dice");
        assert!(files.contains_key("main.lua"));
        let bad = make_zip(&[("plugin.json", r#"{"id":"dice","name":"К","kind":"lua","category":"fun","main":"main.lua"}"#), ("../evil", "x")]);
        assert!(unpack(&bad).unwrap_err().contains("недопустимый путь"));
        let nomain = make_zip(&[("plugin.json", r#"{"id":"dice","name":"К","kind":"lua","category":"fun","main":"main.lua"}"#)]);
        assert!(unpack(&nomain).unwrap_err().contains("нет main.lua"));
    }
}
