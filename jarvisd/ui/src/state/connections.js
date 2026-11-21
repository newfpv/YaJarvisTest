// Подключения: две вкладки на одной странице.
// «Сервисы» — ключи и входы в аккаунты, плитками по разделам; всё остальное — в окне по нажатию.
// «Плагины» — что стоит на колонке (список: включить, настроить) и что можно поставить.
// Сервису, который живёт в плагине (YouTube, умный дом, VPN), окно предлагает поставить плагин и показывает его.
import { tick } from "svelte";
import { act, api, plural, toast } from "#helpers/core.js";
import { S, reload } from "./panelState.js";
import { PL, plCatalog, plEnable, plInstall, plInstalled, plItems, plLoad, plMarkWarned, plOfficialItems,
  plRemove, plRepo, plSave, plState, plUpdate, plUpdateAll, plUpdateCandidates, plUpload, plWarned } from "./plugins.js";

const CONN = { open: "", net: {}, tab: "svc" };
let connectionView;
const servicePlugin = (id) => S.providers?.[id]?.plugin || "home";

export const NET_NAMES = { openai: "OpenAI", gemini: "Gemini", grok: "Grok", elevenlabs: "ElevenLabs", youtube: "YouTube" };
export const VPN_NAMES = [["chatgpt", "Подписка ChatGPT"], ["openai", "OpenAI"], ["gemini", "Google Gemini"], ["grok", "xAI Grok"],
  ["elevenlabs", "ElevenLabs"], ["youtube", "YouTube"], ["smartlife", "Smart Life"], ["xiaomi", "Xiaomi Home"], ["gdrive", "Google Диск"], ["yandex", "Яндекс"], ["plugins", "Плагины"]];

function homeProvider(id, mono, name, about, more) {
  const provider = S[id] || {};
  const count = provider.devices?.length || 0;
  return {
    id, group: "home", plugin: servicePlugin(id), mono, name, about, more,
    on: Boolean(provider.signed_in), wait: !provider.signed_in && Boolean(provider.login?.code),
    state: provider.signed_in ? count + " " + plural(count, "устройство", "устройства", "устройств")
      : provider.login?.code ? "ждёт QR-код" : "не подключено",
  };
}

export function connDefs() {
  const keys = S.keys || {};
  const vpn = S.vless || {};
  const chatgpt = S.chatgpt;
  const youtube = S.youtube || {};
  const account = youtube.account || {};
  const key = (id, options) => ({
    id, plugin: "", group: "voice", ...options, keyField: true,
    on: Boolean(keys[id]), need: !keys[id] && !options.optional,
    state: keys[id] ? "ключ " + String(keys[id]).replace(/^задан,?\s*/, "") : options.optional ? "не подключено" : "нужен ключ",
  });
  return [
    { id: "homeassistant", group: "home", plugin: servicePlugin("homeassistant"), mono: "HA", name: "Home Assistant",
      about: "Устройства дома, Xiaomi Home и уборка выбранных комнат",
      on: Boolean(S.home_assistant?.configured), state: S.home_assistant?.configured ? "подключено" : "не подключено" },
    { id: "chatgpt", plugin: "", group: "voice", mono: "GP", name: "Подписка ChatGPT",
      about: "Голос Джарвиса за счёт подписки Plus или Pro — без оплаты API за минуты",
      more: "Включается у ассистента: движок «GPT-Live по подписке».",
      on: Boolean(chatgpt.signed_in), wait: !chatgpt.signed_in && Boolean(chatgpt.login?.code),
      state: chatgpt.signed_in ? ["вход выполнен", chatgpt.plan].filter(Boolean).join(" · ")
        : chatgpt.login?.code ? "ждёт код" : "не подключено" },
    key("openai", { mono: "AI", name: "OpenAI", place: "sk-…", about: "Голос и «мозг» Джарвиса. Живой разговор — примерно 5 центов в минуту" }),
    key("gemini", { mono: "Ge", name: "Google Gemini", place: "AIza…", optional: true,
      about: "Gemini Live — примерно вдвое дешевле GPT-Live, есть бесплатный уровень. Можно добавить несколько ключей: у каждого свой бесплатный лимит",
      more: "Какой движок отвечает, выбирается у ассистента." }),
    key("grok", { mono: "xA", name: "xAI Grok", place: "xai-…", optional: true, about: "Grok Voice — сам ищет в интернете",
      more: "Какой движок отвечает, выбирается у ассистента." }),
    key("elevenlabs", { mono: "11", name: "ElevenLabs", place: "sk_…", optional: true,
      about: "Распознавание речи и голоса классического режима. Выбирается в настройках ассистента → Мозг" }),
    key("yandex", { group: "home", plugin: servicePlugin("yandex"), mono: "Я", name: "Яндекс Умный дом", place: "y0_…",
      about: "Джарвис сам управляет светом, пылесосом и сценариями", more: "В режиме «через Алису» можно обойтись без него." }),
    homeProvider("smartlife", "SL", "Smart Life", "Лампы, розетки и другие устройства Tuya — напрямую, без Алисы",
      "«Включи/выключи …» выполняется сразу по домашней сети, за доли секунды"),
    homeProvider("xiaomi", "Mi", "Xiaomi Home", "Лампы, розетки, пылесосы и другие устройства из приложения Mi Home",
      "Джарвис управляет ими голосом: «включи лампу в спальне», «запусти пылесос»"),
    { id: "youtube", group: "media", plugin: "youtube", mono: "YT", name: "YouTube", about: "Музыка и видео голосом. Вход — для ваших рекомендаций и видео 18+",
      more: "Там, где YouTube заблокирован (Россия), включите «Через VPN»",
      on: Boolean(account.signed_in || youtube.set), wait: !account.signed_in && Boolean(account.code),
      state: account.signed_in ? "вход выполнен" : youtube.set ? "куки загружены" : account.code ? "ждёт код" : "не подключено" },
    { id: "vpn", group: "system", plugin: "vpn", icon: "shield", name: "VPN для Джарвиса",
      about: "Через VPN идёт только сам Джарвис, Алиса — всегда напрямую. Что именно пускать через VPN — галочки ниже",
      on: Boolean(vpn.set) && !vpn.error, bad: Boolean(vpn.error), need: !vpn.set,
      state: vpn.set ? (vpn.error ? "ошибка" : vpn.name || vpn.host) : "не задан" },
  ];
}

