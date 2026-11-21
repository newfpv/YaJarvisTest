import { classicEnabled, voiceEnabled } from "#state/panelState.js";
import { usesOpenAiModel } from "../profileDraft.js";

const engines = [
  { id: "classic", name: "Свой пайплайн (STT → LLM → TTS)", description: "Соберите цепочку из разных провайдеров и плагинов: распознавание речи, текстовая модель (включая OpenRouter / плагины) и озвучка" },
  { id: "eleven_realtime", name: "ElevenLabs — живой разговор", description: "Eleven Realtime Mini или Max: распознавание, мозг, голос и перебивания вместе. Нужен только ключ ElevenLabs" },
  { id: "chatgpt", name: "GPT-Live по подписке", description: "Ваш ChatGPT Plus или Pro: минуты не оплачиваются, есть лимиты подписки. Нужен вход в «Подключениях»" },
  { id: "openai", name: "GPT-Live", description: "OpenAI по ключу API. Около 5 центов за минуту разговора" },
  { id: "gemini", name: "Gemini Live", description: "Google. Примерно вдвое дешевле, есть бесплатный уровень. Нужен ключ Gemini в «Подключениях»" },
  { id: "grok", name: "Grok Voice", description: "xAI. Около 5 центов за минуту, сам ищет в интернете. Нужен ключ xAI в «Подключениях»" },
];

const sttModels = {
  elevenlabs: [
    ["scribe_v3_live", "Scribe v3 Live — определяет конец мысли"],
    ["scribe_v2_realtime", "Scribe v2 Realtime"],
  ],
  openai: [
    ["gpt-4o-mini-transcribe", "GPT-4o mini Transcribe"],
    ["gpt-4o-transcribe", "GPT-4o Transcribe"],
    ["whisper-1", "Whisper v1"],
  ],
};

const ttsModels = {
  elevenlabs: [
    "eleven_v4_turbo", "eleven_v4", "eleven_v3_conversational", "eleven_v3", "eleven_flash_v2_5", "eleven_multilingual_v2",
  ].map((id) => [id, id]),
  openai: [
    ["gpt-4o-mini-tts", "GPT-4o mini TTS"], ["tts-1", "TTS-1"], ["tts-1-hd", "TTS-1 HD"],
  ],
};

const realtimeModels = [
  ["eleven_realtime_v1_mini", "Realtime v1 Mini — быстрый разговор"],
  ["eleven_realtime_v1_max", "Realtime v1 Max — сложные задачи и инструменты"],
];
const defaultStt = { elevenlabs: "scribe_v2_realtime", openai: "gpt-4o-mini-transcribe" };
const defaultTts = { elevenlabs: "eleven_flash_v2_5", openai: "gpt-4o-mini-tts" };
const textProviders = [
  ["openai", "OpenAI"], ["chatgpt", "ChatGPT по подписке"], ["gemini", "Google Gemini"], ["grok", "xAI Grok"],
];
const defaultTextProvider = ["", "Автовыбор (OpenAI / Gemini / Grok / ChatGPT)"];

export const modelKind = (provider) => provider === "elevenlabs" || provider === "openai" ? provider : "custom";

export function availableEngines() {
  return engines.filter(({ id }) => id === "classic" ? classicEnabled() : voiceEnabled(id));
}

function optionsWithCurrent(options, current, label = current) {
  const items = new Map(options);
  if (current && !items.has(current)) items.set(current, label);
  return [...items];
}

function providerOptions(builtins, plugins = [], current) {
  const options = builtins.flatMap(([id, name]) => {
    if (voiceEnabled(id)) return [[id, name]];
    return current === id ? [[id, `${name} (отключён)`]] : [];
  });
  return optionsWithCurrent([...options, ...plugins.map(({ id, name }) => [id, name])], current, `${current} (отключён)`);
}

function modelOptions(models, defaults, provider, current) {
  const options = {};
  const values = { custom: modelKind(provider) === "custom" ? current || "" : "" };
  for (const [id, list] of Object.entries(models)) {
    const saved = provider === id ? current : "";
    options[id] = optionsWithCurrent(list, saved);
    values[id] = saved || defaults[id];
  }
  return { options, values };
}

function chosen(options, value) {
  return options.some(([id]) => id === value) ? value : options[0]?.[0] || "";
}

export function brainOptions(state, profile) {
  const gemini = state.gemini_model_info?.length
    ? state.gemini_model_info.map(({ id, name, note }) => [id, note ? `${name} — ${note}` : name])
    : state.gemini_models.map((id) => [id, id]);
  const backend = state.backend_models.map((id) => [id, id]);
  const stt = modelOptions(sttModels, defaultStt, profile.stt_engine, profile.stt_model);
  const tts = modelOptions(ttsModels, defaultTts, profile.tts_engine, profile.tts_model);
  return {
    providers: {
      stt: providerOptions([["elevenlabs", "ElevenLabs Scribe"], ["openai", "OpenAI"]], state.stt_plugins, profile.stt_engine),
      llm: [defaultTextProvider, ...providerOptions(textProviders, state.text_plugins, profile.llm_engine)],
      tts: providerOptions([["elevenlabs", "ElevenLabs"], ["openai", "OpenAI"]], state.tts_plugins, profile.tts_engine),
      text: optionsWithCurrent((state.text_plugins || []).map(({ id, name }) => [id, name]),
        profile.llm_plugin, `${profile.llm_plugin} (отключён)`),
    },
    models: { backend, gemini, realtime: realtimeModels, stt: stt.options, tts: tts.options },
    initial: {
      backend: chosen(backend, profile.backend_model),
      gemini: chosen(gemini, profile.live_model),
      realtime: chosen(realtimeModels, profile.live_model),
      stt: stt.values,
      tts: tts.values,
      customLlm: usesOpenAiModel(profile.llm_engine) ? "" : profile.llm_model,
    },
  };
}
