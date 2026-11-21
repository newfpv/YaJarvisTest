<script>
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import Profile from "./components/Profile.svelte";
  import { S } from "#state/panelState.js";

  let id = $derived(page.params.id);
  let isNew = $derived(id === "new");
  let base = $derived(S.profiles.find((profile) => profile.id === id));

  $effect(() => {
    if (!isNew && !base) void goto("/", { replace: true });
  });
</script>

{#key id}
  {#if isNew || base}
    <Profile {base} {isNew} />
  {/if}
{/key}
