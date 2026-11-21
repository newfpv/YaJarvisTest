<script>
  import { countTo, drawIn } from "#helpers/charts.js";
  import { tip } from "#helpers/core.js";
  import { sparkPaths } from "../metrics.js";

  let { metric } = $props();
  const height = 46;
  let cardWidth = $state(0);
  let valueElement = $state();
  let line = $state();
  let area = $state();
  let drawn = false;
  let width = $derived(cardWidth || 200);
  let paths = $derived(sparkPaths(metric.series, width, height));
  let barWidth = $derived(Math.round(Math.min(1, Math.max(0, metric.fraction)) * 100) + "%");

  $effect(() => {
    if (valueElement) countTo(valueElement, metric.value);
  });

  $effect(() => {
    if (!paths || !line || !area || drawn) return;
    drawn = true;
    drawIn(line, area, 1300);
  });
</script>

<div class="kpi" id={"k-" + metric.id} bind:clientWidth={cardWidth}>
  <span class="label">{metric.label}{#if metric.tip} {@html tip(metric.tip)}{/if}</span>
  <div class="value"><b bind:this={valueElement}>—</b><span>{metric.value == null ? "" : metric.unit}</span></div>
  <div class="kbar {metric.level || ""}"><i style:width={barWidth}></i></div>
  <span class="meta">{metric.description}</span>
  <svg class="spark" viewBox={`0 0 ${width} ${height}`} aria-hidden="true">
    <path class="a" d={paths?.area} bind:this={area}></path>
    <path class="l" d={paths?.line} bind:this={line}></path>
  </svg>
</div>
