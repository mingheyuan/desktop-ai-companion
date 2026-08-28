import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSnapshot, EventEnvelope, TextDeltaPayload, TurnEndedPayload } from "../contracts";

export const isTauri = () => typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

export async function getSnapshot(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("app_get_snapshot");
}

export async function submitText(text: string): Promise<void> {
  await invoke("chat_submit_text", {
    request: { request_id: crypto.randomUUID(), text },
  });
}

export async function cancelTurn(turnId: string): Promise<void> {
  await invoke("turn_cancel", {
    request: { request_id: crypto.randomUUID(), turn_id: turnId, reason: "user" },
  });
}

export async function openPanel(): Promise<void> {
  await invoke("pet_open_panel");
}

export interface CoreEventHandlers {
  snapshot: (event: EventEnvelope<AppSnapshot>) => void;
  delta: (event: EventEnvelope<TextDeltaPayload>) => void;
  ended: (event: EventEnvelope<TurnEndedPayload>) => void;
}

export async function listenToCoreEvents(handlers: CoreEventHandlers): Promise<UnlistenFn> {
  const unlisteners = await Promise.all([
    listen<EventEnvelope<AppSnapshot>>("state:changed", ({ payload }) => handlers.snapshot(payload)),
    listen<EventEnvelope<TextDeltaPayload>>("turn:text-delta", ({ payload }) => handlers.delta(payload)),
    listen<EventEnvelope<TurnEndedPayload>>("turn:ended", ({ payload }) => handlers.ended(payload)),
  ]);
  return () => unlisteners.forEach((unlisten) => unlisten());
}
