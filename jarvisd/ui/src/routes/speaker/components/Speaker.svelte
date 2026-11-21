<script module>
  const timezones = [
    ["Europe/Kaliningrad", "Калининград (UTC+2)"],
    ["Europe/Moscow", "Москва (UTC+3)"],
    ["Europe/Minsk", "Минск (UTC+3)"],
    ["Europe/Kyiv", "Киев"],
    ["Europe/Samara", "Самара (UTC+4)"],
    ["Asia/Yekaterinburg", "Екатеринбург (UTC+5)"],
    ["Asia/Almaty", "Алматы (UTC+5)"],
    ["Asia/Omsk", "Омск (UTC+6)"],
    ["Asia/Novosibirsk", "Новосибирск (UTC+7)"],
    ["Asia/Krasnoyarsk", "Красноярск (UTC+7)"],
    ["Asia/Irkutsk", "Иркутск (UTC+8)"],
    ["Asia/Yakutsk", "Якутск (UTC+9)"],
    ["Asia/Vladivostok", "Владивосток (UTC+10)"],
    ["Asia/Magadan", "Магадан (UTC+11)"],
    ["Asia/Kamchatka", "Камчатка (UTC+12)"],
    ["Europe/Berlin", "Берлин"],
    ["Europe/London", "Лондон"],
    ["UTC", "UTC"],
  ];

  const languages = [["ru", "Русский"], ["en", "English"], ["uk", "Українська"], ["", "Определять самому"]];
  const services = [
    ["alice", "Алиса", "Джарвис передаёт команду штатной Алисе — она умеет всё, что подключено к Яндексу", () => true],
    ["yandexmusic", "Яндекс Музыка", "«Включи музыку», «поставь песню…» Джарвис отдаёт Алисе — играет из Яндекс Музыки. Выключено — всё играет с YouTube", () => true],
    ["smartlife", "Smart Life", "Напрямую по домашней сети — быстрее всего", (state) => state.smartlife?.signed_in],
    ["xiaomi", "Xiaomi Home", "Через облако Xiaomi: пылесос, увлажнитель, лампы Mi Home", (state) => state.xiaomi?.signed_in],
    ["yandex", "Яндекс Умный дом", "Сам Джарвис через API Яндекса, без Алисы — нужен токен", (state) => !!state.keys?.yandex],
    ["homeassistant", "Home Assistant", "Ваш сервер Home Assistant", (state) => state.home_assistant?.configured],
  ];

  const ranges = [
    {
      key: "wake_sensitivity", label: "Чувствительность", min: 1, max: 10, step: 1,
      hint: "Выше — слышит издалека и тихо, но чаще срабатывает зря (например, на разговор в Discord). Ниже — реже ошибается, но звать нужно чётче.",
      low: "осторожно", high: "слышит всё", format: String, message: "Чувствительность сохранена",
    },
    {
      key: "mic_gain_db", label: "Общее усиление микрофона", min: 0, max: 30, step: 3,
      hint: "Для колонок, которые слышат вас только вблизи (так бывает на Станции Мини 2: её обработка отдаёт голос тихо). Усиливает звук для всего: слова активации, ИИ и записей для обучения. Прибавляйте по 6 дБ и проверяйте; слишком много — чаще ложные срабатывания. На Лайт обычно не нужно.",
      low: "как есть", high: "+30 дБ", format: (value) => `+${value} дБ`, message: "Общее усиление микрофона сохранено",
    },
    {
      key: "mic_boost_db", label: "Усиление микрофона для ИИ", min: 0, max: 18, step: 3,
      hint: "Если модель плохо слышит и переспрашивает, хотя вы говорите не тихо (так бывает на Станции Лайт 2: её обработка отдаёт голос тише), прибавьте 6–12 дБ. На слово активации это не влияет. Уровень голоса за каждый разговор пишется в журнал: «микрофон: за разговор голос …».",
      low: "как есть", high: "+18 дБ", format: (value) => `+${value} дБ`, message: "Усиление микрофона сохранено",
    },
  ];

  const nightDefaults = { enabled: false, from: "23:00", to: "08:00", voice: 20, bright: 10, speaker: 3 };
</script>

