import { isClassicProfile, voiceEnabled } from "#state/panelState.js";

export const builtInLlm = ["", "openai", "chatgpt", "gemini", "grok"];
export const usesOpenAiModel = (engine) => ["", "openai", "chatgpt"].includes(engine);

export function createProfile(base, state) {
  const profile = structuredClone(base || {
    id: "", name: "", color: state.profiles.length % 6 + 1, mode: "live", engine: "", voice: "",
    effort: "low", priority: true, web_search: true, idle_s: 8, prompt: state.prompts[0]?.id || "",
  });
  profile.led ||= { ...state.settings.led };
  profile.led.brightness ||= 45;
  profile.volume ||= state.settings.volume || 40;
  profile.city ??= state.settings.city || "";
  profile.timezone ||= state.settings.timezone || "UTC";
  if (profile.engine === "live") profile.engine = "openai";

  if (isClassicProfile(profile)) {
    profile.tts_engine ||= profile.engine === "openai" ? "openai" : "elevenlabs";
    if (profile.tts_engine === "openai") profile.tts_voice ||= profile.voice || "alloy";
    profile.engine = "classic";
    profile.stt_engine ||= "elevenlabs";
  } else {
    const builtin = ["elevenlabs", "openai"].find(voiceEnabled);
    profile.stt_engine ||= builtin || state.stt_plugins?.[0]?.id || "elevenlabs";
    profile.tts_engine ||= builtin || state.tts_plugins?.[0]?.id || "elevenlabs";
  }
  profile.llm_engine ||= profile.llm_plugin || "";
  profile.llm_model ||= profile.backend_model || "";
  if (profile.engine === "classic" && usesOpenAiModel(profile.llm_engine)) {
    profile.backend_model = profile.llm_model || state.backend_models[0] || "";
  }
  if (!state.prompts.some(({ id }) => id === profile.prompt)) profile.prompt = state.prompts[0]?.id || "";
  return profile;
}

export function profilePayload(profile, models) {
  const payload = { ...profile, name: profile.name.trim(), idle_s: models.idle || 8, effort: models.effort };
  if (profile.engine === "classic") {
    payload.mode = "classic";
    payload.stt_model = models.stt;
    payload.tts_model = models.tts;
    payload.live_model = "";
    payload.llm_plugin = builtInLlm.includes(profile.llm_engine) ? "" : profile.llm_engine;
    if (usesOpenAiModel(profile.llm_engine)) {
      payload.llm_model = payload.backend_model = models.backend;
    } else {
      payload.llm_model = models.customLlm;
    }
    if (profile.tts_engine === "openai") payload.voice = payload.tts_voice = profile.tts_voice || "alloy";
    else if (profile.tts_engine !== "elevenlabs") payload.voice = profile.tts_voice;
  } else {
    payload.mode = "live";
    payload.llm_model = payload.backend_model = models.backend;
    payload.llm_engine = payload.llm_plugin = profile.llm_plugin || "";
    payload.live_model = models.live;
  }
  const needsElevenVoice = profile.engine === "eleven_realtime" || (profile.engine === "classic" && profile.tts_engine === "elevenlabs");
  if (needsElevenVoice && !payload.tts_voice) throw new Error("Выберите голос ElevenLabs");
  return payload;
}
