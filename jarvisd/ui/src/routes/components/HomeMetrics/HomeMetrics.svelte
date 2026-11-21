<script module>
  let recentPoints = [];
  const historyInterval = 15000;
</script>

<script>
  import { onMount } from "svelte";
  import { api } from "#helpers/core.js";
  import { S } from "#state/panelState.js";
  import { readMetrics } from "./metrics.js";
  import MetricCard from "./MetricCard/MetricCard.svelte";

  const controller = new AbortController();
  const { signal } = controller;
  let system = $state.raw(S.sys);
  let points = $state.raw(recentPoints);
  let cards = $derived(readMetrics(system, points));
  let lastRequest = 0;
  let pending = false;

  onMount(() => {
    void loadHistory();
    return () => controller.abort();
  });

  export function refresh() {
    system = S.sys;
    if (Date.now() - lastRequest > historyInterval) void loadHistory();
  }

  async function loadHistory() {
    if (pending || signal.aborted) return;
    pending = true;
    lastRequest = Date.now();
    try {
      const data = await api("GET", "/api/stats?range=900", undefined, { signal });
      points = recentPoints = data.points || [];
    } catch {
      // При ошибке показываем последнюю полученную историю.
    } finally {
      pending = false;
    }
  }
</script>

<div class="kpis stagger" id="kpis">
  {#each cards as card (card.id)}
    <MetricCard metric={card} />
  {/each}
</div>
