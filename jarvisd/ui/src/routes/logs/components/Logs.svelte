<script module>
  const sources = [
    ["jarvis", "Джарвис"],
    ["alice", "Алиса"],
    ["media", "Звук и музыка"],
  ];
  const preferences = { source: "jarvis", query: "", autoRefresh: true };
</script>

<script>
  import { onMount, tick } from "svelte";
  import { api, copyText, icon, toast } from "#helpers/core.js";
  import { parseLogLine } from "#helpers/logs.js";
  import { getPanel } from "#state/panelContext.js";
  import styles from "./Logs.module.css";

  let source = $state(preferences.source);
  let query = $state(preferences.query);
  let autoRefresh = $state(preferences.autoRefresh);
  let lines = $state.raw([]);
  let entries = $derived(lines.map(parseLogLine));
  let emptyText = $state("Загружаю…");
  let logbox = $state();
  let request;
  let searchTimer;
  const { onPoll } = getPanel();

  onMount(() => {
    const unsubscribe = onPoll(refresh);
    void refresh(true);
    return () => {
      unsubscribe();
      clearTimeout(searchTimer);
      request?.abort();
    };
  });

  async function refresh(force = false) {
    if (!force && (!autoRefresh || searchTimer !== undefined || request)) return;

    request?.abort();
    const controller = new AbortController();
    const { signal } = controller;
    request = controller;
    try {
      const data = await api("GET", `/api/logs?src=${source}&n=400&q=${encodeURIComponent(query)}`, undefined, { signal });
      if (signal.aborted) return;

      const nextLines = data.lines || [];
      const nextEmptyText = data.error || (query ? "Ничего не нашлось" : "Пока пусто");
      const sameLines = nextLines.length === lines.length && nextLines.every((line, index) => line === lines[index]);
      const sameEmptyState = nextLines.length > 0 || nextEmptyText === emptyText;
      if (!force && sameLines && sameEmptyState) return;

      const pinned = force || logbox.scrollHeight - logbox.scrollTop - logbox.clientHeight < 60;
      lines = nextLines;
      emptyText = nextEmptyText;
      if (pinned) {
        await tick();
        if (!signal.aborted) logbox.scrollTop = logbox.scrollHeight;
      }
    } catch (error) {
      if (!signal.aborted && (force || !lines.length)) {
        lines = [];
        emptyText = error.message;
      }
    } finally {
      if (request === controller) request = undefined;
    }
  }

  function selectSource(nextSource) {
    source = preferences.source = nextSource;
    clearTimeout(searchTimer);
    searchTimer = undefined;
    void refresh(true);
  }

  function search(event) {
    query = preferences.query = event.currentTarget.value;
    request?.abort();
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      searchTimer = undefined;
      void refresh(true);
    }, 250);
  }

  function changeAutoRefresh(event) {
    autoRefresh = preferences.autoRefresh = event.currentTarget.checked;
  }

  async function copyLogs() {
    try {
      await copyText(logbox.innerText);
      toast("Лог скопирован");
    } catch {
      toast("Не получилось скопировать", true);
    }
  }
</script>

<header class="ph ph-row">
  <div>
    <h1>Логи</h1>
    <p class="sub">Что делали Джарвис и Алиса — последние 400 строк. Тексты разговоров пишутся только в журнал Джарвиса</p>
  </div>
  <div class="actions">
    <div class="seg" role="group" aria-label="Источник логов">
      {#each sources as [id, label] (id)}
        <button type="button" class:on={source === id} aria-pressed={source === id} onclick={() => selectSource(id)}>
          {label}
        </button>
      {/each}
    </div>
  </div>
</header>

<section class={["card", styles.card]}>
  <div class={styles.toolbar}>
    <label class={styles.search}>
      {@html icon("search")}
      <input type="search" aria-label="Найти в логе" placeholder="Найти в логе" value={query} oninput={search} autocomplete="off">
    </label>
    <label class="sw">
      <input type="checkbox" checked={autoRefresh} onchange={changeAutoRefresh}>
      <span class="tr"></span>
      Обновлять сами
    </label>
    <button class="btn sm" type="button" onclick={copyLogs}>{@html icon("link")}Скопировать</button>
  </div>
  <div class={styles.logbox} bind:this={logbox}>
    {#each entries as { time, message, level }}
      <div class={[styles.line, styles[level]]}><span class={styles.time}>{time}</span>{message}</div>
    {:else}
      <p class={["empty", styles.empty]}>{emptyText}</p>
    {/each}
  </div>
</section>
