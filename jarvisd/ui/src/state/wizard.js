import { writable } from "svelte/store";

export const wizardRequest = writable(null);

export function wizOpen(step = 0) {
  wizardRequest.set({ step });
}

export function closeWizard() {
  wizardRequest.set(null);
}
