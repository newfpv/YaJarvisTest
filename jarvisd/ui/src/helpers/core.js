// Общее: помощники, иконки, запросы к колонке, сообщения, пружины, ползунки, плашка переключателей.
import { S, refreshRoute, reload, showGate } from "#state/panelState.js";
// Веб-настройки Джарвиса на колонке. CSP запрещает inline-стили и обработчики.

export const $ = (s, el = document) => el.querySelector(s);
export const $$ = (s, el = document) => [...el.querySelectorAll(s)];
export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const plural = (n, one, few, many) => { const a = Math.abs(n) % 100, b = a % 10; return a > 10 && a < 20 ? many : b === 1 ? one : b >= 2 && b <= 4 ? few : many; };
export const tip = (t) => `<span class="tip" tabindex="0" data-tip="${esc(t)}">?</span>`;

// Иконки (lucide, 16px, линия 1.5) и кольцо — знак Джарвиса: повторяет световое кольцо колонки.
export const ICONS = {
  edit: `<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/>`,
  trash: `<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M10 11v6M14 11v6"/>`,
  play: `<polygon points="6 3 20 12 6 21 6 3"/>`,
  pause: `<rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/>`,
  skip: `<polygon points="5 4 15 12 5 20 5 4"/><rect x="17" y="5" width="2.5" height="14" rx="1"/>`,
  link: `<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>`,
  speaker: `<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6"/>`,
  x: `<path d="M18 6 6 18M6 6l12 12"/>`,
  right: `<path d="m9 18 6-6-6-6"/>`,
  done: `<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>`,
  todo: `<circle cx="12" cy="12" r="10"/>`,
  mic: `<path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v3"/>`,
  stop: `<rect width="12" height="12" x="6" y="6" rx="2"/>`,
  tune: `<path d="M20 7h-9M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>`,
  read: `<path d="M18 6 7 17l-5-5"/><path d="m22 10-7.5 7.5L13 16"/>`,
  sent: `<path d="M20 6 9 17l-5-5"/>`,
  send: `<path d="m5 12 7-7 7 7M12 19V5"/>`,
  voice: `<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6M19.364 18.364a9 9 0 0 0 0-12.728"/>`,
  prev: `<polygon points="19 20 9 12 19 4 19 20"/><rect x="4.5" y="5" width="2.5" height="14" rx="1"/>`,
  repeat: `<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14"/><path d="m7 22-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>`,
  music: `<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>`,
  radio: `<path d="M4.9 19.1C1 15.2 1 8.8 4.9 4.9M7.8 16.2c-2.3-2.3-2.3-6.1 0-8.5M16.2 7.8c2.3 2.3 2.3 6.1 0 8.5M19.1 4.9C23 8.8 23 15.1 19.1 19"/><circle cx="12" cy="12" r="2"/>`,
  plus: `<path d="M5 12h14M12 5v14"/>`,
  ext: `<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>`,
  minus: `<path d="M5 12h14"/>`,
  shield: `<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z"/>`,
  cloud: `<path d="M17.5 19H8a5 5 0 1 1 1.6-9.7A6 6 0 0 1 21 11.5 3.75 3.75 0 0 1 17.5 19Z"/>`,
  wifi: `<path d="M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M12 19.5h.01M1.5 9a15 15 0 0 1 21 0"/>`,
  search: `<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>`,
  cloud: `<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>`,
  moon: `<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>`,
  puzzle: `<path d="M15.39 4.39a1 1 0 0 0 1.68-.474 2.5 2.5 0 1 1 3.014 3.015 1 1 0 0 0-.474 1.68l1.683 1.682a2.414 2.414 0 0 1 0 3.414L19.61 15.39a1 1 0 0 1-1.68-.474 2.5 2.5 0 1 0-3.014 3.015 1 1 0 0 1 .474 1.68l-1.683 1.682a2.414 2.414 0 0 1-3.414 0L8.61 19.61a1 1 0 0 0-1.68.474 2.5 2.5 0 1 1-3.014-3.015 1 1 0 0 0 .474-1.68l-1.683-1.682a2.414 2.414 0 0 1 0-3.414L4.39 8.61a1 1 0 0 1 1.68.474 2.5 2.5 0 1 0 3.014-3.015 1 1 0 0 1-.474-1.68l1.683-1.682a2.414 2.414 0 0 1 3.414 0z"/>`,
  upload: `<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12"/>`,
  home: `<path d="M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1z"/>`,
  bolt: `<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>`,
  mute: `<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="m22 9-6 6M16 9l6 6"/>`,
};
export const icon = (name, cls = "") => `<svg class="ic${cls ? " " + cls : ""}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`;
export const ring = (state, cls = "") => `<svg class="ring ${state}${cls ? " " + cls : ""}" viewBox="0 0 48 48" aria-hidden="true"><circle class="track" cx="24" cy="24" r="20"/><circle class="arc" cx="24" cy="24" r="20"/></svg>`;
// Раздел настроек: слева название и пояснение, справа сами поля.
export const sec = (title, desc, body, id = "", after = "") => `<section class="sec"><div class="sec-h"><h2${id ? ` id="${id}"` : ""}>${title}</h2>${desc ? `<p>${desc}</p>` : ""}${after}</div><div class="sec-b">${body}</div></section>`;
export let dirty = false;     // несохранённые правки на странице (менять только через setDirty: модули не могут писать в чужие переменные)
export function setDirty(v) { dirty = v; }

