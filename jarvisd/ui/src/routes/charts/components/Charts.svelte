<script module>
  const ranges = [
    [900, "15 мин"],
    [3600, "1 ч"],
    [21600, "6 ч"],
    [86400, "24 ч"],
  ];

  const percentScale = [0, 100];
  const metrics = [
    { key: "cpu", title: "Процессор", unit: "%", subtitle: "загрузка всех ядер", scale: percentScale, skipFirst: true },
    { key: "temp", title: "Температура", unit: "°C", subtitle: "процессор", decimals: 1, zeroIsMissing: true },
    { key: "mem", title: "Память занята", unit: "МБ" },
    { key: "wifi", title: "Сигнал Wi-Fi", unit: "дБм", subtitle: "ближе к нулю — лучше", zeroIsMissing: true },
    { key: "disk", title: "Свободно на /data", unit: "МБ", subtitle: "меньше 5 МБ — обновления могут не встать" },
    { key: "me", title: "Сам Джарвис", unit: "%", subtitle: "сколько процессора колонки занимает Джарвис", scale: percentScale, skipFirst: true },
  ];

  let selectedRange = 3600;
</script>

<script>
  import { onMount } from "svelte";
  import { api } from "#helpers/core.js";
  import { getPanel } from "#state/panelContext.js";
  import { S } from "#state/panelState.js";
  import Chart from "./Chart/Chart.svelte";
  import styles from "./Charts.module.css";

  let range = $state(selectedRange);
  let points = $state.raw([]);
  let memoryTotal = $state(0);
  let animation = $state(0);
  let loading = $state(true);
  let loadError = $state("");
  let lastRequest = 0;
  let request;
  const { onPoll } = getPanel();

  onMount(() => {
    const unsubscribe = onPoll(refresh);
    void refresh(true);
    return () => {
      unsubscribe();
      request?.abort();
    };
  });

  async function refresh(force = false) {
    if (!force && (request || Date.now() - lastRequest < 15000)) return;

    request?.abort();
    const controller = new AbortController();
    const { signal } = controller;
    request = controller;
    lastRequest = Date.now();

    try {
      const data = await api("GET", "/api/stats?range=" + range, undefined, { signal });
      points = data.points || [];
      memoryTotal = S.sys?.mem_total_mb || 0;
      loadError = "";
      if (force) animation++;
    } catch (error) {
      if (!signal.aborted && !points.length) loadError = error.message;
    } finally {
      if (request === controller) {
        request = undefined;
        loading = false;
      }
    }
  }

  function selectRange(seconds) {
    if (seconds !== range) {
      range = selectedRange = seconds;
      points = [];
      loading = true;
      loadError = "";
    }
    void refresh(true);
  }
</script>

<header class="ph ph-row">
  <div>
    <h1>Графики</h1>
    <p class="sub">Раз в 15 секунд за последние сутки. История живёт в памяти колонки и начинается заново после перезапуска Джарвиса</p>
  </div>
  <div class="seg" role="group" aria-label="Период графиков">
    {#each ranges as [seconds, label] (seconds)}
      <button type="button" class:on={seconds === range} aria-pressed={seconds === range}
        onclick={() => selectRange(seconds)}>{label}</button>
    {/each}
  </div>
</header>

{#if loadError}
  <p class="err" role="alert">{loadError}</p>
{/if}
<div class={[styles.charts, "stagger"]} aria-busy={loading}>
  {#each metrics as metric (metric.key)}
    <Chart {metric} {points} {animation} ready={!loading && !loadError}
      subtitle={metric.key === "mem" && memoryTotal ? `из ${memoryTotal} МБ` : metric.subtitle} />
  {/each}
</div>