<script>
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { api, tip, toast } from "#helpers/core.js";
  import { S, reload } from "#state/panelState.js";
  import { loadWakeProfiles, modelLabel, selectWakeProfile } from "#helpers/wake-training.js";
  import Lighting from "./Lighting/Lighting.svelte";
  import NightMode from "./NightMode/NightMode.svelte";
  import styles from "./Speaker.module.css";

  const controller = new AbortController();
  const { signal } = controller;
  const pending = new SvelteSet();
  let writes = Promise.resolve();
  let state = $state.raw(S);
  let profiles = $state.raw(null);
  let profileError = $state("");
  let selected = $state("");
  let rangesInEdit = $state({});
  let settings = $derived(state.settings);
  let word = $derived(profiles?.find((profile) => profile.id === selected)?.phrase || settings.wake_word || "Джарвис");
  let timezone = $derived(settings.timezone || "UTC");
  let language = $derived(settings.language ?? "ru");
  let night = $derived({ ...nightDefaults, ...settings.night });

  onMount(() => {
    if (settings.wake_training) {
      loadWakeProfiles(signal).then((result) => {
        profiles = result.profiles;
        selected = result.selected;
      }).catch((error) => {
        if (!signal.aborted) profileError = error.message;
      });
    }
    return () => controller.abort();
  });

  async function refresh() {
    await reload();
    state = S;
  }

  function enqueue(key, write) {
    if (pending.has(key)) return writes;
    pending.add(key);
    const operation = writes.then(write).catch((error) => {
      if (!signal.aborted) toast(error.message, true);
    }).finally(() => pending.delete(key));
    writes = operation;
    return operation;
  }

  function save(key, getChanges, message) {
    return enqueue(key, async () => {
      const changes = getChanges();
      await api("PUT", "/api/settings", changes);
      const next = { ...settings, ...changes };
      if (changes.home_use) next.home_use = { ...settings.home_use, ...changes.home_use };
      state = { ...state, settings: next };
      await refresh();
      if (!signal.aborted) toast(typeof message === "function" ? message(changes) : message);
    });
  }

  function saveToggle(key, value, onMessage, offMessage) {
    void save(key, () => ({ [key]: value }), value ? onMessage : offMessage);
  }

  function saveService(id, name, value) {
    void save(id, () => ({ home_use: { [id]: value } }), `${name}: ${value ? "используется" : "не используется"}`);
  }

  function saveText(event, key) {
    const input = event.currentTarget;
    const value = input.value.trim();
    input.value = value;
    const message = key === "city" ? "Город сохранён" : value ? `Колонка стоит: ${value}` : "Комната колонки не задана";
    void save(key, () => ({ [key]: value }), message);
  }

  function saveTimezone(event) {
    const input = event.currentTarget;
    const value = input.value;
    input.value = timezone;
    void save("timezone", () => ({ timezone: value }), "Часовой пояс сохранён — время в журнале тоже по нему");
  }

  function saveLanguage(value, label) {
    void save("language", () => ({ language: value }), value ? `Язык: ${label}` : "Язык определяется сам");
  }

  function liveRange(event, key) {
    rangesInEdit[key] = event.currentTarget.valueAsNumber;
  }

  async function saveRange(event, range) {
    const { key, message } = range;
    const value = event.currentTarget.valueAsNumber;
    rangesInEdit[key] = value;
    await save(key, () => ({ [key]: value }), message);
    delete rangesInEdit[key];
  }

  function changeProfile(event) {
    const input = event.currentTarget;
    const id = input.value;
    input.value = selected;
    void enqueue("wake_profile", async () => {
      await selectWakeProfile(id);
      selected = id;
      await refresh();
    });
  }

  function saveNight(key, value) {
    return save("night." + key, () => ({ night: { ...night, [key]: value } }), ({ night }) =>
      night.enabled ? `Ночной режим: ${night.from}–${night.to}` : "Ночной режим выключен");
  }
</script>

