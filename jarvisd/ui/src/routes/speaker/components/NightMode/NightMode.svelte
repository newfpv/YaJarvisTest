<script module>
  const controls = [
    {
      key: "voice", title: "Голос Джарвиса не громче", name: "Голос ночью",
      min: 5, max: 100, step: 5, format: (value) => value + "%",
    },
    {
      key: "bright", title: "Кольцо не ярче", name: "Кольцо ночью",
      min: 0, max: 100, step: 5, format: (value) => value + "%",
    },
    {
      key: "speaker", title: "Колонка не громче", name: "Колонка ночью",
      hint: "Громкость самой колонки по её шкале 0–10. Если ночью сделать громче, через полминуты станет обратно.",
      min: 1, max: 10, step: 1, format: (value) => `${value} из 10`,
    },
  ];
</script>

<script>
  import { icon, jelly, tip } from "#helpers/core.js";
  import styles from "./NightMode.module.css";

  let { night, now, leds, pending, save } = $props();
  let drafts = $state({});

  function changeEnabled(event) {
    const input = event.currentTarget;
    const enabled = input.checked;
    input.checked = night.enabled;
    void save("enabled", enabled);
  }

  function changeTime(event, key) {
    const input = event.currentTarget;
    const value = input.value;
    input.value = night[key];
    if (value) void save(key, value);
  }

  function mountSlider(node, control) {
    const { key, name, min, max, step, format } = control;
    const slider = jelly(node, {
      label: name, min, max, step, format, value: night[key],
      disabled: () => pending.has("night." + key),
      onInput: (value) => drafts[key] = value,
      async onChange(value) {
        const pendingKey = "night." + key;
        if (pending.has(pendingKey)) return;
        drafts[key] = value;
        await save(key, value);
        delete drafts[key];
        if (node.isConnected) slider.set(night[key]);
      },
    });
    return { destroy: () => slider.destroy() };
  }
</script>

<section class="sec">
  <div class="sec-h">
    <h2><span class={styles.moon}>{@html icon("moon")}</span>Ночной режим</h2>
    <p>По расписанию Джарвис говорит тише и короче, кольцо светит слабее, а сама колонка — Алиса, музыка, YouTube — не играет громче заданного</p>
  </div>
  <div class="sec-b">
    <label class="sw">
      <input type="checkbox" checked={night.enabled} disabled={pending.has("night.enabled")} onchange={changeEnabled}>
      <span class="tr"></span>Включать по расписанию
    </label>
    <div class={styles.times}>
      <div class={["f", styles.time]}>
        <label for="n-from">С</label>
        <input type="time" id="n-from" value={night.from} disabled={pending.has("night.from")}
          onchange={(event) => changeTime(event, "from")}>
      </div>
      <div class={["f", styles.time]}>
        <label for="n-to">До</label>
        <input type="time" id="n-to" value={night.to} disabled={pending.has("night.to")}
          onchange={(event) => changeTime(event, "to")}>
      </div>
      {#if now}<span class="badge acc">сейчас ночь</span>{/if}
    </div>
    <div class={styles.sliders}>
      {#each controls as control (control.key)}
        {#if control.key !== "bright" || leds}
          <div class="f">
            <span class={["lbl", styles.label]}>
              {control.title}
              {#if control.hint}{@html tip(control.hint)}{/if}
              <b class={["val", styles.value]}>{control.format(drafts[control.key] ?? night[control.key])}</b>
            </span>
            <div use:mountSlider={control} role="slider" aria-label={control.name}
              aria-valuemin={control.min} aria-valuemax={control.max} aria-valuenow={drafts[control.key] ?? night[control.key]}
              aria-disabled={pending.has("night." + control.key)}></div>
          </div>
        {/if}
      {/each}
    </div>
  </div>
</section>
