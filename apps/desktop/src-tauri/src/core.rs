use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use time::format_description::well_known::Rfc3339;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    contracts::{
        AppError, AppSnapshot, AvatarSemanticState, EventEnvelope, GenerationState, InputState,
        ListeningMode, PetSettings, PlaybackState, ProviderSettings, TextDeltaPayload,
        TurnEndedPayload, TurnState,
    },
    desktop::apply_pet_window_settings,
    provider::{create_provider, LlmEvent, LlmRequest},
    secret::SecretStore,
    storage::Storage,
};

struct RuntimeState {
    revision: u64,
    listening_mode: ListeningMode,
    input_state: InputState,
    turn_state: TurnState,
    generation_state: GenerationState,
    playback_state: PlaybackState,
    active_turn_id: Option<String>,
    generation: u64,
    last_error: Option<AppError>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            revision: 1,
            listening_mode: ListeningMode::Disabled,
            input_state: InputState::Stopped,
            turn_state: TurnState::Idle,
            generation_state: GenerationState::NotStarted,
            playback_state: PlaybackState::Idle,
            active_turn_id: None,
            generation: 0,
            last_error: None,
        }
    }
}

struct ActiveTurn {
    turn_id: String,
    generation: u64,
    cancel: CancellationToken,
}

pub struct AppCore {
    storage: Storage,
    conversation_id: String,
    runtime: RwLock<RuntimeState>,
    provider: RwLock<ProviderSettings>,
    pet: RwLock<PetSettings>,
    secrets: SecretStore,
    active_turn: Mutex<Option<ActiveTurn>>,
    next_generation: AtomicU64,
    event_sequence: AtomicU64,
}

impl AppCore {
    pub fn new(storage: Storage) -> Result<Arc<Self>, AppError> {
        let now = now_ms();
        storage
            .recover_interrupted_turns(now)
            .map_err(storage_error)?;
        let conversation_id = storage.ensure_conversation(now).map_err(storage_error)?;
        let secrets = SecretStore;
        let mut provider = storage.load_provider_settings().map_err(storage_error)?;
        provider.has_api_key = secrets.resolve_llm_api_key()?.is_some();
        let pet = storage.load_pet_settings().map_err(storage_error)?;
        Ok(Arc::new(Self {
            storage,
            conversation_id,
            runtime: RwLock::new(RuntimeState::default()),
            provider: RwLock::new(provider),
            pet: RwLock::new(pet),
            secrets,
            active_turn: Mutex::new(None),
            next_generation: AtomicU64::new(0),
            event_sequence: AtomicU64::new(0),
        }))
    }

    pub async fn snapshot(&self) -> Result<AppSnapshot, AppError> {
        let runtime = self.runtime.read().await;
        let provider = self.provider.read().await.clone();
        let pet = self.pet.read().await.clone();
        let messages = self
            .storage
            .list_messages(&self.conversation_id, 200)
            .map_err(storage_error)?;
        Ok(AppSnapshot {
            revision: runtime.revision,
            listening_mode: runtime.listening_mode,
            input_state: runtime.input_state,
            turn_state: runtime.turn_state,
            generation_state: runtime.generation_state,
            playback_state: runtime.playback_state,
            avatar_state: derive_avatar_state(&runtime),
            active_turn_id: runtime.active_turn_id.clone(),
            generation: runtime.generation,
            conversation_id: self.conversation_id.clone(),
            messages,
            provider,
            pet,
            last_error: runtime.last_error.clone(),
        })
    }

    pub async fn apply_current_pet_settings(&self, app: &AppHandle) -> Result<(), AppError> {
        let settings = self.pet.read().await.clone();
        apply_pet_window_settings(app, &settings)
    }

    pub async fn update_pet_settings(
        &self,
        app: &AppHandle,
        value: PetSettings,
        expected_revision: u64,
    ) -> Result<PetSettings, AppError> {
        let mut pet = self.pet.write().await;
        if pet.revision != expected_revision {
            return Err(AppError::new(
                "conflict",
                "桌宠设置已在其他入口更新，请刷新后重试。",
                true,
            ));
        }
        validate_pet_settings(&value)?;
        let previous = pet.clone();
        let updated = PetSettings {
            revision: pet.revision + 1,
            ..value
        };
        apply_pet_window_settings(app, &updated)?;
        if let Err(error) = self.storage.save_pet_settings(&updated, now_ms()) {
            let _ = apply_pet_window_settings(app, &previous);
            return Err(storage_error(error));
        }
        *pet = updated.clone();
        drop(pet);
        self.runtime.write().await.revision += 1;
        self.emit_snapshot(app).await?;
        Ok(updated)
    }

