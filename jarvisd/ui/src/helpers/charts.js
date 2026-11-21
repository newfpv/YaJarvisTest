import { $, Motion } from "./core.js";

export const fmtN = (v, dec = 0) => (v == null || isNaN(v) ? "—" : Number(v).toLocaleString("ru-RU", { minimumFractionDigits: dec, maximumFractionDigits: dec }).replace("-", "−"));

// число «набегает» к новому значению (при первом показе — от нуля)
export function countTo(el, v, dec = 0) {
  if (v == null) { el.textContent = "—"; delete el.dataset.v; return; }
  const first = el.dataset.v == null, from = first ? 0 : +el.dataset.v;
  if (from === v) { if (first) { el.dataset.v = v; el.textContent = fmtN(v, dec); } return; }
  el.dataset.v = v;
  if (Motion.reduce) { el.textContent = fmtN(v, dec); return; }
  const t0 = performance.now(), d = 800;
  const step = (t) => {
    if (+el.dataset.v !== v) return;
    const k = Math.min(1, (t - t0) / d), e = 1 - Math.pow(1 - k, 3);
    el.textContent = fmtN(from + (v - from) * e, dec);
    if (k < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

// край карточки светится там, где указатель
globalThis.document?.addEventListener("pointermove", (e) => {
  const c = e.target.closest?.(".card, .sec, .np, .kpi");
  if (!c) return;
  const r = c.getBoundingClientRect();
  c.style.setProperty("--mx", `${e.clientX - r.left}px`);
  c.style.setProperty("--my", `${e.clientY - r.top}px`);
}, { passive: true });

// плашка активного пункта меню переезжает на пружине
export function navPill() {
  const nav = $("#nav");
  if (!nav) return;
  let pill = $(".nav-pill", nav);
  if (!pill) { pill = document.createElement("span"); pill.className = "nav-pill instant"; nav.prepend(pill); nav.classList.add("has-pill"); }
  const t = $(".nav > a.on, .nav-ext a.on, .prof.on", nav);
  if (!t || !t.offsetHeight) { pill.style.setProperty("opacity", "0"); return; }
  pill.style.setProperty("transform", `translateY(${t.offsetTop}px)`);
  pill.style.setProperty("height", `${t.offsetHeight}px`);
  pill.style.setProperty("opacity", "1");
  requestAnimationFrame(() => requestAnimationFrame(() => pill.classList.remove("instant")));
}

// ---------------------------------------------------------------- линия графика «прорисовывается» (длина пути считается в пикселях)
export function drawIn(line, area, ms = 1100) {
  if (Motion.reduce || !line.getTotalLength) return;
  const len = line.getTotalLength();
  line.style.setProperty("transition", "none");
  line.style.setProperty("stroke-dasharray", `${len} ${len}`);
  line.style.setProperty("stroke-dashoffset", String(len));
  if (area) { area.style.setProperty("transition", "none"); area.style.setProperty("opacity", "0"); }
  line.getBoundingClientRect();
  requestAnimationFrame(() => {
    line.style.setProperty("transition", `stroke-dashoffset ${ms}ms cubic-bezier(.22,1,.36,1)`);
    line.style.setProperty("stroke-dashoffset", "0");
    if (area) { area.style.setProperty("transition", `opacity ${ms * 0.7}ms ease ${ms * 0.35}ms`); area.style.setProperty("opacity", "1"); }
  });
  setTimeout(() => { if (line.isConnected) { line.style.removeProperty("stroke-dasharray"); line.style.removeProperty("stroke-dashoffset"); } }, ms + 120);
}
