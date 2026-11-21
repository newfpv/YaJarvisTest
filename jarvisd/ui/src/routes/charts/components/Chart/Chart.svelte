<script module>
  import { fmtN } from "#helpers/charts.js";

  const height = 150;
  const top = 6;
  const right = 6;
  const bottom = height - 20;
  const left = 40;
  const timeTicks = [0.12, 0.38, 0.64, 0.9];
  const timeFormat = new Intl.DateTimeFormat("ru-RU", { hour: "2-digit", minute: "2-digit" });

  function formatTime(seconds) {
    return timeFormat.format(seconds * 1000);
  }

  function chartGeometry(samples, metric, width) {
    const values = samples.map((sample) => sample[metric.key]);
    let [low, high] = metric.scale || [Math.min(...values), Math.max(...values)];
    if (!metric.scale) {
      const padding = (high - low) * 0.2 || Math.max(1, Math.abs(high) * 0.05);
      low -= padding;
      high += padding;
    }

    const start = samples[0].t;
    const end = Math.max(samples.at(-1).t, start + 1);
    const plotWidth = width - left - right;
    const x = (time) => left + (time - start) / (end - start) * plotWidth;
    const y = (value) => top + (1 - (value - low) / (high - low)) * (bottom - top);
    const timeAt = (position) => start + (position - left) / plotWidth * (end - start);
    const line = samples.map((sample, index) =>
      `${index ? "L" : "M"}${x(sample.t).toFixed(1)} ${y(sample[metric.key]).toFixed(1)}`).join("");
    const area = `${line}L${x(end).toFixed(1)} ${bottom}L${left} ${bottom}Z`;
    const decimals = high - low < 5 ? 1 : 0;
    const yTicks = [low, (low + high) / 2, high].map((value) => ({ position: y(value), label: fmtN(value, decimals) }));
    const xTicks = timeTicks.map((fraction) => {
      const time = start + (end - start) * fraction;
      return { position: x(time), label: formatTime(time) };
    });

    return { x, y, timeAt, line, area, xTicks, yTicks };
  }
</script>

<script>
  import { countTo, drawIn } from "#helpers/charts.js";
  import styles from "./Chart.module.css";

  let { metric, points, subtitle = "", animation, ready } = $props();
  let containerWidth = $state(0);
  let valueElement = $state();
  let lineElement = $state();
  let areaElement = $state();
  let hovered = $state.raw(null);
  let width = $derived(Math.max(containerWidth || 0, 240));
  let samples = $derived(points.filter((point, index) =>
    point[metric.key] != null && (!metric.zeroIsMissing || point[metric.key] !== 0) && (!metric.skipFirst || index > 0)));
  let latest = $derived(samples.at(-1)?.[metric.key]);
  let plot = $derived(samples.length < 2 ? null : chartGeometry(samples, metric, width));
  let active = $derived(plot && hovered && samples.includes(hovered) ? hovered : null);

  $effect(() => {
    const decimals = metric.decimals || 0;
    if (valueElement) countTo(valueElement, latest == null ? null : +latest.toFixed(decimals), decimals);
  });

  $effect(() => {
    if (lineElement && areaElement) drawIn(lineElement, areaElement);
  });

  function selectPoint(event) {
    const bounds = event.currentTarget.getBoundingClientRect();
    const position = (event.clientX - bounds.left) / bounds.width * width;
    const time = plot.timeAt(position);
    let nearest = samples[0];
    for (const sample of samples) {
      if (Math.abs(sample.t - time) < Math.abs(nearest.t - time)) nearest = sample;
    }
    hovered = nearest;
  }
</script>

<section class="card">
  <div class={styles.heading}>
    <div>
      <h2>{metric.title}</h2>
      <small>{subtitle}</small>
    </div>
    <div class={styles.value}><b bind:this={valueElement}>—</b><span>{metric.unit}</span></div>
  </div>
  <div class={styles.chart} bind:clientWidth={containerWidth}>
    {#if plot}
      <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden="true"
        onpointermove={selectPoint} onpointerleave={() => hovered = null}>
        {#each plot.yTicks as tick}
          <line class={styles.gridLine} x1={left} x2={width - right} y1={tick.position} y2={tick.position} />
          <text class={styles.axis} x={left - 8} y={tick.position + 3.5} text-anchor="end">{tick.label}</text>
        {/each}
        {#each plot.xTicks as tick}
          <text class={styles.axis} x={tick.position} y={height - 4} text-anchor="middle">{tick.label}</text>
        {/each}
        {#key animation}
          <path class={styles.area} d={plot.area} bind:this={areaElement} />
          <path class={styles.line} d={plot.line} bind:this={lineElement} />
        {/key}
        {#if active}
          {@const pointX = plot.x(active.t)}
          <line class={styles.cross} y1={top} y2={bottom} x1={pointX} x2={pointX} />
          <circle class={styles.point} r="4" cx={pointX} cy={plot.y(active[metric.key])} />
        {/if}
      </svg>
      {#if active}
        <div class={styles.tooltip} style:--point-x={`${plot.x(active.t) / width * 100}%`}>
          {fmtN(active[metric.key], metric.decimals || 0)} {metric.unit} · {formatTime(active.t)}
        </div>
      {/if}
    {:else if ready}
      <p class="empty">Данные копятся — первые точки появятся через минуту</p>
    {/if}
  </div>
</section>
