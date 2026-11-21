<script>
  import { refreshConnectionCatalog, updateConnectionPlugins, uploadConnectionPlugin } from "#state/connections.js";
  import { icon } from "#helpers/core.js";
  import PluginRow from "./PluginRow/PluginRow.svelte";

  let { catalog } = $props();
  let installed = $derived(catalog.plugins.filter((plugin) => plugin.x.inst));
  let installedIds = $derived(new Set(installed.map((plugin) => plugin.id)));
  let official = $derived(catalog.plugins.filter((plugin) =>
    !plugin.x.inst && plugin.x.official && !installedIds.has(plugin.id)));
  let community = $derived(catalog.plugins.filter((plugin) => !plugin.x.inst && !plugin.x.official));
  let filePicker = $state();
  let catalogBusy = $state(false);
  let updating = $state(false);
  let uploading = $state(false);

  async function refresh() {
    if (catalogBusy) return;
    catalogBusy = true;
    try { await refreshConnectionCatalog(); }
    finally { catalogBusy = false; }
  }

  async function update() {
    if (updating) return;
    updating = true;
    try { await updateConnectionPlugins(); }
    finally { updating = false; }
  }

  async function importFile(event) {
    const input = event.currentTarget;
    const file = input.files?.[0];
    input.value = "";
    if (!file || uploading) return;
    uploading = true;
    try { await uploadConnectionPlugin(file); }
    finally { uploading = false; }
  }
</script>

{#snippet rows(plugins)}
  <div class="card cx-rows">
    {#each plugins as plugin (plugin.key)}
      <PluginRow {plugin} />
    {/each}
  </div>
{/snippet}

<section class="cn-group">
  <div class="cn-gh">
    <h2>Установлены</h2><span class="spacer"></span>
    <button class="btn sm" id="pl-update-all" type="button" disabled={updating || !catalog.canUpdate} onclick={update}>
      {updating ? "Обновляю плагины…" : "Обновить всё"}
    </button>
  </div>
  {#if installed.length}
    {@render rows(installed)}
  {:else}
    <div class="card plg-empty">
      {@html icon("puzzle")}
      <p class="muted">Пока ни одного плагина. Ядро Джарвиса лёгкое: нужные умения ставятся отдельно — выберите ниже.</p>
    </div>
  {/if}
</section>

<section class="cn-group">
  <div class="cn-gh"><h2>Можно поставить</h2><span class="meta">официальные, от автора YaJarvis</span></div>
  {#if official.length}
    {@render rows(official)}
  {:else}
    <p class="note">{catalog.officialReady ? "Все официальные плагины уже стоят."
      : catalog.officialError ? "Официальный каталог пока недоступен." : "Загружаю каталог…"}</p>
  {/if}
  {#if catalog.officialError}
    <p class="note bad" role="status">Не удалось обновить официальный каталог: {catalog.officialError}</p>
  {/if}
</section>

<section class="cn-group">
  <div class="cn-gh"><h2>От сообщества</h2><span class="meta">их пишут другие люди</span></div>
  <p class="note cx-warn">
    {@html icon("shield")}
    <span>Автор YaJarvis сторонние плагины не проверяет: ставьте то, чему доверяете. Хотите сделать свой —
      <a class="lnk" href={"https://github.com/" + catalog.repo + "/blob/main/docs/PLUGINS.md"} target="_blank" rel="noopener">инструкция</a>.</span>
  </p>
  {#if community.length}
    {@render rows(community)}
  {:else if catalog.communityEmpty}
    <p class="note">Пока никто не опубликовал свой плагин.</p>
  {/if}
  {#if catalog.communityError}
    <p class="note bad" role="status">Не удалось обновить каталог сообщества: {catalog.communityError}</p>
  {/if}
</section>

<div class="cx-foot">
  <span>{catalog.freeSpace ? `Свободно на колонке ${catalog.freeSpace} МБ` : ""}</span>
  <span class="spacer"></span>
  <input type="file" id="pl-file" bind:this={filePicker} accept=".zip,application/zip" hidden onchange={importFile}>
  <button class="btn sm" type="button" disabled={uploading} onclick={() => filePicker.click()}
    data-tip="Архив плагина .zip — например, скачанный из его релиза">
    {@html icon("upload")}{uploading ? "Устанавливаю…" : "Плагин из файла"}
  </button>
  <button class="btn sm ghost" type="button" id="pl-refresh" disabled={catalogBusy} onclick={refresh}>
    {@html icon("search")}Обновить каталог
  </button>
</div>
