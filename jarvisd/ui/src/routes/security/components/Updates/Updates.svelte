<script>
  import { onMount } from "svelte";
  import { api, toast, upload, waitRestart } from "#helpers/core.js";
  import styles from "./Updates.module.css";

  const controller = new AbortController();
  const { signal } = controller;
  let update = $state.raw(null);
  let loadError = $state("");
  let pending = $state("");
  let automatic = $derived(!!(update?.check || update?.auto));

  onMount(() => {
    void load();
    return () => controller.abort();
  });

  async function load() {
    try {
      update = await api("GET", "/api/update", undefined, { signal });
    } catch (error) {
      if (!signal.aborted) loadError = error.message;
    }
  }

  async function run(operation, action) {
    if (pending) return;
    pending = operation;
    try {
      await action();
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      pending = "";
    }
  }

  function savePreferences(changes, message) {
    void run("preferences", async () => {
      await api("PUT", "/api/update", changes);
      if (signal.aborted) return;
      update = { ...update, ...changes };
      toast(message);
    });
  }

  function setAutomatic(on) {
    savePreferences({ check: on, auto: on }, on ? "Автообновление включено" : "Обновления — вручную");
  }

  function setStats(event) {
    const input = event.currentTarget;
    const stats = input.checked;
    input.checked = update.stats;
    savePreferences({ stats }, stats ? "Счётчик включён" : "Счётчик выключен");
  }

  function checkNow() {
    void run("check", async () => {
      const next = await api("POST", "/api/update/check");
      if (!signal.aborted) update = next;
    });
  }

  function install() {
    void run("install", async () => {
      await api("POST", "/api/update/install");
      await waitRestart("Обновление скачано — Джарвис перезапускается");
    });
  }

  function installFile(event) {
    const input = event.currentTarget;
    const file = input.files[0];
    input.value = "";
    if (!file) return;
    void run("upload", async () => {
      toast("Загружаю на колонку…");
      await upload("PUT", "/api/update/upload", file);
      await waitRestart("Файл принят — Джарвис перезапускается");
    });
  }
</script>

<div class={styles.updates}>
  {#if loadError}
    <p class="err" role="alert">{loadError}</p>
  {:else if update}
    <div class={["row", styles.actions]}>
      <span>Версия <b>{update.version}</b></span>
      <a class="btn sm ghost" href="https://github.com/newfpv/YaJavis/releases" target="_blank" rel="noopener noreferrer">GitHub Releases</a>
      {#if update.busy}
        <span class="note"><span class="spin"></span>{update.busy}…</span>
      {:else if update.available}
        <span class="st ok">вышла {update.latest}</span>
        <button class="btn sm primary" type="button" disabled={!!pending} onclick={install}>
          {pending === "install" ? "Скачиваю…" : "Обновить"}
        </button>
      {:else if update.latest}
        <span class="st ok">последняя</span>
      {/if}
      <span class="spacer"></span>
      {#if update.repo}
        <button class="btn sm" type="button" disabled={!!pending} onclick={checkNow}>Проверить</button>
      {/if}
      <label class={["btn sm ghost", styles.file]} data-tip={`Поставить файл ${update.asset} с компьютера`}>
        Файлом…
        <input type="file" disabled={!!pending} onchange={installFile}>
      </label>
    </div>

    <div class="seg" role="group" aria-label="Режим обновлений">
      <button type="button" class:on={!automatic} aria-pressed={!automatic}
        disabled={!!pending} onclick={() => setAutomatic(false)}>Вручную</button>
      <button type="button" class:on={automatic} aria-pressed={automatic}
        disabled={!!pending} onclick={() => setAutomatic(true)}>Автообновление</button>
    </div>
    <small class="muted">
      {#if automatic}
        Прошивка и плагины обновляются сами с 3 до 6 утра, когда колонка молчит{update.checked ? ` · проверено ${update.checked}` : ""}
      {:else if !update.repo}
        Укажите репозиторий обновлений или выберите файл с компьютера.
      {:else}
        Прошивка и плагины обновляются вручную
      {/if}
    </small>

    <label class="sw hu">
      <input type="checkbox" checked={update.stats} disabled={!!pending} onchange={setStats} aria-label="Счётчик установок"
        aria-describedby="update-stats-description">
      <span class="tr"></span>
      <span class="hu-t">
        <b>Счётчик установок</b>
        <small id="update-stats-description">Раз на версию колонка скачивает пустой файл с GitHub — так в README видно, сколько колонок с Джарвисом. Передаются только модель и версия.</small>
      </span>
    </label>
    {#if update.error}
      <p class="err">{update.error}</p>
    {/if}
    {#if update.available && update.notes}
      <details class={["mini", styles.release]}>
        <summary>Что нового в {update.latest}</summary>
        <pre class={styles.notes}>{update.notes}</pre>
      </details>
    {/if}
  {:else}
    <p class="note">…</p>
  {/if}
</div>
