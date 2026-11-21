<script module>
  const backgroundUrl = "/api/train/background";
  const cloudUrl = backgroundUrl + "/cloud";
</script>

<script>
  import { onMount } from "svelte";
  import { api, toast } from "#helpers/core.js";
  import styles from "./BackgroundRecording.module.css";

  let { onconnection } = $props();
  const controller = new AbortController();
  const { signal } = controller;

  let result = $state.raw(null);
  let pending = $state(false);
  let reading = $state(false);
  let error = $state("");
  let provider = $state("nextcloud");
  let link = $state("");
  let email = $state("");
  let password = $state("");
  let minutes = $state(60);
  let timer;
  let revision = 0;

  let status = $derived.by(() => {
    if (!result) return "";
    if (result.error) return result.error;
    const { running, uploading, left_sec, sent, discarded } = result;
    if (!running && !sent && !discarded) return "";
    let activity = "";
    if (running) activity = uploading ? "Отправляю запись. " : `Записываю. Осталось ${Math.ceil(left_sec / 60)} мин. `;
    return `${activity}Архивов отправлено: ${sent}. Фрагментов пропущено: ${discarded}.`;
  });

  onMount(() => {
    void refresh();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  });

  function update(next) {
    result = next;
    error = "";
    onconnection(Boolean(next.provider));
  }

  function scheduleRefresh() {
    clearTimeout(timer);
    if (!signal.aborted && result?.running) timer = setTimeout(refresh, 2500);
  }

  async function refresh() {
    if (pending) return;
    if (reading) {
      scheduleRefresh();
      return;
    }
    const currentRevision = revision;
    reading = true;
    try {
      const next = await api("GET", backgroundUrl, undefined, { signal });
      if (currentRevision === revision) update(next);
    } catch (failure) {
      if (!signal.aborted && currentRevision === revision) error = failure.message;
    } finally {
      reading = false;
      if (currentRevision === revision) scheduleRefresh();
    }
  }

  async function change(method, url, input) {
    if (pending) return false;
    pending = true;
    revision++;
    clearTimeout(timer);
    try {
      update(await api(method, url, input, { signal }));
      return true;
    } catch (failure) {
      if (!signal.aborted) toast(failure.message, true);
      return false;
    } finally {
      pending = false;
      scheduleRefresh();
    }
  }

  async function connect(event) {
    event.preventDefault();
    const input = provider === "koofr" ? { provider, email, password } : { provider, link, password };
    if (await change("PUT", cloudUrl, input)) {
      password = "";
      link = "";
    }
  }

  function start(event) {
    event.preventDefault();
    void change("POST", backgroundUrl, { minutes });
  }
</script>

<section class="sec">
  <div class="sec-h">
    <h2>Длительная запись фона</h2>
    <p>Колонка сама отправляет сжатые записи в облако. Каждая порция занимает примерно минуту и хранится в памяти до отправки.</p>
  </div>
  <div class="sec-b">
    {#if result?.provider}
      <form class={styles.form} onsubmit={start}>
        <div class="row">
          <span class="st ok">{result.provider === "koofr" ? "Koofr подключён" : "Nextcloud подключён"}</span>
          {#if !result.running}
            <button type="button" class="btn ghost" disabled={pending} onclick={() => change("DELETE", cloudUrl)}>Отключить</button>
          {/if}
        </div>
        <div class="f">
          <label for="bg-length">Продолжительность</label>
          <select id="bg-length" bind:value={minutes} disabled={pending || result.running}>
            <option value={15}>15 минут</option>
            <option value={60}>1 час</option>
            <option value={240}>4 часа</option>
            <option value={480}>8 часов</option>
          </select>
        </div>
        <div class="row">
          {#if result.running}
            <button type="button" class="btn" disabled={pending} onclick={() => change("DELETE", backgroundUrl)}>Остановить</button>
          {:else}
            <button type="submit" class="btn primary" disabled={pending}>Записать фон</button>
          {/if}
        </div>
      </form>
    {:else if result}
      <form class={styles.form} onsubmit={connect}>
        <div class="f">
          <label for="bg-provider">Облако</label>
          <select id="bg-provider" bind:value={provider} disabled={pending} onchange={() => { password = ""; }}>
            <option value="nextcloud">Nextcloud</option>
            <option value="koofr">Koofr</option>
          </select>
        </div>
        {#if provider === "koofr"}
          <div class="f">
            <label for="bg-email">Почта Koofr</label>
            <input id="bg-email" type="email" bind:value={email} required autocomplete="off" disabled={pending}>
          </div>
        {:else}
          <div class="f">
            <label for="bg-link">Ссылка Nextcloud на папку «Только загрузка»</label>
            <input id="bg-link" type="url" bind:value={link} required placeholder="https://cloud.example.com/s/…" autocomplete="off" disabled={pending}>
          </div>
        {/if}
        <div class="f">
          <label for="bg-password">{provider === "koofr" ? "Пароль приложения Koofr" : "Пароль ссылки, если задан"}</label>
          <input id="bg-password" type="password" bind:value={password} required={provider === "koofr"}
            maxlength="512" autocomplete="off" disabled={pending}>
        </div>
        {#if provider === "koofr"}
          <small class="muted">В <a href="https://app.koofr.net" target="_blank" rel="noopener noreferrer">Koofr</a> откройте настройки пароля и создайте отдельный пароль приложения.</small>
        {:else}
          <small class="muted">В Nextcloud откройте «Поделиться» у папки, выберите «Только загрузка» и скопируйте ссылку.</small>
        {/if}
        <div><button type="submit" class="btn primary" disabled={pending}>Подключить облако</button></div>
      </form>
    {:else if !error}
      <p class="note" role="status">Проверяю подключение…</p>
    {/if}
    {#if error || status}
      <p class="note" class:err={Boolean(error || result?.error)} role="status">{error || status}</p>
    {/if}
    {#if error && !result}
      <div><button type="button" class="btn" disabled={reading} onclick={refresh}>Повторить</button></div>
    {/if}
  </div>
</section>
