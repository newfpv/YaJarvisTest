<script>
  import { icon } from "#helpers/core.js";
  let { warning, close } = $props();
  let remember = $state(false);
</script>

<div class="cn-mh">
  <span class="char-av cn-av warn">{@html icon("shield")}</span>
  <div class="char-n"><h2 id="cn-mt">Неофициальный плагин</h2><span class="st warn">{warning.name}</span></div>
  <button class="ib" type="button" data-close aria-label="Закрыть" onclick={close}>{@html icon("x")}</button>
</div>
<p>Этот плагин написал не автор YaJarvis, и никто его не проверял. Он будет работать на вашей колонке и сможет ходить в интернет,
  а нативный плагин — вообще всё, что может программа на колонке.</p>
<p class="muted">Автор YaJarvis не отвечает за сторонние плагины и их последствия. Ставьте только то, чему доверяете: лучше — с открытым кодом и от известного автора.</p>
<label class="sw"><input type="checkbox" id="pl-warn-ok" bind:checked={remember}><span class="tr"></span>Больше не показывать</label>
<div class="row cn-foot">
  <span class="spacer"></span><button class="btn ghost" type="button" data-close onclick={close}>Отмена</button>
  <button class="btn primary" type="button" id="pl-warn-yes" onclick={() => warning.confirm(remember)}>Понимаю, установить</button>
</div>
