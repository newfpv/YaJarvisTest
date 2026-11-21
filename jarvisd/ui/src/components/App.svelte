<script>
  import { onMount } from "svelte";
  import { afterNavigate, beforeNavigate } from "$app/navigation";
  import { page } from "$app/state";
  import Shell from "#components/Shell/Shell.svelte";
  import { api, dirty, setDirty, toast } from "#helpers/core.js";
  import { setPanel } from "#state/panelContext.js";
  import { S, access, readJournal, readOrb, readSidebar, reload, routeRequests, showGate, speaker } from "#state/panelState.js";
  import { musicData, refreshVolume } from "#state/player.js";
  import { closeWizard, wizardRequest, wizOpen } from "#state/wizard.js";

  let { children } = $props();
  const lifetime = new AbortController();
  let sidebar = $derived($speaker ? readSidebar($speaker) : null);
  let navigation = $derived({
    nav: page.params.id !== undefined ? "p:" + page.params.id : page.url.pathname.split("/")[1] || "home",
    path: page.url.pathname + page.url.search,
    revision: $routeRequests,
  });
  let orb = $state.raw({ state: "idle", text: "Состояние колонки" });
  let Wizard = $state.raw(null);
  let pageRefresh;
  let session;
  let interval;
  let fastPoll;

  setPanel({ poll, onPoll });

  beforeNavigate(({ willUnload, cancel }) => {
    if (!willUnload && dirty && !confirm("Есть несохранённые изменения. Уйти без сохранения?")) cancel();
  });
  afterNavigate(() => setDirty(false));

  onMount(() => {
    const unsubscribeRoutes = routeRequests.subscribe(() => setDirty(false));
    const unsubscribeAccess = access.subscribe(({ status }) => {
      stopPolling();
      if (status !== "active") {
        setDirty(false);
        closeWizard();
        return;
      }
      session = new AbortController();
      orb = readOrb();
      if (!S.settings.onboarded) wizOpen(0);
      interval = setInterval(poll, 2000);
      void poll();
    });

    void start().catch(() => {});
    return () => {
      lifetime.abort();
      stopPolling();
      unsubscribeRoutes();
      unsubscribeAccess();
      closeWizard();
    };
  });

  $effect(() => {
    if ($access.status !== "active" || !$wizardRequest || Wizard) return;
    let cancelled = false;
    void import("#components/Wizard/Wizard.svelte").then(({ default: component }) => {
      if (!cancelled) Wizard = component;
    }).catch((error) => {
      if (!cancelled) toast(error.message, true);
    });
    return () => { cancelled = true; };
  });

  async function start() {
    await reload(lifetime.signal);
    access.set({ status: "active" });
  }

  async function login(setup, password) {
    await api("POST", setup ? "/api/setup" : "/api/login", { password }, { signal: lifetime.signal });
    await start();
  }

  async function logout() {
    await api("POST", "/api/logout", undefined, { signal: lifetime.signal }).catch(() => {});
    if (!lifetime.signal.aborted) showGate(false);
  }

  function stopPolling() {
    session?.abort();
    session = undefined;
    clearInterval(interval);
    clearTimeout(fastPoll);
  }

  function onPoll(refresh) {
    pageRefresh = refresh;
    return () => {
      if (pageRefresh === refresh) pageRefresh = undefined;
    };
  }

  async function poll() {
    if (!session) return;
    const { signal } = session;
    const view = navigation;
    try {
      if (!await readJournal(signal) || signal.aborted) return;
    } catch {
      return;
    }
    orb = readOrb();
    refreshVolume();
    if (view === navigation) void pageRefresh?.();
    void musicData();
    clearTimeout(fastPoll);
    if (S.speaker.talking && navigation.nav === "home") fastPoll = setTimeout(poll, 500);
  }
</script>

<svelte:window onbeforeunload={(event) => { if (dirty) event.preventDefault(); }} />

<Shell access={$access} {login} {logout} {sidebar} {orb} {navigation}>
  {#if $access.status === "active"}
    {#key $routeRequests}
      {@render children?.()}
    {/key}
  {/if}
</Shell>

{#if $access.status === "active" && $wizardRequest && Wizard}
  <Wizard request={$wizardRequest} />
{/if}
