import { api } from "./core.js";
import { showGate } from "#state/panelState.js";

export const PROFILES = "/api/train/wake/profiles";
const ACTIVE = "/api/train/wake/active";

export function modelLabel(profile) {
  return profile.id ? profile.phrase : profile.phrase + " (по умолчанию)";
}

export function loadWakeProfiles(signal) {
  return api("GET", PROFILES, undefined, { signal });
}

export function selectWakeProfile(id) {
  return api("PUT", ACTIVE, { id });
}

async function capture(signal) {
  const response = await fetch("/api/train/wake/capture", {
    method: "POST", credentials: "same-origin", headers: { "X-Jarvis": "1" }, signal,
  });
  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    signal.throwIfAborted();
    if (response.status === 401 || response.status === 428) showGate(data.setup);
    throw new Error(data.error || "Не удалось записать фразу");
  }
  return response.arrayBuffer();
}

export async function recordWakeSample(signal, preparation, speak) {
  await new Promise((resolve) => setTimeout(resolve, preparation));
  signal.throwIfAborted();
  const prompt = setTimeout(() => { if (!signal.aborted) speak(); }, 250);
  try {
    return await capture(signal);
  } finally {
    clearTimeout(prompt);
  }
}

export function wakeSampleUrl(pcm) {
  const header = new ArrayBuffer(44);
  const wav = new DataView(header);
  const tag = (at, text) => [...text].forEach((c, i) => wav.setUint8(at + i, c.charCodeAt(0)));
  tag(0, "RIFF");
  wav.setUint32(4, 36 + pcm.byteLength, true);
  tag(8, "WAVEfmt ");
  wav.setUint32(16, 16, true);
  wav.setUint16(20, 1, true);
  wav.setUint16(22, 1, true);
  wav.setUint32(24, 16000, true);
  wav.setUint32(28, 32000, true);
  wav.setUint16(32, 2, true);
  wav.setUint16(34, 16, true);
  tag(36, "data");
  wav.setUint32(40, pcm.byteLength, true);
  return URL.createObjectURL(new Blob([header, pcm], { type: "audio/wav" }));
}
