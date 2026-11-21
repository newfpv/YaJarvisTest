<script>
  import { untrack } from "svelte";
  import { tip } from "#helpers/core.js";
  import { avatar } from "#state/panelState.js";

  const colors = [1, 2, 3, 4, 5, 6];
  let { initial, prompts, isNew, active, onedit } = $props();
  let draft = $state({ ...untrack(() => initial) });
  let savedName = $state(untrack(() => initial.name));

  function edit(field, value) {
    if (draft[field] === value) return;
    draft[field] = value;
    onedit({ [field]: value });
  }

  export function updateSavedName(name, updateDraft) {
    savedName = name;
    if (updateDraft) draft.name = name;
  }
</script>

<header class="ph who">
  <span id="pav" class="ringbox">{@html avatar(draft, true)}</span>
  <div>
    <h1>{isNew ? "Новый ассистент" : savedName}</h1>
    <p class="sub">{isNew ? "Свой голос, модель и характер" : active ? "Активный профиль" : "Не активен"}</p>
  </div>
</header>

<section class="sec" id="profile-basics">
  <div class="sec-h">
    <h2>Основное</h2>
    <p>Как ассистент выглядит в списке и кем он себя считает</p>
  </div>
  <div class="sec-b">
    <div class="grid g2">
      <div class="f">
        <label for="p-name">Имя {@html tip("Как ассистент называется в списке слева. На слово активации не влияет.")}</label>
        <input id="p-name" type="text" maxlength="40" value={draft.name} placeholder="Например, Джарвис"
          oninput={(event) => edit("name", event.currentTarget.value)}>
      </div>
      <div class="f">
        <span class="lbl">Цвет</span>
        <div class="colors" id="p-color" role="group" aria-label="Цвет ассистента">
          {#each colors as color (color)}
            <button class={"av c" + color} class:on={draft.color === color} data-c={color} type="button"
              aria-label={"Цвет " + color} aria-pressed={draft.color === color} onclick={() => edit("color", color)}></button>
          {/each}
        </div>
      </div>
    </div>
    <div class="f">
      <label for="p-prompt">Характер {@html tip("Системный промпт: кто такой Джарвис и как он общается. Характеры редактируются в разделе «Характеры».")}</label>
      <select id="p-prompt" value={draft.prompt} onchange={(event) => edit("prompt", event.currentTarget.value)}>
        {#each prompts as prompt (prompt.id)}
          <option value={prompt.id}>{prompt.name}</option>
        {/each}
      </select>
    </div>
  </div>
</section>
