<script>
  let { enabled, showRecords } = $props();
</script>

<header class="ph"><h1>Обучение</h1></header>
{#if enabled}
  <div>
    {#await import("./Training.svelte") then { default: Training }}
      <Training {showRecords} />
    {:catch error}
      <p class="err">{error.message}</p>
    {/await}
  </div>
{:else}
  <p class="note">Включите плагин «Обучение слова» в <a href="/connections/training">подключениях</a>.</p>
{/if}
