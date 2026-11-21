<script module>
  const pageSize = 40;
  const storageLimits = [1, 2, 3, 4, 5, 10, 20, 30, 40, 50];
  let archiveView = { filter: "all", limit: pageSize, skipped: new Set() };
</script>

<script>
  import { onMount } from "svelte";
  import { api, icon, toast } from "#helpers/core.js";
  import BackgroundRecording from "../BackgroundRecording/BackgroundRecording.svelte";
  import RecordReview from "../RecordReview/RecordReview.svelte";
  import { clipKey, clipProb, clipSec, clipUrl, clipWhen, needsReview, recordsUrl, reviewQueue } from "./records.js";
  import styles from "./TrainingRecords.module.css";

  const controller = new AbortController();
  const { signal } = controller;
  let data = $state.raw(null);
  let error = $state("");
  let pending = $state(false);
  let connected = $state(null);
  let view = $state.raw(archiveView);
  let autoplay = $state(false);
  let audio = $state();
  let selectedKey = $state("");
  let archivePlaying = $state(false);

  let clips = $derived(data?.clips || []);
  let counts = $derived(data?.counts || {});
  let offload = $derived(data?.offload || {});
  let hasCloud = $derived(connected ?? Boolean(offload.connected));
  let queue = $derived(reviewQueue(clips, view.skipped));
  let current = $derived(queue[0]);
  let nextRemoteUrl = $derived(queue[1]?.remote ? clipUrl(queue[1]) : "");
  let visibleClips = $derived(clips.filter((clip) => {
    if (view.filter === "all") return true;
    if (view.filter === "todo") return needsReview(clip);
    return clip.kind === view.filter;
  }));
  let filters = $derived([
    ["all", "Все", clips.length],
    ["pos", "Джарвис", counts.pos || 0],
    ["false", "Не Джарвис", counts.false || 0],
    ["bg", "Фон", counts.bg || 0],
    ["todo", "Не проверено", clips.filter(needsReview).length],
  ]);
  let busy = $derived(pending || Boolean(error));
  let limitOptions = $derived(data && !storageLimits.includes(data.limit_mb)
    ? [...storageLimits, data.limit_mb].sort((a, b) => a - b)
    : storageLimits);

  onMount(() => {
    void run(refresh);
    return () => controller.abort();
  });

  $effect(() => {
    if (!nextRemoteUrl) return;
    const prefetch = new AbortController();
    void fetch(nextRemoteUrl, { signal: prefetch.signal }).catch(() => {});
    return () => prefetch.abort();
  });

  async function refresh() {
    try {
      data = await api("GET", recordsUrl, undefined, { signal });
      error = "";
      if (selectedKey && !data.clips?.some((clip) => clipKey(clip) === selectedKey)) {
        audio.pause();
        audio.removeAttribute("src");
        audio.load();
        selectedKey = "";
      }
    } catch (failure) {
      if (!signal.aborted) error = failure.message;
    }
  }

  async function run(action) {
    if (pending) return;
    pending = true;
    try {
      await action();
    } catch (failure) {
      if (!signal.aborted) toast(failure.message, true);
    } finally {
      pending = false;
    }
  }

  function archiveAudio(node) {
    return { destroy: () => node.pause() };
  }

  function stopArchive() {
    audio?.pause();
    archivePlaying = false;
  }

  function playClip(clip) {
    const key = clipKey(clip);
    if (selectedKey === key && !audio.paused) {
      stopArchive();
      return;
    }
    if (selectedKey !== key) {
      selectedKey = key;
      audio.src = clipUrl(clip);
    }
    void audio.play().catch(() => {});
  }

  function saveView(change) {
    view = archiveView = { ...view, ...change };
  }

  function changeSettings(event, field, value) {
    const input = event.currentTarget;
    if (field === "mode") input.checked = data.mode;
    else input.value = data.limit_mb;
    if (busy) return;
    void run(async () => {
      await api("PUT", recordsUrl, { [field]: value }, { signal });
      data = { ...data, [field]: value };
    });
  }

  function moveClip(clip, to) {
    if (busy) return;
    return run(async () => {
      await api("POST", recordsUrl + "/move", { kind: clip.kind, name: clip.name, to, reviewed: true }, { signal });
      await refresh();
    });
  }

  function answer(to) {
    if (busy || !current) return;
    autoplay = true;
    stopArchive();
    if (to === "skip") saveView({ skipped: new Set(view.skipped).add(clipKey(current)) });
    else void moveClip(current, to);
  }

  function clearRecords() {
    if (busy || !confirm("Удалить все записи для обучения? Сначала лучше скачать архив.")) return;
    void run(async () => {
      await api("DELETE", recordsUrl, undefined, { signal });
      toast("Записи удалены");
      await refresh();
    });
  }

  function restoreModel() {
    void run(async () => {
      await api("POST", "/api/train/model/restore", undefined, { signal });
      toast("Прежняя модель сохранена для «Джарвиса»");
    });
  }
