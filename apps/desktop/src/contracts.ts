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

export interface VisionProviderSettings {
  enabled: boolean;
  endpoint: string;
  model: string;
  has_api_key: boolean;
}

export interface ProviderSettings {
  provider: "mock" | "openai_compatible";
  endpoint: string;
  model: string;
  has_api_key: boolean;
  allow_memory_context: boolean;
  vision: VisionProviderSettings;
  revision: number;
}

export interface MemoryItem {
  id: string;
  scope: "user" | "relationship";
  character_id: string | null;
  kind: string;
  content: string;
  importance: number;
  version: number;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface MemoryCandidate {
  id: string;
  scope: string;
  kind: string;
  content: string;
  confidence: number;
  sensitivity: string;
  status: string;
  created_at_ms: number;
  reviewed_at_ms: number | null;
  /** Exact normalized match found in an existing memory or earlier pending candidate. */
  duplicate_of?: string;
  duplicate_content?: string;
}

export interface DataSummary {
  message_count: number;
  memory_count: number;
  memory_candidate_count: number;
}

export interface VoicePermissions {
  microphone: "not_determined" | "denied" | "restricted" | "authorized";
  speech_recognition: "not_determined" | "denied" | "restricted" | "authorized";
}

export interface VoiceSettings {
  tts_enabled: boolean;
  revision: number;
}

export interface PetSettings {
  visible: boolean;
  click_through: boolean;
  scale: number;
  revision: number;
}

export interface CharacterSettings {
  character_id: string;
  display_name: string;
  model_path: string | null;
  model_file_name: string | null;
  model_size_bytes: number | null;
  revision: number;
}

export interface ToolApproval {
  tool_call_id: string;
  tool_name: string;
  display_name: string;
  summary: string;
  url?: string;
  host?: string;
  title?: string;
  due_at_ms?: number;
  expires_at_ms: number;
}

export interface TimerItem {
  id: string;
  title: string;
  due_at_ms: number;
  timezone: string;
  original_request: string;
  status: string;
  created_at_ms: number;
}

export interface TimerHistoryItem {
  id: string;
  title: string;
  due_at_ms: number;
  status: "fired" | "cancelled" | string;
  completed_at_ms: number | null;
  created_at_ms: number;
}

export interface ToolRunSummary {
  id: string;
  tool_name: string;
  origin: string;
  status: string;
  result_summary: string | null;
  created_at_ms: number;
  completed_at_ms: number | null;
}

export interface ScreenObservation {
  id: string;
  captured_at_ms: number;
  expires_at_ms: number;
  scope: "all_displays" | "main_display";
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
  pet: PetSettings;
  character: CharacterSettings;
  audio_host_state: "stopped" | "launching" | "handshaking" | "ready" | "failed";
  voice_permissions: VoicePermissions;
  voice_settings: VoiceSettings;
  asr_partial: string;
  voice_input_route: "unknown" | "on_device" | "system_service";
  mouth_level: number;
  pending_tool_approval: ToolApproval | null;
  timers: TimerItem[];
  timer_history: TimerHistoryItem[];
  tool_history: ToolRunSummary[];
  screen_observation: ScreenObservation | null;
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

export interface MouthLevelPayload {
  level: number;
}

export interface TurnEndedPayload {
  status: "completed" | "cancelled" | "failed";
  reason?: string;
}
