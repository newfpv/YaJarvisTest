<script>
  import { connNetTest, NET_NAMES, serviceAction, VPN_NAMES } from "#state/connections.js";
  import AccountSettings from "./AccountSettings.svelte";
  import HomeAssistant from "./HomeAssistant.svelte";
  import KeySettings from "./KeySettings.svelte";
  import VpnSwitch from "./VpnSwitch.svelte";

  let { service, state: snapshot, refresh } = $props();
  let id = $derived(service.id);
  let vpn = $derived(snapshot.vless || {});
  let multipleKeys = $derived((id === "gemini" && snapshot.gemini_keys?.length > 1) || (id === "elevenlabs" && snapshot.eleven_keys?.length > 1));
  let logout = $derived({ chatgpt: "cg-out", smartlife: "sl-out", xiaomi: "xm-out", youtube: "ya-out" }[id]);
  let body = $state();
  let homeAssistant = $state();
  let network = $state.raw(null);

  function action(event) {
    return serviceAction(event.currentTarget, body, refresh);
  }

  function keydown(event) {
    if (event.key !== "Enter") return;
    const target = event.currentTarget;
    if (target.id === "sl-code") body.querySelector("#sl-in")?.click();
    else body.querySelector("[data-key]")?.click();
  }
</script>

<p class="cn-about">{service.about}{service.more ? ". " + service.more : ""}</p>
<div class="cn-body" bind:this={body}>
  {#if service.keyField}
    <KeySettings {service} state={snapshot} {action} {keydown} />
  {:else if id === "homeassistant"}
    <HomeAssistant state={snapshot} {refresh} bind:this={homeAssistant} />
  {:else if id === "vpn"}
    {#if vpn.set && vpn.host}
      <div class={["vpn", vpn.error ? "bad" : "ok"]}><b>{vpn.name || vpn.host}</b><small>{vpn.host}:{vpn.port} · {vpn.network} + {vpn.security}</small></div>
    {:else if vpn.set}
      <p class="err">{vpn.error || "VPN запускается…"}</p>
    {:else}
      <p class="note">Пока не задан — колонка ходит в интернет напрямую. Вставьте ссылку vless://… из вашего VPN (3x-ui, Marzban, Hiddify и т. п.).</p>
    {/if}
    {#if vpn.error}<p class="err">{vpn.error}</p>{/if}
    <textarea id="vless" class="code" placeholder="vless://…" spellcheck="false" aria-label="Ссылка VLESS"></textarea>
    <div class="row">
      <button type="button" class="btn primary" id="vless-save" onclick={action}>{vpn.set ? "Заменить ссылку" : "Сохранить ссылку"}</button>
      <span class="spacer"></span><button type="button" class="btn" data-net
        onclick={(event) => connNetTest(event.currentTarget, (result) => { network = result; })}>Проверить связь</button>
    </div>
    <div class="netres" id="net-res">
      {#if network?.pending}
        <div class="nr"><span class="spin"></span>Проверяю…</div>
      {:else if network}
        {#each Object.entries(network.results) as [name, result] (name)}
          <div class={["nr", result.ok ? "ok" : "bad"]}><b>{NET_NAMES[name] || name}</b>{result.ok ? `работает · ${result.ms} мс` : result.error || "нет связи"}</div>
        {/each}
      {/if}
    </div>
    <h3>Что идёт через VPN</h3>
    <div class="vpn-list">
      {#each VPN_NAMES as [serviceId, label] (serviceId)}
        <VpnSwitch id={serviceId} {label} state={snapshot} {action} describe={false} />
      {/each}
    </div>
  {:else}
    <AccountSettings {id} state={snapshot} {action} {keydown} />
  {/if}
</div>

{#snippet drop(buttonId, label, onclick = action)}
  <div class="row cn-foot">
    <span class="spacer"></span>
    <button type="button" class="btn ghost danger" id={buttonId} {onclick}>{label}</button>
  </div>
{/snippet}

{#if service.keyField && service.on}
  <div class="row cn-foot"><span class="spacer"></span>
    <button type="button" class="btn ghost danger" data-clear={id} onclick={action}>{multipleKeys ? "Убрать все ключи" : "Убрать ключ"}</button>
  </div>
{:else if id === "homeassistant" && service.on}
  {@render drop("ha-disconnect", "Отключить", (event) => homeAssistant.disconnect(event))}
{:else if id === "vpn" && vpn.set}
  {@render drop("vless-clear", "Убрать VPN")}
{:else if logout && (id === "youtube" ? snapshot.youtube?.account?.signed_in : snapshot[id]?.signed_in)}
  {@render drop(logout, "Выйти")}
{:else if id === "youtube" && snapshot.youtube?.set}
  {@render drop("yt-clear", "Убрать куки")}
{/if}
