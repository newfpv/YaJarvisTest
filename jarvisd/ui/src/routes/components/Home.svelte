<script>
  import { onMount } from "svelte";
  import { icon } from "#helpers/core.js";
  import { getPanel } from "#state/panelContext.js";
  import { S } from "#state/panelState.js";
  import HomeChat from "./HomeChat/HomeChat.svelte";
  import HomeLogs from "./HomeLogs/HomeLogs.svelte";
  import HomeMetrics from "./HomeMetrics/HomeMetrics.svelte";
  import HomeOverview from "./HomeOverview/HomeOverview.svelte";
  import { readOverview } from "./HomeOverview/overview.js";
  import HomePlayer from "./HomePlayer/HomePlayer.svelte";
  import HomeVolume from "./HomeVolume/HomeVolume.svelte";

  const { poll, onPoll } = getPanel();
  const supportPreference = "yajavis.star.dismissed";
  let overview = $state.raw(readOverview(S));
  let chat = $state();
  let metrics = $state();
  let logs = $state();
  let supportDismissed = $state(readSupportPreference());

  onMount(() => onPoll(refresh));

  function refresh() {
    overview = readOverview(S);
    chat?.refresh();
    metrics?.refresh();
    logs?.refresh();
  }

  function readSupportPreference() {
    try { return localStorage.getItem(supportPreference) === "1"; }
    catch { return false; }
  }

  function dismissSupport() {
    supportDismissed = true;
    try { localStorage.setItem(supportPreference, "1"); }
    catch { /* Карточка в любом случае скрыта до ухода со страницы. */ }
  }
</script>

<header class="ph ph-row">
  <div>
    <h1>Главная</h1>
    <p class="sub"><span class="dot" class:on={overview.micOnline} class:off={!overview.micOnline} id="home-dot"></span><span id="home-sub">{overview.summary}</span></p>
  </div>
</header>

<HomeMetrics bind:this={metrics} />
<div class="home">
  <section class="tg" id="home-chat" aria-label="Переписка с ассистентом">
    <HomeChat {poll} bind:this={chat} />
  </section>
  <aside class="rail">
    <HomeOverview {overview} />
    {#if !supportDismissed}
      <section class="card github-support" id="github-support" aria-label="Поддержать проект">
        <button class="github-close" id="github-dismiss" type="button"
          aria-label="Больше не показывать просьбу поддержать проект" title="Больше не показывать"
          onclick={dismissSupport}>{@html icon("x")}</button>
        <span class="github-mark" aria-hidden="true">
          <svg viewBox="0 0 24 24"><path d="M9 19c-4.3 1.3-4.3-2.2-6-2.6M15 22v-3.9a3.4 3.4 0 0 0-.9-2.6c3-.3 6.2-1.5 6.2-6.9a5.4 5.4 0 0 0-1.5-3.7 5.3 5.3 0 0 0-.1-3.7S17.5.8 15 2.6a12.8 12.8 0 0 0-6 0C6.5.8 5.3 1.2 5.3 1.2a5 5 0 0 0-.1 3.7 5.4 5.4 0 0 0-1.5 3.7c0 5.4 3.2 6.2 6.2 6.9a3.4 3.4 0 0 0-.9 2.6V22"/></svg>
        </span>
        <div class="github-copy">
          <h2>Поддержать YaJavis</h2><p>Нравится проект? Оставьте звезду на GitHub.</p>
          <a class="github-link" href="https://github.com/newfpv/YaJavis" target="_blank" rel="noopener noreferrer">Поставить звезду {@html icon("ext")}</a>
        </div>
      </section>
    {/if}
    <HomePlayer />
    <HomeVolume />
    <HomeLogs bind:this={logs} />
  </aside>
</div>
