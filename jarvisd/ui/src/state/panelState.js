import { writable } from "svelte/store";
import { api, esc } from "#helpers/core.js";

export let S = null;
export let journal = [];

export const speaker = writable(null);
export const access = writable({ status: "loading" });
export const routeRequests = writable(0);

export function showGate(setup) {
  access.set({ status: "gate", setup: Boolean(setup) });
}

export function refreshRoute() {
  routeRequests.update((revision) => revision + 1);
}

export async function reload(signal) {
  const state = await api("GET", "/api/state", undefined, { signal });
  state.live_voices ||= [];
  state.prompts ||= [];
  state.profiles ||= [];
  state.security ||= [];
  state.gemini_models ||= [];
  state.gemini_voice_groups ||= [];
  state.chatgpt_voice_groups ||= [];
  state.grok_voice_groups ||= [];
  state.chatgpt ||= {};
  state.sys ||= {};
  S = state;
  speaker.set(state);
}

export async function readJournal(signal) {
  const data = await api("GET", "/api/journal", undefined, { signal });
  if (data.version && S.version && data.version !== S.version) {
    location.reload();
    return false;
  }

  journal = data.items || [];
  S.speaker.mic = data.mic;
  S.speaker.mic_why = data.mic_why || "";
  S.speaker.talking = data.talking;
  S.speaker.phase = data.phase || "";
  if (data.volume >= 0) S.speaker.volume = data.volume;
  S.sys = data.sys || {};
  return true;
}

// Что сейчас делает колонка: вид кольца, заголовок и пояснение.
export function speakerState() {
  const sp = S.speaker, word = S.settings.wake_word || "Джарвис";
  if (sp.talking) return { ring: "talk", title: "Идёт разговор", text: "Говорите — пока разговор открыт, звать по имени не нужно" };
  if (!sp.mic) return { ring: "off", title: "Микрофон не подключён", text: sp.mic_why ? `Колонка не присылает звук: ${sp.mic_why}. Подробности — в «Журнале».` : "Колонка не присылает звук. Проверьте, что она включена и установка завершилась." };
  if (sp.wake_model && S.settings.wake_enabled) return { ring: "idle", title: `Жду фразу «${word}»`, text: `Скажите «${word}, …» или нажмите «Поговорить»` };
  return { ring: "mute", title: "Жду кнопку «Поговорить»", text: S.settings.wake_enabled ? `Модель для «${word}» ещё не установлена` : `Отклик на «${word}» выключен` };
}

export const ORB_TEXT = { idle: "Колонка ждёт фразу активации", listening: "Слушает", thinking: "Думает", speaking: "Говорит", off: "Нет звука с колонки", mute: "На связи, голосовая активация недоступна" };
export function readOrb() {
  const st = speakerState(), ph = S.speaker.phase;
  const o = st.ring === "off" ? "off" : S.speaker.talking ? (ph || "listening") : st.ring === "mute" ? "mute" : "idle";
  return { state: o, text: ORB_TEXT[o] };
}

// ---------------------------------------------------------------- сайдбар
export const avatar = (p, big = false) => `<span class="av c${p.color || 1}${big ? " big" : ""}">${esc((p.name || "?").trim()[0] || "?")}</span>`;
export const voiceName = (id) => (S.live_voices.find((v) => v.id === id)?.name) || id || "cedar";
const BUILTIN_VOICE = ["chatgpt", "openai", "gemini", "grok", "elevenlabs"];
export const voicePlugin = (engine) => ({ live: "openai", eleven_realtime: "elevenlabs" })[engine] || engine;
export const voiceEnabled = (engine) => {
  if (!engine) return false;
  const id = voicePlugin(engine);
  const builtin = S.plugins?.builtin || BUILTIN_VOICE;
  return builtin.includes(id) || !!S.plugins?.enabled?.includes(id);
};
export const isClassicProfile = (p) => !!p && p.engine !== "eleven_realtime" && (p.mode === "classic" || p.engine === "elevenlabs" || p.engine === "classic");
export const sttReady = (id) => ["openai", "elevenlabs"].includes(id) ? voiceEnabled(id) : (S.stt_plugins || []).some((x) => x.id === id);
export const ttsReady = (id) => ["openai", "elevenlabs"].includes(id) ? voiceEnabled(id) : (S.tts_plugins || []).some((x) => x.id === id);
export const classicEnabled = () =>
  (voiceEnabled("openai") || voiceEnabled("elevenlabs") || (S.stt_plugins || []).length > 0)
  && (voiceEnabled("openai") || voiceEnabled("elevenlabs") || (S.tts_plugins || []).length > 0);
export const engineName = (p) => {
  if (isClassicProfile(p)) {
    const stt = p.stt_engine || "elevenlabs";
    const tts = p.tts_engine || (p.engine === "openai" ? "openai" : "elevenlabs");
    if (!sttReady(stt) || !ttsReady(tts)) return "Плагин пайплайна выключен";
    return stt === "elevenlabs" && tts === "elevenlabs" ? "ElevenLabs" : "Пайплайн";
  }
  if (!p.engine) return "Голос не настроен";
  if (!voiceEnabled(p.engine)) return "Голосовой плагин выключен";
  return ({ openai: "GPT-Live", live: "GPT-Live", gemini: "Gemini Live", chatgpt: "GPT-Live по подписке", grok: "Grok Voice", elevenlabs: "ElevenLabs", eleven_realtime: "ElevenLabs Live" })[p.engine] || "Голосовой плагин";
};

export function readSidebar(state) {
  const profiles = state.profiles.map((p) => {
    const classic = isClassicProfile(p);
    const tts = p.tts_engine || (p.engine === "openai" ? "openai" : "elevenlabs");
    const showVoice = classic ? tts === "openai" && ttsReady(tts) : voiceEnabled(p.engine);
    const sideVoice = showVoice && p.voice ? voiceName(p.voice) : "";
    return {
      id: p.id, name: p.name, initial: (p.name || "?").trim()[0] || "?", color: p.color || 1,
      detail: engineName(p) + (sideVoice ? " · " + sideVoice : ""), active: p.id === state.active,
    };
  });
  return { profiles, version: state.version, training: Boolean(state.settings.wake_training), updateAvailable: Boolean(state.update_available) };
}
