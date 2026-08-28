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
pub struct ProviderSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub has_api_key: bool,
    pub revision: u64,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            provider: "mock".into(),
            endpoint: "https://api.openai.com/v1".into(),
            model: "gpt-4.1-mini".into(),
            has_api_key: std::env::var("DESKTOP_AI_LLM_API_KEY").is_ok(),
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
pub struct TurnEndedPayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
