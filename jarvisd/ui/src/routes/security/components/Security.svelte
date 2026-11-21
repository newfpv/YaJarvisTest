<script>
  import { act, api, icon, tip } from "#helpers/core.js";
  import { S, reload } from "#state/panelState.js";
  import { wizOpen } from "#state/wizard.js";
  import Updates from "./Updates/Updates.svelte";

  let currentPassword = $state("");
  let newPassword = $state("");
  let changingPassword = $state(false);
  let lanOnly = $state(!!S.settings.lan_only);
  let changingAccess = $state(false);
  let checks = $state.raw(S.security);

  async function changePassword(event) {
    event.preventDefault();
    if (changingPassword) return;
    changingPassword = true;
    await act(null, async () => {
      await api("PUT", "/api/password", { old: currentPassword, new: newPassword });
      currentPassword = "";
      newPassword = "";
    }, "Пароль сменён");
    changingPassword = false;
  }

  async function changeAccess(event) {
    const input = event.currentTarget;
    const allowedOnlyFromLan = input.checked;
    if (!allowedOnlyFromLan && !confirm("Разрешить вход в настройки не только из домашней сети?")) {
      input.checked = lanOnly;
      return;
    }
    changingAccess = true;
    await act(null, async () => {
      await api("PUT", "/api/settings", { lan_only: allowedOnlyFromLan });
      lanOnly = allowedOnlyFromLan;
      await reload();
      checks = S.security;
    });
    input.checked = lanOnly;
    changingAccess = false;
  }
</script>

<header class="ph">
  <h1>Безопасность</h1>
  <p class="sub">Кто может попасть в колонку и её настройки</p>
</header>

<section class="sec">
  <div class="sec-h">
    <h2>Пароль настроек</h2>
    <p>После смены все остальные браузеры выйдут из настроек</p>
  </div>
  <form class="sec-b" onsubmit={changePassword}>
    <div class="grid g2">
      <div class="f">
        <label for="pw-old">Текущий</label>
        <input type="password" id="pw-old" autocomplete="current-password" bind:value={currentPassword} disabled={changingPassword}>
      </div>
      <div class="f">
        <label for="pw-new">Новый {@html tip("Минимум 6 символов.")}</label>
        <input type="password" id="pw-new" autocomplete="new-password" bind:value={newPassword} disabled={changingPassword}>
      </div>
    </div>
    <div><button class="btn" type="submit" disabled={changingPassword}>Сменить пароль</button></div>
  </form>
</section>

<section class="sec">
  <div class="sec-h">
    <h2>Доступ к настройкам</h2>
    <p>После 5 неверных паролей вход временно блокируется. Ключи и ссылка VLESS не уходят обратно в браузер</p>
  </div>
  <div class="sec-b">
    <label class="sw">
      <input type="checkbox" checked={lanOnly} disabled={changingAccess} onchange={changeAccess}>
      <span class="tr"></span>
      Только из домашней сети
      {@html tip("Настройки открываются только с устройств в вашей Wi-Fi сети. Выключайте, только если точно понимаете зачем (например, доступ через свой VPN).")}
    </label>
  </div>
</section>

<section class="sec">
  <div class="sec-h">
    <h2>Что открыто на колонке</h2>
    <p>Служебные входы, через которые в колонку можно попасть из домашней сети</p>
  </div>
  <div class="sec-b">
    <div class="checks">
      {#each checks as check}
        <div class="check">
          <div class="body">
            <b>{check.title}</b>
            <small>{check.hint}</small>
          </div>
          <span class={["st", check.open ? "warn" : "ok"]}>{check.open ? "открыт" : "закрыт"}</span>
        </div>
      {:else}
        <p class="note">Проверять нечего</p>
      {/each}
    </div>
  </div>
</section>

<section class="sec">
  <div class="sec-h"><h2>Обновления</h2></div>
  <div class="sec-b"><Updates /></div>
</section>

<section class="sec">
  <div class="sec-h">
    <h2>Знакомство</h2>
    <p>Что такое Джарвис, что он умеет, кто думает и какие плагины поставить</p>
  </div>
  <div class="sec-b">
    <div class="row">
      <button class="btn" type="button" onclick={() => wizOpen(0)}>{@html icon("right")}Показать знакомство</button>
      <a class="btn ghost" href="https://github.com/newfpv/YaJavis" target="_blank" rel="noopener noreferrer">
        {@html icon("ext")}Поставить звезду на GitHub
      </a>
    </div>
  </div>
</section>
