<script>
  import { icon, tip } from "#helpers/core.js";
  import VpnSwitch from "./VpnSwitch.svelte";

  let { id, state, action, keydown } = $props();
  let account = $derived(state[id] || {});
  let login = $derived(account.login || {});
  let devices = $derived(account.devices || []);
  let local = $derived(devices.filter((device) => device.local).length);
  let youtube = $derived(account.account || {});
  let smartLife = $derived(id === "smartlife");
</script>

{#if id === "chatgpt"}
  {#if account.signed_in}
    <div class="cn-ok">{@html icon("done")}<span>Колонка вошла в ChatGPT{#if account.plan} · <b>{account.plan}</b>{/if}{#if account.email}<small>{account.email}</small>{/if}</span></div>
    {#if account.limited_until * 1000 > Date.now()}
      <p class="err">Кончился лимит голоса в подписке. До {new Date(account.limited_until * 1000).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" })} Джарвис говорит через Gemini{state.keys?.gemini ? "" : " — но ключа Gemini нет, добавьте его ниже"}.</p>
    {/if}
    <p class="note">Все звонки — один разговор с общей памятью. Что Джарвис помнит, видно и правится на странице <a class="lnk" href="/memory">«Память»</a>.</p>
  {:else if login.code}
    <ol class="how cn-steps">
      <li>Откройте <a href={login.url} target="_blank" rel="noopener">{login.url?.replace("https://", "")}</a> (из России и Беларуси — через VPN) и войдите в свой ChatGPT.</li>
      <li>Введите код: <b class="code-big">{login.code}</b></li>
      <li>Это окно само увидит вход — ничего нажимать не нужно.</li>
    </ol>
  {:else}
    {#if login.error}<p class="err">{login.error}</p>{/if}
    <div class="row"><button type="button" class="btn primary" id="cg-in" onclick={action}>Войти в ChatGPT</button></div>
    <p class="note">Колонка получит свой вход, пароль она не видит. Способ неофициальный, как у Codex: OpenAI может его изменить.</p>
    <VpnSwitch {id} {state} {action} />
  {/if}
{:else if id === "youtube"}
  {#if youtube.code}
    <div class="ycode">
      <span>Откройте <a href={youtube.url} target="_blank" rel="noopener">{youtube.url?.replace("https://", "")}</a> и введите</span>
      <b class="code-big" role="button" tabindex="0" title="Нажмите, чтобы скопировать" onkeydown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          event.currentTarget.click();
        }
      }}>{youtube.code}</b>
      <small class="muted">окно само увидит вход</small>
    </div>
  {:else if youtube.signed_in}
    <div class="cn-ok">{@html icon("done")}<span>Колонка вошла в YouTube<small>видео 18+ тоже играют</small></span></div>
  {:else}
    {#if youtube.error}<p class="err">{youtube.error}</p>{/if}
    <div class="row"><button type="button" class="btn primary" id="ya-in" onclick={action}>Войти в YouTube</button><small class="muted">кодом, как на телевизоре — пароль колонка не видит</small></div>
    <details class="mini">
      <summary>Не получается кодом? Куки браузера</summary>
      <ol class="how"><li>Расширение «Get cookies.txt LOCALLY» → окно инкогнито → войдите на youtube.com.</li><li>Export (Netscape) → скопируйте весь текст сюда → закройте окно, не выходя из аккаунта.</li></ol>
      <textarea id="yt-cookies" class="code" placeholder="# Netscape HTTP Cookie File…" spellcheck="false" aria-label="Куки YouTube"></textarea>
      <div class="row"><button type="button" class="btn sm" id="yt-save" onclick={action}>{account.set ? "Заменить" : "Сохранить и проверить"}</button></div>
    </details>
    <VpnSwitch {id} {state} {action} />
  {/if}
{:else if account.signed_in}
  <div class="cn-ok">{@html icon("done")}<span>Колонка вошла в {smartLife ? "Smart Life" : "Xiaomi Home"}{#if !smartLife && account.region}
    <small>{account.region.includes(",") ? "серверы" : "сервер"} {account.region}</small>
  {/if}</span></div>
  {#if devices.length}
    <div class="xm-devs">
      {#each devices as device}
        <div class={["xm-dev", { "sl-dev": smartLife, off: !device.online }]}>
          {#if smartLife}
            <span class={["sl-way", device.local ? "lan" : "cloud"]} data-tip={device.local ? "По домашней сети — мгновенно" : "Через облако Tuya — около полсекунды. Устройства Zigbee (через шлюз) всегда идут так"}>{@html icon(device.local ? "wifi" : "cloud")}</span>
          {/if}
          <b>{device.name}</b>
          <small>{(smartLife ? [device.room, device.product] : [device.room, device.type || device.model, device.region && "сервер " + device.region]).filter(Boolean).join(" · ")}{device.online ? "" : " · не в сети"}</small>
        </div>
      {/each}
    </div>
    {#if smartLife}
      <p class="note">{local ? `По домашней сети, мгновенно: ${local} из ${devices.length}. Остальные — через облако Tuya.` : "Устройства в домашней сети ещё не откликнулись — пока команды идут через облако Tuya. Обычно они находятся за минуту."}</p>
    {/if}
  {:else}
    <p class="note">Устройств не нашлось. Если они есть в {smartLife ? "Smart Life" : "Mi Home"} — нажмите «Обновить устройства».</p>
  {/if}
  <div class="row"><button type="button" class="btn" id={smartLife ? "sl-refresh" : "xm-refresh"} onclick={action}>Обновить устройства</button></div>
  <VpnSwitch {id} {state} {action} />
{:else if login.code}
  <div class="xm-qr">
    <img src={`/api/${id}/qr?t=${login.at || 0}`} alt={`QR-код для входа в ${smartLife ? "Smart Life" : "Xiaomi"}`} width="220" height="220">
    <ol class="how cn-steps">
      {#if smartLife}
        <li>Откройте Smart Life → значок сканера в правом верхнем углу.</li><li>Отсканируйте код и подтвердите вход.</li>
      {:else}
        <li>Отсканируйте код камерой телефона или в приложении Mi Home: <b>+</b> → <b>Сканировать</b>.</li>
        <li>Войдите в свой аккаунт Xiaomi и подтвердите вход.</li>
      {/if}
      <li>Это окно само увидит вход и найдёт устройства.</li>
    </ol>
  </div>
  {#if !smartLife && login.url}
    <p class="note">Открыли эту страницу на телефоне? <a href={login.url} target="_blank" rel="noopener">Войти по ссылке</a></p>
  {/if}
{:else}
  {#if login.error}<p class="err">{login.error}</p>{/if}
  {#if smartLife}
    <div class="f">
      <label for="sl-code">Код пользователя {@html tip("В Smart Life: «Я» → шестерёнка «Настройки» → «Аккаунт и безопасность» → «Код пользователя». Это не пароль.")}</label>
      <div class="row">
        <input type="text" id="sl-code" placeholder="Например, AbC123x" autocomplete="off" spellcheck="false" onkeydown={keydown}>
        <button type="button" class="btn primary" id="sl-in" onclick={action}>Получить QR-код</button>
      </div>
    </div>
    <p class="note">Код пользователя — в Smart Life: «Я» → «Настройки» → «Аккаунт и безопасность». Потом отсканируете QR в самом приложении. Пароль колонка не видит.</p>
  {:else}
    <div class="row"><button type="button" class="btn primary" id="xm-in" onclick={action}>Войти по QR-коду</button></div>
    <p class="note">Пароль колонка не видит: вы подтверждаете вход на телефоне. Устройства берутся из того же аккаунта, что и в Mi Home.</p>
  {/if}
  <VpnSwitch {id} {state} {action} />
{/if}