export async function api(method, url, body, { signal } = {}) {
  const r = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json", "X-Jarvis": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: "same-origin",
    signal,
  });
  let d = {};
  try { d = await r.json(); } catch { /* пусто */ }
  signal?.throwIfAborted();
  if (r.status === 401 || r.status === 428) { showGate(d.setup); throw new Error(d.error || "Нужно войти"); }
  if (!r.ok) throw new Error(d.error || `Ошибка ${r.status}`);
  return d;
}

export function toast(msg, bad = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "toast show" + (bad ? " bad" : "");
  clearTimeout(toast.t);
  toast.t = setTimeout(() => (t.className = "toast"), bad ? 4500 : 2200);
}
export async function act(btn, fn, okMsg) {
  if (btn) btn.disabled = true;
  try { await fn(); if (okMsg) toast(okMsg); } catch (e) { toast(e.message, true); } finally { if (btn) btn.disabled = false; }
}

// ---------------------------------------------------------------- загрузка файлов (тело — сам файл)
export async function upload(method, url, file, { signal } = {}) {
  const r = await fetch(url, { method, headers: { "X-Jarvis": "1", "Content-Type": "application/octet-stream" }, body: file, credentials: "same-origin", signal });
  let d = {};
  try { d = await r.json(); } catch { /* пусто */ }
  signal?.throwIfAborted();
  if (r.status === 401 || r.status === 428) { showGate(d.setup); throw new Error(d.error || "Нужно войти"); }
  if (!r.ok) throw new Error(d.error || `Ошибка ${r.status}`);
  return d;
}

// после обновления или новой модели колонка перезапускается — ждём, пока поднимется
export async function waitRestart(msg) {
  toast(msg);
  const old = S.version;
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    try { await reload(); if (S.version !== old) { location.reload(); return; } toast("Колонка перезапустилась"); refreshRoute(); return; } catch { /* ещё перезапускается */ }
  }
  toast("Колонка долго не отвечает — обновите страницу через минуту", true);
}

// коды входа (YouTube, Google Диск, ChatGPT…) копируются нажатием
globalThis.document?.addEventListener("click", (e) => {
  const c = e.target.closest?.(".code-big");
  if (c) copyText(c.textContent.trim()).then(() => toast("Код скопирован"), () => toast("Не получилось скопировать", true));
});
// скопировать текст: на http (адрес колонки в сети) navigator.clipboard недоступен — запасной способ
export async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return; } catch { /* ниже */ }
  const t = document.createElement("textarea");
  t.value = text; t.setAttribute("readonly", ""); t.style.position = "fixed"; t.style.opacity = "0";
  document.body.appendChild(t); t.select();
  try { document.execCommand("copy"); } finally { t.remove(); }
}

