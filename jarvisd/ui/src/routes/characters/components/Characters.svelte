<script>
  import { onMount } from "svelte";
  import { api, icon, setDirty, toast } from "#helpers/core.js";
  import { S, reload } from "#state/panelState.js";
  import styles from "./Characters.module.css";

  const presets = [
    ["Дворецкий", "Ты — Джарвис, голосовой помощник в умной колонке. Говоришь по-русски, коротко и по делу, с лёгкой иронией, как дворецкий из фильма. Обращаешься к хозяину на «ты»."],
    ["Кореш", "Ты — Джарвис, но общаешься как близкий друг: на «ты», с юмором и подколами, живо и по-простому. Говоришь коротко и честно: не знаешь — так и скажи."],
    ["Учёный", "Ты — Джарвис, спокойный и точный, как учёный. Объясняешь просто, но без ошибок, приводишь одну яркую деталь или цифру. Отвечаешь в 2–3 предложения, если не просят подробнее."],
    ["Лаконичный", "Ты — Джарвис. Отвечай максимально коротко: одно предложение, без вступлений и вежливых оборотов. Только суть."],
    ["Детский", "Ты — Джарвис, добрый помощник для ребёнка. Говоришь просто, тепло и весело, без сложных слов и без страшного. Хвалишь за интересные вопросы."],
  ];

  let prompts = $state.raw(S.prompts);
  let profiles = $state.raw(S.profiles);
  let editing = $state.raw(null);
  let name = $state("");
  let text = $state("");
  let busy = $state(false);
  let errorMessage = $state("");
  let dialog = $state();
  let nameInput = $state();
  let disposed = false;
  let used = $derived(editing && profiles.some((profile) => profile.prompt === editing.id));

  onMount(() => {
    const editor = dialog;
    function closeFromBackdrop(event) {
      if (event.target === editor && !busy) closeEditor();
    }
    editor.addEventListener("click", closeFromBackdrop);
    return () => {
      disposed = true;
      if (editor.open) setDirty(false);
      editor.removeEventListener("click", closeFromBackdrop);
    };
  });

  function openEditor(character = null) {
    if (busy) return;
    editing = character;
    name = character?.name || "";
    text = character?.text || "";
    errorMessage = "";
    dialog.showModal();
    nameInput.focus();
  }

  function closeEditor() {
    dialog.close();
    setDirty(false);
  }

  function cancelEditor(event) {
    event.preventDefault();
    if (!busy) closeEditor();
  }

  function markDirty() {
    setDirty(true);
  }

  function applyPreset(presetName, presetText) {
    if (text.trim() && !confirm("Заменить текст образцом?")) return;
    text = presetText;
    if (!name.trim()) name = presetName;
    markDirty();
  }

  async function changeCharacter(write, message) {
    if (busy) return;
    busy = true;
    errorMessage = "";
    try {
      await write();
      if (!disposed) closeEditor();
      await reload();
      if (disposed) return;
      prompts = S.prompts;
      profiles = S.profiles;
      toast(message);
    } catch (error) {
      if (disposed) return;
      if (dialog.open) errorMessage = error.message;
      else toast(error.message, true);
    } finally {
      busy = false;
    }
  }

  function save(event) {
    event.preventDefault();
    const character = { id: editing?.id || "", name: name.trim(), text: text.trim() };
    void changeCharacter(() => api("PUT", "/api/prompt", character), "Характер сохранён");
  }

  function deleteCharacter() {
    if (!editing || used || busy || !confirm(`Удалить характер «${editing.name}»?`)) return;
    const url = "/api/prompt?id=" + encodeURIComponent(editing.id);
    void changeCharacter(() => api("DELETE", url), "Удалено");
  }
</script>

<header class="ph ph-row">
  <div>
    <h1>Характеры</h1>
    <p class="sub">Кто такой Джарвис и как он с вами общается. Характер выбирается в настройках ассистента</p>
  </div>
  <div class="actions">
    <button class="btn primary" type="button" onclick={() => openEditor()}>{@html icon("plus")}Новый характер</button>
  </div>
</header>

<div class={[styles.characters, "stagger"]}>
  {#each prompts as character (character.id)}
    {@const assigned = profiles.filter((profile) => profile.prompt === character.id)}
    <button class={["card", styles.character]} type="button" aria-label={`Изменить «${character.name}»`}
      onclick={() => openEditor(character)}>
      <span class={styles.heading}>
        <span class={styles.avatar} aria-hidden="true">{character.name.trim()[0] || "?"}</span>
        <span class={styles.name}>
          <strong>{character.name}</strong>
          <span class="meta">{assigned.length ? "у " + assigned.map((profile) => profile.name).join(", ") : "пока ни у кого"}</span>
        </span>
      </span>
      <span class={styles.text}>{character.text}</span>
      <span class={styles.footer}>
        <span class="meta">{character.text.length} симв.</span>
        <span class="spacer"></span>
        <span class={styles.edit}>{@html icon("tune")}Изменить</span>
      </span>
    </button>
  {/each}
  <button class={[styles.character, styles.add]} type="button" onclick={() => openEditor()}>
    {@html icon("plus")}
    <b>Новый характер</b>
    <small>Опишите манеру — и Джарвис заговорит по-другому</small>
  </button>
</div>

<dialog class={styles.dialog} bind:this={dialog} oncancel={cancelEditor} aria-labelledby="character-editor-title">
  <form class={styles.editor} onsubmit={save}>
    <div class="card-head">
      <h2 id="character-editor-title">{editing ? `Изменить «${editing.name}»` : "Новый характер"}</h2>
      <button class="ib" type="button" onclick={closeEditor} disabled={busy} aria-label="Закрыть">{@html icon("x")}</button>
    </div>
    <div class="f">
      <label for="character-name">Название</label>
      <input id="character-name" bind:this={nameInput} bind:value={name} oninput={markDirty} disabled={busy}
        type="text" maxlength="40" placeholder="Например, Дворецкий" autocomplete="off">
    </div>
    <div class="f" role="group" aria-labelledby="character-presets">
      <span class="lbl" id="character-presets">Начать с образца</span>
      <div class={styles.presets}>
        {#each presets as [presetName, presetText] (presetName)}
          <button class={["chip", styles.preset]} type="button" disabled={busy}
            onclick={() => applyPreset(presetName, presetText)}>{presetName}</button>
        {/each}
      </div>
    </div>
    <div class="f">
      <div class={styles.textLabel}>
        <label class="lbl" for="character-text">Как он говорит</label>
        <span class="meta" id="character-count">{text.length} симв.</span>
      </div>
      <textarea class="big" id="character-text" bind:value={text} oninput={markDirty} disabled={busy}
        aria-describedby="character-count"
        placeholder="Ты — Джарвис… Опишите манеру: тон, обращение, длину ответов, любимые словечки"></textarea>
    </div>
    <p class="note">Пишите от второго лица: «Ты — …». Не вставляйте ключи и пароли — текст уходит модели с каждым разговором.</p>
    {#if errorMessage}
      <p class="err" role="alert">{errorMessage}</p>
    {/if}
    <div class="row">
      <button class="btn primary" type="submit" disabled={busy}>Сохранить</button>
      <button class="btn ghost" type="button" onclick={closeEditor} disabled={busy}>Отмена</button>
      {#if editing && !used}
        <span class="spacer"></span>
        <button class="btn ghost danger" type="button" onclick={deleteCharacter} disabled={busy}>Удалить</button>
      {/if}
    </div>
  </form>
</dialog>
