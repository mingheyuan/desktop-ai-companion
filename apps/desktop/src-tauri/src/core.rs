use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
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
        ListeningMode, PlaybackState, ProviderSettings, TextDeltaPayload, TurnEndedPayload,
        TurnState,
    },
    provider::{create_provider, LlmEvent, LlmRequest},
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
        let provider = storage.load_provider_settings().map_err(storage_error)?;
        Ok(Arc::new(Self {
            storage,
            conversation_id,
            runtime: RwLock::new(RuntimeState::default()),
            provider: RwLock::new(provider),
            active_turn: Mutex::new(None),
            next_generation: AtomicU64::new(0),
            event_sequence: AtomicU64::new(0),
        }))
    }

    pub async fn snapshot(&self) -> Result<AppSnapshot, AppError> {
        let runtime = self.runtime.read().await;
        let provider = self.provider.read().await.clone();
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
            last_error: runtime.last_error.clone(),
        })
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
        let provider = match create_provider(&settings) {
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
        let mut stream = match provider
            .start(
                LlmRequest {
                    user_text: text,
                    history,
                },
                cancel.clone(),
            )
            .await
        {
            Ok(stream) => stream,
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
        while let Some(event) = stream.next().await {
            if !self.is_current(&turn_id, generation).await {
                return;
            }
            match event {
                Ok(LlmEvent::TextDelta(delta)) => {
                    response.push_str(&delta);
                    let _ = self
                        .emit(
                            &app,
                            "turn:text-delta",
                            Some(turn_id.clone()),
                            Some(generation),
                            TextDeltaPayload { text: delta },
                        )
                        .await;
                }
                Ok(LlmEvent::Completed) => {
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
                Err(error) if error.code == "cancelled" => return,
                Err(error) => {
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
            }
        }
        self.fail_turn(
            &app,
            &turn_id,
            generation,
            "stream_ended",
            AppError::new("provider_unavailable", "LLM 流在完成事件前结束。", true),
            &response,
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
    ) -> Result<ProviderSettings, AppError> {
        let mut provider = self.provider.write().await;
        if provider.revision != expected_revision {
            return Err(AppError::new(
                "conflict",
                "Provider 设置已在其他窗口更新，请刷新后重试。",
                true,
            ));
        }
        if !matches!(value.provider.as_str(), "mock" | "openai_compatible") {
            return Err(AppError::new(
                "invalid_request",
                "不支持的 Provider。",
                false,
            ));
        }
        let updated = ProviderSettings {
            revision: provider.revision + 1,
            ..value
        };
        self.storage
            .save_provider_settings(&updated, now_ms())
            .map_err(storage_error)?;
        *provider = updated.clone();
        Ok(updated)
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
