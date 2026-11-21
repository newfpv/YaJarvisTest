<script>
  import { page } from "$app/state";
  import Connections from "./components/Connections.svelte";

  let openKey = $derived(page.params.openKey || "");
</script>

{#key openKey}
  <Connections {openKey} />
{/key}