</script>

{#if error}
  <div class="row" role="status">
    <span class="err">{error}</span>
    <button class="btn" type="button" disabled={pending} onclick={() => run(refresh)}>Повторить</button>
  </div>
{/if}

{#if data}
  <section class="sec">
    <div class="sec-h">
      <h2>Короткие записи</h2>
      <p>Колонка сохраняет примеры «Джарвиса» для разметки.</p>
    </div>
    <div class="sec-b">
      <label class="sw">
        <input type="checkbox" checked={data.mode} disabled={busy}
          onchange={(event) => changeSettings(event, "mode", event.currentTarget.checked)}>
        <span class="tr"></span>Собирать примеры
      </label>
      <div class="f">
        <label for="train-limit">Лимит на колонке <b class="val">{(data.used_kb / 1024).toFixed(1)} МБ занято</b></label>
        <select id="train-limit" value={data.limit_mb} disabled={busy}
          onchange={(event) => changeSettings(event, "limit_mb", Number(event.currentTarget.value))}>
          {#each limitOptions as megabytes (megabytes)}
            <option value={megabytes}>{megabytes} МБ</option>
          {/each}
        </select>
      </div>
      <p class="note">
        {hasCloud
          ? "Когда лимит заполнен, 10 самых старых непроверенных записей отправляются в подключённое облако одним ZIP."
          : "Подключите облако ниже, чтобы старые записи при заполнении лимита уходили туда пачками по 10."}
        {offload.running ? " Отправляю…" : ""}
        {offload.sent ? ` Отправлено записей: ${offload.sent}.` : ""}
      </p>
      {#if hasCloud && offload.error}<p class="note err">{offload.error}. Записи остались на колонке.</p>{/if}
      {#if data.legacy_kb}<p class="note">Старый архив занимает {(data.legacy_kb / 1024).toFixed(1)} МБ.</p>{/if}
    </div>
  </section>

  <BackgroundRecording onconnection={(value) => { connected = value; }} />

  <section class="sec">
    <div class="sec-h">
      <h2>Проверка записей «Джарвис»</h2>
      <p>Прослушайте сохранённые записи и отметьте, где был оклик. Клавиши 1 и 2 отмечают ответ, пробел ставит запись на паузу.</p>
    </div>
    <div class="sec-b">
      {#if current}
        {#key clipKey(current)}
          <RecordReview clip={current} remaining={queue.length} {autoplay} {archivePlaying} {busy} onanswer={answer}
            onplay={() => { autoplay = true; stopArchive(); }} />
        {/key}
      {:else}
        <div class={styles.done}>{@html icon("done")}<b>Всё проверено</b></div>
      {/if}
    </div>
  </section>

  <section class="sec">
    <div class="sec-h">
      <h2>Дообучить на компьютере</h2>
      <p>В архив входят размеченные записи, модель и скрипт дообучения.</p>
    </div>
    <div class="sec-b {styles.finetune}">
      <div class="row">
        <div class="chips">
          <span class="chip k-pos">Джарвис <b>{counts.pos || 0}</b></span>
          <span class="chip k-false">Не Джарвис <b>{counts.false || 0}</b></span>
          <span class="chip">Фон <b>{counts.bg || 0}</b></span>
        </div>
        <span class="spacer"></span>
        {#if clips.length}
          <a class="btn primary" href={recordsUrl + "/download"} download>Скачать записи</a>
        {:else}
          <button class="btn" type="button" disabled>Скачать записи</button>
        {/if}
      </div>
      <ol class="steps">
        <li><b>Скачайте и распакуйте архив.</b> Записи уже разложены по вашим ответам.</li>
        <li><b>Дообучите модель.</b> В распакованной папке установите зависимости командой <code>pip install torch numpy</code> и запустите <code>python finetune.py</code>.</li>
        <li><b>Загрузите</b> <code>wake_new.bin</code> в разделе «Обучить фразу» с именем «Джарвис». Проверьте и выберите модель в списке.</li>
      </ol>
      {#if data.prev_model}
        <div><button class="btn ghost {styles.restore}" type="button" disabled={busy} onclick={restoreModel}>Вернуть прежнюю модель «Джарвис»</button></div>
      {/if}
    </div>
  </section>

  <section class="sec">
    <div class="sec-h"><h2>Все записи</h2></div>
    <div class="sec-b">
      <audio
        bind:this={audio}
        use:archiveAudio
        preload="none"
        onplay={() => { archivePlaying = true; }}
        onpause={() => { archivePlaying = false; }}
        onended={() => { archivePlaying = false; }}
        onerror={() => toast("Не удалось воспроизвести запись.", true)}
      ></audio>
      <div class="row">
        <div class="seg {styles.filters}" role="group" aria-label="Фильтр записей">
          {#each filters as [value, name, count] (value)}
            <button type="button" class:on={view.filter === value} aria-pressed={view.filter === value}
              onclick={() => saveView({ filter: value, limit: pageSize })}>{name} <small>{count}</small></button>
          {/each}
        </div>
        <span class="spacer"></span>
        {#if clips.length}
          <button class="btn sm ghost danger" type="button" disabled={busy} onclick={clearRecords}>Удалить все</button>
        {/if}
      </div>
      <div class={styles.list}>
        {#each visibleClips.slice(0, view.limit) as clip (clipKey(clip))}
          {@const playing = selectedKey === clipKey(clip) && archivePlaying}
          <div class={styles.clip}>
            <button class="ib {styles.play}" class:playing type="button" aria-label={playing ? "Пауза" : "Прослушать"}
              onclick={() => playClip(clip)}>{@html icon(playing ? "pause" : "play", "solid")}</button>
            <span class={styles.meta}>
              <b>
                {clipWhen(clip)}
                {#if clip.remote}
                  <span class="on-cloud" title="На Google Диске — колонка докачает при прослушивании"
                    aria-label="На Google Диске" role="img">{@html icon("cloud")}</span>
                {/if}
              </b>
              <small>{clipSec(clip).toFixed(1)} с{clipProb(clip) ? ` · ${clipProb(clip)}%` : ""}{clip.reviewed ? " · проверено" : ""}</small>
            </span>
            <div class="seg sm {styles.labels}">
              {#if clip.kind === "bg"}
                <button type="button" class="on" disabled>Фон</button>
              {:else}
                <button type="button" class:on={clip.kind === "pos"} aria-pressed={clip.kind === "pos"} disabled={busy}
                  onclick={() => clip.kind !== "pos" && moveClip(clip, "pos")}>Джарвис</button>
                <button type="button" class:on={clip.kind === "false"} aria-pressed={clip.kind === "false"} disabled={busy}
                  onclick={() => clip.kind !== "false" && moveClip(clip, "false")}>Не Джарвис</button>
              {/if}
            </div>
            <button class="ib {styles.delete}" type="button" aria-label="Удалить" disabled={busy}
              onclick={() => moveClip(clip, "delete")}>{@html icon("x")}</button>
          </div>
        {:else}
          <p class="note">Здесь пока пусто</p>
        {/each}
      </div>
      {#if visibleClips.length > view.limit}
        {@const remaining = visibleClips.length - view.limit}
        <button class="btn sm ghost" type="button" onclick={() => saveView({ limit: view.limit + pageSize })}>
          Показать ещё {Math.min(pageSize, remaining)} из {remaining}
        </button>
      {/if}
    </div>
  </section>
{:else if pending}
  <p class="note" role="status">Загружаю записи…</p>
{/if}