{#snippet settingSection(title, description, fields)}
  <section class="sec">
    <div class="sec-h"><h2>{title}</h2><p>{description}</p></div>
    <div class="sec-b">{@render fields()}</div>
  </section>
{/snippet}

{#snippet toggle(label, key, checked, onChange, hint = "")}
  <label class="sw">
    <input type="checkbox" {checked} disabled={pending.has(key)} onchange={(event) => {
      const input = event.currentTarget;
      const next = input.checked;
      input.checked = checked;
      onChange(next);
    }}>
    <span class="tr"></span>
    {label} {#if hint}{@html tip(hint)}{/if}
  </label>
{/snippet}

<header class="ph">
  <h1>Колонка</h1>
  <p class="sub">Изменения сохраняются сразу</p>
</header>

{#snippet homeFields()}
  <div class={styles.services}>
    {#each services as [id, name, description, ready] (id)}
      {@const available = !!ready(state)}
      {@const checked = available && settings.home_use?.[id] !== false}
      <label class={["sw hu", { off: !available }]}>
        <input type="checkbox" {checked} disabled={!available || pending.has(id)}
          onchange={(event) => {
            const input = event.currentTarget;
            const next = input.checked;
            input.checked = checked;
            saveService(id, name, next);
          }}>
        <span class="tr"></span>
        <span class="hu-t">
          <b>{name}</b>
          <small>{description}{#if !available} · <a href="/connections">сначала подключите</a> — тогда галочка включится сама{/if}</small>
        </span>
      </label>
    {/each}
  </div>
{/snippet}
{@render settingSection("Умный дом и музыка",
  "Через что Джарвис может управлять домом и откуда включать музыку. Можно оставить несколько: быстрые команды сначала пробуют Smart Life по домашней сети, потом Алису, а сложные просьбы модель решает сама теми способами, что здесь включены",
  homeFields)}

{#snippet locationFields()}
  <div class="f">
    <label for="s-room">Комната, где стоит колонка {@html tip("«Выключи свет», «включи кондиционер» без названия комнаты — Джарвис сделает это здесь, как Алиса.")}</label>
    <input id="s-room" type="text" list="s-rooms" value={settings.speaker_room || ""} disabled={pending.has("speaker_room")}
      placeholder="Например, Зал" autocomplete="off" onchange={(event) => saveText(event, "speaker_room")}>
    <datalist id="s-rooms">
      {#each state.rooms || [] as room}<option value={room}></option>{/each}
    </datalist>
  </div>
  <div class="grid g2">
    <div class="f">
      <label for="s-city">Город</label>
      <input id="s-city" type="text" value={settings.city || ""} disabled={pending.has("city")}
        placeholder="Например, Москва" onchange={(event) => saveText(event, "city")}>
    </div>
    <div class="f">
      <label for="s-tz">Часовой пояс</label>
      <select id="s-tz" value={timezone} disabled={pending.has("timezone")} onchange={saveTimezone}>
        {#if !timezones.some(([id]) => id === timezone)}
          <option value={timezone}>{timezone}</option>
        {/if}
        {#each timezones as [id, label] (id)}<option value={id}>{label}</option>{/each}
      </select>
    </div>
  </div>
{/snippet}
{@render settingSection("Где вы",
  "Комната колонки — для команд без комнаты («выключи свет»). Город и пояс — для погоды, новостей и времени: часы колонки идут по Гринвичу",
  locationFields)}

{#snippet wakeFields()}
  {#if settings.wake_training}
    <div class="f">
      <label for="s-wake-profile">Выбранная фраза</label>
      <select id="s-wake-profile" value={selected} disabled={!profiles || pending.has("wake_profile")} onchange={changeProfile}>
        {#each profiles || [] as profile (profile.id)}
          <option value={profile.id}>{modelLabel(profile)}</option>
        {:else}
          <option value={selected}>{word}</option>
        {/each}
      </select>
      {#if profileError}<p class="err" role="alert">{profileError}</p>{/if}
      <a href="/training">Обучение слова</a>
    </div>
  {/if}
  {@render toggle(`Откликаться на «${word}»`, "wake_enabled", !!settings.wake_enabled,
    (value) => saveToggle("wake_enabled", value, "Джарвис снова слушает", "Джарвис больше не откликается на слово"),
    "Выключите, если Джарвис нужен только по кнопке «Поговорить». Алиса работает в любом случае.")}
  {@render toggle("Быстрый голосовой отклик", "wake_reply_enabled", settings.wake_reply_enabled !== false,
    (value) => saveToggle("wake_reply_enabled", value, "Быстрый отклик включён", "Быстрый отклик выключен"),
    "После слова активации Джарвис сразу говорит «слушаю», «угу» или другую короткую фразу голосом активного ассистента. Вопрос можно начать сразу.")}
  {#if !settings.wake_word}
    {@render toggle("Проверять имя перед ответом", "wake_check", !!settings.wake_check,
      (value) => saveToggle("wake_check", value, "Сначала проверяю имя, потом отвечаю", "Отклик сразу, как у Алисы"),
      "Выключено — как у Алисы: услышал слово, сразу загорается кольцо и Джарвис слушает. Включено — сначала облако проверяет, что прозвучало именно «Джарвис»: ложных разговоров меньше, но отклик на полсекунды-секунду позже.")}
  {/if}
  {#each ranges as range (range.key)}
    {@const value = rangesInEdit[range.key] ?? settings[range.key] ?? range.min}
    <div class="f">
      <label for={"speaker-" + range.key}>
        <span id={"label-" + range.key}>{range.label}</span>
        {@html tip(range.hint)}
        <b class="val" aria-hidden="true">{range.format(value)}</b>
      </label>
      <input type="range" id={"speaker-" + range.key} min={range.min} max={range.max} step={range.step}
        aria-labelledby={"label-" + range.key} {value} disabled={pending.has(range.key)}
        oninput={(event) => liveRange(event, range.key)} onchange={(event) => saveRange(event, range)}>
      <div class={styles.scale}><span>{range.low}</span><span>{range.high}</span></div>
    </div>
  {/each}
  {#if !state.speaker.wake_model}
    <p class="note">Модель слова ещё не установлена — пока Джарвиса можно позвать только кнопкой «Поговорить» на главной.</p>
  {/if}
{/snippet}
{@render settingSection("Слово активации", "Колонка распознаёт его без интернета", wakeFields)}

{#snippet languageFields()}
  <div class="seg" role="group" aria-label="Язык общения">
    {#each languages as [id, label] (id)}
      <button type="button" class:on={language === id} aria-pressed={language === id}
        disabled={pending.has("language")} onclick={() => saveLanguage(id, label)}>{label}</button>
    {/each}
  </div>
{/snippet}
{@render settingSection("Язык",
  "На каком языке вы говорите с Джарвисом. Распознавание речи и ответы будут на нём — так он не «услышит» испанский там, где был русский",
  languageFields)}

<NightMode {night} now={settings.night_now} leds={state.speaker.leds} {pending} save={saveNight} />

{#snippet lightingFields()}
  <Lighting />
{/snippet}
{@render settingSection("Подсветка",
  "Кольцо Джарвиса. Цвета у каждого ассистента свои — в его настройках. У Алисы подсветка своя и не меняется",
  lightingFields)}