// Разделы вкладки «Сервисы».
export const CONN_GROUPS = [
  ["voice", "Голос и мозг", "Чем Джарвис говорит и думает. Достаточно одного"],
  ["home", "Умный дом", "Свет, розетки, пылесосы и сценарии"],
  ["media", "Музыка и видео", ""],
  ["system", "Сеть", ""],
];
const plKey = (x) => "pl:" + (x.official ? x.id : (x.repo || "file") + "/" + x.id);

// items — всё, что на странице: сервисы ({kind:"svc"}) и плагины ({kind:"pl"}).
function connItems() {
  // сервис из плагина виден, только когда плагин стоит и включён: оболочку человек собирает сам
  const svc = connDefs().filter((c) => !c.plugin || plInstalled(c.plugin)?.enabled).map((c) => ({ ...c, kind: "svc", key: "svc:" + c.id }));
  const pls = plItems().map((x) => ({ kind: "pl", key: plKey(x), x, id: x.id, name: x.name, about: x.description || "", icon: x.inst?.icon || x.icon }));
  return [...svc, ...pls];
}

export function bindConnections(modal, render, renderDialog, setPhase, openKey) {
  const controller = new AbortController();
  const { signal } = controller;
  const view = { modal, render, renderDialog, setPhase, phase: "hidden" };
  connectionView = view;
  CONN.open = "";

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && view.phase === "in") {
      event.preventDefault();
      closeConnectionDialog();
    }
  }, { signal });

  const refresh = () => {
    if (connectionView !== view) return;
    drawConns();
    if (openKey && connOpen(openKey)) openKey = "";
  };
  const requests = [plLoad(), plCatalog(false), plCatalog(false, "community")];
  refresh();
  for (const request of requests) void request.then(refresh);
  return () => {
    controller.abort();
    clearTimeout(view.wait);
    clearTimeout(view.focus);
    clearTimeout(view.move);
    clearTimeout(view.hide);
    if (connectionView === view) {
      connectionView = undefined;
      CONN.open = "";
    }
    view.finishWarning?.(false);
  };
}

export function selectConnectionTab(tab) {
  if (CONN.tab === tab) return;
  CONN.tab = tab;
  drawConns();
}

function tileState(it) {
  if (it.kind === "pl") return plState(it.x);
  const net = CONN.net[it.id];
  if (net) return net.ok ? { cls: "ok", text: `${it.state} · ${net.ms} мс` } : { cls: "bad", text: net.error || "нет связи" };
  return { cls: it.bad ? "bad" : it.on ? "ok" : it.need ? "warn" : "", text: it.state };
}

