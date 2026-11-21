<script>
  import { onMount } from "svelte";
  import { icon, toast } from "#helpers/core.js";
  import { music, playMusicQuery, subscribeMusic } from "#state/player.js";
  import styles from "./PlayerQueue.module.css";

  let state = $state.raw(null);
  let query = $state("");
  let busy = $state(false);
  let disposed = false;
  let playing = $derived(!!(state?.playing || state?.paused));
  let yandex = $derived(state?.source === "yandex");
  let current = $derived(state?.track || {});
  let queued = $derived(state?.queue || 0);
  let next = $derived(yandex ? [] : state?.next || []);
  let history = $derived((state?.history || []).filter((track) => !(playing && !yandex && track.ID === current.ID)).slice(0, 12));

  onMount(() => {
    const unsubscribe = subscribeMusic((snapshot) => state = snapshot);
    return () => {
      disposed = true;
      unsubscribe();
    };
  });

  async function run(action) {
    if (busy) return;
    busy = true;
    try {
      await action();
    } finally {
      busy = false;
    }
  }

  function submit(event) {
    event.preventDefault();
    const text = query.trim();
    if (!text) return;
    const addToQueue = event.submitter?.value === "queue";
    void run(async () => {
      const ok = addToQueue ? await music({ action: "queue", query: text }) : await playMusicQuery(text);
      if (!ok || disposed) return;
      query = "";
      if (addToQueue) toast("Добавлено в очередь");
    });
  }

  function play(track, index) {
    if (!track.ID) return;
    void run(async () => {
      if (index != null && !(await music({ action: "unqueue", index }))) return;
      await music({ action: "id", query: track.ID });
    });
  }

  function remove(index) {
    void run(() => music({ action: "unqueue", index }));
  }

  function thumbnail(track) {
    return track.ID && !/^\d+$/.test(track.ID) ? `https://i.ytimg.com/vi/${encodeURIComponent(track.ID)}/mqdefault.jpg` : "";
  }
</script>

{#snippet trackInfo(track, image, badge, isCurrent = false)}
  {#if image}
    <img class={styles.image} src={image} alt="" loading="lazy">
  {:else}
    <span class={styles.image}></span>
  {/if}
  <span class={styles.description}><b>{track.Title || "…"}</b><span>{track.Author || ""}</span></span>
  {#if isCurrent}
    <span class={["np-eq", styles.equalizer]} aria-hidden="true"><i></i><i></i><i></i></span>
  {:else if badge}
    <span class="badge">{badge}</span>
  {/if}
{/snippet}

{#snippet row(track, badge = "", index = null)}
  <li class={styles.row}>
    <button class={styles.track} type="button" aria-label={`Включить «${track.Title || "…"}»`}
      disabled={busy || !track.ID} onclick={() => play(track, index)}>
      {@render trackInfo(track, thumbnail(track), badge)}
    </button>
    {#if index != null}
      <button class={["ib", styles.remove]} type="button" disabled={busy}
        aria-label={`Убрать из очереди «${track.Title || "…"}»`} data-tip="Убрать" onclick={() => remove(index)}>
        {@html icon("x")}
      </button>
    {/if}
  </li>
{/snippet}

<section class="card" aria-labelledby="player-queue-title">
  <div class="card-head">
    <h2 id="player-queue-title">Очередь</h2>
    {#if state}
      <span class="meta">{yandex ? "Яндекс Музыка" : `${queued} в очереди${state.repeat ? " · повтор" : state.radio ? " · дальше похожее" : ""}`}</span>
    {/if}
  </div>
  <form class={styles.form} onsubmit={submit}>
    <label class={styles.field}>
      {@html icon("search")}
      <input type="text" placeholder="Название или ссылка на YouTube" autocomplete="off"
        aria-label="Что включить" bind:value={query} readonly={busy}>
    </label>
    <button class="cbtn" type="submit" value="play" disabled={busy || !query.trim()}
      data-tip="Включить сейчас" aria-label="Включить сейчас">{@html icon("play", "solid")}</button>
    <button class="cbtn" type="submit" value="queue" disabled={busy || !query.trim()}
      data-tip="В очередь" aria-label="В очередь">{@html icon("plus")}</button>
  </form>
  {#if playing || next.length}
    <ul class={styles.list}>
      {#if playing}
        <li class={[styles.row, styles.current]} aria-current="true">
          <div class={styles.track}>{@render trackInfo(current, (yandex && state.cover) || thumbnail(current), "", true)}</div>
        </li>
      {/if}
      {#each next as track, index}
        {@render row(track, index < queued ? String(index + 1) : state.radio ? "похожее" : "дальше", index < queued ? index : null)}
      {/each}
    </ul>
  {/if}
  {#if playing && yandex}
    <p class="empty">Очередью Яндекс Музыки управляет Алиса: «Алиса, следующий трек», «Алиса, включи похожее».</p>
  {:else if !playing && !next.length}
    <p class="empty">{state ? "Очередь пуста. Найдите трек выше и нажмите «+» или скажите «Джарвис, добавь в очередь …»" : "Загружаю очередь…"}</p>
  {/if}
</section>

<section class="card" aria-labelledby="player-history-title">
  <div class="card-head"><h2 id="player-history-title">Недавно играло</h2><span class="meta">YouTube · нажмите, чтобы включить</span></div>
  {#if history.length}
    <ul class={styles.list}>
      {#each history as track}{@render row(track)}{/each}
    </ul>
  {:else if state}
    <p class="empty">Здесь появится то, что включали с YouTube</p>
  {/if}
</section>