    pub async fn toggle_pet_visibility(&self, app: &AppHandle) -> Result<PetSettings, AppError> {
        let current = self.pet.read().await.clone();
        self.update_pet_settings(
            app,
            PetSettings {
                visible: !current.visible,
                ..current.clone()
            },
            current.revision,
        )
        .await
    }

    pub async fn toggle_pet_click_through(&self, app: &AppHandle) -> Result<PetSettings, AppError> {
        let current = self.pet.read().await.clone();
        self.update_pet_settings(
            app,
            PetSettings {
                click_through: !current.click_through,
                ..current.clone()
            },
            current.revision,
        )
        .await
    }

    pub async fn start_text_turn(
        self: &Arc<Self>,
        app: AppHandle,
        text: String,
    ) -> Result<(), AppError> {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(AppError::new("invalid_request", "消息不能为空。", false));
        }
        if text.chars().count() > 16_000 {
            return Err(AppError::new(
                "invalid_request",
                "消息超过 16000 字符限制。",
                false,
            ));
        }

        self.cancel_active(&app, "superseded").await?;
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let turn_id = Uuid::now_v7().to_string();
        self.storage
            .begin_text_turn(&self.conversation_id, &turn_id, generation, &text, now_ms())
            .map_err(storage_error)?;

        let cancel = CancellationToken::new();
        *self.active_turn.lock().await = Some(ActiveTurn {
            turn_id: turn_id.clone(),
            generation,
            cancel: cancel.clone(),
        });
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Preparing;
            runtime.generation_state = GenerationState::NotStarted;
            runtime.active_turn_id = Some(turn_id.clone());
            runtime.generation = generation;
            runtime.last_error = None;
        }
        self.emit_snapshot(&app).await?;

        let core = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            core.run_text_turn(app, turn_id, generation, text, cancel)
                .await;
        });
        Ok(())
    }

    async fn run_text_turn(
        self: Arc<Self>,
        app: AppHandle,
        turn_id: String,
        generation: u64,
        text: String,
        cancel: CancellationToken,
    ) {
        let history = match self.storage.list_messages(&self.conversation_id, 24) {
            Ok(history) => history
                .into_iter()
                .filter(|message| message.turn_id != turn_id)
                .collect(),
            Err(error) => {
                self.fail_turn(
                    &app,
                    &turn_id,
                    generation,
                    "storage_error",
                    storage_error(error),
                    "",
                )
                .await;
                return;
            }
        };
        let settings = self.provider.read().await.clone();
        let api_key = match self.resolve_llm_api_key().await {
            Ok(value) => value,
            Err(error) => {
                self.fail_turn(&app, &turn_id, generation, "keychain_error", error, "")
                    .await;
                return;
            }
        };
        let provider = match create_provider(&settings, api_key) {
            Ok(provider) => provider,
            Err(error) => {
                self.fail_turn(
                    &app,
                    &turn_id,
                    generation,
                    "provider_unavailable",
                    error,
                    "",
                )
                .await;
                return;
            }
        };
        let mut stream = match tokio::time::timeout(
            Duration::from_secs(15),
            provider.start(
                LlmRequest {
                    user_text: text,
                    history,
                },
                cancel.clone(),
            ),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                self.fail_turn(
                    &app,
                    &turn_id,
                    generation,
                    "provider_unavailable",
                    error,
                    "",
                )
                .await;
                return;
            }
            Err(_) => {
                self.fail_turn(
                    &app,
                    &turn_id,
                    generation,
                    "timeout",
                    AppError::new("timeout", "LLM 在 15 秒内未建立连接。", true),
                    "",
                )
                .await;
                return;
            }
        };
        if !self.is_current(&turn_id, generation).await {
            return;
        }
        if let Err(error) = self.storage.mark_turn_active(&turn_id) {
            self.fail_turn(
                &app,
                &turn_id,
                generation,
                "storage_error",
                storage_error(error),
                "",
            )
            .await;
            return;
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Active;
            runtime.generation_state = GenerationState::Streaming;
        }
        let _ = self.emit_snapshot(&app).await;

        let mut response = String::new();
        let mut pending_delta = String::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        let idle_timeout = tokio::time::sleep(Duration::from_secs(15));
        tokio::pin!(idle_timeout);

        loop {
            tokio::select! {
                _ = ticker.tick(), if !pending_delta.is_empty() => {
                    self.emit_pending_delta(&app, &turn_id, generation, &mut pending_delta).await;
                }
                _ = &mut idle_timeout => {
                    self.fail_turn(
                        &app,
                        &turn_id,
                        generation,
                        "timeout",
                        AppError::new("timeout", "LLM 流长时间没有返回数据。", true),
                        &response,
                    ).await;
                    return;
                }
                event = stream.next() => {
                    if !self.is_current(&turn_id, generation).await {
                        return;
                    }
                    idle_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
                    match event {
                Some(Ok(LlmEvent::TextDelta(delta))) => {
                    response.push_str(&delta);
                    pending_delta.push_str(&delta);
                    if pending_delta.chars().count() >= 24 {
                        self.emit_pending_delta(&app, &turn_id, generation, &mut pending_delta).await;
                    }
                }
                Some(Ok(LlmEvent::Completed)) => {
                    self.emit_pending_delta(&app, &turn_id, generation, &mut pending_delta).await;
                    if response.trim().is_empty() {
                        self.fail_turn(
                            &app,
                            &turn_id,
                            generation,
                            "empty_response",
                            AppError::new("empty_response", "模型没有返回可显示文本。", true),
                            "",
                        )
                        .await;
                        return;
                    }
                    self.complete_turn(&app, &turn_id, generation, &response)
                        .await;
                    return;
                }
                Some(Err(error)) if error.code == "cancelled" => return,
                Some(Err(error)) => {
                    self.fail_turn(
                        &app,
                        &turn_id,
                        generation,
                        "provider_error",
                        error,
                        &response,
                    )
                    .await;
                    return;
                }
                None => {
                    self.fail_turn(
                        &app,
                        &turn_id,
                        generation,
                        "stream_ended",
                        AppError::new("provider_unavailable", "LLM 流在完成事件前结束。", true),
                        &response,
                    ).await;
                    return;
                }
                    }
                }
            }
        }
    }

    async fn emit_pending_delta(
        &self,
        app: &AppHandle,
        turn_id: &str,
        generation: u64,
        pending: &mut String,
    ) {
        if pending.is_empty() {
            return;
        }
        let text = std::mem::take(pending);
        let _ = self
            .emit(
                app,
                "turn:text-delta",
                Some(turn_id.to_owned()),
                Some(generation),
                TextDeltaPayload { text },
            )
            .await;
    }

    async fn complete_turn(&self, app: &AppHandle, turn_id: &str, generation: u64, response: &str) {
        if !self.is_current(turn_id, generation).await {
            return;
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Finalizing;
            runtime.generation_state = GenerationState::Finished;
        }
        let _ = self.emit_snapshot(app).await;
        if let Err(error) =
            self.storage
                .complete_turn(&self.conversation_id, turn_id, response, now_ms())
        {
            self.fail_turn(
                app,
                turn_id,
                generation,
                "storage_error",
                storage_error(error),
                response,
            )
            .await;
            return;
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Completed;
            runtime.active_turn_id = None;
        }
        *self.active_turn.lock().await = None;
        let _ = self.emit_snapshot(app).await;
        let _ = self
            .emit(
                app,
                "turn:ended",
                Some(turn_id.to_owned()),
                Some(generation),
                TurnEndedPayload {
                    status: "completed".into(),
                    reason: None,
                },
            )
            .await;
    }

    async fn fail_turn(
        &self,
        app: &AppHandle,
        turn_id: &str,
        generation: u64,
        failure_code: &str,
        error: AppError,
        partial: &str,
    ) {
        if !self.is_current(turn_id, generation).await {
            return;
        }
        let _ = self.storage.finish_turn_with_status(
            turn_id,
            "failed",
            Some(failure_code),
            partial,
            &self.conversation_id,
            now_ms(),
        );
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Failed;
            runtime.generation_state = GenerationState::Failed;
            runtime.active_turn_id = None;
            runtime.last_error = Some(error);
        }
        *self.active_turn.lock().await = None;
        let _ = self.emit_snapshot(app).await;
        let _ = self
            .emit(
                app,
                "turn:ended",
                Some(turn_id.to_owned()),
                Some(generation),
                TurnEndedPayload {
                    status: "failed".into(),
                    reason: Some(failure_code.into()),
                },
            )
            .await;
    }

    pub async fn cancel_turn(
        &self,
        app: &AppHandle,
        turn_id: &str,
        reason: &str,
    ) -> Result<(), AppError> {
        let active = self.active_turn.lock().await;
        if active
            .as_ref()
            .is_none_or(|active| active.turn_id != turn_id)
        {
            return Ok(());
        }
        drop(active);
        self.cancel_active(app, reason).await
    }

    async fn cancel_active(&self, app: &AppHandle, reason: &str) -> Result<(), AppError> {
        let Some(active) = self.active_turn.lock().await.take() else {
            return Ok(());
        };
        active.cancel.cancel();
        self.next_generation
            .fetch_max(active.generation, Ordering::SeqCst);
        self.storage
            .finish_turn_with_status(
                &active.turn_id,
                "cancelled",
                Some(reason),
                "",
                &self.conversation_id,
                now_ms(),
            )
            .map_err(storage_error)?;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Cancelled;
            runtime.generation_state = GenerationState::Finished;
            runtime.active_turn_id = None;
        }
        self.emit_snapshot(app).await?;
        self.emit(
            app,
            "turn:ended",
            Some(active.turn_id),
            Some(active.generation),
            TurnEndedPayload {
                status: "cancelled".into(),
                reason: Some(reason.into()),
            },
        )
        .await
    }

    pub async fn update_provider(
        &self,
        value: ProviderSettings,
        expected_revision: u64,
        api_key: Option<String>,
        clear_api_key: bool,
    ) -> Result<ProviderSettings, AppError> {
        let mut provider = self.provider.write().await;
        if provider.revision != expected_revision {
            return Err(AppError::new(
                "conflict",
                "Provider 设置已在其他窗口更新，请刷新后重试。",
                true,
            ));
        }
        validate_provider_settings(&value)?;

        let previous_secret = self.get_stored_llm_api_key().await?;
        if clear_api_key {
            self.delete_llm_api_key().await?;
        } else if let Some(secret) = api_key
            .as_deref()
            .filter(|secret| !secret.trim().is_empty())
        {
            self.set_llm_api_key(secret.to_owned()).await?;
        }
        let has_api_key = self.resolve_llm_api_key().await?.is_some();
        let updated = ProviderSettings {
            revision: provider.revision + 1,
            has_api_key,
            ..value
        };
        if let Err(error) = self.storage.save_provider_settings(&updated, now_ms()) {
            match previous_secret {
                Some(secret) => {
                    let _ = self.set_llm_api_key(secret).await;
                }
                None => {
                    let _ = self.delete_llm_api_key().await;
                }
            }
            return Err(storage_error(error));
        }
        *provider = updated.clone();
        Ok(updated)
    }

    async fn resolve_llm_api_key(&self) -> Result<Option<String>, AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.resolve_llm_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn get_stored_llm_api_key(&self) -> Result<Option<String>, AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.get_stored_llm_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn set_llm_api_key(&self, value: String) -> Result<(), AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.set_llm_api_key(&value))
            .await
            .map_err(secret_task_error)?
    }

    async fn delete_llm_api_key(&self) -> Result<(), AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.delete_llm_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn is_current(&self, turn_id: &str, generation: u64) -> bool {
        self.active_turn
            .lock()
            .await
            .as_ref()
            .is_some_and(|active| active.turn_id == turn_id && active.generation == generation)
    }

    async fn emit_snapshot(&self, app: &AppHandle) -> Result<(), AppError> {
        let snapshot = self.snapshot().await?;
        self.emit(
            app,
            "state:changed",
            snapshot.active_turn_id.clone(),
            Some(snapshot.generation),
            snapshot,
        )
        .await
    }

    async fn emit<T: Serialize + Clone>(
        &self,
        app: &AppHandle,
        channel: &str,
        turn_id: Option<String>,
        generation: Option<u64>,
        payload: T,
    ) -> Result<(), AppError> {
        let revision = self.runtime.read().await.revision;
        let envelope = EventEnvelope {
            protocol_version: "1.0",
            event_id: Uuid::now_v7().to_string(),
            event_seq: self.event_sequence.fetch_add(1, Ordering::SeqCst) + 1,
            snapshot_revision: revision,
            occurred_at: time::OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
            event_type: channel.into(),
            turn_id,
            generation,
            payload,
        };
        app.emit(channel, envelope)
            .map_err(|error| AppError::new("internal", format!("事件发送失败：{error}"), true))
    }
}

