<script>
  import { icon } from "#helpers/core.js";
  import VpnSwitch from "./VpnSwitch.svelte";

  let { service, state, action, keydown } = $props();
  let id = $derived(service.id);
  let gemini = $derived(state.gemini_keys || []);
  let eleven = $derived(state.eleven_keys || []);
  let append = $derived(service.on && ["gemini", "elevenlabs"].includes(id));
  const date = (seconds) => new Date(seconds * 1000).toLocaleDateString("ru-RU", { day: "numeric", month: "long" });
</script>

{#if id === "gemini"}
  {#if gemini.length > 1}
    <div class="ek-list">
      {#each gemini as key, index}
        <div class="ek" class:on={!key.spent && !gemini.slice(0, index).some((earlier) => earlier.spent < earlier.models)} class:rest={key.spent >= key.models}>
          <b>{index + 1}</b><code>{key.tail}</code>
          <span class="ek-st">{key.spent >= key.models ? "лимит кончился · ждёт" : key.spent ? `лимит кончился у ${key.spent} из ${key.models} моделей` : "лимит есть"}</span>
          <span class="spacer"></span>
          <button type="button" class="ib" data-gk-remove={index} aria-label={`Убрать ключ ${index + 1}`} onclick={action}>{@html icon("x")}</button>
        </div>
      {/each}
    </div>
    <p class="note">У каждого ключа свой бесплатный лимит. Кончился у всех моделей первого — Джарвис сам переходит на следующий.</p>
  {:else if gemini[0]?.spent}
    <p class="note">Лимит кончился у {gemini[0].spent} из {gemini[0].models} моделей этого ключа. Добавьте ещё ключ — Джарвис перейдёт на него сам.</p>
  {/if}
{:else if id === "elevenlabs" && eleven.length}
  <div class="ek-list">
    {#each eleven as key, index}
      <div class="ek" class:on={key.active} class:rest={key.until}>
        <b>{index + 1}</b><code>{key.tail}</code>
        <span class="ek-st">{key.until ? `квота кончилась · ждёт до ${date(key.until)}` : key.active ? "сейчас работает" : "в запасе"}</span>
        <span class="spacer"></span>
        {#if key.until}<button type="button" class="btn sm ghost" data-ek-reset={index} data-tip="Например, если докупили символы" onclick={action}>Вернуть</button>{/if}
        <button type="button" class="ib" data-ek-remove={index} aria-label={`Убрать ключ ${index + 1}`} onclick={action}>{@html icon("x")}</button>
      </div>
    {/each}
  </div>
  <p class="note">Работает первый ключ с квотой. Кончилась — ключ откладывается до сброса лимита (обычно через месяц), и Джарвис сам берёт следующий.</p>
{:else if id === "yandex"}
  <div><b>Выдайте OAuth-приложению два разрешения</b><ul class="how">
    <li><code>iot:view</code> · видеть устройства и их состояние.</li>
    <li><code>iot:control</code> · управлять устройствами, группами и сценариями.</li>
  </ul></div>
{/if}

<div class="f">
  <label for={"k-" + id}>{id === "yandex" ? "OAuth-токен Яндекс ID" : append ? "Ещё ключ — в запас" : service.on ? "Новый ключ" : "Ключ"}</label>
  <div class="row">
    <input type="password" id={"k-" + id} placeholder={service.place} autocomplete="off" onkeydown={keydown}>
    <button type="button" class="btn primary" data-key={id} onclick={action}>{append ? "Добавить" : service.on ? "Заменить" : "Сохранить"}</button>
  </div>
</div>
{#if id !== "elevenlabs"}
  <details open={!service.on}>
    <summary>{id === "yandex" ? "Как получить токен" : "Где его взять"}</summary>
    <ol class="how">
      {#if id === "openai"}
        <li>Откройте platform.openai.com и войдите.</li>
        <li>Слева — API keys → Create new secret key.</li>
        <li>Скопируйте ключ (начинается с sk-) и вставьте сюда.</li>
        <li>Пополните баланс в разделе Billing — от 5 $.</li>
      {:else if id === "gemini"}
        <li>Откройте aistudio.google.com и войдите в аккаунт Google (из России и Беларуси — через VPN).</li>
        <li>Нажмите Get API key → Create API key.</li>
        <li>Скопируйте ключ и вставьте сюда.</li>
      {:else if id === "grok"}
        <li>Откройте console.x.ai и войдите.</li>
        <li>Пополните баланс (Billing → Credits) — без кредитов ключ не работает.</li>
        <li>API Keys → Create API key, скопируйте ключ (начинается с xai-) и вставьте сюда.</li>
      {:else}
        <li>В <a href="https://oauth.yandex.ru/client/new/" target="_blank" rel="noopener">Яндекс OAuth</a> создайте приложение «Для доступа к API или отладки». В разделе «Доступы» добавьте оба разрешения.</li>
        <li>Укажите Redirect URI <code>https://oauth.yandex.ru/verification_code</code> и сохраните приложение.</li>
        <li>Войдите в тот Яндекс ID, где добавлены устройства, и <a href="https://yandex.ru/dev/id/doc/ru/tokens/debug-token" target="_blank" rel="noopener">получите токен вручную</a>. Вставьте сюда значение <code>access_token</code>.</li>
      {/if}
    </ol>
  </details>
{/if}
<VpnSwitch {id} {state} {action} />
