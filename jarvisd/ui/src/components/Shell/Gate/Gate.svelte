<script>
  import { tick } from "svelte";

  let { access, login } = $props();
  let visible = $derived(access.status === "gate");
  let setup = $derived(Boolean(access.setup));
  let input = $state();
  let password = $state("");
  let confirmation = $state("");
  let error = $state("");
  let pending = $state(false);

  $effect(() => {
    if (access.status === "gate") void tick().then(() => input?.focus());
    else password = confirmation = error = "";
  });

  async function submit(event) {
    event.preventDefault();
    if (pending) return;
    error = "";
    if (setup && password !== confirmation) {
      error = "Пароли не совпадают";
      return;
    }
    pending = true;
    try {
      await login(setup, password);
    } catch (err) {
      error = err.message;
    } finally {
      pending = false;
    }
  }
</script>

<div id="gate" class="gate" class:hidden={!visible}>
  <form id="gate-form" class="gate-card" autocomplete="on" onsubmit={submit}>
    <div class="brand"><span class="logo"></span>Джарвис</div>
    <p id="gate-text" class="gate-text">{setup ? "Придумайте пароль для настроек колонки. Он понадобится, чтобы зайти сюда снова." : "Пароль от настроек колонки"}</p>
    <input id="gate-pass" type="password" autocomplete={setup ? "new-password" : "current-password"}
      placeholder="Пароль" required bind:this={input} bind:value={password}>
    <input id="gate-pass2" type="password" autocomplete="new-password" placeholder="Пароль ещё раз"
      class:hidden={!setup} bind:value={confirmation}>
    <button class="btn primary" id="gate-btn" type="submit" disabled={pending}>{setup ? "Сохранить пароль" : "Войти"}</button>
    <p id="gate-err" class="err">{error}</p>
    <details id="gate-forgot" class="gate-forgot" class:hidden={setup}>
      <summary>Забыли пароль?</summary>
      <p>Нажмите на колонке кнопки громкости по очереди 8 раз: <b>+ − + − + − + −</b>, между нажатиями не дольше полутора секунд. Колонка скажет, что пароль сброшен, — в течение 30 минут обновите эту страницу и задайте новый.</p>
    </details>
  </form>
</div>