fn derive_avatar_state(runtime: &RuntimeState) -> AvatarSemanticState {
    if runtime.input_state == InputState::Recovering {
        AvatarSemanticState::Recovering
    } else if runtime.last_error.is_some() {
        AvatarSemanticState::Error
    } else if runtime.playback_state == PlaybackState::Playing {
        AvatarSemanticState::Speaking
    } else if matches!(
        runtime.input_state,
        InputState::Capturing | InputState::Recognizing
    ) {
        AvatarSemanticState::Listening
    } else if matches!(
        runtime.turn_state,
        TurnState::Preparing | TurnState::Active | TurnState::Finalizing
    ) {
        AvatarSemanticState::Thinking
    } else if runtime.listening_mode == ListeningMode::QuietHours {
        AvatarSemanticState::Sleeping
    } else {
        AvatarSemanticState::Idle
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn storage_error(error: rusqlite::Error) -> AppError {
    AppError::new("storage_error", format!("本地数据库错误：{error}"), false)
}

fn secret_task_error(error: tokio::task::JoinError) -> AppError {
    AppError::new(
        "keychain_error",
        format!("Keychain 后台任务失败：{error}"),
        true,
    )
}

fn validate_provider_settings(value: &ProviderSettings) -> Result<(), AppError> {
    if !matches!(value.provider.as_str(), "mock" | "openai_compatible") {
        return Err(AppError::new(
            "invalid_request",
            "不支持的 Provider。",
            false,
        ));
    }
    if value.provider == "openai_compatible" {
        let endpoint = reqwest::Url::parse(&value.endpoint).map_err(|_| {
            AppError::new("invalid_request", "Provider endpoint 不是有效 URL。", false)
        })?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(AppError::new(
                "invalid_request",
                "Provider endpoint 只允许 HTTP 或 HTTPS。",
                false,
            ));
        }
        if value.model.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "模型名称不能为空。",
                false,
            ));
        }
    }
    Ok(())
}

