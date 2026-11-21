import { Motion } from "#helpers/core.js";

// Ответ Джарвиса «печатается»: пузырь открывается из пилюли «Думаю» и растёт пружиной (260/26),
// каждое новое слово поднимается и проясняется на своей пружине (340/26) с шагом 70 мс, а высота
// пузыря идёт строка за строкой вслед за последним показанным словом. Если человек внизу переписки,
// окно каждый кадр следует за растущим пузырём — текст не уходит за край.
export function animateAnswer(node, options) {
  const st = { el: node.parentElement, text: "", anim: null };

  function update({ text, animate, follow, measure }) {
    if (text === st.text) return;
    if (!animate || Motion.reduce) {
      stopAnimation(st);
      node.textContent = st.text = text;
      return;
    }
    const old = measure();
    let added = text;
    if (text.startsWith(st.text)) added = text.slice(st.text.length);
    else {
      stopAnimation(st);
      node.replaceChildren();
    }
    st.text = text;
    const words = [];
    for (const part of added.split(/(\s+)/).filter(Boolean)) {
      if (/^\s+$/.test(part)) {
        node.append(part);
        continue;
      }
      const word = document.createElement("span");
      word.className = "wd";
      word.textContent = part;
      word.style.opacity = "0";
      node.append(word);
      words.push(word);
    }
    const start = old || { w: Math.min(st.el.offsetWidth, 96), h: Math.min(st.el.offsetHeight, 36) };
    feedMorph(st, start, words, follow);
  }

  update(options);
  return { update, destroy: () => stopAnimation(st) };
}

function stopAnimation(st) {
  if (!st.anim) return;
  Motion.remove(st.anim.comp);
  clearStyles(st.el, st.anim);
  st.anim = null;
}

function clearStyles(el, animation) {
  el.style.height = el.style.clipPath = el.style.minWidth = "";
  if (animation.meta) animation.meta.style.right = "";
  el.classList.remove("growing");
  for (const word of animation.words) word.el.style.cssText = "";
}

// feedMorph — пузырь плавно меняет размер и показывает новые слова, продолжая уже идущую анимацию.
// Раскладка текста сразу окончательная (ширина пузыря в потоке — полная), поэтому показанные слова никогда
// не перескакивают между строками. Видимую ширину открывает маска clip-path за последним показанным словом,
// высоту — пружина (overflow: hidden): строка за строкой. Подпись со временем едет вместе с правым краем.
function feedMorph(st, old, spans, follow) {
  const el = st.el, meta = el.querySelector(":scope > .m");
  let a = st.anim;
  el.style.height = el.style.clipPath = "";
  if (meta) meta.style.right = "";
  if (!a) el.classList.add("growing");
  if (meta) el.style.minWidth = meta.offsetWidth + 24 + "px";   // подпись в анимации вне потока — ширину держим сами
  const natural = { w: el.offsetWidth, h: el.offsetHeight };
  if (!a) {
    if (!spans.length && Math.abs(old.w - natural.w) < 1 && Math.abs(old.h - natural.h) < 1) {
      el.classList.remove("growing"); el.style.minWidth = "";
      return;
    }
    const cs = getComputedStyle(el);
    a = st.anim = {
      W: Motion.S(Math.min(old.w, natural.w), 260, 26), H: Motion.S(old.h, 260, 26), words: [], clock: 0, next: 0, natural,
      meta, padB: parseFloat(cs.paddingBottom) || 8, padR: parseFloat(cs.paddingRight) || 12,
      round: `${cs.borderTopLeftRadius} ${cs.borderTopRightRadius} ${cs.borderBottomRightRadius} ${cs.borderBottomLeftRadius}`, reveal: false,
    };
    a.comp = {
      frame(dt) {
        a.clock += dt;
        for (const w of a.words) {
          if (!w.on && a.clock >= w.at) { w.on = true; w.s.t = 1; a.reveal = true; }
          if (w.on && !w.fin) Motion.step(w.s, dt);
        }
        Motion.step(a.W, dt); Motion.step(a.H, dt);
      },
      render() {
        if (!el.isConnected) return;
        if (a.reveal) { a.reveal = false; feedTargets(a); }
        feedClip(el, a);
        for (const w of a.words) {
          if (!w.on || w.fin) continue;
          const v = Motion.clamp(w.s.x, 0, 1.2), o = Motion.clamp(v, 0, 1);
          w.el.style.opacity = o.toFixed(3);
          w.el.style.transform = `translateY(${((1 - w.s.x) * 6).toFixed(2)}px)`;
          w.el.style.filter = v < 0.98 ? `blur(${((1 - o) * 4).toFixed(2)}px)` : "none";
          if (Motion.settled(w.s)) { w.fin = true; w.el.style.cssText = ""; }
        }
        follow();
      },
      done() {
        const fin = !el.isConnected || (a.words.every((w) => w.fin) && Motion.settled(a.W) && Motion.settled(a.H));
        if (fin) {
          if (el.isConnected) clearStyles(el, a);
          if (st.anim === a) st.anim = null;
          follow();
        }
        return fin;
      },
    };
  }
  a.natural = natural;
  a.next = Math.max(a.next, a.clock);
  for (const w of spans) { a.words.push({ el: w, s: Motion.S(0, 340, 26), on: false, fin: false, at: a.next }); a.next += 0.07; }
  feedTargets(a);
  feedClip(el, a);
  Motion.add(a.comp);   // уже в цикле — просто будит его
}

// Цели пружин: ширина — до правого края самого дальнего показанного слова (и не уже подписи),
// высота — до низа последнего показанного слова; когда показано всё — полный размер пузыря.
function feedTargets(a) {
  let last = null, all = true, right = 0;
  for (const w of a.words) {
    if (!w.on) { all = false; continue; }
    last = w; right = Math.max(right, w.el.offsetLeft + w.el.offsetWidth);
  }
  if (all) { a.W.t = a.natural.w; a.H.t = a.natural.h; return; }
  const metaW = a.meta ? a.meta.offsetWidth + 2 * a.padR : 0;
  a.W.t = Math.min(a.natural.w, Math.max(a.W.t, right + a.padR, metaW));
  if (last) a.H.t = Math.min(a.natural.h, Math.max(a.H.t, last.el.offsetTop + last.el.offsetHeight + a.padB));
}

function feedClip(el, a) {
  const cut = Math.max(0, a.natural.w - Math.min(a.W.x, a.natural.w));
  el.style.height = a.H.x.toFixed(1) + "px";
  el.style.clipPath = cut < 0.5 ? "" : `inset(0 ${cut.toFixed(1)}px 0 0 round ${a.round})`;
  if (a.meta) a.meta.style.right = (a.padR + cut).toFixed(1) + "px";
}
