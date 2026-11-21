<script>
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { api, setDirty, toast } from "#helpers/core.js";
  import { refreshRoute, reload, S } from "#state/panelState.js";
  import { profilePath } from "#helpers/paths.js";
  import { createProfile, profilePayload } from "./profileDraft.js";
  import { availableEngines } from "./ProfileBrain/options.js";
  import ProfileBasics from "./ProfileBasics/ProfileBasics.svelte";
  import ProfileBrain from "./ProfileBrain/ProfileBrain.svelte";
  import ProfileLighting from "./ProfileLighting/ProfileLighting.svelte";
  import ProfileVoice from "./ProfileVoice/ProfileVoice.svelte";

  let { base, isNew } = $props();
  const initial = createProfile(untrack(() => base), S);
  const active = initial.id === S.active;
  const engines = availableEngines();
  const controller = new AbortController();
  const { signal } = controller;
  let profile = $state({ ...initial, led: { ...initial.led } });
  let dirty = $state(false);
  let working = $state("");
  let revision = 0;
  let basics = $state();
  let brain = $state();
  let voice = $state();
  let ttsProvider = $derived(profile.engine === "classic" ? profile.tts_engine : profile.engine === "eleven_realtime" ? "elevenlabs" : profile.engine);
  let engineReady = $derived(engines.some(({ id }) => id === profile.engine));

  onMount(() => () => controller.abort());

  $effect(() => {
    const section = voice;
    const engine = profile.engine;
    const provider = ttsProvider;
    const ready = engineReady;
    if (section) untrack(() => section.configure(engine, provider, ready, engines.length > 0));
  });

  function mark() {
    revision++;
    dirty = true;
    setDirty(true);
  }

  function clearDirty() {
    dirty = false;
    setDirty(false);
  }

  function edit(changes, edited = true) {
    Object.assign(profile, changes);
    if (edited) mark();
  }

  function editLighting(changes) {
    Object.assign(profile.led, changes);
    mark();
  }

  function previewModels(engine) {
    const models = brain.getValues();
    if (engine === "classic") return { mode: "classic", tts_engine: profile.tts_engine, tts_model: models.tts };
    return { tts_model: "", live_model: models.live };
  }

  async function run(action, task) {
    if (working) return;
    working = action;
    try {
      await task();
    } catch (error) {
      if (!signal.aborted) toast(error.message, true);
    } finally {
      working = "";
    }
  }

  function save() {
    return run("save", async () => {
      const payload = profilePayload($state.snapshot(profile), brain.getValues());
      const savedRevision = revision;
      const { id } = await api("PUT", "/api/profile", payload, { signal });
      await reload();
      if (signal.aborted) return;
      toast("Сохранено");
      if (isNew) {
        clearDirty();
        await goto(profilePath(id));
        return;
      }
      const stored = S.profiles.find((item) => item.id === id);
      const unchanged = revision === savedRevision;
      if (stored) basics.updateSavedName(stored.name, unchanged);
      if (unchanged) {
        if (stored) {
          profile.name = stored.name;
          brain.updateIdle(stored.idle_s);
        }
        clearDirty();
      }
    });
  }

  function activate() {
    return run("activate", async () => {
      await api("POST", "/api/active", { id: profile.id }, { signal });
      await reload();
      if (signal.aborted) return;
      toast(`Теперь отвечает «${profile.name}»`);
      refreshRoute();
    });
  }

  function remove() {
    if (!confirm(`Удалить ассистента «${profile.name}»?`)) return;
    return run("delete", async () => {
      await api("DELETE", "/api/profile?id=" + encodeURIComponent(profile.id), undefined, { signal });
      await reload();
      if (signal.aborted) return;
      clearDirty();
      toast("Удалено");
      await goto("/");
    });
  }
</script>

<ProfileBasics bind:this={basics} initial={{ name: initial.name, color: initial.color, prompt: initial.prompt }}
  prompts={S.prompts} {isNew} {active} onedit={edit} />
<ProfileBrain bind:this={brain} {initial} {profile} {engines} {ttsProvider} {engineReady}
  onedit={edit} onchange={mark} onrefreshvoices={() => voice?.refreshEleven()} />
<ProfileVoice bind:this={voice} initial={{
  voice: initial.voice, ttsVoice: initial.tts_voice,
  ttsProvider: initial.engine === "eleven_realtime" ? "elevenlabs" : initial.tts_engine,
  volume: initial.volume,
}} onedit={edit} {previewModels} />
<ProfileLighting initial={initial.led} previewAvailable={Boolean(S.speaker.leds)}
  globallyEnabled={S.settings.led?.enabled !== false} onedit={editLighting} />

<div class="savebar">
  <button class="btn primary" type="button" id="p-save" disabled={Boolean(working)} onclick={save}>{isNew ? "Создать" : "Сохранить"}</button>
  {#if !isNew && !active}
    <button class="btn" type="button" id="p-activate" disabled={Boolean(working)} onclick={activate}
      data-tip="Этот ассистент будет отвечать на «Джарвис»">Сделать активным</button>
    <button class="btn ghost danger" type="button" id="p-del" disabled={Boolean(working)} onclick={remove}>Удалить</button>
  {/if}
  <span class="dirty" class:hidden={!dirty} id="dirty">есть несохранённые изменения</span>
</div>
