<script>
  import TrainingPage from "./components/TrainingPage.svelte";
  import { S } from "#state/panelState.js";
</script>

<TrainingPage enabled={Boolean(S.settings.wake_training)} showRecords={Boolean(S.settings.training_records)} />
