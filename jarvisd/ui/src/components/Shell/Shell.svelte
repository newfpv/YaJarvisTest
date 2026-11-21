<script>
  import { onMount, tick } from "svelte";
  import { navPill } from "#helpers/charts.js";
  import { act, api, icon, observePills, tip } from "#helpers/core.js";
  import { reload } from "#state/panelState.js";
  import { subscribeMusic } from "#state/player.js";
  import { profilePath } from "#helpers/paths.js";
  import Gate from "#components/Shell/Gate/Gate.svelte";

  let { access, login, logout, sidebar, orb, navigation, children } = $props();
  let active = $derived(access.status === "active");
  let profiles = $derived(sidebar?.profiles || []);
  let page = $derived(navigation?.nav || "home");
  let view = $state();
  let menuOpen = $state(false);
  let dragged = $state("");
  let playing = $state(false);

  onMount(() => subscribeMusic((state) => { playing = !!state?.playing && !state.paused; }));

  $effect(() => {
    document.body.classList.toggle("menu-open", menuOpen);
  });

  $effect(() => {
    const route = navigation;
    closeMenu();
    if (active && route && view) {
      view.classList.remove("enter");
      void view.offsetWidth;
      view.classList.add("enter");
    }
  });

  $effect(() => {
    if (active && sidebar && navigation) void tick().then(navPill);
  });

  function closeMenu() { menuOpen = false; }

  function startDrag(event, id) {
    dragged = id;
    event.dataTransfer.setData("text/plain", id);
  }

  async function reorder(event, to) {
    event.preventDefault();
    const from = event.dataTransfer.getData("text/plain");
    const ids = profiles.map((profile) => profile.id);
    if (!ids.includes(from) || from === to) return;
    ids.splice(ids.indexOf(from), 1);
    ids.splice(ids.indexOf(to), 0, from);
    await act(null, async () => {
      await api("POST", "/api/profiles/order", { ids });
      await reload();
    });
  }
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape") closeMenu(); }} />

<Gate {access} {login} />
<div id="app" class="app" class:hidden={!active}>
  <aside class="side" class:open={menuOpen} id="side">
    <a class="brand" href="/"><span class="orb" id="orb" data-s={orb.state} role="img" aria-label={orb.text} title={orb.text}><i></i></span><span class="brand-t"><b>YaJarvis</b><small id="ver">v{sidebar?.version ?? "…"}</small></span></a>
    <nav class="nav" id="nav">
      <a href="/" data-page="home" class:on={page === "home"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3h7v7H3zM14 3h7v7h-7zM14 14h7v7h-7zM3 14h7v7H3z"/></svg>Главная</a>
      <a href="/player" data-page="player" class:on={page === "player"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>Плеер<span class="nav-eq" class:hidden={!playing} id="nav-eq"><i></i><i></i><i></i></span></a>
      <div class="nav-group">Ассистенты {@html tip("Ассистент — набор настроек: голос, модель, характер. Активный отвечает на «Джарвис». Перетаскивайте, чтобы поменять порядок.")}</div>
      <div id="profiles" class="profiles">
        {#each profiles as profile (profile.id)}
          <a href={profilePath(profile.id)} class="prof" class:on={page === "p:" + profile.id} class:drag={dragged === profile.id}
            draggable="true" data-id={profile.id}
            ondragstart={(event) => startDrag(event, profile.id)} ondragend={() => { dragged = ""; }}
            ondragover={(event) => event.preventDefault()} ondrop={(event) => reorder(event, profile.id)}>
            <span class={"av c" + profile.color}>{profile.initial}</span>
            <div class="meta"><div class="n">{profile.name}</div><div class="s">{profile.detail}</div></div>
            {#if profile.active}<span class="act" title="Активный профиль"></span>{/if}
          </a>
        {/each}
      </div>
      <a href={profilePath("new")} class="navbtn" id="add-profile"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14M12 5v14"/></svg>Новый ассистент</a>
      <a href="/characters" data-page="characters" class:on={page === "characters"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2M9 9h.01M15 9h.01"/></svg>Характеры</a>
      <a href="/memory" data-page="memory" class:on={page === "memory"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4M17.599 6.5a3 3 0 0 0 .399-1.375M6.003 5.125A3 3 0 0 0 6.401 6.5M3.477 10.896a4 4 0 0 1 .585-.396M19.938 10.5a4 4 0 0 1 .585.396M6 18a4 4 0 0 1-1.967-.516M19.967 17.484A4 4 0 0 1 18 18"/></svg>Память</a>
      <div class="nav-group">Колонка</div>
      <a href="/speaker" data-page="speaker" class:on={page === "speaker"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect width="16" height="20" x="4" y="2" rx="2"/><path d="M12 6h.01"/><circle cx="12" cy="14" r="4"/></svg>Колонка</a>
      <a href="/connections" data-page="connections" class:on={page === "connections"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 22v-5M9 8V2M15 8V2M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/></svg>Подключения</a>
      <div id="nav-ext" class="nav-ext">{#if sidebar?.training}<a href="/training" data-page="training" class:on={page === "training"}>{@html icon("mic")}Обучение</a>{/if}</div>
      <div class="nav-group">Состояние</div>
      <a href="/charts" data-page="charts" class:on={page === "charts"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3v18h18"/><path d="m7 15 4-5 4 3 5-7"/></svg>Графики</a>
      <a href="/logs" data-page="logs" class:on={page === "logs"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6M8 13h8M8 17h5"/></svg>Логи</a>
      <div class="nav-sep"></div>
      <a href="/security" data-page="security" class:on={page === "security"}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>Безопасность<span class="dot" class:hidden={!sidebar?.updateAvailable} id="upd-dot"></span></a>
    </nav>
    <div class="spacer"></div>
    <button class="navbtn" id="logout" type="button" onclick={logout}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/></svg>Выйти</button>
  </aside>
  <header class="mbar"><button class="ib" id="menu" type="button" aria-label="Меню" aria-expanded={menuOpen} aria-controls="side" onclick={() => { menuOpen = !menuOpen; }}><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16"/></svg></button><span class="brand"><span class="logo"></span><b>Джарвис</b></span></header>
  <main id="view" class="view" use:observePills bind:this={view}>{@render children?.()}</main>
</div>
<div id="toast" class="toast" role="status"></div>
<button type="button" class="scrim" tabindex="-1" aria-label="Закрыть меню" onclick={closeMenu}></button>
