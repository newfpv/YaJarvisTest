<script>
  let { id, state, label = "Через VPN", describe = true, action } = $props();
  let checked = $derived(Boolean(state.vpn?.[id]));
</script>

{#snippet toggle()}
  <label class="sw">
    <input type="checkbox" data-vpn={id} {checked} onchange={action}>
    <span class="tr"></span>{label}
  </label>
{/snippet}

{#if describe}
  <div class="cn-vpn">
    {@render toggle()}
    <small class="muted">{!state.vless?.set ? "VPN не задан — галочка заработает, когда добавите его в «Сеть → VPN»" : checked ? "идёт через ваш VPN" : "напрямую, без VPN"}</small>
  </div>
{:else}
  {@render toggle()}
{/if}
