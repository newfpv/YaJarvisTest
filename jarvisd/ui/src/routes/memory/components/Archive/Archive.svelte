<script module>
  let lastQuery = "";
  const timeFormat = new Intl.DateTimeFormat("ru-RU", { hour: "2-digit", minute: "2-digit" });
</script>

<script>
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { api, icon, toast } from "#helpers/core.js";
  import sectionStyles from "../Memory.module.css";
  import styles from "./Archive.module.css";

  let { busy, run, onforget } = $props();
  const selected = new SvelteSet();
  let query = $state(lastQuery);
  let turns = $state.raw([]);
  let hasMore = $state(false);
  let loading = $state(true);
  let loadError = $state("");
  let request;
  let searchTimer;
  let disposed = false;

  onMount(() => {
    void refresh();
    return () => {
      disposed = true;
      clearTimeout(searchTimer);
      request?.abort();
    };
  });

  export function refresh(clearSelection = false) {
    clearTimeout(searchTimer);
    searchTimer = undefined;
    if (clearSelection) selected.clear();
    return load();
  }

  async function load(more = false) {
    if (disposed) return;
    request?.abort();
    const controller = new AbortController();
    const { signal } = controller;
    request = controller;
    loading = true;
    loadError = "";
    const before = more ? turns.at(-1)?.n || 0 : 0;
    const parameters = new URLSearchParams({ limit: "40", before: String(before), q: query });
    try {
      const result = await api("GET", "/api/memory/archive?" + parameters, undefined, { signal });
      turns = more ? [...turns, ...result.turns] : result.turns;
      hasMore = !!result.more;
    } catch (error) {
      if (!signal.aborted) loadError = error.message;
    } finally {
      if (request === controller) {
        request = undefined;
        loading = false;
      }
    }
  }

  function search(event) {
    query = lastQuery = event.currentTarget.value;
    request?.abort();
    request = undefined;
    clearTimeout(searchTimer);
    turns = [];
    hasMore = false;
    loadError = "";
    loading = true;
    searchTimer = setTimeout(() => {
      searchTimer = undefined;
      void load();
    }, 300);
  }

  function toggle(id, checked) {
    if (checked) selected.add(id);
    else selected.delete(id);
  }

  function forget(ids) {
    void run(async () => {
      const result = await api("POST", "/api/memory/forget", { n: ids });
      if (disposed) return;
      const removed = new Set(ids);
      for (const id of removed) selected.delete(id);
      turns = turns.filter((turn) => !removed.has(turn.n));
      if (request || searchTimer) void refresh();
      await onforget();
      if (!disposed) toast(result.deleted === 1 ? "Реплика удалена" : `Удалено реплик: ${result.deleted}`);
    });
  }

  function forgetSelected() {
    if (selected.size && confirm(`Удалить выбранные реплики (${selected.size})? Джарвис их больше не вспомнит.`)) {
      forget([...selected]);
    }
  }
</script>

<section class={["card", sectionStyles.section]} aria-labelledby="memory-archive-title">
  <div class={sectionStyles.heading}>
    <div>
      <h2 id="memory-archive-title">{@html icon("voice")}Архив разговоров</h2>
      <p class="muted">Дословно, последние 5000 реплик. На «как я рассказывал» Джарвис ищет здесь сам</p>
    </div>
  </div>
  <div class={styles.tools}>
    <label class="cx-search">
      {@html icon("search")}
      <input type="search" placeholder="Найти в разговорах" value={query} autocomplete="off" aria-label="Поиск по архиву" oninput={search}>
    </label>
    {#if selected.size}
      <button class="btn danger" type="button" disabled={busy} onclick={forgetSelected}>
        {@html icon("trash")}Удалить выбранные ({selected.size})
      </button>
    {/if}
  </div>
  {#if turns.length}
    <div class={styles.turns} aria-busy={loading}>
      {#each turns as turn, index (turn.n)}
        {@const event = !turn.user && /^\[.*\]$/.test(turn.bot || "")}
        {#if turn.day && turns[index - 1]?.day !== turn.day}
          <div class={styles.day}>{turn.day}</div>
        {/if}
        <div class={[styles.turn, { [styles.selected]: selected.has(turn.n), [styles.event]: event }]}>
          <label class={styles.check}>
            <input type="checkbox" aria-label="Выбрать реплику" checked={selected.has(turn.n)} disabled={busy}
              onchange={(event) => toggle(turn.n, event.currentTarget.checked)}>
          </label>
          <div class={styles.messages}>
            <small>{timeFormat.format(turn.ts * 1000)}</small>
            {#if turn.user}<p class={styles.user}><b>Вы</b><span>{turn.user}</span></p>{/if}
            {#if turn.bot}<p><b>{event ? "Событие" : "Джарвис"}</b><span>{event ? turn.bot.slice(1, -1) : turn.bot}</span></p>{/if}
          </div>
          <button class="ib danger" type="button" aria-label="Удалить реплику" disabled={busy}
            onclick={() => forget([turn.n])}>{@html icon("trash")}</button>
        </div>
      {/each}
    </div>
  {/if}
  {#if loadError}
    <p class="err" role="alert">{loadError}</p>
  {:else if !turns.length}
    <p class={sectionStyles.empty} role="status">{loading ? "Загружаю архив…" : query ? "Ничего не нашлось." : "Архив пока пуст."}</p>
  {/if}
  {#if hasMore}
    <div class="row"><button class="btn ghost" type="button" disabled={busy || loading} onclick={() => load(true)}>Показать ещё</button></div>
  {/if}
</section>
