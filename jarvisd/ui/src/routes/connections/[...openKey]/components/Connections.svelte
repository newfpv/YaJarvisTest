<script>
  import { onMount } from "svelte";
  import {
    bindConnections, closeConnectionDialog, connNetTest, connOpen, CONN_GROUPS, selectConnectionTab,
  } from "#state/connections.js";
  import { icon } from "#helpers/core.js";
  import { plIcon } from "#state/plugins.js";
  import PluginSettings from "./ConnectionDialog/PluginSettings.svelte";
  import PluginWarning from "./ConnectionDialog/PluginWarning.svelte";
  import PluginCatalog from "./PluginCatalog/PluginCatalog.svelte";
  import ServiceEditor from "#components/ServiceEditor/ServiceEditor.svelte";
  import styles from "./Connections.module.css";

  const tabs = [["svc", "Сервисы"], ["pl", "Плагины"]];
  let { openKey } = $props();
  let modal = $state();
  let dialog = $state.raw(null);
  let phase = $state("hidden");
  let catalog = $state.raw(null);
  let counts = $derived({
    svc: catalog?.services.filter((service) => service.on).length,
    pl: catalog?.plugins.filter((plugin) => plugin.x.inst).length,
  });

  onMount(() => bindConnections(modal,
    (next) => { catalog = next; },
    (next) => { dialog = next; },
    (next) => { phase = next; },
    openKey,
  ));

  function navigateTabs(event, current) {
    const key = event.key;
    let tab;
    if (key === "Home") tab = "svc";
    else if (key === "End") tab = "pl";
    else if (key === "ArrowLeft" || key === "ArrowRight") tab = current === "svc" ? "pl" : "svc";
    else return;
    event.preventDefault();
    selectConnectionTab(tab);
    event.currentTarget.parentElement.querySelector(`[data-tab="${tab}"]`).focus();
  }
</script>

<header class="ph ph-row">
  <div>
    <h1>Подключения</h1>
    <p class="sub">Сервисы — ключи и входы в аккаунты. Плагины — умения, которые ставятся на колонку отдельно</p>
  </div>
  <div class="actions">
    <button class="btn" type="button" data-net onclick={(event) => connNetTest(event.currentTarget)}
      data-tip="Колонка достучится до облаков с вашими ключами и покажет время ответа">
      {@html icon("radio")}Проверить связь
    </button>
  </div>
</header>

{#if catalog}
  <div class="seg cx-tabs {styles.tabs}" id="cx-tabs" role="tablist" aria-label="Подключения">
    {#each tabs as [id, name] (id)}
      <button type="button" role="tab" id={"cx-tab-" + id} data-tab={id} class:on={catalog.tab === id}
        aria-selected={catalog.tab === id} aria-controls="conns" tabindex={catalog.tab === id ? 0 : -1}
        onclick={() => selectConnectionTab(id)} onkeydown={(event) => navigateTabs(event, id)}>
        {name}<small>{counts[id] || ""}</small>
      </button>
    {/each}
  </div>
  <div id="conns" class="cx-list" role="tabpanel" aria-labelledby={"cx-tab-" + catalog.tab}>
    {#if catalog.tab === "svc"}
      {#each CONN_GROUPS as [group, title, description] (group)}
        {@const services = catalog.services.filter((service) => service.group === group)
          .sort((a, b) => Number(b.on) - Number(a.on))}
        {@const connected = services.filter((service) => service.on).length}
        {#if services.length}
          <section class="cn-group">
            <div class="cn-gh">
              <h2>{title}</h2>
              {#if connected || description}
                <span class="meta">{connected ? `подключено ${connected} из ${services.length}` : description}</span>
              {/if}
            </div>
            <div class="cn-grid">
              {#each services as service (service.key)}
                <article class="card cn" class:on={service.on} data-item={service.key}>
                  <div class="cn-h">
                    <span class="char-av cn-av">
                      {#if service.icon}{@html icon(service.icon)}{:else}{service.mono}{/if}
                    </span>
                    <h2>{service.name}</h2>
                  </div>
                  <p class="cn-t">{service.about}</p>
                  <div class="cn-f">
                    <span class="st {service.status.cls}">{service.status.text}</span>
                    <span class="spacer"></span>
                    <span class="char-edit">{@html icon(service.on ? "tune" : "plus")}{service.on ? "Настроить" : "Подключить"}</span>
                  </div>
                  <button type="button" class={styles.serviceAction} aria-label={service.name}
                    onclick={() => connOpen(service.key)}></button>
                </article>
              {/each}
            </div>
          </section>
        {/if}
      {/each}
    {:else}
      <PluginCatalog {catalog} />
    {/if}
  </div>
{/if}

<div class="modal" class:hidden={phase === "hidden"} class:in={phase === "in"} class:out={phase === "out"}
  id="cn-modal" role="dialog" aria-modal="true" aria-labelledby="cn-mt" bind:this={modal}>
  <div class="modal-bg" role="presentation" data-close onclick={closeConnectionDialog}></div>
  <div class="modal-card cn-card" id="cn-card">
    {#if dialog?.warning}
      <PluginWarning warning={dialog.warning} close={closeConnectionDialog} />
    {:else if dialog?.item}
      {@const item = dialog.item}
      {#key item.key}
        <div class="cn-mh">
          <span class="char-av cn-av">
            {#if item.kind === "pl"}{@html plIcon(item.icon)}{:else if item.icon}{@html icon(item.icon)}{:else}{item.mono}{/if}
          </span>
          <div class="char-n"><h2 id="cn-mt">{item.x?.inst?.name || item.name}</h2><span class={["st", dialog.status.cls]}>{dialog.status.text}</span></div>
          <button class="ib" type="button" data-close aria-label="Закрыть" onclick={closeConnectionDialog}>{@html icon("x")}</button>
        </div>
        {#if item.kind === "svc"}
          <ServiceEditor service={item} state={dialog.state} refresh={dialog.refresh} />
        {:else}
          <PluginSettings plugin={item.x} services={dialog.services}
            training={item.id === "training" ? dialog.state.settings.wake_training : undefined} />
        {/if}
      {/key}
    {/if}
  </div>
</div>