fn validate_pet_settings(value: &PetSettings) -> Result<(), AppError> {
    if !value.scale.is_finite() || !(0.6..=1.5).contains(&value.scale) {
        return Err(AppError::new(
            "invalid_request",
            "桌宠缩放比例必须在 60% 到 150% 之间。",
            false,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_openai_compatible_settings() {
        let valid = ProviderSettings {
            provider: "openai_compatible".into(),
            endpoint: "https://api.example.com/v1".into(),
            model: "model".into(),
            has_api_key: false,
            revision: 0,
        };
        assert!(validate_provider_settings(&valid).is_ok());

        let invalid_scheme = ProviderSettings {
            endpoint: "file:///tmp/provider".into(),
            ..valid.clone()
        };
        assert!(validate_provider_settings(&invalid_scheme).is_err());

        let empty_model = ProviderSettings {
            model: " ".into(),
            ..valid
        };
        assert!(validate_provider_settings(&empty_model).is_err());
    }

    #[test]
    fn validates_pet_scale() {
        assert!(validate_pet_settings(&PetSettings::default()).is_ok());
        assert!(validate_pet_settings(&PetSettings {
            scale: 0.5,
            ..PetSettings::default()
        })
        .is_err());
        assert!(validate_pet_settings(&PetSettings {
            scale: f64::NAN,
            ..PetSettings::default()
        })
        .is_err());
    }
}
