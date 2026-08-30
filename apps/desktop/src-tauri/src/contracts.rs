use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningMode {
    Disabled,
    Armed,
    QuietHours,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputState {
    Stopped,
    WakeListening,
    Capturing,
    Recognizing,
    Recovering,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    Idle,
    Preparing,
    Active,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenerationState {
    NotStarted,
    Streaming,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Idle,
    Buffering,
    Playing,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvatarSemanticState {
    Idle,
    Listening,
    Thinking,
    Speaking,
    Sleeping,
    NeedsInput,
    Ready,
    Recovering,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Pending,
    Streaming,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub turn_id: String,
    pub role: MessageRole,
    pub canonical_text: String,
    pub status: MessageStatus,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionProviderSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_vision_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_vision_model")]
    pub model: String,
    #[serde(default)]
    pub has_api_key: bool,
}

impl Default for VisionProviderSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_vision_endpoint(),
            model: default_vision_model(),
            has_api_key: std::env::var("DESKTOP_AI_VISION_API_KEY").is_ok(),
        }
    }
}

fn default_vision_endpoint() -> String {
    "https://api.openai.com/v1".into()
}

fn default_vision_model() -> String {
    "gpt-4.1-mini".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub has_api_key: bool,
    #[serde(default)]
    pub allow_memory_context: bool,
    #[serde(default)]
    pub vision: VisionProviderSettings,
    pub revision: u64,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            provider: "mock".into(),
            endpoint: "https://api.openai.com/v1".into(),
            model: "gpt-4.1-mini".into(),
            has_api_key: std::env::var("DESKTOP_AI_LLM_API_KEY").is_ok(),
            allow_memory_context: false,
            vision: VisionProviderSettings::default(),
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PetSettings {
    pub visible: bool,
    pub click_through: bool,
    pub scale: f64,
    pub revision: u64,
}

impl Default for PetSettings {
    fn default() -> Self {
        Self {
            visible: true,
            click_through: false,
            scale: 1.0,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterSettings {
    pub character_id: String,
    pub display_name: String,
    pub model_path: Option<String>,
    pub model_file_name: Option<String>,
    pub model_size_bytes: Option<u64>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryItem {
    pub id: String,
    pub scope: String,
    pub character_id: Option<String>,
    pub kind: String,
    pub content: String,
    pub importance: f64,
    pub version: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCandidate {
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub content: String,
    pub confidence: f64,
    pub sensitivity: String,
    pub status: String,
    pub created_at_ms: i64,
    pub reviewed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataSummary {
    pub message_count: u64,
    pub memory_count: u64,
    pub memory_candidate_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoicePermissions {
    pub microphone: String,
    pub speech_recognition: String,
}

impl Default for VoicePermissions {
    fn default() -> Self {
        Self {
            microphone: "not_determined".into(),
            speech_recognition: "not_determined".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceSettings {
    pub tts_enabled: bool,
    pub revision: u64,
}

impl Default for CharacterSettings {
    fn default() -> Self {
        Self {
            character_id: "aris".into(),
            display_name: "爱丽丝".into(),
            model_path: None,
            model_file_name: None,
            model_size_bytes: None,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolApproval {
    pub tool_call_id: String,
    pub tool_name: String,
    pub display_name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimerItem {
    pub id: String,
    pub title: String,
    pub due_at_ms: i64,
    pub timezone: String,
    pub original_request: String,
    pub status: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimerHistoryItem {
    pub id: String,
    pub title: String,
    pub due_at_ms: i64,
    pub status: String,
    pub completed_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolRunSummary {
    pub id: String,
    pub tool_name: String,
    pub origin: String,
    pub status: String,
    pub result_summary: Option<String>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenObservation {
    pub id: String,
    pub captured_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenObservationPreview {
    pub id: String,
    pub captured_at_ms: i64,
    pub data_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub diagnostic_id: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            diagnostic_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub revision: u64,
    pub listening_mode: ListeningMode,
    pub input_state: InputState,
    pub turn_state: TurnState,
    pub generation_state: GenerationState,
    pub playback_state: PlaybackState,
    pub avatar_state: AvatarSemanticState,
    pub active_turn_id: Option<String>,
    pub generation: u64,
    pub conversation_id: String,
    pub messages: Vec<ChatMessage>,
    pub provider: ProviderSettings,
    pub pet: PetSettings,
    pub character: CharacterSettings,
    pub audio_host_state: String,
    pub voice_permissions: VoicePermissions,
    pub voice_settings: VoiceSettings,
    pub asr_partial: String,
    pub voice_input_route: String,
    pub mouth_level: f64,
    pub pending_tool_approval: Option<ToolApproval>,
    pub timers: Vec<TimerItem>,
    pub timer_history: Vec<TimerHistoryItem>,
    pub tool_history: Vec<ToolRunSummary>,
    pub screen_observation: Option<ScreenObservation>,
    pub last_error: Option<AppError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventEnvelope<T: Serialize> {
    pub protocol_version: &'static str,
    pub event_id: String,
    pub event_seq: u64,
    pub snapshot_revision: u64,
    pub occurred_at: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    pub payload: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitTextRequest {
    pub request_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelTurnRequest {
    pub request_id: String,
    pub turn_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextDeltaPayload {
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MouthLevelPayload {
    pub level: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnEndedPayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateVisionProviderRequest {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProviderRequest {
    pub request_id: String,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    #[serde(default)]
    pub allow_memory_context: bool,
    #[serde(default)]
    pub vision: UpdateVisionProviderRequest,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePetSettingsRequest {
    pub request_id: String,
    pub visible: bool,
    pub click_through: bool,
    pub scale: f64,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchMemoriesRequest {
    pub request_id: String,
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_memory_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemoryRequest {
    pub request_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMemoryRequest {
    pub request_id: String,
    pub id: String,
    pub content: String,
    pub expected_version: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteMemoryRequest {
    pub request_id: String,
    pub id: String,
    pub expected_version: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewMemoryCandidateRequest {
    pub request_id: String,
    pub candidate_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMemoryCandidateRequest {
    pub request_id: String,
    pub candidate_id: String,
    pub content: String,
}

fn default_memory_limit() -> usize {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDecisionRequest {
    pub request_id: String,
    pub tool_call_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelTimerRequest {
    pub request_id: String,
    pub timer_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreenObservationRequest {
    pub request_id: String,
    pub observation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetListeningRequest {
    pub request_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateVoiceSettingsRequest {
    pub request_id: String,
    pub tts_enabled: bool,
    pub expected_revision: u64,
}