function drawConns() {
  const view = connectionView;
  if (!view) return;
  const items = connItems();
  const services = items.filter((it) => it.kind === "svc");
  view.render({
    tab: CONN.tab,
    services: services.map((service) => ({
      key: service.key, name: service.name, about: service.about, on: service.on,
      group: service.group, mono: service.mono, icon: service.icon, status: tileState(service),
    })),
    plugins: items.filter((it) => it.kind === "pl").map((plugin) => ({ ...plugin, status: tileState(plugin) })),
    officialReady: plOfficialItems().length > 0,
    officialError: PL.official.request ? "" : PL.official.error,
    communityEmpty: Boolean(PL.community.items && !PL.community.request && !PL.community.error),
    communityError: PL.community.request ? "" : PL.community.error,
    freeSpace: PL.data?.free_mb,
    repo: plRepo(),
    canUpdate: plUpdateCandidates().length > 0,
  });
  if (CONN.open && !connBody(CONN.open, items)) connClose();

  // Вход кодом завершается на другом устройстве.
  clearTimeout(view.wait);
  if (!view.checking && services.some((service) => service.wait)) {
    view.wait = setTimeout(async () => {
      const was = { cg: S.chatgpt.signed_in, yt: S.youtube?.account?.signed_in, xm: S.xiaomi?.signed_in, sl: S.smartlife?.signed_in };
      view.checking = true;
      try {
        await reload();
        if (connectionView !== view) return;
        if (!was.sl && S.smartlife?.signed_in) toast("Smart Life подключён — ищу устройства");
        if (!was.xm && S.xiaomi?.signed_in) toast("Xiaomi Home подключён — ищу устройства");
        if (!was.cg && S.chatgpt.signed_in) toast("ChatGPT подключён");
        if (!was.yt && S.youtube?.account?.signed_in) toast("YouTube подключён");
      } catch (error) {
        if (connectionView === view) toast(error.message, true);
      } finally {
        view.checking = false;
        if (connectionView === view) drawConns();
      }
    }, 3000);
  }
}

function connBody(key, items = connItems()) {
  const view = connectionView;
  const item = items.find((entry) => entry.key === key);
  if (!view || !item) return false;
  const plugin = item.x?.inst;
  view.renderDialog({
    item,
    state: S,
    status: tileState(item),
    services: plugin ? connDefs().filter((service) => service.plugin === plugin.id) : [],
    refresh: () => refreshConnectionState(view),
  });
  return true;
}

export function getService(id) {
  const item = connDefs().find((service) => service.id === id);
  if (!item) return null;
  return { item, status: tileState(item), state: S };
}

function modalShow(view) {
  clearTimeout(view.hide);
  view.setPhase(view.phase = "in");
}

// connOpen открывает окно по ключу «svc:youtube» / «pl:weather» или id сервиса из /connections/youtube.
export function connOpen(key) {
  const view = connectionView;
  if (!view || !key || view.finishWarning) return false;
  if (key === "plugins" || key === "services") {
    selectConnectionTab(key === "plugins" ? "pl" : "svc");
    return true;
  }
  const items = connItems();
  if (!key.includes(":")) key = items.find((item) => item.id === key)?.key;
  if (!key || !items.some((x) => x.key === key)) return false;
  const tab = key.startsWith("pl:") ? "pl" : "svc";
  if (CONN.tab !== tab) { CONN.tab = tab; CONN.open = ""; drawConns(); }
  if (!connBody(key, items)) return false;
  CONN.open = key;
  modalShow(view);
  // на телефоне не фокусируем поле: иначе сразу выпрыгивает клавиатура и закрывает окно
  clearTimeout(view.focus);
  if (matchMedia("(hover: hover)").matches) view.focus = setTimeout(() => {
    view.modal.querySelector("#cn-card input:not([type=checkbox]), #cn-card textarea, #cn-card .btn.primary")?.focus();
  }, 60);
  return true;
}

function connWarn(view, name) {
  if (plWarned()) return Promise.resolve(true);
  if (view.finishWarning) return Promise.resolve(false);
  return new Promise((resolve) => {
    const back = CONN.open;
    CONN.open = "";
    clearTimeout(view.focus);
    let finishing = false;
    const finish = async (yes, remember = false) => {
      if (finishing) return;
      finishing = true;
      if (yes && remember) await plMarkWarned();
      view.finishWarning = undefined;
      if (connectionView !== view) { resolve(false); return; }
      if (back && yes && connBody(back)) CONN.open = back;
      else connClose();
      resolve(yes);
    };
    view.finishWarning = finish;
    view.renderDialog({ warning: { name, confirm: (remember) => finish(true, remember) } });
    modalShow(view);
    void tick().then(() => {
      if (view.finishWarning === finish) view.modal.querySelector("button[data-close]")?.focus();
    });
  });
}

