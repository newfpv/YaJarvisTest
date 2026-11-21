// Плеер: карточка на главной и отдельная страница.
import { Motion, act, api, copyText, jelly, toast } from "#helpers/core.js";
import { S } from "./panelState.js";

let M = null;
let mAt = 0;
const musicListeners = new Set();
let musicReadPending = false;
let musicWrites = 0;
let musicRevision = 0;
let bufferingTimer = 0;
export const clockStr = (s) => { s = Math.max(0, Math.floor(s || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
const mPlaying = () => !!M?.playing && !M.paused;
const hash = (n) => { const s = Math.sin(n * 127.1) * 43758.5453; return s - Math.floor(s); };
const noise = (x) => { const i = Math.floor(x), f = x - i, u = f * f * (3 - 2 * f); return hash(i) * (1 - u) + hash(i + 1) * u; };

function renderMusic() {
  for (const listener of musicListeners) listener(M);
}

function acceptMusic(state) {
  M = state;
  mAt = Date.now();
  clearTimeout(bufferingTimer);
  bufferingTimer = 0;
  if (state?.playing && state?.loading) {
    bufferingTimer = setTimeout(() => { bufferingTimer = 0; void musicData(); }, 400);
  }
  renderMusic();
}

export function subscribeMusic(listener) {
  listener(M);
  musicListeners.add(listener);
  return () => musicListeners.delete(listener);
}

export async function musicData() {
  if (musicReadPending || musicWrites) return;
  musicReadPending = true;
  const revision = musicRevision;
  try {
    const state = await api("GET", "/api/music");
    if (revision === musicRevision) acceptMusic(state);
  } catch {
    // Следующий опрос повторит запрос.
  } finally {
    musicReadPending = false;
  }
}

// обложка: YouTube — крупная (если есть), иначе средняя; Яндекс Музыка — как пришла
export function setArt(img, art, fallback) {
  if (img.dataset.src === art) return;
  img.dataset.src = art; img.classList.remove("ok");
  if (!art) { img.removeAttribute("src"); return; }
  const canFallback = () => fallback && fallback !== art && img.getAttribute("src") === art;
  img.onload = () => {
    if (img.naturalWidth <= 120 && canFallback()) { img.src = fallback; return; }
    img.classList.add("ok");
  };
  img.onerror = () => { if (canFallback()) img.src = fallback; };
  img.src = art;
}

// управление музыкой — общее для главной и страницы «Плеер»
export function musicAct(a, b, { disableControl = true } = {}) {
  if (!M) return;
  if (b) { b.classList.remove("k-prev", "k-next", "k-pop"); void b.offsetWidth; b.classList.add(a === "prev" ? "k-prev" : a === "next" ? "k-next" : "k-pop"); }
  const control = disableControl ? b : undefined;
  if (a === "copy") {
    const link = musicLink();
    if (link) return copyText(link).then(() => toast("Ссылка скопирована"), () => toast("Не получилось скопировать", true));
    return;
  }
  if (a === "radio") {
    return music({ action: "radio", on: !M.radio }, control)
      .then((ok) => ok && toast(M.radio ? "Радио: после очереди — похожее" : "Радио выключено: после очереди тишина"));
  }
  if (a === "repeat") {
    return music({ action: "repeat", on: !M.repeat }, control)
      .then((ok) => ok && toast(M.repeat ? "Повтор трека включён" : "Повтор выключен"));
  }
  if (a === "prev" && M.source !== "yandex") {
    const previous = M.history?.[1];
    if (previous) return music({ action: "id", query: previous.ID }, control);
    return;
  }
  return music({ action: a === "toggle" ? (M.paused ? "resume" : "pause") : a }, control);
}

// перекатывающиеся цифры: сменившаяся уезжает вверх, новая въезжает снизу (назад — наоборот)
function rollTo(el, text, dir) {
  if (el.dataset.v === text) return;
  const old = el.dataset.v || "";
  el.dataset.v = text;
  if (Motion.reduce || old.length !== text.length) { el.innerHTML = [...text].map((ch) => `<span class="rd"><span>${ch}</span></span>`).join(""); return; }
  [...text].forEach((ch, i) => {
    const slot = el.children[i];
    if (!slot || old[i] === ch) return;
    slot.innerHTML = `<span class="out ${dir > 0 ? "up" : "down"}">${old[i]}</span><span class="in ${dir > 0 ? "up" : "down"}">${ch}</span>`;
  });
}

export function mountPlayback(seekEl, timeEl, eqEl, onMusic) {
  let seekHeld = null;
  let lastSec = -1;
  let lastActive = false;
  let elapsed = 0;
  const bars = eqEl.querySelectorAll("i");
  const anim = {
    frame(dt) { elapsed += dt; },
    render() {
      const playing = mPlaying();
      const t = Motion.reduce ? 0.37 : elapsed;
      const env = playing ? 1 : 0;
      bars.forEach((bar, i) => {
        const lvl = 0.18 + 0.82 * (0.6 * noise(t * 4.3 + i * 17.3) + 0.4 * noise(t * 9.1 + i * 31.7 + 100)) * env;
        bar.style.transform = `scaleY(${lvl.toFixed(3)})`;
      });
      const position = seekHeld ?? (playing && !M.loading
        ? Math.min(M.dur || Infinity, M.pos + (Date.now() - mAt) / 1000)
        : M?.pos ?? 0);
      if (M?.dur && !seek.dragging) {
        seek.range(0, M.dur);
        seek.set(position);
      }
      const sec = Math.floor(position);
      const active = !!(M?.playing || M?.paused);
      if (sec !== lastSec || active !== lastActive) {
        rollTo(timeEl, active ? clockStr(position) : "", sec >= lastSec ? 1 : -1);
        lastSec = sec;
        lastActive = active;
      }
    },
    done() { return !mPlaying(); },
  };

  const seek = jelly(seekEl, { label: "Перемотка", min: 0, max: Math.max(1, M?.dur || 1), value: 0, format: clockStr, keyStep: 5,
    disabled: () => !M?.dur,
    onInput(v) {
      seekHeld = v;
      Motion.add(anim);
    },
    onChange(v) {
      seekHeld = null;
      if (!M?.dur) return;
      M.pos = v;
      mAt = Date.now();
      Motion.add(anim);
      void music({ action: "seek", sec: Math.round(v) });
    },
  });
  const unsubscribe = subscribeMusic((state) => {
    onMusic(state);
    Motion.add(anim);
  });
  return () => {
    unsubscribe();
    Motion.remove(anim);
    seek.destroy();
  };
}

// Громкость общая для обеих страниц. Частые нажатия объединяются в один запрос.
const volumeListeners = new Set();
let pendingVolume = null;
let volumeTimer = 0;

function volumeLevel() {
  const level = pendingVolume?.level ?? S.speaker.volume;
  return typeof level === "number" && level >= 0 ? level : null;
}

export function mountVolume(element, onLevel) {
  let previewing = false;
  const slider = jelly(element, { label: "Громкость колонки", min: 0, max: 10, step: 1, value: volumeLevel() ?? 5,
    onInput(level) {
      previewing = true;
      onLevel(level);
    },
    onChange(level) {
      previewing = false;
      setVolume(level);
    },
  });
  function update(level) {
    if (previewing || slider.dragging) return;
    onLevel(level);
    if (level !== null) slider.set(level, true);
  }
  update(volumeLevel());
  volumeListeners.add(update);
  return () => {
    volumeListeners.delete(update);
    slider.destroy();
  };
}

export function refreshVolume() {
  const level = volumeLevel();
  for (const listener of volumeListeners) listener(level);
}

export function setVolume(level) {
  const request = { level };
  pendingVolume = request;
  refreshVolume();
  clearTimeout(volumeTimer);
  volumeTimer = setTimeout(async () => {
    try {
      const result = await api("POST", "/api/speaker/volume", request);
      if (pendingVolume === request) S.speaker.volume = result.volume;
    } catch (error) {
      toast(error.message, true);
    } finally {
      if (pendingVolume === request) {
        pendingVolume = null;
        refreshVolume();
      }
    }
  }, 250);
}

export async function music(body, btn) {
  const revision = ++musicRevision;
  musicWrites++;
  try {
    let state;
    await act(btn, async () => { state = await api("POST", "/api/music", body); });
    if (!state) return false;
    if (revision === musicRevision) acceptMusic(state);
    return true;
  } finally {
    musicWrites--;
  }
}

export async function playMusicQuery(query) {
  const before = M;
  const loading = { ...before, loading: true };
  M = loading;
  renderMusic();
  const ok = await music({ action: "play", query });
  if (!ok && M === loading) {
    M = before || { loading: false };
    renderMusic();
  }
  return ok;
}

export function artOf(track, big, state = M) {
  if (!track?.ID) return "";
  if (state?.source === "yandex") return state.cover || "";
  return `https://i.ytimg.com/vi/${encodeURIComponent(track.ID)}/${big ? "maxresdefault" : "mqdefault"}.jpg`;
}

export function musicLink(state = M) {
  return state?.track?.ID ? state.link || `https://youtu.be/${encodeURIComponent(state.track.ID)}` : "";
}