// ---------------------------------------------------------------- пружины (как в designbookmark motion: stiffness/damping)
export const Motion = (() => {
  const reduce = globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  const S = (x, k = 400, c = 30) => ({ x, v: 0, t: x, k, c: reduce ? 2 * Math.sqrt(k) : c });
  const step = (s, dt) => { const a = -s.k * (s.x - s.t) - s.c * s.v; s.v += a * dt; s.x += s.v * dt; };
  const settled = (s) => Math.abs(s.x - s.t) < 0.002 && Math.abs(s.v) < 0.01;
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const timers = [], comps = new Set();
  const after = (sec, fn) => { timers.push({ t: sec, fn }); };
  let last = performance.now(), running = false;
  function loop(now) {
    const dt = Math.min((now - last) / 1000, 1 / 30); last = now;
    for (let i = timers.length - 1; i >= 0; i--) if ((timers[i].t -= dt) <= 0) timers.splice(i, 1)[0].fn();
    for (let k = 0; k < 4; k++) comps.forEach((c) => c.frame(dt / 4));
    comps.forEach((c) => c.render());
    comps.forEach((c) => { if (c.done && c.done()) comps.delete(c); });   // отыгравшее — из цикла
    if (comps.size || timers.length) requestAnimationFrame(loop); else running = false;
  }
  const wake = () => { if (!running) { running = true; last = performance.now(); requestAnimationFrame(loop); } };
  return { S, step, settled, clamp, reduce, after: (s, f) => { after(s, f); wake(); }, add: (c) => { comps.add(c); wake(); }, remove: (c) => comps.delete(c) };
})();

// ---------------------------------------------------------------- «желейный» ползунок (designbookmark Jelly slider)
// Кружок идёт за пальцем на пружине (900/50) и растягивается от скорости; при перетаскивании из него
// поднимается капля со значением (420/22) — через goo-фильтр они сливаются, как жидкость. За краем —
// резиновое сопротивление и отскок (300/14). Хвататься можно за любое место полосы.
export function jelly(root, o) {
  const R = 9, TR = 11, D = 16, OX = 32, OY = 64, CY = 16, RISE = -34, REST = 0.45;
  root.classList.add("jelly");
  root.innerHTML = `<div class="j-track"><div class="j-fill"></div></div>
    <div class="j-goo"><div class="j-blob j-drop"></div><div class="j-blob j-thumb"></div></div><div class="j-label"></div>`;
  root.tabIndex = 0;
  root.setAttribute("role", "slider");
  root.setAttribute("aria-label", o.label || "");
  const fill = $(".j-fill", root), thumb = $(".j-thumb", root), drop = $(".j-drop", root), lab = $(".j-label", root);
  let W = root.clientWidth || 0, min = o.min ?? 0, max = o.max ?? 100;
  const span = () => Math.max(W - 2 * R, 1);
  const toX = (v) => R + ((v - min) / Math.max(max - min, 1e-9)) * span();
  const toV = (x) => min + Motion.clamp((x - R) / span(), 0, 1) * (max - min);
  const snap = (v) => o.step ? Math.round(v / o.step) * o.step : v;
  const X = Motion.S(toX(o.value ?? min), 900, 50), DX = Motion.S(X.x, 260, 18), DY = Motion.S(CY, 420, 22), DS = Motion.S(REST, 420, 22), PS = Motion.S(1, 520, 16);
  const tune = (s, k, c) => { s.k = k; s.c = Motion.reduce ? 2 * Math.sqrt(k) : c; };
  const rubber = (x, a, b, d = 18) => x < a ? a - (1 - 1 / ((a - x) * 0.55 / d + 1)) * d : x > b ? b + (1 - 1 / ((x - b) * 0.55 / d + 1)) * d : x;
  const localX = (e) => e.clientX - root.getBoundingClientRect().left;
  let dragging = false, hover = false, lifted = false, lastSent = null;
  const sync = () => { const up = dragging || lifted; DY.t = up ? CY + RISE : CY; DS.t = up ? 1 : REST; PS.t = dragging ? 1.18 : hover ? 1.1 : 1; Motion.add(anim); };
  const value = () => snap(toV(Motion.clamp(X.t, R, W - R)));

  root.addEventListener("pointerdown", (e) => {
    if (o.disabled?.()) return;
    dragging = true; try { root.setPointerCapture(e.pointerId); } catch { /* */ }
    tune(X, 900, 50); X.t = rubber(localX(e), R, W - R); sync();
  });
  root.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    X.t = rubber(localX(e), R, W - R);
    const v = value(); if (v !== lastSent) { lastSent = v; o.onInput?.(v); }
  });
  const end = () => {
    if (!dragging) return;
    dragging = false; tune(X, 300, 14); X.t = Motion.clamp(X.t, R, W - R);
    if (o.step) X.t = toX(value());
    sync(); o.onChange?.(value());
  };
  root.addEventListener("pointerup", end); root.addEventListener("pointercancel", end);
  root.addEventListener("pointerenter", (e) => { if (e.pointerType === "mouse") { hover = true; sync(); } });
  root.addEventListener("pointerleave", () => { hover = false; sync(); });
  root.addEventListener("keydown", (e) => {
    const k = o.keyStep || o.step || (max - min) / 20;
    const d = { ArrowRight: k, ArrowUp: k, ArrowLeft: -k, ArrowDown: -k }[e.key];
    if (d === undefined || o.disabled?.()) return;
    e.preventDefault();
    const v = snap(Motion.clamp(value() + d, min, max));
    tune(X, 420, 34); X.t = toX(v); lifted = true; sync();
    o.onInput?.(v); clearTimeout(root.kt); root.kt = setTimeout(() => o.onChange?.(v), 350);
  });
  root.addEventListener("blur", () => { lifted = false; sync(); });
  const resize = new ResizeObserver(() => { const w = root.clientWidth; if (!w || w === W) return; const v = toV(X.t); W = w; X.x = X.t = DX.x = DX.t = toX(v); X.v = DX.v = 0; Motion.add(anim); });
  resize.observe(root);

  const anim = {
    frame(dt) { DX.t = X.x; Motion.step(X, dt); Motion.step(DX, dt); Motion.step(DY, dt); Motion.step(DS, dt); Motion.step(PS, dt); },
    render() {
      const x = X.x, st = 1 + Math.min(Math.abs(X.v) / 2400, 0.4);
      thumb.style.transform = `translate(${(x - TR + OX).toFixed(2)}px,${(CY - TR + OY).toFixed(2)}px) scale(${(st * PS.x).toFixed(3)},${(PS.x / st).toFixed(3)})`;
      const tilt = Motion.clamp((DX.x - X.x) * 0.9, -28, 28);
      const tr = `translate(${(DX.x - D).toFixed(2)}px,${(DY.x - D).toFixed(2)}px) rotate(${tilt.toFixed(2)}deg) scale(${DS.x.toFixed(3)})`;
      drop.style.transform = `translate(${OX}px,${OY}px) ` + tr;
      lab.style.transform = tr;
      lab.style.opacity = Motion.clamp((DS.x - 0.7) / 0.25, 0, 1).toFixed(3);
      lab.textContent = (o.format || String)(snap(toV(x)));
      fill.style.width = Math.max(0, x).toFixed(2) + "px";
      root.setAttribute("aria-valuenow", String(snap(toV(x))));
    },
    done() { return !root.isConnected || (!dragging && !lifted && [X, DX, DY, DS, PS].every(Motion.settled)); },
  };
  Motion.add(anim);
  return {
    destroy() {
      resize.disconnect();
      clearTimeout(root.kt);
    },
    // значение снаружи (например, ход трека): пока человек тянет — не мешаем
    set(v, glide = false) {
      if (dragging) return;
      v = Motion.clamp(v, min, max);
      if (glide) { tune(X, 420, 34); X.t = toX(v); } else { X.x = X.t = toX(v); X.v = 0; }
      Motion.add(anim);
    },
    range(a, b) { min = a; max = Math.max(b, a + 1e-9); },
    get dragging() { return dragging; },
  };
}