export function closeConnectionDialog() {
  if (connectionView?.finishWarning) void connectionView.finishWarning(false);
  else connClose();
}

function connClose() {
  const view = connectionView;
  CONN.open = "";
  if (!view || view.phase !== "in") return;
  clearTimeout(view.focus);
  view.setPhase(view.phase = "out");
  view.hide = setTimeout(() => {
    view.setPhase(view.phase = "hidden");
    view.renderDialog(null);
  }, 260);
}

export function openRelatedService(id) {
  const view = connectionView;
  if (!view) return;
  connClose();
  clearTimeout(view.move);
  view.move = setTimeout(() => connOpen("svc:" + id), 280);
}

export async function connNetTest(button, show = () => {}) {
  await act(button, async () => {
    show({ pending: true });
    let results;
    try {
      results = await api("POST", "/api/test/net");
    } catch (error) {
      show(null);
      throw error;
    }
    CONN.net = results;
    drawConns();
    show({ results });
    const bad = Object.entries(results).filter(([, result]) => !result.ok).map(([name]) => NET_NAMES[name] || name);
    if (!Object.keys(results).length) toast("Нечего проверять — нет ни одного ключа", true);
    else if (bad.length) toast("Нет связи: " + bad.join(", "), true);
    else toast("Связь со всеми есть");
  });
}

export function updateConnectionPlugin(id, button) {
  const view = connectionView;
  return plUpdate(button, id, () => refreshConnectionState(view));
}

export function saveConnectionPlugin(id, values, button, saved) {
  const view = connectionView;
  return plSave(button, id, values, async () => {
    saved();
    await refreshConnectionState(view);
  });
}

async function refreshConnectionState(view) {
  if (!view || connectionView !== view) return;
  await reload();
  if (connectionView === view) drawConns();
}

export function refreshConnectionCatalog() {
  const view = connectionView;
  const requests = [plCatalog(true), plCatalog(true, "community")];
  drawConns();
  return Promise.all(requests.map((request) => request.then(() => {
    if (connectionView === view) drawConns();
  })));
}

export function updateConnectionPlugins() {
  const view = connectionView;
  return plUpdateAll(() => refreshConnectionState(view));
}

export async function installConnectionPlugin(plugin) {
  const view = connectionView;
  if (!view || (!plugin.official && !(await connWarn(view, plugin.id))) || connectionView !== view) return;
  await plInstall(plugin.repo, plugin.id, () => refreshConnectionState(view));
}

export function setConnectionPluginEnabled(id, enabled) {
  const view = connectionView;
  return plEnable(id, enabled, () => refreshConnectionState(view));
}

export function removeConnectionPlugin(id, button = null) {
  const view = connectionView;
  return plRemove(button, id, async () => {
    if (connectionView !== view) return;
    if (!CONN.open.startsWith("svc:")) connClose();
    await refreshConnectionState(view);
  });
}

export async function uploadConnectionPlugin(file) {
  const view = connectionView;
  if (!view || !(await connWarn(view, file.name)) || connectionView !== view) return;
  await plUpload(file, async (plugin) => {
    await refreshConnectionState(view);
    if (connectionView === view && plugin?.id) connOpen(plKey({ ...plugin, repo: plugin.source }));
  });
}

