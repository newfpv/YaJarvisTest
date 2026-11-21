<script module>
  const defaultPhrase = "Джарвис";
  const positives = 10;
  const negatives = 3;
  const totalSamples = positives + negatives;
  const savedMessage = "Модель сохранена. Выберите её в списке.";

  function sampleLabel(index) {
    return index < positives ? `Фраза ${index + 1}` : `Фон ${index - positives + 1}`;
  }

  function sampleInstructions(count) {
    if (count === totalSamples) return "Записи готовы. Нажмите «Обучить».";
    if (count >= positives) return `Фон ${count - positives + 1} из ${negatives}. Скажите обычную фразу без слова активации.`;
    return `Запись ${count + 1} из ${positives}. Когда появится «Говорите», произнесите фразу один раз.`;
  }
</script>

<script>
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { api, toast, upload } from "#helpers/core.js";
  import { reload } from "#state/panelState.js";
  import { PROFILES, loadWakeProfiles, modelLabel, recordWakeSample, selectWakeProfile, wakeSampleUrl } from "#helpers/wake-training.js";
  import styles from "./WakeTraining.module.css";

  const controller = new AbortController();
  const { signal } = controller;
  const results = new SvelteMap();

  let data = $state.raw(null);
  let loadError = $state("");
  let busy = $state("");
  let phrase = $state(defaultPhrase);
  let phraseInput = $state();
  let samples = $state.raw([]);
  let reviewed = $state(0);
  let prompt = $state("");
  let previewUrl = $state("");
  let audio = $state();
  let complete = $derived(samples.length === totalSamples);
  let instructions = $derived(busy === "train" ? "Обучаю и проверяю модель…" : prompt || sampleInstructions(samples.length));

  onMount(() => {
    refresh().catch((error) => {
      if (!signal.aborted) loadError = error.message;
    });
    return () => {
      controller.abort();
      clearPreview();
    };
  });

  async function refresh() {
    data = await loadWakeProfiles(signal);
  }

  async function run(operation, action) {
    if (busy) return;
    busy = operation;
    try {
      await action();
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      busy = "";
    }
  }

  function changeProfile(event) {
    const select = event.currentTarget;
    const id = select.value;
    select.value = data.selected;
    void run("select", async () => {
      await selectWakeProfile(id);
      if (signal.aborted) return;
      data = { ...data, selected: id };
      await reload();
    });
  }

  function removeProfile(profile) {
    if (!confirm(`Удалить модель «${profile.phrase}»?`)) return;
    void run("remove", async () => {
      await api("DELETE", PROFILES + "/" + encodeURIComponent(profile.id));
      if (signal.aborted) return;
      results.delete(profile.id);
      await refresh();
    });
  }

  function testProfile(profile) {
    void run("test", async () => {
      audio?.pause();
      results.set(profile.id, "Приготовьтесь…");
      try {
        const pcm = await recordWakeSample(signal, 800, () => results.set(profile.id, `Говорите «${profile.phrase}»`));
        const result = await upload("POST", "/api/train/wake/test/" + encodeURIComponent(profile.id), pcm, { signal });
        results.set(profile.id, result.detected ? "Фраза распознана" : "Фраза не распознана");
      } catch (error) {
        if (!signal.aborted) results.set(profile.id, error.message);
      }
    });
  }

  function clearPreview() {
    audio?.pause();
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = "";
  }

  function listen(index) {
    clearPreview();
    reviewed = index;
    if (samples[index]) previewUrl = wakeSampleUrl(samples[index]);
  }

  function record(index) {
    const word = phrase.trim();
    if (!word) {
      toast("Введите слово или фразу", true);
      phraseInput.focus();
      return;
    }
    void run("record", async () => {
      clearPreview();
      prompt = "Приготовьтесь…";
      try {
        const pcm = await recordWakeSample(signal, 700, () => {
          prompt = index < positives ? `Говорите «${word}»` : "Говорите обычную фразу";
        });
        signal.throwIfAborted();
        const next = samples.slice();
        next[index] = pcm;
        samples = next;
      } finally {
        if (!signal.aborted) {
          prompt = "";
          listen(index < samples.length ? index : reviewed);
        }
      }
    });
  }

  function uploadModel(event) {
    const input = event.currentTarget;
    const file = input.files[0];
    input.value = "";
    if (!file) return;
    const modelUrl = PROFILES + "?phrase=" + encodeURIComponent(phrase.trim());
    void run("upload", async () => {
      await upload("PUT", modelUrl, file);
      if (signal.aborted) return;
      phrase = defaultPhrase;
      await refresh();
      toast(savedMessage);
    });
  }

  function train() {
    if (!complete) return;
    const trainUrl = "/api/train/wake/train?phrase=" + encodeURIComponent(phrase.trim());
    void run("train", async () => {
      audio?.pause();
      await upload("POST", trainUrl, new Blob(samples));
      if (signal.aborted) return;
      clearPreview();
      samples = [];
      reviewed = 0;
      phrase = defaultPhrase;
      await refresh();
      toast(savedMessage);
    });
  }
