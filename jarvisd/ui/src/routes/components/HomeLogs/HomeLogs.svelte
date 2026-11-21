<script>
  import { tick } from "svelte";
  import { api, icon } from "#helpers/core.js";
  import { parseShortLogLine } from "#helpers/logs.js";

  const minHeight = 170;
  const lineHeight = 19;
  const refreshInterval = 5000;

  let height = $state(0);
  let visible = $derived(height >= minHeight);
  let logbox = $state();
  let entries = $state.raw([]);
  let emptyText = $state("");
  let request;
  let lastRequest = 0;

  $effect(() => {
    if (!visible || !logbox) return;
    void loadLogs();
    return () => {
      request?.abort();
      request = undefined;
    };
  });

  export function refresh() {
    if (visible && Date.now() - lastRequest >= refreshInterval) void loadLogs();
  }

  async function loadLogs() {
    if (request) return;
    const controller = new AbortController();
    const { signal } = controller;
    const limit = Math.max(10, Math.ceil(logbox.clientHeight / lineHeight) + 4);
    request = controller;
    lastRequest = Date.now();

    try {
      const data = await api("GET", `/api/logs?src=jarvis&n=${limit}`, undefined, { signal });
      const nextEntries = (data.lines || []).map(parseShortLogLine);
      const nextEmptyText = nextEntries.length ? "" : data.error || "Пока пусто";
      const sameEntries = nextEntries.length === entries.length && nextEntries.every(({ time, message, level }, index) => {
        const previous = entries[index];
        return time === previous.time && message === previous.message && level === previous.level;
      });
      if (sameEntries && nextEmptyText === emptyText) return;

      entries = nextEntries;
      emptyText = nextEmptyText;
      await tick();
      if (!signal.aborted) logbox.scrollTop = logbox.scrollHeight;
    } catch {
      // При ошибке сохраняем последние полученные строки.
    } finally {
      if (request === controller) request = undefined;
    }
  }
</script>

<section class="card home-logs" class:tight={!visible} id="hlogs" bind:clientHeight={height}>
  <div class="card-head">
    <h2>Логи</h2>
    <a class="meta hl-all" href="/logs">Все логи {@html icon("right")}</a>
  </div>
  <div class="logbox" id="hlogbox" bind:this={logbox}>
    {#each entries as { time, message, level }}
      <div class={["ll", level]}><span class="lt">{time}</span>{message}</div>
    {:else}
      {#if emptyText}<p class="empty">{emptyText}</p>{/if}
    {/each}
  </div>
</section>
