<script>
  import { onMount } from "svelte";
  import { icon } from "#helpers/core.js";
  import { artOf, clockStr, mountPlayback, musicAct, musicData, playMusicQuery, setArt } from "#state/player.js";

  let state = $state.raw(null);
  let cover = $state();
  let bars = $state();
  let time = $state();
  let seek = $state();
  let query = $state("");
  let pending = $state("");

  let track = $derived(state?.track || {});
  let active = $derived(!!(state?.playing || state?.paused));
  let yandex = $derived(state?.source === "yandex");
  let source = $derived(yandex ? "Яндекс Музыка" : "YouTube");
  let playback = $derived(active ? (state.paused ? "paused" : "playing") : state?.loading ? "loading" : "idle");
  let title = $derived(active ? track.Title || "…" : playback === "loading" ? "Ищу и включаю…" : "Тишина");
  let artist = $derived(active ? track.Author || source : "Включите что-нибудь с YouTube или попросите Алису");
  let kicker = $derived(active ? source + (state.paused ? " · пауза" : "") : playback === "loading" ? "ищу…" : state ? "Плеер" : "");
  let image = $derived(active ? artOf(track, true, state) : "");
  let fallback = $derived(active ? artOf(track, false, state) : "");
  let duration = $derived(state?.dur ? clockStr(state.dur) : "");

  onMount(() => {
    const release = mountPlayback(seek, time, bars, (snapshot) => { state = snapshot; });
    void musicData();
    return release;
  });

  $effect(() => {
    if (cover) setArt(cover, image, fallback);
  });

  async function control(event, action) {
    if (pending) return;
    pending = action;
    try {
      await musicAct(action, event.currentTarget, { disableControl: false });
    } finally {
      pending = "";
    }
  }

  function submit(event) {
    event.preventDefault();
    const value = query.trim();
    if (!value || playback === "loading") return;
    query = "";
    void playMusicQuery(value);
  }
</script>

<section class="card np" id="np" data-state={playback}>
  <div class="mu-top">
    <a class="cover" href="/player" id="np-cover" aria-label="Открыть плеер">
      {@html icon("music")}<img id="np-art" bind:this={cover} alt="">
    </a>
    <div class="np-info">
      <div class="np-src" id="np-kicker">
        <span class="np-eq" class:hidden={!active} bind:this={bars} aria-hidden="true"><i></i><i></i><i></i></span>{kicker}
      </div>
      {#key title}
        <div class="np-title" class:swap={state !== null} id="np-title" {title}>{title}</div>
      {/key}
      {#key artist}
        <div class="np-artist" class:swap={state !== null} id="np-artist" title={artist}>{artist}</div>
      {/key}
    </div>
  </div>
  <div class="progress np-play-ui">
    <span class="roll" id="np-cur" bind:this={time}></span><div id="np-seek" bind:this={seek}></div><span id="np-dur">{duration}</span>
  </div>
  <div class="controls np-play-ui">
    <button class="cbtn" type="button" data-m="stop" aria-label="Выключить" disabled={pending === "stop"}
      onclick={(event) => control(event, "stop")}>{@html icon("stop", "solid")}</button>
    <button class="cbtn" type="button" data-m="prev" id="np-prev" aria-label="Предыдущий"
      disabled={pending === "prev" || (!yandex && (state?.history?.length || 0) < 2)}
      onclick={(event) => control(event, "prev")}>{@html icon("prev", "solid")}</button>
    <button class="cbtn big" type="button" data-m="toggle" id="np-toggle" disabled={pending === "toggle"}
      aria-label={state?.paused ? "Продолжить" : "Пауза"}
      onclick={(event) => control(event, "toggle")}>{@html icon(playback === "playing" ? "pause" : "play", "solid")}</button>
    <button class="cbtn" type="button" data-m="next" id="np-next" aria-label="Следующий"
      disabled={pending === "next" || (!yandex && !state?.next?.length)}
      onclick={(event) => control(event, "next")}>{@html icon("skip", "solid")}</button>
    <button class="cbtn" type="button" data-m="copy" aria-label="Скопировать ссылку" disabled={pending === "copy"}
      onclick={(event) => control(event, "copy")}>{@html icon("link")}</button>
  </div>
  <form class="np-form" id="np-form" onsubmit={submit}>
    <input type="text" id="np-q" placeholder="Название или ссылка на YouTube" aria-label="Что включить" autocomplete="off" bind:value={query}>
    <button class="cbtn big" type="submit" aria-label="Включить" disabled={playback === "loading"}>{@html icon("play", "solid")}</button>
  </form>
</section>