</script>

{#if loadError}
  <p class="err" role="alert">{loadError}</p>
{:else if !data}
  <p class="note" role="status">Загружаю фразы…</p>
{:else}
  <section class="sec">
    <div class="sec-h"><h2>Сохранённые фразы</h2></div>
    <div class={["sec-b", styles.fields]}>
      <div class="stack">
        <div class="f">
          <label for="wake-pick">Активная фраза</label>
          <select id="wake-pick" value={data.selected} disabled={!!busy} onchange={changeProfile}>
            {#each data.profiles as profile (profile.id)}
              <option value={profile.id}>{modelLabel(profile)}</option>
            {/each}
          </select>
        </div>
        {#each data.profiles.filter((profile) => profile.id) as profile (profile.id)}
          <div class="row">
            <span class={styles.savedPhrase}>{profile.phrase}</span>
            {#if data.training}
              <button class="btn" type="button" disabled={!!busy} onclick={() => testProfile(profile)}>Проверить</button>
              <span role="status">{results.get(profile.id) || ""}</span>
            {/if}
            {#if profile.id !== data.selected}
              <button class="btn" type="button" disabled={!!busy} onclick={() => removeProfile(profile)}>Удалить</button>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </section>

  <section class="sec">
    <div class="sec-h">
      <h2>Обучить фразу</h2>
      <p>{data.training
        ? "Оставьте «Джарвис», чтобы дообучить его на своём голосе, или введите свою фразу. Проверьте модель перед включением."
        : "Загрузите модель для «Джарвис» или другой фразы."}</p>
    </div>
    <div class={["sec-b", styles.fields]}>
      <div class="f">
        <label for="wake-phrase">Слово или фраза</label>
        <input id="wake-phrase" type="text" maxlength="48" autocomplete="off" bind:this={phraseInput} bind:value={phrase}
          readonly={!!busy || samples.length > 0}>
      </div>
      {#if data.training}
        <div class={styles.recorder}>
          <p class="muted" role="status">{instructions}</p>
          <div class="row">
            <button class="btn" type="button" disabled={!!busy || complete} onclick={() => record(samples.length)}>Записать</button>
            <select value={reviewed} disabled={!!busy || !samples.length} aria-label="Запись для прослушивания"
              onchange={(event) => listen(Number(event.currentTarget.value))}>
              {#each samples as _, index (index)}<option value={index}>{sampleLabel(index)}</option>{/each}
            </select>
            <button class="btn" type="button" disabled={!!busy || !samples.length} onclick={() => record(reviewed)}>Перезаписать</button>
            <button class="btn primary" type="button" disabled={!!busy || !complete} onclick={train}>Обучить</button>
          </div>
          {#if previewUrl}
            <audio bind:this={audio} src={previewUrl} controls preload="metadata"></audio>
          {/if}
        </div>
      {/if}
      <div class="f">
        <label for="wake-model">Загрузить готовую модель</label>
        <input id="wake-model" class={styles.file} type="file" accept=".bin,application/octet-stream"
          disabled={!!busy || samples.length > 0} onchange={uploadModel}>
      </div>
    </div>
  </section>
{/if}
