<script>
  import { onMount } from "svelte";
  import { api, icon, plural, tip, toast } from "#helpers/core.js";
  import { reload } from "#state/panelState.js";
  import Archive from "./Archive/Archive.svelte";
  import styles from "./Memory.module.css";

  const providers = [
    ["chatgpt", "Подписка ChatGPT"],
    ["gemini", "Google Gemini — ключ"],
    ["openai", "OpenAI — ключ, платно"],
  ];
  const dateFormat = new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "long", year: "numeric" });
  const summaryFormat = new Intl.DateTimeFormat("ru-RU", {
    day: "numeric", month: "long", hour: "2-digit", minute: "2-digit",
  });
  const controller = new AbortController();
  const { signal } = controller;

  let data = $state.raw(null);
  let loadError = $state("");
  let busy = $state(false);
  let archiveView = $state();
  let editor = $state.raw(null);
  let draft = $state("");
  let newFact = $state("");
  let memory = $derived(data?.memory || {});
  let settings = $derived(data?.settings || {});
  let facts = $derived((memory.profile || "").split("\n").map((line) => line.trim())
    .filter(Boolean).map((line) => line.replace(/^[-•*]\s*/, "")));
  let chronicle = $derived((memory.chronicle || []).slice().reverse());
  let archiveCount = $derived(memory.archive || 0);
  let provider = $derived(settings.provider || "");
  let available = $derived(settings.has || {});
  let modelProvider = $derived(available[provider] ? provider : settings.active);

  onMount(() => {
    loadMemory().catch((error) => {
      if (!signal.aborted) loadError = error.message;
    });
    return () => controller.abort();
  });

  async function loadMemory() {
    data = await api("GET", "/api/memory", undefined, { signal });
  }

  async function run(action) {
    if (busy) return;
    busy = true;
    try {
      await action();
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      busy = false;
    }
  }

  function edit(kind, text, key) {
    editor = { kind, key };
    draft = text;
  }

  function focus(input) {
    input.focus();
  }

  function cancelEdit() {
    if (!busy) editor = null;
  }

  function save(path, body, message, afterSave) {
    void run(async () => {
      const next = await api("PUT", path, body);
      if (signal.aborted) return;
      data = next;
      editor = null;
      afterSave?.();
      toast(message);
    });
  }

  function saveFacts(list, message, afterSave) {
    const profile = list.map((fact) => fact.trim()).filter(Boolean).map((fact) => "- " + fact).join("\n");
    save("/api/memory", { profile }, message, afterSave);
  }

  function addFact(event) {
    event.preventDefault();
    if (!newFact.trim()) return toast("Напишите, что Джарвису стоит знать", true);
    saveFacts([...facts, newFact], "Запомнил", () => newFact = "");
  }

  function saveFact(event, index) {
    event.preventDefault();
    const next = facts.slice();
    if (draft.trim()) next[index] = draft;
    else next.splice(index, 1);
    saveFacts(next, "Сохранено");
  }

  function saveSummary(event) {
    event.preventDefault();
    save("/api/memory", { summary: draft }, "Конспект сохранён");
  }

  function saveMonth(event, month) {
    event.preventDefault();
    if (!draft.trim()) return toast("Пустой месяц — удалите его кнопкой с корзиной", true);
    save("/api/memory/month", { month, text: draft.trim() }, "Летопись сохранена");
  }

  function deleteMonth(event, month) {
    event.preventDefault();
    if (!confirm("Удалить этот месяц из летописи? Джарвис забудет, о чём тогда говорили.")) return;
    save("/api/memory/month", { month, text: "" }, "Месяц удалён");
  }

  function changeSettings(changes, message = "Сохранено") {
    const next = { off: !!settings.off, provider, model: settings.model || "", ...changes };
    void run(async () => {
      await api("PUT", "/api/settings", { memory: next });
      if (signal.aborted) return;
      await reload();
      await loadMemory();
      toast(message);
    });
  }

  function changeEnabled(event) {
    const input = event.currentTarget;
    const off = !input.checked;
    input.checked = !settings.off;
    changeSettings({ off }, off ? "Память выключена — каждый разговор с чистого листа" : "Джарвис снова запоминает разговоры");
  }

  function changeProvider(event) {
    const input = event.currentTarget;
    const value = input.value;
    input.value = provider;
    changeSettings({ provider: value, model: "" });
  }

  function changeVersion(event) {
    const input = event.currentTarget;
    const model = input.value;
    input.value = settings.active_model ?? "";
    changeSettings({ model });
  }

  function providerName(id) {
    return providers.find(([key]) => key === id)?.[1] || id;
  }

  function versionName(version) {
    return version || "по очереди: кончился лимит — следующая";
  }

  function compact() {
    void run(async () => {
      const result = await api("POST", "/api/memory/compact");
      if (signal.aborted) return;
      await loadMemory();
      await archiveView.refresh();
      if (!signal.aborted) toast(result.turns ? "Досье и конспект обновлены" : "Сжимать пока нечего");
    });
  }

  function resetChat(all) {
    const prompt = all
      ? "Стереть всю память Джарвиса — досье о вас, конспект, летопись и архив разговоров? Это не отменить."
      : "Начать новый чат? Джарвис забудет конспект и недавние реплики. Досье о вас, летопись и архив останутся.";
    if (!confirm(prompt)) return;
    void run(async () => {
      await api("POST", all ? "/api/chatgpt/newchat?all=1" : "/api/chatgpt/newchat");
      if (signal.aborted) return;
      editor = null;
      await loadMemory();
      await archiveView.refresh(all);
      if (!signal.aborted) toast(all ? "Память стёрта" : "Новый чат начат");
    });
  }
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape") cancelEdit(); }} />

