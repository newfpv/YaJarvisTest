// Плагины: данные и действия (поставить, обновить, включить, настроить, удалить).
// Своей страницы у плагинов больше нет — они живут на странице «Подключения» рядом с сервисами (connections.js).
import { ICONS, act, api, icon, toast, upload } from "#helpers/core.js";
import { S } from "./panelState.js";

export const PL = {
  data: null,
  official: { items: null, error: "", request: null },
  community: { items: null, error: "", request: null },
};
export const PL_KIND = { lua: "Lua", rust: "Плагин", module: "Расширение" };
export const plIcon = (n) => icon(ICONS[n] ? n : "puzzle");
export const plSize = (b) => (b < 1048576 ? `${Math.max(1, Math.round(b / 1024))} КБ` : `${(b / 1048576).toFixed(1).replace(".", ",")} МБ`);

export async function plLoad() {
  try { PL.data = await api("GET", "/api/plugins"); } catch (e) { toast(e.message, true); }
  return PL.data;
}

export function plCatalog(fresh, scope = "official") {
  const catalog = PL[scope];
  if (catalog.request) return catalog.request;
  catalog.request = api("GET", `/api/plugins/catalog?scope=${scope}${fresh ? "&fresh=1" : ""}`).then((r) => {
    if (!Array.isArray(r.items)) throw new Error("Колонка вернула неверный формат каталога.");
    const items = r.items.filter((item) => item.official === (scope === "official"));
    catalog.error = r.error || "";
    if (scope === "official" && !items.length) {
      catalog.error ||= "Колонка вернула пустой официальный каталог.";
    } else if (!catalog.error || items.length) {
      catalog.items = items;
    }
  }).catch((error) => {
    catalog.error = error.message;
  }).finally(() => {
    catalog.request = null;
  });
  return catalog.request;
}

export const plOfficialItems = () => PL.official.items || [];
export function plInstalled(id) { return (PL.data?.installed || []).find((p) => p.id === id); }
// plCatItem — запись каталога для установленного плагина (официальный — по id, сторонний — по репозиторию)
export function plCatItem(p) {
  return p.official ? plOfficial(p.id) : PL.community.items?.find((c) => c.id === p.id && c.repo === p.source);
}
export function plHasUpdate(p, catalog = plCatItem(p)) {
  if (!catalog?.version || p.kind === "module") return false;
  const parts = (version) => (version || "").replace(/^v+/, "").split(/[-+]/)[0].split(".").map(Number);
  const a = parts(catalog.version), b = parts(p.version);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const diff = (a[i] || 0) - (b[i] || 0);
    if (diff) return diff > 0;
  }
  return false;
}
export function plUpdateCandidates() {
  return (PL.data?.installed || []).filter((p) => plHasUpdate(p)
    || (!p.official && /^[\w.-]+\/[\w.-]+$/.test(p.source) && !plCatItem(p)?.version));
}
const updateRequest = (p) => ({ id: p.id, repo: p.source, expected_version: plCatItem(p)?.version || "", update: true });
export function plDescription(p, catalog = plCatItem(p)) {
  const values = p.official ? [catalog?.description, p.description] : [p.description, catalog?.description];
  return values.find((value) => value && !/\?{3,}/.test(value)) || "";
}
export function plOfficial(id) { return plOfficialItems().find((c) => c.id === id); }
export const plRepo = () => PL.data?.repo || S.repo || "newfpv/YaJavis";

// plItems — всё, что можно показать: каталог плюс установленное, чего в каталоге нет (из файла, снятое с каталога).
export function plItems() {
  const inst = PL.data?.installed || [];
  const out = [...plOfficialItems(), ...(PL.community.items || [])].map((c) => {
    const p = inst.find((p) => p.id === c.id && (c.official ? p.official : p.source === c.repo));
    return { ...c, name: p?.name || c.name, description: p ? plDescription(p, c) : c.description, inst: p };
  });
  for (const p of inst) {
    if (out.some((x) => x.inst === p)) continue;
    out.push({ id: p.id, repo: p.source, name: p.name, description: plDescription(p), category: p.category, icon: p.icon, kind: p.kind,
      version: p.version, author: p.author, official: p.official, url: p.homepage, inst: p });
  }
  return out;
}

// plState — коротко для плитки: что с плагином сейчас.
export function plState(x) {
  const p = x.inst;
  if (!p) return { cls: "", text: "не установлен" };
  const c = plCatItem(p);
  if (p.error) return p.error.startsWith("заработает") ? { cls: "warn", text: "ждёт обновления" } : { cls: "bad", text: "ошибка" };
  if (!p.enabled) return { cls: "", text: "выключен" };
  if (plHasUpdate(p, c)) return { cls: "warn", text: "есть обновление" };
  return p.running ? { cls: "ok", text: "работает" } : { cls: "warn", text: "запускается" };
}