// ---------------------------------------------------------------- плавная плашка у переключателей (Tab pill)
export function pills() {
  for (const seg of document.querySelectorAll("#view .seg, .gate .seg")) {
    let pill = seg.querySelector(":scope > .seg-pill");
    if (!pill) { pill = document.createElement("span"); pill.className = "seg-pill"; seg.prepend(pill); seg.classList.add("has-pill"); seg.dataset.fresh = "1"; }
    const on = seg.querySelector(":scope > button.on");
    if (!on) { pill.style.opacity = "0"; continue; }
    if (seg.dataset.fresh) { pill.classList.add("instant"); delete seg.dataset.fresh; } else pill.classList.remove("instant");
    pill.style.opacity = "1";
    pill.style.width = on.offsetWidth + "px"; pill.style.height = on.offsetHeight + "px";
    pill.style.transform = `translate(${on.offsetLeft}px, ${on.offsetTop}px)`;
  }
}
export let pillQueued = false;
export function observePills(view) {
  const observer = new MutationObserver(() => {
    if (pillQueued) return;
    pillQueued = true;
    requestAnimationFrame(() => { pillQueued = false; pills(); });
  });
  observer.observe(view, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
  addEventListener("resize", pills);
  return {
    destroy() {
      observer.disconnect();
      removeEventListener("resize", pills);
    },
  };
}