<header class="ph ph-row">
  <div>
    <h1>Память</h1>
    <p class="sub">Что Джарвис помнит о вас — общее для всех ассистентов. Неверное поправьте, лишнее удалите</p>
  </div>
  {#if data}
    <div class="actions">
      <label class={["sw", styles.toggle]}>
        <input type="checkbox" checked={!settings.off} disabled={busy} onchange={changeEnabled}>
        <span class="tr"></span>
        Запоминать разговоры
      </label>
    </div>
  {/if}
</header>

{#snippet stat(count, label, since = "")}
  <div class={styles.stat}>
    <b>{count}</b><span>{label}</span>
    {#if since}<small>{since}</small>{/if}
  </div>
{/snippet}

{#snippet editorActions(small = false)}
  <div class="row">
    <span class="spacer"></span>
    <button class={["btn ghost", { sm: small }]} type="button" disabled={busy} onclick={cancelEdit}>Отмена</button>
    <button class={["btn primary", { sm: small }]} type="submit" disabled={busy}>Сохранить</button>
  </div>
{/snippet}

{#if loadError}
  <p class="err" role="alert">{loadError}</p>
{:else if !data}
  <div class="card plg-empty" role="status"><span class="spin"></span><p class="muted">Загружаю память…</p></div>
{:else}
  <div class={styles.memory}>
    {#if settings.off}
      <p class={["note", styles.off]}>
        {@html icon("moon")}
        <span>Память выключена: каждый разговор начинается с чистого листа. То, что уже записано, осталось — его можно смотреть и править.</span>
      </p>
    {/if}

    <div class={styles.stats}>
      {@render stat(facts.length, plural(facts.length, "факт о вас", "факта о вас", "фактов о вас"))}
      {@render stat(archiveCount, plural(archiveCount, "реплика в архиве", "реплики в архиве", "реплик в архиве"),
        memory.archive_since ? "с " + dateFormat.format(memory.archive_since * 1000) : "")}
      {@render stat(chronicle.length, plural(chronicle.length, "месяц в летописи", "месяца в летописи", "месяцев в летописи"))}
      {@render stat(memory.turns || 0, "в текущем чате")}
    </div>

    <section class={["card", styles.section]}>
      <div class={styles.heading}>
        <div>
          <h2>{@html icon("read")}Что Джарвис знает о вас</h2>
          <p class="muted">Досье: он видит его в начале каждого разговора. Пополняется само, когда разговор сжимается</p>
        </div>
      </div>
      {#if facts.length}
        <ul class={styles.facts}>
          {#each facts as fact, index}
            <li>
              {#if editor?.kind === "fact" && editor.key === index}
                <form class={[styles.fact, styles.editFact]} onsubmit={(event) => saveFact(event, index)}>
                  <input type="text" aria-label="Факт" bind:value={draft} use:focus disabled={busy}>
                  <button class="btn sm primary" type="submit" disabled={busy}>Сохранить</button>
                  <button class="btn sm ghost" type="button" onclick={cancelEdit} disabled={busy}>Отмена</button>
                </form>
              {:else}
                <div class={styles.fact}>
                  <span>{fact}</span>
                  <button class="ib" type="button" aria-label="Изменить" onclick={() => edit("fact", fact, index)}
                    disabled={busy}>{@html icon("edit")}</button>
                  <button class="ib danger" type="button" aria-label="Удалить"
                    disabled={busy} onclick={() => saveFacts(facts.filter((_, item) => item !== index), "Факт удалён")}>{@html icon("trash")}</button>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class={styles.empty}>Пока пусто. {archiveCount ? "Заполнится, когда разговор сожмётся, — или добавьте сами." : "Расскажите Джарвису о себе или добавьте сами."}</p>
      {/if}
      <form class={styles.add} onsubmit={addFact}>
        <input type="text" aria-label="Новый факт" placeholder="Добавить: например, «не ем мясо» или «дочь зовут Аня»"
          maxlength="300" bind:value={newFact} disabled={busy}>
        <button class="btn" type="submit" disabled={busy}>{@html icon("plus")}Добавить</button>
      </form>
    </section>

    <section class={["card", styles.section]}>
      <div class={styles.heading}>
        <div>
          <h2>{@html icon("todo")}Конспект недавних разговоров</h2>
          <p class="muted">{memory.summary_at ? "Обновлён " + summaryFormat.format(memory.summary_at * 1000) : "Кто что сказал и когда — сжато"}</p>
        </div>
        <div class="row">
          {#if !settings.off && settings.active && archiveCount}
            <button class="btn sm" type="button" disabled={busy} onclick={compact}
              data-tip="Сжать разговор в досье и конспект сейчас, не дожидаясь длинного разговора">Обновить сейчас</button>
          {/if}
          {#if memory.summary && editor?.kind !== "summary"}
            <button class="btn sm ghost" type="button" disabled={busy}
              onclick={() => edit("summary", memory.summary)}>{@html icon("edit")}Изменить</button>
          {/if}
        </div>
      </div>
      {#if editor?.kind === "summary"}
        <form class={styles.editText} onsubmit={saveSummary}>
          <textarea rows="10" aria-label="Конспект" bind:value={draft} use:focus disabled={busy}></textarea>
          {@render editorActions()}
        </form>
      {:else if memory.summary}
        <div class={styles.text}>{memory.summary}</div>
      {:else}
        <p class={styles.empty}>Пока пусто: появится после первого длинного разговора.</p>
      {/if}
    </section>

    {#if chronicle.length}
      <section class={["card", styles.section]}>
        <div class={styles.heading}>
          <div>
            <h2>{@html icon("moon")}Летопись</h2>
            <p class="muted">Разговоры старше архива, сжатые по месяцам</p>
          </div>
        </div>
        <div class={styles.months}>
          {#each chronicle as item (item.month)}
            {#if editor?.kind === "month" && editor.key === item.month}
              <form class={[styles.month, styles.editMonth]} onsubmit={(event) => saveMonth(event, item.month)}>
                <div class={styles.monthHeading}><b>{item.label}</b></div>
                <div class={styles.editText}>
                  <textarea rows="8" aria-label="Текст летописи" bind:value={draft} use:focus disabled={busy}></textarea>
                  {@render editorActions(true)}
                </div>
              </form>
            {:else}
              <details class={styles.month}>
                <summary>
                  <b>{item.label}</b><small>{item.turns} {plural(item.turns, "реплика", "реплики", "реплик")}</small>
                  <button class={["ib", styles.push]} type="button" aria-label="Изменить" disabled={busy}
                    onclick={(event) => { event.preventDefault(); edit("month", item.text, item.month); }}>{@html icon("edit")}</button>
                  <button class="ib danger" type="button" aria-label="Удалить" disabled={busy}
                    onclick={(event) => deleteMonth(event, item.month)}>{@html icon("trash")}</button>
                </summary>
                <div class={styles.text}>{item.text}</div>
              </details>
            {/if}
          {/each}
        </div>
      </section>
    {/if}

    <Archive bind:this={archiveView} {busy} {run} onforget={loadMemory} />

    {#if !settings.off}
      <section class={["card", styles.section]}>
        <div class={styles.heading}>
          <div>
            <h2>{@html icon("tune")}Кто сжимает разговоры</h2>
            <p class="muted">Короткий текстовый запрос раз в несколько десятков реплик: старое — в досье и конспект</p>
          </div>
        </div>
        <div class="row">
          <div class="f half">
            <label for="mem-prov">Модель {@html tip("«Сам выбирает» берёт сначала бесплатное: подписку ChatGPT, потом Gemini, потом платный ключ OpenAI. Если один не ответил — следующий.")}</label>
            <select id="mem-prov" value={provider} disabled={busy} onchange={changeProvider}>
              <option value="">Сам выбирает</option>
              {#each providers as [id, name] (id)}
                <option value={id} disabled={!available[id]}>{name}{available[id] ? "" : " — не подключено"}</option>
              {/each}
              <option value="none">Не сжимать</option>
            </select>
          </div>
          {#if modelProvider && provider !== "none"}
            <div class="f half">
              <label for="mem-model">Версия</label>
              <select id="mem-model" value={settings.active_model ?? ""} disabled={busy} onchange={changeVersion}>
                {#each settings.models?.[modelProvider] || [] as model (model)}
                  <option value={model}>{versionName(model)}</option>
                {/each}
              </select>
            </div>
          {/if}
        </div>
        {#if provider === "none"}
          <p class="note">Не сжимается: Джарвис видит только последние реплики, а старое находит в архиве.</p>
        {:else if settings.active}
          <p class="note">Сейчас сжимает: <b>{providerName(settings.active)}</b> · {versionName(settings.active_model)}.{provider && !available[provider] ? " Выбранное не подключено — пока работает автовыбор." : ""}</p>
        {:else}
          <p class="err">Нужна текстовая модель: подписка ChatGPT, ключ Gemini или ключ OpenAI. Пока её нет, старое не сжимается, но архив пишется. <a class="lnk" href="/connections">Подключения</a></p>
        {/if}
      </section>
    {/if}

    <section class={["card", styles.section, styles.danger]}>
      <div class={styles.heading}>
        <div>
          <h2>Начать заново</h2>
          <p class="muted">Новый чат — Джарвис забудет конспект и недавние реплики, досье и архив останутся. Стереть — удалить всё, без возврата</p>
        </div>
      </div>
      <div class="row">
        <button class="btn" type="button" disabled={busy || !(memory.turns || memory.summary)}
          onclick={() => resetChat(false)}>Начать новый чат</button>
        <span class="spacer"></span>
        <button class="btn ghost danger" type="button" disabled={busy || !(archiveCount || facts.length || memory.summary)}
          onclick={() => resetChat(true)}>{@html icon("trash")}Стереть всю память</button>
      </div>
    </section>
  </div>
{/if}
