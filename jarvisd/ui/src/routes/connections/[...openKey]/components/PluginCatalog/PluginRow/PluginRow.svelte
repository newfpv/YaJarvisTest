<script>
  import { connOpen, installConnectionPlugin, removeConnectionPlugin, setConnectionPluginEnabled } from "#state/connections.js";
  import { icon } from "#helpers/core.js";
  import { plIcon, plSize } from "#state/plugins.js";
  import styles from "./PluginRow.module.css";

  let { plugin } = $props();
  let installed = $derived(plugin.x.inst);
  let busy = $state(false);

  async function run(action) {
    if (busy) return;
    busy = true;
    try { await action(); }
    finally { busy = false; }
  }

  function changeEnabled(event) {
    const input = event.currentTarget;
    const enabled = input.checked;
    input.checked = installed.enabled;
    void run(() => setConnectionPluginEnabled(installed.id, enabled));
  }
</script>

<div class="cx-row {styles.row}" class:add={!installed} class:off={installed && !installed.enabled} data-item={plugin.key}>
  <span class="char-av cn-av">{@html plIcon(plugin.icon)}</span>
  <div class="cx-row-t">
    <button class={styles.title} type="button" onclick={() => connOpen(plugin.key)}
      aria-label={plugin.name + (installed ? " — настроить" : " — подробнее")}>
      <b>
        {plugin.name}
        {#if installed && !plugin.x.official}<span class="plg-b warn">неофициальный</span>{/if}
      </b>
    </button>
    <small>{plugin.about}</small>
  </div>
  {#if installed}
    <span class="st {plugin.status.cls}">{plugin.status.text}</span>
    <div class="cx-acts {styles.actions}">
      <label class="sw" data-tip={installed.enabled ? "Выключить" : "Включить"}>
        <input type="checkbox" data-pl-on={installed.id} checked={installed.enabled} disabled={busy}
          aria-label={(installed.enabled ? "Выключить " : "Включить ") + plugin.name} onchange={changeEnabled}>
        <span class="tr"></span>
      </label>
      <button class="btn sm" type="button" onclick={() => connOpen(plugin.key)}>{@html icon("tune")}Настроить</button>
      <button class="btn sm ghost danger" type="button" disabled={busy}
        aria-label={"Удалить " + plugin.name} onclick={() => run(() => removeConnectionPlugin(installed.id))}>
        {@html icon("trash")}<span>Удалить</span>
      </button>
    </div>
  {:else}
    <span class="cx-size">{plugin.x.size ? plSize(plugin.x.size) : ""}</span>
    <button class="btn sm {styles.install}" type="button" disabled={busy} onclick={() => run(() => installConnectionPlugin(plugin.x))}>
      {#if busy}<span class="spin"></span>Ставлю…{:else}{@html icon("plus")}Установить{/if}
    </button>
  {/if}
</div>
