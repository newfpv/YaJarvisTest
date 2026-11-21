<script>
  import { untrack } from "svelte";
  import {
    installConnectionPlugin, openRelatedService, removeConnectionPlugin, saveConnectionPlugin,
    setConnectionPluginEnabled, updateConnectionPlugin,
  } from "#state/connections.js";
  import { icon, tip } from "#helpers/core.js";
  import { PL_KIND, plCatItem, plHasUpdate, plSize } from "#state/plugins.js";

  let { plugin, services, training } = $props();
  let installed = $derived(plugin.inst);
  let item = $derived(installed || plugin);
  let fields = $derived(installed?.settings || []);
  let update = $derived(installed && plCatItem(installed));
  let url = $derived(installed ? installed.homepage : plugin.url);
  let metadata = $derived([
    item.version && `v${item.version}`, PL_KIND[item.kind] || item.kind, item.size && plSize(item.size), item.author,
    !installed && plugin.stars ? `★ ${plugin.stars}` : "",
  ].filter(Boolean));
  let draft = $state(untrack(() => Object.fromEntries(fields.map((field) => [field.key, initialValue(field)]))));
  let toggling = $state(false);
  let installing = $state(false);

  function initialValue(field) {
    const value = installed?.values?.[field.key];
    if (field.type === "password") return "";
    if (field.type === "bool") return Boolean(value);
    if (field.type === "select") return field.options?.includes(value) ? value : field.options?.[0] ?? "";
    return value ?? "";
  }

  $effect.pre(() => {
    for (const field of fields) {
      if (!Object.hasOwn(draft, field.key)) draft[field.key] = initialValue(field);
    }
  });

  async function toggle(event) {
    const input = event.currentTarget;
    const next = input.checked;
    input.checked = !next;
    toggling = true;
    try { await setConnectionPluginEnabled(installed.id, next); }
    finally { toggling = false; }
  }

  function save(event) {
    const values = Object.fromEntries(fields.map(({ key, type }) => {
      const value = draft[key];
      return [key, type === "number" ? (value === "" || value == null ? null : Number(value)) : value];
    }));
    return saveConnectionPlugin(installed.id, values, event.currentTarget, () => {
      for (const { key, type } of fields) {
        if (type === "password" && draft[key] === values[key]) draft[key] = "";
      }
    });
  }

  async function install() {
    installing = true;
    try { await installConnectionPlugin(plugin); }
    finally { installing = false; }
  }
</script>

{#snippet details()}
  <div class="plg-meta">
    {#if item.official}<span class="plg-b ok">{@html icon("shield")}официальный</span>
    {:else}<span class="plg-b warn">неофициальный</span>{/if}
    {#each metadata as value}<span>{value}</span>{/each}
    {#if url}<a class="lnk" href={url} target="_blank" rel="noopener">{@html icon("ext")}код</a>{/if}
  </div>
{/snippet}

{#if !installed}
  <p class="cn-about">{plugin.description || "Без описания"}</p>
  {@render details()}
  <div class="row cn-foot"><span class="spacer"></span>
    <button class="btn primary" type="button" data-pl-install={plugin.repo} data-id={plugin.id} data-official={plugin.official ? 1 : ""}
      disabled={installing} onclick={install}>
      {#if installing}<span class="spin"></span>Ставлю…{:else}{@html icon("plus")}Установить{/if}
    </button>
  </div>
{:else}
  <p class="cn-about">{training !== undefined ? "Меняет фразу активации или дообучает «Джарвис» по вашим записям." : plugin.description || ""}</p>
  {#if fields.length}
    <div class="cn-body plg-form">
      {#each fields as field (field.key)}
        {@const value = installed.values?.[field.key]}
        {@const id = "pl-setting-" + field.key}
        {#snippet label()}
          {field.label || field.key}{#if field.hint} {@html tip(field.hint)}{/if}
        {/snippet}
        {#if field.type === "bool"}
          <label class="sw"><input type="checkbox" data-k={field.key} data-type={field.type} bind:checked={draft[field.key]}><span class="tr"></span>{@render label()}</label>
        {:else}
          <div class="f">
            <label for={id}>{@render label()}</label>
            {#if field.type === "select"}
              <select {id} data-k={field.key} data-type={field.type} bind:value={draft[field.key]}>
                {#each field.options || [] as option}<option>{option}</option>{/each}
              </select>
            {:else if field.type === "textarea"}
              <textarea {id} rows="4" data-k={field.key} data-type={field.type} bind:value={draft[field.key]}></textarea>
            {:else if field.type === "password"}
              <input {id} type="password" autocomplete="off" data-k={field.key} data-type={field.type}
                placeholder={value ? `задан ${value} — оставьте пустым, чтобы не менять` : "не задан"} bind:value={draft[field.key]}>
            {:else if field.type === "number"}
              <input {id} type="number" data-k={field.key} data-type={field.type} bind:value={draft[field.key]}>
            {:else}
              <input {id} type="text" data-k={field.key} data-type={field.type} bind:value={draft[field.key]}>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
    <div class="row"><span class="spacer"></span><button class="btn primary" type="button" data-pl-save={installed.id} onclick={save}>Сохранить</button></div>
  {/if}
  {#if services.length}
    <div class="cx-links">
      <b>{services.length > 1 ? "Входы и ключи этого плагина" : "Вход и настройки"}</b>
      {#each services as service (service.id)}
        <button class="cx-link" type="button" data-goto={"svc:" + service.id} onclick={() => openRelatedService(service.id)}>
          <span class="char-av cn-av">{#if service.icon}{@html icon(service.icon)}{:else}{service.mono}{/if}</span>
          <span>{service.name}<small>{service.state}</small></span>{@html icon("right")}
        </button>
      {/each}
    </div>
  {:else if training !== undefined}
    {#if training}<div><a class="btn primary" href="/training">Открыть обучение</a></div>{/if}
  {:else if !fields.length}
    <p class="note">У этого плагина нет настроек: он просто работает. Выключить или удалить его можно ниже.</p>
  {/if}
  <div class="cx-plug">
    <div class="cx-plug-h">
      <span class="cx-plug-n">{@html icon("puzzle")}<b>О плагине</b></span>
      <label class="sw" data-tip={installed.enabled ? "Выключить" : "Включить"}>
        <input type="checkbox" data-pl-on={installed.id} checked={installed.enabled} disabled={toggling}
          aria-label={installed.enabled ? "Выключить" : "Включить"} onchange={toggle}><span class="tr"></span>
      </label>
    </div>
    {@render details()}
    {#if installed.error}<p class="note bad">{installed.error}</p>
    {:else if installed.status && installed.enabled}<p class="note">{installed.status}</p>{/if}
    <div class="row">
      {#if plHasUpdate(installed, update)}
        <button class="btn sm" type="button" data-pl-update={installed.id} onclick={(event) => updateConnectionPlugin(installed.id, event.currentTarget)}>Обновить до {update.version}</button>
      {/if}
      <span class="spacer"></span>
      <button class="btn sm ghost danger" type="button" data-pl-del={installed.id} onclick={(event) => removeConnectionPlugin(installed.id, event.currentTarget)}>Удалить плагин</button>
    </div>
  </div>
{/if}
