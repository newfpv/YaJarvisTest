<script>
  import { tick, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { goto } from "$app/navigation";
  import ServiceEditor from "#components/ServiceEditor/ServiceEditor.svelte";
  import { connDefs, getService } from "#state/connections.js";
  import { api, icon, toast } from "#helpers/core.js";
  import { S, reload, voicePlugin } from "#state/panelState.js";
  import { PL, plCatalog, plDescription, plIcon, plLoad, plOfficialItems } from "#state/plugins.js";

  let { request } = $props();
  const steps = ["Начало", "Плагины", "Настройка", "Готово"];
  const groups = [
    ["assistant", "Голосовые ассистенты"], ["home", "Умный дом"], ["media", "Музыка"],
    ["system", "Колонка и сеть"], ["info", "Справки"], ["fun", "Игры"],
  ];
  const voiceIds = ["chatgpt", "gemini", "openai", "grok", "elevenlabs"];
  const serviceIds = { yandexhome: "yandex" };
  const picked = new SvelteSet();
  const failed = new SvelteMap();
  const added = new SvelteSet();

  let visible = $state(false);
  let showReturn = $state(false);
  let step = $state(0);
  let busy = $state(false);
  let error = $state("");
  let installing = $state("");
  let snapshot = $state.raw(S);
  let services = $state.raw(connDefs());
  let installed = $state.raw(new Map());
  let available = $state.raw([]);
  let catalogError = $state("");
  let catalogLoading = $state(false);
  let configuration = $derived(readConfiguration(snapshot, services, installed));
  let selected = $derived(step === 1 && available.some((item) => picked.has(item.id) && !installed.has(item.id) && !added.has(item.id)));
  let voice = $state("");

  let editor = $state.raw(null);
  let serviceId = $state("");
  let card = $state();
  let serviceTitle = $state();
  let returnScroll = 0;

  $effect(() => {
    const { step } = request;
    untrack(() => open(step));
  });

  function open(at) {
    editor = null;
    serviceId = "";
    showReturn = false;
    visible = true;
    voice = "";
    error = "";
    sync();
    changeStep(at);
    if (at === 1) void refreshCatalog();
  }

  function sync() {
    snapshot = S;
    services = connDefs();
    installed = new Map((PL.data?.installed || []).map((plugin) => [plugin.id, plugin]));
    available = plOfficialItems();
    catalogError = PL.official.error;
    catalogLoading = Boolean(PL.official.request);
  }

  function readConfiguration(state, definitions, present) {
    const readyServices = new Map(definitions.map((service) => [service.id, service.on]));
    const builtin = definitions.filter((service) => !service.plugin && service.group === "voice")
      .map((service) => ({ id: service.id, name: service.name, icon: "bolt", description: service.about, running: true }));
    const official = [...present.values()].filter((plugin) => plugin.official
      && !builtin.some((service) => service.id === plugin.id)
      && (readyServices.has(serviceIds[plugin.id] || plugin.id) || plugin.id === "training"));
    const connections = [...builtin, ...official].sort((a, b) => Number(b.id === "vpn") - Number(a.id === "vpn")
      || Number(voiceIds.includes(b.id)) - Number(voiceIds.includes(a.id))).map((plugin) => ({
      ...plugin,
      ready: plugin.running && (plugin.id === "training" ? Boolean(state.settings.wake_profile) : readyServices.get(serviceIds[plugin.id] || plugin.id)),
    }));
    const current = state.profiles?.find((profile) => profile.id === state.active)?.engine;
    const voices = connections.filter((plugin) => voiceIds.includes(plugin.id) && plugin.ready).map((plugin) => ({
      ...plugin, engine: plugin.id === "elevenlabs" && current !== "elevenlabs" ? "eleven_realtime" : plugin.id,
    }));
    return { connections, voices, defaultVoice: (voices.find((plugin) => plugin.id === voicePlugin(current)) || voices[0])?.engine || "" };
  }

  $effect(() => {
    if (!configuration.voices.some((option) => option.engine === voice)) voice = configuration.defaultVoice;
  });

  $effect(() => {
    if (!editor?.item.wait) return;
    const timer = setTimeout(() => {
      void refreshService().catch((err) => toast(err.message, true));
    }, 3000);
    return () => clearTimeout(timer);
  });

  async function refreshCatalog(fresh = false) {
    const pending = Promise.all([plCatalog(fresh), PL.data || plLoad()]);
    sync();
    await pending;
    sync();
  }

  function changeStep(next) {
    step = next;
    if (card) card.scrollTop = 0;
  }

  function pick(event, id) {
    if (event.currentTarget.checked) picked.add(id);
    else picked.delete(id);
    failed.delete(id);
  }

  async function install() {
    const catalog = new Map(available.map((item) => [item.id, item]));
    const ids = [...picked].filter((id) => !installed.has(id) && catalog.has(id));
    if (!ids.length) return true;
    for (const id of ids) {
      failed.delete(id);
      installing = id;
      try {
        await api("POST", "/api/plugins/install", { repo: catalog.get(id).repo || S.repo, id });
        picked.delete(id);
        added.add(id);
      } catch (err) {
        failed.set(id, err.message);
      }
    }
    installing = "";
    await Promise.all([reload(), plLoad()]);
    sync();
    added.clear();
    return !ids.some((id) => failed.has(id));
  }

  async function finish() {
    await api("POST", "/api/onboarded");
    S.settings.onboarded = true;
    visible = showReturn = false;
    editor = null;
  }

  async function run(action) {
    if (busy) return;
    error = "";
    busy = true;
    try { await action(); }
    catch (err) { error = err.message; }
    finally { busy = false; }
  }

  function next() {
    return run(async () => {
      if (step === 3) return finish();
      if (step === 1 && !await install()) {
        error = "Часть плагинов не установилась. Можно повторить или продолжить с теми, что уже поставлены.";
        return;
      }
      if (step === 2) {
        const profile = S.profiles?.find((item) => item.id === S.active);
        if (voice && profile && profile.engine !== voice) {
          await api("PUT", "/api/profile", { ...profile, engine: voice, mode: "live", voice: "", live_model: "" });
          await reload();
          sync();
        }
      }
      changeStep(step + 1);
      if (step === 1) void refreshCatalog();
    });
  }

  async function refreshService() {
    const id = serviceId;
    await reload();
    sync();
    if (serviceId === id && id) editor = getService(serviceIds[id] || id);
  }

  async function openConnection(id) {
    if (id === "training") {
      visible = false;
      showReturn = true;
      await goto("/training");
      return;
    }
    serviceId = id;
    returnScroll = card.scrollTop;
    editor = getService(serviceIds[id] || id);
    await tick();
    card.scrollTop = 0;
    serviceTitle?.focus({ preventScroll: true });
  }

  async function leaveService() {
    const id = serviceId;
    serviceId = "";
    editor = null;
    sync();
    await tick();
    card.scrollTop = returnScroll;
    card.querySelector(`[data-wz-conn="${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
  }

  async function returnToWizard() {
    try {
      await Promise.all([reload(), plLoad()]);
      open(2);
    } catch (err) { toast(err.message, true); }
  }
</script>

<svelte:window onkeydown={(event) => { if (visible && serviceId && event.key === "Escape") void leaveService(); }} />

<div id="wiz" class="wiz" class:hidden={!visible} role="dialog" aria-modal="true" aria-label="Первоначальная настройка">
  <div class="wiz-card" bind:this={card}>
    <div class="wz-top">
      <nav class="wz-dots">
        {#each steps as name, index}
          <span class="wz-dot" class:on={step === index} class:done={index < step} aria-current={step === index ? "step" : undefined}>
            <i></i><span>{name}</span>
          </span>
        {/each}
      </nav>
      <button class="btn ghost sm" type="button" data-wz-skip disabled={busy} onclick={() => run(finish)}>Пропустить</button>
    </div>
    <div class="wz-body" class:hidden={serviceId}>
      {#if step === 0}
        <div class="wz-hero"><span class="orb big" data-s="idle"><i></i></span><h1>Привет! Я Джарвис</h1>
          <p>Голосовые движки (ChatGPT, Gemini, OpenAI, Grok, ElevenLabs) уже в ядре. Ниже при желании можно добавить музыку, умный дом, VPN или свои команды.</p>
        </div>
      {:else if step === 1}
        <h1>Выберите плагины</h1>
        <p>Выберите нужные дополнения (умный дом, YouTube, VPN, обучение слова). Голосовые движки уже встроены, а плагины всегда можно добавить или удалить в «Подключениях».</p>
        {#if !available.length && (catalogLoading || !catalogError)}
          <p role="status">Загружаем официальный каталог…</p>
        {:else}
          {#if catalogError}
            <p role="status">Не удалось обновить каталог: {catalogError}</p>
            <button class="btn" type="button" data-wz-retry disabled={busy} onclick={() => refreshCatalog(true)}>Обновить каталог</button>
          {/if}
          {#each [...groups, ["other", "Другие плагины"]] as [category, name] (category)}
            {@const items = available.filter((item) => category === "other" ? !groups.some(([id]) => id === item.category) : item.category === category)}
            {#if items.length}
              <h3>{name}</h3>
              <div class="wz-list">
                {#each items as item (item.id)}
                  {@const current = installed.get(item.id)}
                  {@const status = installing === item.id ? "устанавливается…" : current?.running || added.has(item.id) ? "установлен" : current ? "выключен" : ""}
                  <label class="wz-ai" class:on={current || added.has(item.id)} data-wz-row={item.id}>
                    {@html plIcon(item.icon)}
                    <div><b>{item.name}</b><span>{item.description}</span>
                      <span class="err" class:hidden={!failed.has(item.id)} data-wz-row-error role="alert">{failed.get(item.id) || ""}</span>
                    </div>
                    <span class="st" class:hidden={!status} data-wz-row-status role="status">{status}</span>
                    {#if !current && !added.has(item.id)}
                      <span class="sw" class:hidden={status} data-wz-switch>
                        <input type="checkbox" data-wz-pick={item.id} checked={picked.has(item.id)} disabled={busy}
                          aria-label={item.name} onchange={(event) => pick(event, item.id)}><span class="tr"></span>
                      </span>
                    {/if}
                  </label>
                {/each}
              </div>
            {/if}
          {/each}
        {/if}
      {:else if step === 2}
        <h1>Настройте подключения</h1>
        <p>Откройте нужный сервис и следуйте инструкции в его настройках. Если потребуется VPN, начните с него.</p>
        {#if configuration.connections.length}
          <div class="wz-list">
            {#each configuration.connections as plugin (plugin.id)}
              <div class="wz-ai wz-connection" class:on={plugin.ready}>
                {@html plIcon(plugin.icon)}
                <div><b>{plugin.name}</b><span>{plugin.description || plDescription(plugin)}</span>
                  {#if plugin.ready}<span class="st ok">Настроено</span>{/if}
                </div>
                <button class="btn sm" type="button" data-wz-conn={plugin.id} disabled={busy}
                  onclick={() => openConnection(plugin.id)}>{plugin.ready ? "Открыть" : "Настроить"}</button>
              </div>
            {/each}
          </div>
        {:else}
          <p class="muted">Подключения можно добавить позже. Чтобы Джарвис отвечал голосом, настройте один из голосовых движков.</p>
        {/if}
        {#if configuration.voices.length}
          <div class="f"><label for="wz-voice">Кто будет отвечать</label>
            <select id="wz-voice" bind:value={voice}>
              {#each configuration.voices as option (option.id)}<option value={option.engine}>{option.name}</option>{/each}
            </select>
          </div>
        {/if}
      {:else}
        <div class="wz-hero"><span class="orb big" data-s="idle"><i></i></span><h1>Готово</h1>
          <p>Если настроили голосовой движок, скажите <b>«Джарвис, привет»</b>. Добавить и удалить плагины можно в «Подключениях».</p>
          <a class="btn" href="https://github.com/newfpv/YaJavis" target="_blank" rel="noopener noreferrer">{@html icon("ext")}Поставить звезду на GitHub</a>
          <p class="muted">Мастер можно открыть снова в разделе «Безопасность».</p>
        </div>
      {/if}
      <p class="err" class:hidden={!error} data-wz-error role="alert">{error}</p>
    </div>
    <div class="wz-foot" class:hidden={serviceId}>
      {#if step > 0}
        <button class="btn ghost" type="button" data-wz-back disabled={busy} onclick={() => { error = ""; changeStep(step - 1); }}>Назад</button>
      {/if}
      <span class="spacer"></span>
      {#if step === 1}
        <button class="btn ghost" class:hidden={!selected || busy} type="button" data-wz-later disabled={busy}
          onclick={() => { error = ""; changeStep(2); }}>Продолжить без них</button>
      {/if}
      <button class="btn primary" type="button" data-wz-next disabled={busy} onclick={next}>
        {busy && step === 1 ? "Устанавливаем…" : step === 3 ? "Начать" : selected ? "Установить выбранные" : "Дальше"}
      </button>
    </div>
    {#if serviceId}
      <section class="wz-body">
        {#if editor}
          <h1 id="wz-service-title" tabindex="-1" bind:this={serviceTitle}>{editor.item.name}</h1>
          <span class={["st", editor.status.cls]}>{editor.status.text}</span>
          <ServiceEditor service={editor.item} state={editor.state} refresh={refreshService} />
        {:else}
          <p>Настройки этого подключения недоступны.</p>
        {/if}
      </section>
      <div class="wz-foot"><button class="btn" type="button" onclick={leaveService}>К подключениям</button></div>
    {/if}
  </div>
</div>

{#if showReturn}
  <button class="btn primary wiz-pill" type="button" id="wiz-pill" onclick={returnToWizard}>{@html icon("right")}Вернуться к настройке</button>
{/if}
