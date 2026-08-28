export type ListeningMode = "disabled" | "armed" | "quiet_hours";
export type InputState = "stopped" | "wake_listening" | "capturing" | "recognizing" | "recovering";
export type TurnState = "idle" | "preparing" | "active" | "finalizing" | "completed" | "cancelled" | "failed";
export type GenerationState = "not_started" | "streaming" | "finished" | "failed";
export type PlaybackState = "idle" | "buffering" | "playing" | "stopping" | "failed";
export type AvatarSemanticState =
  | "idle"
  | "listening"
  | "thinking"
  | "speaking"
  | "sleeping"
  | "needs_input"
  | "ready"
  | "recovering"
  | "error";

export interface ChatMessage {
  id: string;
  turn_id: string;
  role: "user" | "assistant";
  canonical_text: string;
  status: "pending" | "streaming" | "completed" | "cancelled" | "failed";
  created_at_ms: number;
}

export interface ProviderSettings {
  provider: "mock" | "openai_compatible";
  endpoint: string;
  model: string;
  has_api_key: boolean;
  revision: number;
}

export interface AppSnapshot {
  revision: number;
  listening_mode: ListeningMode;
  input_state: InputState;
  turn_state: TurnState;
  generation_state: GenerationState;
  playback_state: PlaybackState;
  avatar_state: AvatarSemanticState;
  active_turn_id: string | null;
  generation: number;
  conversation_id: string;
  messages: ChatMessage[];
  provider: ProviderSettings;
  last_error: AppError | null;
}

export interface AppError {
  code: string;
  message: string;
  retryable: boolean;
  diagnostic_id: string;
}

export interface EventEnvelope<T> {
  protocol_version: "1.0";
  event_id: string;
  event_seq: number;
  snapshot_revision: number;
  occurred_at: string;
  type: string;
  turn_id?: string;
  generation?: number;
  payload: T;
}

export interface TextDeltaPayload {
  text: string;
}

export interface TurnEndedPayload {
  status: "completed" | "cancelled" | "failed";
  reason?: string;
}
