<script>
  import WakeTraining from "./WakeTraining/WakeTraining.svelte";
  import TrainingRecords from "./TrainingRecords/TrainingRecords.svelte";

  let { showRecords } = $props();
</script>

<div id="wake-training"><WakeTraining /></div>
{#if showRecords}
  <div id="training-records"><TrainingRecords /></div>
{/if}
