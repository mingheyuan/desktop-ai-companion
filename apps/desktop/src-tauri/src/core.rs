use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use time::format_description::well_known::Rfc3339;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    audio::{self, AudioWriter},
    contracts::{
        AppError, AppSnapshot, AvatarSemanticState, CharacterSettings, EventEnvelope,
        GenerationState, InputState, ListeningMode, MemoryItem, PetSettings, PlaybackState,
        ProviderSettings, TextDeltaPayload, TurnEndedPayload, TurnState, VoicePermissions,
        VoiceSettings,
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
    audio_host_state: String,
    voice_permissions: VoicePermissions,
    asr_partial: String,
    active_voice_source: Option<String>,
    active_voice_capture_id: Option<String>,
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
            audio_host_state: "stopped".into(),
            voice_permissions: VoicePermissions::default(),
            asr_partial: String::new(),
            active_voice_source: None,
            active_voice_capture_id: None,
        }
    }
}

struct ActiveTurn {
    turn_id: String,
    generation: u64,
    cancel: CancellationToken,
}

struct VoiceFinal {
    app: AppHandle,
    text: String,
    source: String,
}

pub struct AppCore {
    storage: Storage,
    conversation_id: String,
    runtime: RwLock<RuntimeState>,
    provider: RwLock<ProviderSettings>,
    pet: RwLock<PetSettings>,
    character: RwLock<CharacterSettings>,
    voice_settings: RwLock<VoiceSettings>,
    secrets: SecretStore,
    active_turn: Mutex<Option<ActiveTurn>>,
    active_playback: Mutex<Option<(String, u64)>>,
    audio_writer: Mutex<Option<AudioWriter>>,
    audio_launch: Mutex<()>,
    voice_command: Mutex<()>,
    audio_generation: AtomicU64,
    voice_final_tx: tokio::sync::mpsc::UnboundedSender<VoiceFinal>,
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
        let character = storage.load_character_settings().map_err(storage_error)?;
        let voice_settings = storage.load_voice_settings().map_err(storage_error)?;
        let (voice_final_tx, mut voice_final_rx) = tokio::sync::mpsc::unbounded_channel();
        let core = Arc::new(Self {
            storage,
            conversation_id,
            runtime: RwLock::new(RuntimeState::default()),
            provider: RwLock::new(provider),
            pet: RwLock::new(pet),
            character: RwLock::new(character),
            voice_settings: RwLock::new(voice_settings),
            secrets,
            active_turn: Mutex::new(None),
            active_playback: Mutex::new(None),
            audio_writer: Mutex::new(None),
            audio_launch: Mutex::new(()),
            voice_command: Mutex::new(()),
            audio_generation: AtomicU64::new(0),
            voice_final_tx,
            next_generation: AtomicU64::new(0),
            event_sequence: AtomicU64::new(0),
        });
        let weak_core = Arc::downgrade(&core);
        tauri::async_runtime::spawn(async move {
            while let Some(final_input) = voice_final_rx.recv().await {
                let Some(core) = weak_core.upgrade() else {
                    break;
                };
                let _ = core
                    .start_input_turn(final_input.app, final_input.text, &final_input.source)
                    .await;
            }
        });
        Ok(core)
    }

    pub async fn snapshot(&self) -> Result<AppSnapshot, AppError> {
        let runtime = self.runtime.read().await;
        let provider = self.provider.read().await.clone();
        let pet = self.pet.read().await.clone();
        let character = self.character.read().await.clone();
        let voice_settings = self.voice_settings.read().await.clone();
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
            character,
            audio_host_state: runtime.audio_host_state.clone(),
            voice_permissions: runtime.voice_permissions.clone(),
            voice_settings,
            asr_partial: runtime.asr_partial.clone(),
            last_error: runtime.last_error.clone(),
        })
    }

    pub async fn import_character_vrm(
        &self,
        app: &AppHandle,
        source: std::path::PathBuf,
    ) -> Result<CharacterSettings, AppError> {
        let active_character = self.character.read().await.clone();
        if active_character.character_id.is_empty()
            || !active_character
                .character_id
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
        {
            return Err(AppError::new(
                "character_import",
                "当前角色 ID 不能用于管理目录。",
                false,
            ));
        }
        let data_dir = app.path().app_data_dir().map_err(|error| {
            AppError::new(
                "character_import",
                format!("无法定位角色目录：{error}"),
                false,
            )
        })?;
        let managed_dir = data_dir
            .join("characters")
            .join(&active_character.character_id);
        let source_for_copy = source.clone();
        let copied = tokio::task::spawn_blocking(move || {
            validate_and_copy_vrm(&source_for_copy, &managed_dir)
        })
        .await
        .map_err(|error| {
            AppError::new(
                "character_import",
                format!("角色复制任务失败：{error}"),
                true,
            )
        })??;

        let mut character = self.character.write().await;
        let updated = CharacterSettings {
            model_path: Some(copied.managed_path.to_string_lossy().into_owned()),
            model_file_name: Some(copied.original_file_name),
            model_size_bytes: Some(copied.size_bytes),
            revision: character.revision + 1,
            ..character.clone()
        };
        if let Err(error) = self.storage.save_character_settings(&updated, now_ms()) {
            let _ = std::fs::remove_file(&copied.managed_path);
            return Err(storage_error(error));
        }
        *character = updated.clone();
        drop(character);
        self.runtime.write().await.revision += 1;
        self.emit_snapshot(app).await?;
        Ok(updated)
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
        self.start_input_turn(app, text, "text").await
    }

    async fn start_input_turn(
        self: &Arc<Self>,
        app: AppHandle,
        text: String,
        input_source: &str,
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

        self.interrupt_playback().await;
        self.cancel_active(&app, "superseded").await?;
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let turn_id = Uuid::now_v7().to_string();
        self.storage
            .begin_turn(
                &self.conversation_id,
                &turn_id,
                generation,
                input_source,
                &text,
                now_ms(),
            )
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
        let task: Pin<Box<dyn Future<Output = ()> + Send>> = Box::pin(async move {
            core.run_text_turn(app, turn_id, generation, text, cancel)
                .await;
        });
        tauri::async_runtime::spawn(task);
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
        let memory_context = if settings.provider == "mock" || settings.allow_memory_context {
            let character_id = self.character.read().await.character_id.clone();
            let memories = match self.storage.search_memories(&text, &character_id, 6) {
                Ok(memories) => memories,
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
            let _ = self.storage.mark_memories_used(
                &memories
                    .iter()
                    .map(|memory| memory.id.clone())
                    .collect::<Vec<_>>(),
                now_ms(),
            );
            memory_context_with_budget(memories, 2_400)
        } else {
            Vec::new()
        };
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
                    memory_context,
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

    async fn complete_turn(
        self: &Arc<Self>,
        app: &AppHandle,
        turn_id: &str,
        generation: u64,
        response: &str,
    ) {
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
        self.speak_response(app, turn_id, generation, response)
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

    pub async fn search_memories(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, AppError> {
        let character_id = self.character.read().await.character_id.clone();
        self.storage
            .search_memories(query, &character_id, limit)
            .map_err(storage_error)
    }

    pub fn create_memory(&self, content: &str) -> Result<MemoryItem, AppError> {
        let content = validate_memory_content(content)?;
        self.storage
            .create_user_memory(&content, now_ms())
            .map_err(storage_error)
    }

    pub fn update_memory(
        &self,
        id: &str,
        content: &str,
        expected_version: u64,
    ) -> Result<MemoryItem, AppError> {
        let content = validate_memory_content(content)?;
        self.storage
            .update_memory(id, &content, expected_version, now_ms())
            .map_err(storage_error)?
            .ok_or_else(|| AppError::new("conflict", "记忆已被修改或删除，请刷新后重试。", true))
    }

    pub fn delete_memory(&self, id: &str, expected_version: u64) -> Result<(), AppError> {
        if !self
            .storage
            .delete_memory(id, expected_version, now_ms())
            .map_err(storage_error)?
        {
            return Err(AppError::new(
                "conflict",
                "记忆已被修改或删除，请刷新后重试。",
                true,
            ));
        }
        Ok(())
    }

    pub async fn request_voice_permissions(
        self: &Arc<Self>,
        app: &AppHandle,
    ) -> Result<(), AppError> {
        let writer = self.ensure_audio_host(app).await?;
        audio::send(&writer, json!({"type": "permissions.request"})).await
    }

    pub async fn set_listening(
        self: &Arc<Self>,
        app: &AppHandle,
        enabled: bool,
    ) -> Result<(), AppError> {
        if !enabled {
            if let Some(writer) = self.audio_writer.lock().await.clone() {
                audio::send(&writer, json!({"type": "listening.set", "enabled": false})).await?;
            }
            {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.listening_mode = ListeningMode::Disabled;
                runtime.input_state = InputState::Stopped;
                runtime.active_voice_capture_id = None;
                runtime.active_voice_source = None;
                runtime.asr_partial.clear();
            }
            return self.emit_snapshot(app).await;
        }
        let permissions = self.runtime.read().await.voice_permissions.clone();
        if permissions.microphone != "authorized" || permissions.speech_recognition != "authorized"
        {
            return Err(AppError::new(
                "permission_required",
                "请先点击“授权语音”，完成麦克风和语音识别授权。",
                true,
            ));
        }
        let writer = self.ensure_audio_host(app).await?;
        audio::send(&writer, json!({"type": "listening.set", "enabled": true})).await?;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.listening_mode = ListeningMode::Armed;
            runtime.input_state = InputState::WakeListening;
            runtime.last_error = None;
        }
        self.emit_snapshot(app).await
    }

    pub async fn start_push_to_talk(self: &Arc<Self>, app: &AppHandle) -> Result<(), AppError> {
        let _voice_guard = self.voice_command.lock().await;
        let permissions = self.runtime.read().await.voice_permissions.clone();
        if permissions.microphone != "authorized" || permissions.speech_recognition != "authorized"
        {
            return Err(AppError::new(
                "permission_required",
                "按键说话需要先完成麦克风和语音识别授权。",
                true,
            ));
        }
        self.cancel_active(app, "voice_input").await?;
        let writer = self.ensure_audio_host(app).await?;
        let capture_id = Uuid::now_v7().to_string();
        let generation = self.audio_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = audio::send(&writer, json!({"type": "tts.stop"})).await;
        *self.active_playback.lock().await = None;
        audio::send(
            &writer,
            json!({
                "type": "asr.start",
                "capture_id": capture_id,
                "generation": generation,
            }),
        )
        .await?;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.input_state = InputState::Capturing;
            runtime.playback_state = PlaybackState::Idle;
            runtime.active_voice_capture_id = Some(capture_id);
            runtime.active_voice_source = Some("push_to_talk".into());
            runtime.asr_partial.clear();
            runtime.last_error = None;
        }
        self.emit_snapshot(app).await
    }

    pub async fn finish_push_to_talk(&self, app: &AppHandle) -> Result<(), AppError> {
        let _voice_guard = self.voice_command.lock().await;
        let writer = self
            .audio_writer
            .lock()
            .await
            .clone()
            .ok_or_else(|| AppError::new("audio_host_failed", "AudioHost 尚未启动。", true))?;
        audio::send(&writer, json!({"type": "asr.finalize"})).await?;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.input_state = InputState::Recognizing;
        }
        self.emit_snapshot(app).await
    }

    pub async fn stop_playback(&self, app: &AppHandle) -> Result<(), AppError> {
        if let Some(writer) = self.audio_writer.lock().await.clone() {
            audio::send(&writer, json!({"type": "tts.stop"})).await?;
        }
        *self.active_playback.lock().await = None;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.playback_state = PlaybackState::Idle;
        }
        self.emit_snapshot(app).await
    }

    pub async fn update_voice_settings(
        &self,
        app: &AppHandle,
        tts_enabled: bool,
        expected_revision: u64,
    ) -> Result<VoiceSettings, AppError> {
        let mut settings = self.voice_settings.write().await;
        if settings.revision != expected_revision {
            return Err(AppError::new(
                "conflict",
                "语音设置已更新，请刷新后重试。",
                true,
            ));
        }
        let updated = VoiceSettings {
            tts_enabled,
            revision: settings.revision + 1,
        };
        self.storage
            .save_voice_settings(&updated, now_ms())
            .map_err(storage_error)?;
        *settings = updated.clone();
        drop(settings);
        if !tts_enabled {
            self.stop_playback(app).await?;
        } else {
            self.runtime.write().await.revision += 1;
            self.emit_snapshot(app).await?;
        }
        Ok(updated)
    }

    async fn ensure_audio_host(self: &Arc<Self>, app: &AppHandle) -> Result<AudioWriter, AppError> {
        if let Some(writer) = self.audio_writer.lock().await.clone() {
            return Ok(writer);
        }
        let _launch_guard = self.audio_launch.lock().await;
        if let Some(writer) = self.audio_writer.lock().await.clone() {
            return Ok(writer);
        }
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.audio_host_state = "launching".into();
            runtime.last_error = None;
        }
        self.emit_snapshot(app).await?;
        let bundled_audio_host = app
            .path()
            .resource_dir()
            .ok()
            .map(|path| path.join("AudioHost.app"));
        let mut connection = match audio::launch(bundled_audio_host).await {
            Ok(connection) => connection,
            Err(error) => {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.audio_host_state = "failed".into();
                runtime.last_error = Some(error.clone());
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
                return Err(error);
            }
        };
        *self.audio_writer.lock().await = Some(connection.writer.clone());
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.audio_host_state = "handshaking".into();
        }
        self.emit_snapshot(app).await?;
        let core = Arc::clone(self);
        let app = app.clone();
        let task: Pin<Box<dyn Future<Output = ()> + Send>> = Box::pin(async move {
            while let Some(event) = connection.events.recv().await {
                core.handle_audio_event(&app, event).await;
            }
        });
        tauri::async_runtime::spawn(task);
        Ok(connection.writer)
    }

    async fn handle_audio_event(self: &Arc<Self>, app: &AppHandle, event: Value) {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "host.ready" => {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.audio_host_state = "ready".into();
                runtime.last_error = None;
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "permissions.status" => {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.voice_permissions = VoicePermissions {
                    microphone: event
                        .get("microphone")
                        .and_then(Value::as_str)
                        .unwrap_or("restricted")
                        .into(),
                    speech_recognition: event
                        .get("speech_recognition")
                        .and_then(Value::as_str)
                        .unwrap_or("restricted")
                        .into(),
                };
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "capture.started" => {
                let capture_id = event
                    .get("capture_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mode = event
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.active_voice_capture_id = Some(capture_id);
                runtime.input_state = if mode == "wake" {
                    runtime.active_voice_source = None;
                    runtime.asr_partial.clear();
                    InputState::WakeListening
                } else {
                    InputState::Recognizing
                };
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "wake.detected" => {
                let capture_id = event.get("capture_id").and_then(Value::as_str);
                let mut runtime = self.runtime.write().await;
                if runtime.active_voice_capture_id.as_deref() != capture_id {
                    return;
                }
                runtime.revision += 1;
                runtime.input_state = InputState::Recognizing;
                runtime.active_voice_source = Some("wake_word".into());
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "asr.partial" => {
                let capture_id = event.get("capture_id").and_then(Value::as_str);
                let mut runtime = self.runtime.write().await;
                if runtime.active_voice_capture_id.as_deref() != capture_id {
                    return;
                }
                runtime.revision += 1;
                runtime.input_state = InputState::Recognizing;
                runtime.asr_partial = event
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "asr.final" => {
                let capture_id = event.get("capture_id").and_then(Value::as_str);
                let text = event
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let source = {
                    let mut runtime = self.runtime.write().await;
                    if runtime.active_voice_capture_id.as_deref() != capture_id {
                        return;
                    }
                    runtime.revision += 1;
                    runtime.input_state = if runtime.listening_mode == ListeningMode::Armed {
                        InputState::WakeListening
                    } else {
                        InputState::Stopped
                    };
                    runtime.asr_partial.clear();
                    runtime.active_voice_capture_id = None;
                    runtime
                        .active_voice_source
                        .take()
                        .unwrap_or_else(|| "push_to_talk".into())
                };
                let _ = self.emit_snapshot(app).await;
                if !text.is_empty() {
                    let _ = self.voice_final_tx.send(VoiceFinal {
                        app: app.clone(),
                        text,
                        source,
                    });
                }
            }
            "tts.started" => {
                let turn_id = event.get("turn_id").and_then(Value::as_str);
                let generation = event.get("generation").and_then(Value::as_u64);
                let active = self.active_playback.lock().await;
                if active
                    .as_ref()
                    .is_none_or(|(active_turn, active_generation)| {
                        Some(active_turn.as_str()) != turn_id
                            || Some(*active_generation) != generation
                    })
                {
                    return;
                }
                drop(active);
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.playback_state = PlaybackState::Playing;
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "tts.finished" => {
                let turn_id = event.get("turn_id").and_then(Value::as_str);
                let generation = event.get("generation").and_then(Value::as_u64);
                let mut active = self.active_playback.lock().await;
                if active
                    .as_ref()
                    .is_none_or(|(active_turn, active_generation)| {
                        Some(active_turn.as_str()) != turn_id
                            || Some(*active_generation) != generation
                    })
                {
                    return;
                }
                *active = None;
                drop(active);
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.playback_state = PlaybackState::Idle;
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "voice.error" | "host.error" => {
                let code = event
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("audio_host_failed");
                let message = event
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("AudioHost 发生错误。")
                    .to_owned();
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.input_state = InputState::Stopped;
                runtime.active_voice_capture_id = None;
                runtime.active_voice_source = None;
                runtime.asr_partial.clear();
                if matches!(
                    code,
                    "on_device_wake_unavailable"
                        | "wake_recognition_unavailable"
                        | "permission_denied"
                ) {
                    runtime.listening_mode = ListeningMode::Disabled;
                }
                runtime.last_error = Some(AppError::new(code, message, true));
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "host.disconnected" => {
                *self.audio_writer.lock().await = None;
                *self.active_playback.lock().await = None;
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.audio_host_state = "failed".into();
                runtime.input_state = InputState::Stopped;
                runtime.playback_state = PlaybackState::Idle;
                runtime.listening_mode = ListeningMode::Disabled;
                runtime.active_voice_capture_id = None;
                runtime.active_voice_source = None;
                runtime.asr_partial.clear();
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            _ => {}
        }
    }

    async fn speak_response(
        self: &Arc<Self>,
        app: &AppHandle,
        turn_id: &str,
        generation: u64,
        text: &str,
    ) {
        if !self.voice_settings.read().await.tts_enabled || text.trim().is_empty() {
            return;
        }
        let writer = match self.ensure_audio_host(app).await {
            Ok(writer) => writer,
            Err(_) => return,
        };
        *self.active_playback.lock().await = Some((turn_id.to_owned(), generation));
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.playback_state = PlaybackState::Buffering;
        }
        let _ = self.emit_snapshot(app).await;
        if audio::send(
            &writer,
            json!({"type": "tts.speak", "turn_id": turn_id, "generation": generation, "text": text}),
        )
        .await
        .is_err()
        {
            *self.active_playback.lock().await = None;
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.playback_state = PlaybackState::Idle;
            drop(runtime);
            let _ = self.emit_snapshot(app).await;
        }
    }

    async fn interrupt_playback(&self) {
        *self.active_playback.lock().await = None;
        if let Some(writer) = self.audio_writer.lock().await.clone() {
            let _ = audio::send(&writer, json!({"type": "tts.stop"})).await;
        }
        let mut runtime = self.runtime.write().await;
        if runtime.playback_state != PlaybackState::Idle {
            runtime.revision += 1;
            runtime.playback_state = PlaybackState::Idle;
        }
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

fn validate_memory_content(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    let length = value.chars().count();
    if length == 0 || length > 1_000 {
        return Err(AppError::new(
            "invalid_request",
            "记忆正文必须是 1 到 1000 个字符。",
            false,
        ));
    }
    Ok(value.to_owned())
}

fn memory_context_with_budget(memories: Vec<MemoryItem>, budget: usize) -> Vec<String> {
    let mut remaining = budget;
    memories
        .into_iter()
        .filter_map(|memory| {
            if remaining == 0 {
                return None;
            }
            let content = memory
                .content
                .chars()
                .take(remaining.min(600))
                .collect::<String>();
            remaining = remaining.saturating_sub(content.chars().count());
            (!content.is_empty()).then_some(content)
        })
        .collect()
}

const MAX_VRM_BYTES: u64 = 200 * 1024 * 1024;

struct CopiedVrm {
    managed_path: std::path::PathBuf,
    original_file_name: String,
    size_bytes: u64,
}

fn validate_and_copy_vrm(
    source: &std::path::Path,
    managed_dir: &std::path::Path,
) -> Result<CopiedVrm, AppError> {
    use std::io::Read;

    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("vrm") {
        return Err(AppError::new(
            "invalid_character_asset",
            "只能导入 .vrm 角色文件。",
            false,
        ));
    }
    let metadata = std::fs::metadata(source).map_err(|error| {
        AppError::new(
            "character_import",
            format!("无法读取 VRM 文件信息：{error}"),
            false,
        )
    })?;
    if !metadata.is_file() || metadata.len() < 20 || metadata.len() > MAX_VRM_BYTES {
        return Err(AppError::new(
            "invalid_character_asset",
            "VRM 文件必须是 20 字节到 200 MB 的普通文件。",
            false,
        ));
    }
    let mut file = std::fs::File::open(source).map_err(|error| {
        AppError::new(
            "character_import",
            format!("无法打开 VRM 文件：{error}"),
            false,
        )
    })?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).map_err(|error| {
        AppError::new(
            "invalid_character_asset",
            format!("无法读取 VRM 文件头：{error}"),
            false,
        )
    })?;
    let version = u32::from_le_bytes(header[4..8].try_into().expect("fixed header"));
    let declared_length = u32::from_le_bytes(header[8..12].try_into().expect("fixed header"));
    if &header[0..4] != b"glTF" || version != 2 || u64::from(declared_length) != metadata.len() {
        return Err(AppError::new(
            "invalid_character_asset",
            "文件不是有效的 glTF 2.0 二进制容器。",
            false,
        ));
    }
    let mut chunk_header = [0_u8; 8];
    file.read_exact(&mut chunk_header).map_err(|error| {
        AppError::new(
            "invalid_character_asset",
            format!("无法读取 VRM JSON 块：{error}"),
            false,
        )
    })?;
    let json_length =
        u32::from_le_bytes(chunk_header[0..4].try_into().expect("fixed chunk header")) as u64;
    if &chunk_header[4..8] != b"JSON" || json_length > metadata.len().saturating_sub(20) {
        return Err(AppError::new(
            "invalid_character_asset",
            "VRM 缺少有效的 glTF JSON 块。",
            false,
        ));
    }
    let mut json_bytes = vec![0_u8; json_length as usize];
    file.read_exact(&mut json_bytes).map_err(|error| {
        AppError::new(
            "invalid_character_asset",
            format!("无法读取 VRM 元数据：{error}"),
            false,
        )
    })?;
    while matches!(json_bytes.last(), Some(b' ' | 0)) {
        json_bytes.pop();
    }
    let document: serde_json::Value = serde_json::from_slice(&json_bytes).map_err(|_| {
        AppError::new(
            "invalid_character_asset",
            "VRM 的 glTF 元数据不是有效 JSON。",
            false,
        )
    })?;
    let has_vrm_extension = ["VRMC_vrm", "VRM"].iter().any(|extension| {
        document
            .get("extensions")
            .and_then(|value| value.get(extension))
            .is_some()
            || document
                .get("extensionsUsed")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(extension)))
    });
    if !has_vrm_extension {
        return Err(AppError::new(
            "invalid_character_asset",
            "所选 glTF 文件不包含 VRM 0.x 或 VRM 1.0 扩展。",
            false,
        ));
    }

    std::fs::create_dir_all(managed_dir).map_err(|error| {
        AppError::new(
            "character_import",
            format!("无法创建角色目录：{error}"),
            false,
        )
    })?;
    let id = Uuid::now_v7();
    let temporary = managed_dir.join(format!(".{id}.tmp"));
    let destination = managed_dir.join(format!("model-{id}.vrm"));
    std::fs::copy(source, &temporary).map_err(|error| {
        AppError::new(
            "character_import",
            format!("无法复制 VRM 文件：{error}"),
            false,
        )
    })?;
    if let Err(error) = std::fs::rename(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(AppError::new(
            "character_import",
            format!("无法完成 VRM 导入：{error}"),
            false,
        ));
    }
    Ok(CopiedVrm {
        managed_path: destination,
        original_file_name: source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("model.vrm")
            .to_owned(),
        size_bytes: metadata.len(),
    })
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
            allow_memory_context: false,
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

    #[test]
    fn validates_and_copies_vrm_magic() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("avatar.vrm");
        let mut json = br#"{"extensions":{"VRMC_vrm":{}}}"#.to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total_length = 12 + 8 + json.len();
        let mut file = std::fs::File::create(&source).unwrap();
        file.write_all(b"glTF").unwrap();
        file.write_all(&2_u32.to_le_bytes()).unwrap();
        file.write_all(&(total_length as u32).to_le_bytes())
            .unwrap();
        file.write_all(&(json.len() as u32).to_le_bytes()).unwrap();
        file.write_all(b"JSON").unwrap();
        file.write_all(&json).unwrap();
        let copied = validate_and_copy_vrm(&source, &temp.path().join("managed")).unwrap();
        assert!(copied.managed_path.exists());
        assert_eq!(copied.original_file_name, "avatar.vrm");

        let invalid = temp.path().join("invalid.vrm");
        let mut file = std::fs::File::create(&invalid).unwrap();
        file.write_all(b"nope0123456789012345").unwrap();
        assert!(validate_and_copy_vrm(&invalid, &temp.path().join("managed")).is_err());
    }
}
