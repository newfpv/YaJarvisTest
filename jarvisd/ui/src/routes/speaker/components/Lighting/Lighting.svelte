<script module>
  const effects = [
    ["listening", "Дыхание"],
    ["thinking", "Огонёк по кругу"],
    ["speaking", "Пульс"],
  ];
  const presets = ["#ffffff", "#ff3b30", "#ff9500", "#ffcc00", "#34c759", "#00c7be", "#00b4ff", "#3d6bff", "#5a2dff", "#ff2d92"];
  const previewSeconds = 30;
  const previewInterval = 140;
  const initialHue = 197;

  function toHex(hue, saturation) {
    function channel(offset) {
      const position = (offset + hue / 60) % 6;
      const intensity = 1 - saturation * Math.max(0, Math.min(position, 4 - position, 1));
      return Math.round(intensity * 255).toString(16).padStart(2, "0");
    }
    return "#" + [5, 3, 1].map(channel).join("");
  }

  function fromHex(hex) {
    const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
    if (!match) return null;
    const rgb = Number.parseInt(match[1], 16);
    const red = (rgb >> 16 & 255) / 255;
    const green = (rgb >> 8 & 255) / 255;
    const blue = (rgb & 255) / 255;
    const high = Math.max(red, green, blue);
    const difference = high - Math.min(red, green, blue);
    let angle = 0;
    if (difference) {
      if (high === red) angle = (green - blue) / difference;
      else if (high === green) angle = (blue - red) / difference + 2;
      else angle = (red - green) / difference + 4;
    }
    return { hue: (angle * 60 + 360) % 360, saturation: high ? difference / high : 0 };
  }

  const percent = (value) => value + "%";
</script>

