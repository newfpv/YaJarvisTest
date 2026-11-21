<script>
  import { onMount } from "svelte";
  import { musicData } from "#state/player.js";
  import PlayerQueue from "./PlayerQueue/PlayerQueue.svelte";
  import NowPlaying from "./NowPlaying/NowPlaying.svelte";

  onMount(() => { void musicData(); });
</script>

<header class="ph">
  <h1>Плеер</h1>
  <p class="sub">Всё, что играет на колонке: YouTube Джарвиса и Яндекс Музыка Алисы</p>
</header>

<NowPlaying />
<div class="cols stagger" id="player-queue">
  <PlayerQueue />
</div>