export async function serviceAction(t, root, redraw) {
  const field = (selector) => root.querySelector(selector);
  if (t.matches("input[data-vpn]")) {
    const svc = t.dataset.vpn, on = t.checked;
    try { await api("PUT", "/api/settings", { vpn: { [svc]: on } }); delete CONN.net[svc]; await redraw(); toast(`${VPN_NAMES.find(([id]) => id === svc)?.[1] || svc}: ${on ? "через VPN" : "напрямую"}`); }
    catch (err) { t.checked = !on; toast(err.message, true); }
    return;
  }
  const gk = t.closest("[data-gk-remove]");
  if (gk) { if (confirm("Убрать этот ключ Google с колонки?")) act(gk, async () => { await api("PUT", "/api/gemini-keys", { remove: +gk.dataset.gkRemove }); await redraw(); }, "Ключ убран"); return; }
  const ek = t.closest("[data-ek-remove], [data-ek-reset]");
  if (ek) {
    const rm = ek.dataset.ekRemove !== undefined, i = +(rm ? ek.dataset.ekRemove : ek.dataset.ekReset);
    if (rm && !confirm(`Убрать ключ ${i + 1} с колонки?`)) return;
    return act(ek, async () => { await api("PUT", "/api/eleven-keys", rm ? { remove: i } : { reset: i }); await redraw(); }, rm ? "Ключ убран" : "Ключ снова в работе");
  }
  const b = t.closest("[data-key]"), c = t.closest("[data-clear]");
  if (b) {
    const input = field("#k-" + b.dataset.key);
    const val = input.value.trim();
    if (!val) return toast("Сначала вставьте ключ в поле", true);
    return act(b, async () => { await api("PUT", "/api/keys", { [b.dataset.key]: val }); input.value = ""; delete CONN.net[b.dataset.key]; await redraw(); }, (b.dataset.key === "elevenlabs" && (S.eleven_keys || []).length > 1) || (b.dataset.key === "gemini" && (S.gemini_keys || []).length > 1) ? "Ключ добавлен в запас" : "Ключ сохранён");
  }
  if (c) { if (confirm(c.dataset.clear === "elevenlabs" && (S.eleven_keys || []).length > 1 ? "Убрать все ключи ElevenLabs с колонки?" : c.dataset.clear === "gemini" && (S.gemini_keys || []).length > 1 ? "Убрать все ключи Google с колонки?" : "Убрать этот ключ с колонки?")) act(c, async () => { await api("PUT", "/api/keys", { [c.dataset.clear]: "" }); delete CONN.net[c.dataset.clear]; await redraw(); }); return; }
  const btn = t.closest("button[id]");
  if (!btn) return;
  switch (btn.id) {
    case "cg-in": return act(btn, async () => { await api("POST", "/api/chatgpt/login"); await redraw(); });
    case "cg-out": if (confirm("Выйти из ChatGPT на колонке?")) act(btn, async () => { await api("DELETE", "/api/chatgpt"); await redraw(); }); return;
    case "sl-in": {
      const code = field("#sl-code").value.trim();
      if (!code) return toast("Сначала вставьте код пользователя из Smart Life", true);
      return act(btn, async () => { await api("POST", "/api/smartlife/login", { user_code: code }); await redraw(); });
    }
    case "sl-refresh": return act(btn, async () => { const r = await api("POST", "/api/smartlife/refresh"); await redraw(); toast(`Устройств: ${r.devices}`); });
    case "sl-out": if (confirm("Выйти из Smart Life на колонке?")) act(btn, async () => { await api("DELETE", "/api/smartlife"); await redraw(); }); return;
    case "xm-in": return act(btn, async () => { await api("POST", "/api/xiaomi/login"); await redraw(); });
    case "xm-refresh": return act(btn, async () => { const r = await api("POST", "/api/xiaomi/refresh"); await redraw(); toast(`Устройств: ${r.devices}`); });
    case "xm-out": if (confirm("Выйти из Xiaomi Home на колонке?")) act(btn, async () => { await api("DELETE", "/api/xiaomi"); await redraw(); }); return;
    case "ya-in": return act(btn, async () => { await api("POST", "/api/youtube/login"); await redraw(); });
    case "ya-out": if (confirm("Выйти из YouTube на колонке?")) act(btn, async () => { await api("DELETE", "/api/youtube/login"); await redraw(); }); return;
    case "yt-save": {
      const input = field("#yt-cookies");
      const txt = input.value.trim();
      if (!txt) return toast("Сначала вставьте текст файла cookies.txt", true);
      return act(btn, async () => { await api("PUT", "/api/youtube", { cookies: txt }); input.value = ""; await redraw(); }, "YouTube принял вход");
    }
    case "yt-clear": if (confirm("Убрать вход в YouTube с колонки?")) act(btn, async () => { await api("DELETE", "/api/youtube"); await redraw(); }); return;
    case "vless-save": {
      const input = field("#vless");
      return act(btn, async () => { await api("PUT", "/api/vless", { link: input.value }); input.value = ""; await redraw(); }, "Ссылка сохранена — проверьте связь");
    }
    case "vless-clear": if (confirm("Убрать VPN? Колонка пойдёт в интернет напрямую.")) act(btn, async () => { await api("PUT", "/api/vless", { link: "" }); await redraw(); }); return;
  }
}
