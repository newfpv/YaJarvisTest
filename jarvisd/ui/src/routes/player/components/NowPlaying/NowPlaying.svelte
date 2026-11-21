<script>
  import { onMount } from "svelte";
  import { icon } from "#helpers/core.js";
  import { artOf, clockStr, mountPlayback, mountVolume, musicAct, musicLink, setArt } from "#state/player.js";
  import styles from "./NowPlaying.module.css";

  let state = $state.raw(null);
  let cover = $state();
  let bars = $state();
  let time = $state();
  let seek = $state();
  let volume = $state();
  let level = $state(null);
  let titleWidth = $state(0);
  let textWidth = $state(0);
  let pending = $state(false);

  let track = $derived(state?.track || {});
  let active = $derived(!!(state?.playing || state?.paused));
  let yandex = $derived(state?.source === "yandex");
  let source = $derived(yandex ? "Яндекс Музыка" : "YouTube");
  let playback = $derived(active ? (state.paused ? "paused" : "playing") : state?.loading ? "loading" : "idle");
  let title = $derived(active ? track.Title || "…" : playback === "loading" ? "Ищу и включаю…" : "Ничего не играет");
  let artist = $derived(active ? track.Author || source : "Скажите «Джарвис, включи с ютуба …» или попросите Алису");
  let kicker = $derived(active ? source + (state.paused ? " · пауза" : "") : playback === "loading" ? "YouTube · загрузка" : "Тишина");
  let image = $derived(active ? artOf(track, true, state) : "");
  let fallback = $derived(active ? artOf(track, false, state) : "");
  let link = $derived(active ? musicLink(state) : "");
  let overflow = $derived(Math.max(0, textWidth - titleWidth));
  let duration = $derived(state?.dur ? clockStr(state.dur) : "");

  onMount(() => {
    const releasePlayback = mountPlayback(seek, time, bars, (snapshot) => { state = snapshot; });
    const releaseVolume = mountVolume(volume, (value) => { level = value; });
    return () => {
      releasePlayback();
      releaseVolume();
    };
  });

  $effect(() => {
    if (cover) setArt(cover, image, fallback);
  });

  async function control(event, action) {
    if (pending) return;
    pending = true;
    try {
      await musicAct(action, event.currentTarget, { disableControl: false });
    } finally {
      pending = false;
    }
  }
</script>

<section class={["card", styles.card]} id="plh" data-state={playback} aria-labelledby="pl-title">
  <img class={styles.glow} src={fallback || undefined} alt="">
  <div class={styles.top}>
    <div class={styles.cover}>
      {@html icon("music")}
      <img bind:this={cover} alt="">
    </div>
    <div class={styles.meta}>
      <div class={styles.kicker}>
        <span class="np-eq" bind:this={bars} aria-hidden="true"><i></i><i></i><i></i></span><span>{kicker}</span>
      </div>
      {#key title}
        <h2 class={[styles.title, { [styles.scroll]: overflow > 4 }]} id="pl-title" {title}
          bind:clientWidth={titleWidth} style:--distance={`${-overflow - 24}px`}
          style:--duration={`${Math.max(6, overflow / 28)}s`}>
          <span class={styles.titleText} bind:clientWidth={textWidth}>{title}</span>
        </h2>
      {/key}
      {#key artist}
        <div class={styles.artist} title={artist}>{artist}</div>
      {/key}
      {#if active}
        <div class={styles.chips}>
          <span class="chip">{yandex ? "через Алису" : "через Джарвиса"}</span>
          {#if state.album}<span class="chip">Альбом «{state.album}»</span>{/if}
          {#if !yandex && state.radio}<span class="chip">радио</span>{/if}
          {#if !yandex && state.repeat}<span class="chip">повтор</span>{/if}
        </div>
      {/if}
    </div>
  </div>

  <div class={styles.bottom}>
    <div class={["progress", styles.seek]} aria-hidden={!active}>
      <span class="roll" id="pl-cur" bind:this={time}></span><div bind:this={seek}></div><span>{duration}</span>
    </div>
    <div class={styles.bar}>
      <div class={styles.actions}>
        {#if !yandex}
          <button class="cbtn" class:on={!!state?.radio} id="pl-radio" type="button" disabled={pending}
            data-tip="Радио: когда очередь кончится, играть похожее" aria-label="Радио" aria-pressed={!!state?.radio}
            onclick={(event) => control(event, "radio")}>{@html icon("radio")}</button>
        {/if}
        {#if link}
          <button class="cbtn" type="button" disabled={pending} data-tip="Скопировать ссылку" aria-label="Скопировать ссылку"
            onclick={(event) => control(event, "copy")}>{@html icon("link")}</button>
          <a class="cbtn" href={link} target="_blank" rel="noopener" aria-label="Открыть в браузере"
            data-tip={yandex ? "Открыть в Яндекс Музыке" : "Открыть на YouTube"}>{@html icon("ext")}</a>
        {/if}
      </div>
      <div class={styles.controls}>
        <button class="cbtn" type="button" disabled={!active || pending} data-tip="Выключить" aria-label="Выключить"
          onclick={(event) => control(event, "stop")}>{@html icon("stop", "solid")}</button>
        <button class="cbtn" id="pl-prev" type="button" disabled={!active || pending || (!yandex && (state?.history?.length || 0) < 2)}
          aria-label="Предыдущий" onclick={(event) => control(event, "prev")}>{@html icon("prev", "solid")}</button>
        <button class="cbtn big" id="pl-toggle" type="button" disabled={!active || pending}
          aria-label={playback === "playing" ? "Пауза" : "Продолжить"}
          onclick={(event) => control(event, "toggle")}>{@html icon(playback === "playing" ? "pause" : "play", "solid")}</button>
        <button class="cbtn" id="pl-next" type="button" disabled={!active || pending || (!yandex && !state?.next?.length)}
          aria-label="Следующий" onclick={(event) => control(event, "next")}>{@html icon("skip", "solid")}</button>
        <button class="cbtn" class:on={!yandex && !!state?.repeat} id="pl-repeat" type="button"
          disabled={!active || pending || yandex}
          data-tip={yandex ? "Повтор доступен для YouTube" : state?.repeat ? "Повтор включён: этот трек по кругу" : "Повторять этот трек"}
          aria-label="Повтор" aria-pressed={!yandex && !!state?.repeat}
          onclick={(event) => control(event, "repeat")}>{@html icon("repeat")}</button>
      </div>
      <div class={styles.volume} class:hidden={level === null} id="np-vol" title="Громкость колонки">
        {@html icon("speaker")}<div bind:this={volume}></div><b class="vol-big" id="vol-big" data-v={level ?? undefined}>{level ?? "–"}</b>
      </div>
    </div>
  </div>
</section>