<script>
  import { onMount } from "svelte";
  import { api, jelly, toast } from "#helpers/core.js";
  import { S, reload } from "#state/panelState.js";
  import styles from "./Lighting.module.css";

  const available = S.speaker.leds;
  const controller = new AbortController();
  const { signal } = controller;
  let enabled = $state(!!S.settings.led?.enabled);
  let saving = $state(false);
  let hue = $state(initialHue);
  let saturation = $state(1);
  let brightness = $state(70);
  let effect = $state("listening");
  let hex = $state(toHex(initialHue, 1).toUpperCase());
  let invalidHex = $state(false);
  let lit = $state(false);
  let saturationTrack = $state();
  let brightnessTrack = $state();
  let saturationSlider;
  let pointer;
  let queued;
  let sending = false;
  let sendTimer;
  let statusTimer;
  let lastPreview = 0;
  let color = $derived(toHex(hue, saturation));
  let angle = $derived((hue - 90) * Math.PI / 180);

  onMount(() => {
    if (!available) return;
    saturationSlider = jelly(saturationTrack, {
      label: "Насыщенность", min: 0, max: 100, step: 1, value: 100, format: percent,
      onInput: (value) => changeSaturation(value, false),
      onChange: (value) => changeSaturation(value, true),
    });
    const brightnessSlider = jelly(brightnessTrack, {
      label: "Яркость", min: 5, max: 100, step: 5, value: brightness, format: percent,
      onInput: (value) => changeBrightness(value, false),
      onChange: (value) => changeBrightness(value, true),
    });
    return () => {
      controller.abort();
      clearTimeout(sendTimer);
      clearTimeout(statusTimer);
      saturationSlider.destroy();
      brightnessSlider.destroy();
    };
  });

  async function changeEnabled(event) {
    const input = event.currentTarget;
    const next = input.checked;
    input.checked = enabled;
    saving = true;
    try {
      await api("PUT", "/api/settings", { led: { enabled: next } });
      if (signal.aborted) return;
      enabled = next;
      await reload();
      if (!signal.aborted) toast(next ? "Подсветка включена" : "Подсветка выключена");
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      saving = false;
    }
  }

  function preview(immediate = true) {
    enqueue({ mode: effect, color, brightness, seconds: previewSeconds }, immediate);
  }

  function enqueue(body, immediate) {
    queued = { body, immediate };
    flushPreview();
  }

  function flushPreview() {
    clearTimeout(sendTimer);
    if (signal.aborted || sending || !queued) return;
    const delay = previewInterval - (Date.now() - lastPreview);
    if (!queued.immediate && delay > 0) {
      sendTimer = setTimeout(flushPreview, delay);
      return;
    }
    const { body } = queued;
    queued = undefined;
    void sendPreview(body);
  }

  async function sendPreview(body) {
    sending = true;
    lastPreview = Date.now();
    try {
      await api("POST", "/api/leds/preview", body, { signal });
      clearTimeout(statusTimer);
      lit = body.mode !== "off";
      if (lit) statusTimer = setTimeout(() => lit = false, previewSeconds * 1000);
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      sending = false;
      flushPreview();
    }
  }

  function syncHex() {
    hex = color.toUpperCase();
    invalidHex = false;
  }

  function applyColor(next) {
    hue = next.hue;
    saturation = next.saturation;
    clearTimeout(saturationTrack.kt);
    saturationSlider.set(Math.round(saturation * 100), true);
  }

  function changeHex(event) {
    hex = event.currentTarget.value;
    const parsed = fromHex(hex);
    const trimmed = hex.trim();
    invalidHex = !parsed && trimmed.replace(/^#/, "").length >= 6;
    if (!parsed) return;
    applyColor(parsed);
    preview(false);
  }

  function choosePreset(preset) {
    applyColor(fromHex(preset));
    syncHex();
    preview();
  }

  function changeSaturation(value, immediate) {
    if (immediate && value === Math.round(saturation * 100)) return;
    saturation = value / 100;
    syncHex();
    preview(immediate);
  }

  function changeBrightness(value, immediate) {
    if (immediate && value === brightness) return;
    brightness = value;
    syncHex();
    preview(immediate);
  }

  function changeHue(value) {
    hue = (value + 360) % 360;
    syncHex();
    preview(false);
  }

  function dragHue(event) {
    if (event.pointerId !== pointer) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - bounds.left - bounds.width / 2;
    const y = event.clientY - bounds.top - bounds.height / 2;
    changeHue(Math.atan2(y, x) * 180 / Math.PI + 90);
  }

  function startHue(event) {
    if (pointer != null || event.button !== 0) return;
    pointer = event.pointerId;
    event.currentTarget.setPointerCapture(pointer);
    dragHue(event);
  }

  function endHue(event) {
    if (event.pointerId === pointer) pointer = undefined;
  }

  function hueKey(event) {
    const delta = { ArrowRight: 5, ArrowUp: 5, ArrowLeft: -5, ArrowDown: -5 }[event.key];
    if (delta == null) return;
    event.preventDefault();
    changeHue(hue + delta);
  }

  function selectEffect(mode) {
    effect = mode;
    syncHex();
    preview();
  }
</script>

{#if available}
  <div class={styles.lighting}>
    <label class="sw">
      <input type="checkbox" checked={enabled} disabled={saving} onchange={changeEnabled}>
      <span class="tr"></span>
      Подсвечивать кольцо, когда говорит Джарвис
    </label>
    <div class={styles.lab}>
      <div class={styles.wheel}>
        <div class={styles.hue} role="slider" tabindex="0" aria-label="Оттенок"
          aria-valuemin="0" aria-valuemax="359" aria-valuenow={Math.floor(hue)}
          onpointerdown={startHue} onpointermove={dragHue} onpointerup={endHue} onlostpointercapture={endHue}
          onpointercancel={endHue} onkeydown={hueKey}>
          <span class={styles.knob} style:left={`${50 + 40 * Math.cos(angle)}%`}
            style:top={`${50 + 40 * Math.sin(angle)}%`} style:--c={toHex(hue, 1)}></span>
        </div>
        <button class={styles.core} type="button" aria-label="Предпросмотр подсветки" data-effect={effect}
          style:--c={color} style:--b={0.35 + 0.65 * brightness / 100} onclick={() => preview()}>
          <span class={styles.glow}></span>
          <span class={styles.dot}></span>
        </button>
      </div>

      <div class={styles.controls}>
        <div class="seg" role="group" aria-label="Эффект подсветки">
          {#each effects as [mode, label] (mode)}
            <button type="button" class:on={effect === mode} aria-pressed={effect === mode}
              onclick={() => selectEffect(mode)}>{label}</button>
          {/each}
        </div>
        <div class={styles.hexRow}>
          <span class={styles.sample} style:--c={color}></span>
          <input type="text" aria-label="Цвет в HEX" aria-invalid={invalidHex} maxlength="7" spellcheck="false"
            autocomplete="off" value={hex} oninput={changeHex}>
          <span class="meta">впишите цвет или крутите кольцо</span>
        </div>
        <div class="f">
          <span class="lbl">Насыщенность <b class="val">{Math.round(saturation * 100)}%</b></span>
          <div bind:this={saturationTrack} role="slider" aria-label="Насыщенность" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(saturation * 100)}></div>
        </div>
        <div class="f">
          <span class="lbl">Яркость <b class="val">{brightness}%</b></span>
          <div bind:this={brightnessTrack} role="slider" aria-label="Яркость" aria-valuemin="5" aria-valuemax="100" aria-valuenow={brightness}></div>
        </div>
        <div class={styles.presets} role="group" aria-label="Готовые цвета">
          {#each presets as preset (preset)}
            <button type="button" class="swatch" class:on={preset === color} aria-pressed={preset === color}
              aria-label={"Цвет " + preset} style:--c={preset} onclick={() => choosePreset(preset)}></button>
          {/each}
        </div>
        <div class="row">
          <button class="btn primary" type="button" onclick={() => preview()}>Зажечь на 30 секунд</button>
          <button class="btn ghost" type="button" onclick={() => enqueue({ mode: "off" }, true)}>Погасить</button>
          <span class={["meta", styles.status]} role="status">{lit ? "горит на колонке" : ""}</span>
        </div>
      </div>
    </div>
  </div>
{:else}
  <p class="note">У этой колонки нет подсветки, которой может управлять Джарвис.</p>
{/if}
