import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSnapshot, CharacterSettings, DataSummary, EventEnvelope, MemoryCandidate, MemoryItem, MemorySettings, MouthLevelPayload, PetSettings, ProviderSettings, ScreenUnderstandingMode, TextDeltaPayload, TurnEndedPayload, VoiceSettings } from "../contracts";

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

export interface UpdateProviderInput {
  provider: ProviderSettings["provider"];
  endpoint: string;
  model: string;
  api_key?: string;
  clear_api_key: boolean;
  allow_memory_context: boolean;
  vision: {
    enabled: boolean;
    endpoint: string;
    model: string;
    api_key?: string;
    clear_api_key: boolean;
  };
  expected_revision: number;
}

export async function updateProvider(input: UpdateProviderInput): Promise<ProviderSettings> {
  return invoke<ProviderSettings>("settings_update_provider", {
    request: { request_id: crypto.randomUUID(), ...input },
  });
}

export async function testProvider(): Promise<string> {
  return invoke<string>("settings_test_provider", {
    request: { request_id: crypto.randomUUID() },
  });
}

export interface UpdatePetSettingsInput {
  visible: boolean;
  click_through: boolean;
  scale: number;
  expected_revision: number;
}

export async function updatePetSettings(input: UpdatePetSettingsInput): Promise<PetSettings> {
  return invoke<PetSettings>("pet_update_settings", {
    request: { request_id: crypto.randomUUID(), ...input },
  });
}

export async function importCharacterVrm(): Promise<CharacterSettings | null> {
  return invoke<CharacterSettings | null>("character_import_vrm");
}

export function characterModelUrl(settings: CharacterSettings): string | undefined {
  if (!settings.model_path || !isTauri()) return undefined;
  return convertFileSrc(settings.model_path);
}

export async function searchMemories(query = "", limit = 20): Promise<MemoryItem[]> {
  return invoke<MemoryItem[]>("memory_search", {
    request: { request_id: crypto.randomUUID(), query, limit },
  });
}

export async function listMemoryCandidates(): Promise<MemoryCandidate[]> {
  return invoke<MemoryCandidate[]>("memory_list_candidates", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function getMemorySettings(): Promise<MemorySettings> {
  return invoke<MemorySettings>("memory_get_settings", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function updateMemorySettings(
  automaticCandidatesEnabled: boolean,
  expectedRevision: number,
): Promise<MemorySettings> {
  return invoke<MemorySettings>("memory_update_settings", {
    request: {
      request_id: crypto.randomUUID(),
      automatic_candidates_enabled: automaticCandidatesEnabled,
      expected_revision: expectedRevision,
    },
  });
}

export async function reviewMemoryCandidate(
  candidateId: string,
  approved: boolean,
  resolution?: "replace" | "keep_both",
): Promise<MemoryItem | null> {
  return invoke<MemoryItem | null>("memory_review_candidate", {
    request: {
      request_id: crypto.randomUUID(),
      candidate_id: candidateId,
      approved,
      resolution,
    },
  });
}

export async function updateMemoryCandidate(candidateId: string, content: string): Promise<MemoryCandidate> {
  return invoke<MemoryCandidate>("memory_update_candidate", {
    request: {
      request_id: crypto.randomUUID(),
      candidate_id: candidateId,
      content,
    },
  });
}

export async function createMemory(content: string): Promise<MemoryItem> {
  return invoke<MemoryItem>("memory_create", {
    request: { request_id: crypto.randomUUID(), content },
  });
}

export async function updateMemory(memory: MemoryItem, content: string): Promise<MemoryItem> {
  return invoke<MemoryItem>("memory_update", {
    request: {
      request_id: crypto.randomUUID(),
      id: memory.id,
      content,
      expected_version: memory.version,
    },
  });
}

export async function deleteMemory(memory: MemoryItem): Promise<void> {
  await invoke("memory_delete", {
    request: {
      request_id: crypto.randomUUID(),
      id: memory.id,
      expected_version: memory.version,
    },
  });
}

export async function getDataSummary(): Promise<DataSummary> {
  return invoke<DataSummary>("data_get_summary", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function clearConversationData(): Promise<DataSummary> {
  return invoke<DataSummary>("data_clear_conversation", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function clearMemoryData(): Promise<DataSummary> {
  return invoke<DataSummary>("data_clear_memories", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function requestVoicePermissions(): Promise<void> {
  await invoke("voice_request_permissions", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function openVoicePrivacySettings(): Promise<void> {
  await invoke("voice_open_privacy_settings", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function setListening(enabled: boolean): Promise<void> {
  await invoke("voice_set_listening_mode", {
    request: { request_id: crypto.randomUUID(), enabled },
  });
}

export async function startPushToTalk(): Promise<void> {
  await invoke("voice_ptt_start", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function finishPushToTalk(): Promise<void> {
  await invoke("voice_ptt_finish", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function stopPlayback(): Promise<void> {
  await invoke("playback_stop", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function decideTool(toolCallId: string, approved: boolean): Promise<void> {
  await invoke("tool_decide", {
    request: {
      request_id: crypto.randomUUID(),
      tool_call_id: toolCallId,
      approved,
    },
  });
}

export async function cancelTimer(timerId: string): Promise<void> {
  await invoke("timer_cancel", {
    request: {
      request_id: crypto.randomUUID(),
      timer_id: timerId,
    },
  });
}

export interface ScreenObservationPreview {
  id: string;
  captured_at_ms: number;
  data_url: string;
  ocr_text?: string;
}

export async function getScreenObservationPreview(observationId: string): Promise<ScreenObservationPreview> {
  return invoke<ScreenObservationPreview>("screen_get_observation_preview", {
    request: {
      request_id: crypto.randomUUID(),
      observation_id: observationId,
    },
  });
}

export async function clearScreenObservation(): Promise<void> {
  await invoke("screen_clear_observation", {
    request: { request_id: crypto.randomUUID() },
  });
}

export async function setScreenUnderstandingMode(mode: ScreenUnderstandingMode): Promise<ScreenUnderstandingMode> {
  return invoke<ScreenUnderstandingMode>("screen_set_understanding_mode", {
    request: { request_id: crypto.randomUUID(), mode },
  });
}

export async function updateVoiceSettings(current: VoiceSettings, ttsEnabled: boolean): Promise<VoiceSettings> {
  return invoke<VoiceSettings>("voice_update_settings", {
    request: {
      request_id: crypto.randomUUID(),
      tts_enabled: ttsEnabled,
      expected_revision: current.revision,
    },
  });
}

export interface CoreEventHandlers {
  snapshot: (event: EventEnvelope<AppSnapshot>) => void;
  delta: (event: EventEnvelope<TextDeltaPayload>) => void;
  ended: (event: EventEnvelope<TurnEndedPayload>) => void;
  mouthLevel: (event: EventEnvelope<MouthLevelPayload>) => void;
}

export async function listenToCoreEvents(handlers: CoreEventHandlers): Promise<UnlistenFn> {
  const unlisteners = await Promise.all([
    listen<EventEnvelope<AppSnapshot>>("state:changed", ({ payload }) => handlers.snapshot(payload)),
    listen<EventEnvelope<TextDeltaPayload>>("turn:text-delta", ({ payload }) => handlers.delta(payload)),
    listen<EventEnvelope<TurnEndedPayload>>("turn:ended", ({ payload }) => handlers.ended(payload)),
    listen<EventEnvelope<MouthLevelPayload>>("avatar:mouth-level", ({ payload }) => handlers.mouthLevel(payload)),
  ]);
  return () => unlisteners.forEach((unlisten) => unlisten());
}