export const plWarned = () => !!(PL.data?.warned || S.settings.plugins_warned);
export async function plMarkWarned() {
  await api("POST", "/api/plugins/warned").catch(() => {});
  if (PL.data) PL.data.warned = true;
  S.settings.plugins_warned = true;
}

export async function plInstall(repo, id, then) {
  await act(null, async () => {
    const r = await api("POST", "/api/plugins/install", { repo, id });
    if (r.restarting && !await waitModuleChange()) return;
    await verifyInstall([r.plugin], r.reboot_from);
    toast(`«${r.plugin?.name || id}» установлен`);
    await then?.();
  });
}

export async function plUpdate(btn, id, then) {
  const p = plInstalled(id);
  if (!p || !plHasUpdate(p)) return;
  await act(btn, async () => {
    const r = await api("POST", "/api/plugins/install", updateRequest(p));
    await verifyInstall([r.plugin], r.reboot_from);
    await then?.();
    toast(r.changed ? "Плагин обновлён" : "Уже установлена актуальная версия");
  });
}

export async function plUpdateAll(then) {
  const plugins = plUpdateCandidates();
  await act(null, async () => {
    const r = await api("POST", "/api/plugins/update-all", plugins.map(updateRequest));
    await verifyInstall(r.updated, r.reboot_from);
    await then?.();
    const summary = `Обновлено плагинов: ${r.updated.length}`;
    const errors = r.errors.map(({ id, error }) => `${plugins.find((p) => p.id === id)?.name || id}: ${error}`);
    if (errors.length) throw new Error([summary, ...errors].join(". "));
    toast(r.updated.length ? summary : "Плагины уже актуальны");
  });
}

async function verifyInstall(manifests, rebootFrom) {
  if (rebootFrom) {
    toast("Колонка перезагружается. Жду подключения…");
    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      try {
        const res = await fetch("/healthz", { cache: "no-store", signal: AbortSignal.timeout(2000) });
        const boot = res.ok && (await res.json()).boot_id;
        if (boot && boot !== rebootFrom) break;
      } catch { /* The speaker is restarting. */ }
    }
  }
  PL.data = await api("GET", "/api/plugins");
  if (rebootFrom && (!PL.data.boot_id || PL.data.boot_id === rebootFrom)) throw new Error("Колонка не подтвердила перезагрузку");
  for (const m of manifests) {
    const p = plInstalled(m.id);
    if (!p || p.version !== m.version) throw new Error(`Колонка не подтвердила установку «${m.name}» ${m.version}`);
    if (p.enabled && !p.running) throw new Error(`«${p.name}» ${p.version}: ${p.error || "не запустился"}`);
  }
}

export async function plEnable(id, on, then) {
  await act(null, async () => { await api("POST", `/api/plugins/${encodeURIComponent(id)}/enable`, { on }); await plLoad(); await then?.(); }, on ? "Плагин включён" : "Плагин выключен");
}

export async function plSave(btn, id, vals, then) {
  await act(btn, async () => { await api("POST", `/api/plugins/${encodeURIComponent(id)}/settings`, vals); await plLoad(); await then?.(); }, "Сохранено");
}

export async function plRemove(btn, id, then) {
  const p = plInstalled(id);
  const kept = ["home", "vpn", "youtube"].includes(p?.id) ? " Вход в аккаунты и ключи останутся на колонке." : "";
  const linked = p?.kind === "module" ? " Код будет освобождён после перехода на точную сборку без этого модуля. Колонка применит её автоматически, если она есть в релизе и автообновление включено." : "";
  if (!confirm(`Удалить плагин «${p?.name || id}» вместе с его отдельными настройками и данными?${kept}${linked}`)) return;
  await act(btn, async () => { const r = await api("DELETE", `/api/plugins/${encodeURIComponent(id)}`); if (r.restarting && !await waitModuleChange()) return; await plLoad(); await then?.(); }, "Плагин удалён");
}

async function waitModuleChange() {
  toast("Применяю изменения. Панель переподключится сама");
  for (let n = 0; n < 90; n++) {
    await new Promise((resolve) => setTimeout(resolve, 1000));
    try {
      const state = await api("GET", "/api/plugins");
      if (!state.composition_pending) { location.reload(); return true; }
    } catch { /* The speaker is restarting. */ }
  }
  throw new Error("Колонка не подтвердила применение изменений");
}

export async function plUpload(file, then) {
  await act(null, async () => { const r = await upload("POST", "/api/plugins/upload", file); await verifyInstall([r.plugin], r.reboot_from); toast(`«${r.plugin?.name}» установлен`); await then?.(r.plugin); });
}
