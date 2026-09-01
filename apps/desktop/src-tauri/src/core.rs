use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{Datelike, Duration as ChronoDuration, Local, LocalResult, TimeZone};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use time::format_description::well_known::Rfc3339;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    audio::{self, AudioWriter},
    contracts::{
        AppError, AppSnapshot, AvatarSemanticState, CharacterSettings, ChatMessage, DataSummary,
        EventEnvelope, GenerationState, InputState, ListeningMode, MemoryCandidate,
        MemoryContextEntry, MemoryItem, MemorySettings, MemorySource, MouthLevelPayload,
        PetSettings, PlaybackState, ProviderSettings, ScreenCaptureScope, ScreenObservation,
        ScreenObservationPreview, ScreenUnderstandingMode, TextDeltaPayload, TimerItem,
        ToolApproval, TurnEndedPayload, TurnState, VoicePermissions, VoiceSettings,
    },
    desktop::apply_pet_window_settings,
    notifications,
    provider::{
        analyze_image, create_provider, read_webpage, LlmEvent, LlmMessage, LlmRequest, ToolCall,
    },
    secret::SecretStore,
    storage::{
        MemoryCandidateV2Input, MemoryConfirmResult, MemoryConflictResolution, Storage, TimerInput,
        ToolRunInput,
    },
    tools::{
        normalize_open_url, validate_memory_content, OpenUrlRequest, ToolAction, ToolPolicy,
        ToolRegistry,
    },
};

type PendingToolAction = ToolAction;

struct RuntimeState {
    revision: u64,
    screen_understanding_mode: ScreenUnderstandingMode,
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
    voice_input_route: String,
    mouth_level: f64,
    active_voice_source: Option<String>,
    active_voice_capture_id: Option<String>,
    pending_tool: Option<PendingTool>,
}

struct ScreenObservationRecord {
    metadata: ScreenObservation,
    png: Vec<u8>,
    ocr_text: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            revision: 1,
            screen_understanding_mode: ScreenUnderstandingMode::AskEachTime,
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
            voice_input_route: "unknown".into(),
            mouth_level: 0.0,
            active_voice_source: None,
            active_voice_capture_id: None,
            pending_tool: None,
        }
    }
}

struct ActiveTurn {
    turn_id: String,
    generation: u64,
    cancel: CancellationToken,
}

#[derive(Clone)]
struct PendingTool {
    approval: ToolApproval,
    turn_id: String,
    generation: u64,
    action: PendingToolAction,
    origin: PendingToolOrigin,
}

#[derive(Clone)]
enum PendingToolOrigin {
    Explicit,
    Model {
        call: ToolCall,
        messages: Vec<LlmMessage>,
        memory_context: Vec<MemoryContextEntry>,
        tool_steps: u8,
    },
}

struct VoiceFinal {
    app: AppHandle,
    text: String,
    source: String,
}

struct ModelToolContext<'a> {
    app: &'a AppHandle,
    turn_id: &'a str,
    generation: u64,
    messages: &'a [LlmMessage],
    memory_context: &'a [MemoryContextEntry],
    response: &'a str,
    tool_steps: u8,
}

#[derive(Clone)]
struct ExplicitScreenObserveRequest {
    request: String,
    scope: ScreenCaptureScope,
}

const MAX_TOOL_STEPS: u8 = 3;
// 模型可以在自然语言对话中按需提出工具调用；每次只允许一个调用，并且
// 仍然必须经过 ToolPolicy 和用户确认。这样普通聊天仍由模型直接回答。
const MODEL_TOOL_CALLS_ENABLED: bool = true;
const MAX_PROVIDER_START_ATTEMPTS: u8 = 3;
const SCREEN_OBSERVATION_TTL_MS: i64 = 5 * 60 * 1_000;
const TOOL_APPROVAL_TIMEOUT_MS: i64 = 2 * 60 * 1_000;
const TOOL_APPROVAL_TIMEOUT: Duration = Duration::from_millis(TOOL_APPROVAL_TIMEOUT_MS as u64);

pub struct AppCore {
    storage: Storage,
    conversation_id: String,
    runtime: RwLock<RuntimeState>,
    provider: RwLock<ProviderSettings>,
    pet: RwLock<PetSettings>,
    character: RwLock<CharacterSettings>,
    voice_settings: RwLock<VoiceSettings>,
    automatic_memory_candidates_enabled: AtomicBool,
    secrets: SecretStore,
    active_turn: Mutex<Option<ActiveTurn>>,
    interaction_command: Mutex<()>,
    active_playback: Mutex<Option<(String, u64)>>,
    audio_writer: Mutex<Option<AudioWriter>>,
    audio_connection_id: Mutex<Option<String>>,
    audio_last_pong: Mutex<Option<(String, Instant)>>,
    audio_launch: Mutex<()>,
    voice_command: Mutex<()>,
    listening_requested: AtomicBool,
    audio_recovery_running: AtomicBool,
    audio_generation: AtomicU64,
    voice_final_tx: tokio::sync::mpsc::UnboundedSender<VoiceFinal>,
    audio_recovery_tx: tokio::sync::mpsc::UnboundedSender<AppHandle>,
    screen_observation: RwLock<Option<ScreenObservationRecord>>,
    timer_notify: Arc<Notify>,
    timer_scheduler_started: AtomicBool,
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
        let latest_turn_generation = storage
            .latest_turn_generation(&conversation_id)
            .map_err(storage_error)?;
        let secrets = SecretStore;
        let mut provider = storage.load_provider_settings().map_err(storage_error)?;
        provider.has_api_key = secrets.resolve_llm_api_key()?.is_some();
        provider.vision.has_api_key = secrets.resolve_vision_api_key()?.is_some();
        let pet = storage.load_pet_settings().map_err(storage_error)?;
        let character = storage.load_character_settings().map_err(storage_error)?;
        let voice_settings = storage.load_voice_settings().map_err(storage_error)?;
        let memory_settings = storage.load_memory_settings().map_err(storage_error)?;
        let (voice_final_tx, mut voice_final_rx) = tokio::sync::mpsc::unbounded_channel();
        let (audio_recovery_tx, mut audio_recovery_rx) = tokio::sync::mpsc::unbounded_channel();
        let timer_notify = Arc::new(Notify::new());
        let core = Arc::new(Self {
            storage,
            conversation_id,
            runtime: RwLock::new(RuntimeState::default()),
            provider: RwLock::new(provider),
            pet: RwLock::new(pet),
            character: RwLock::new(character),
            voice_settings: RwLock::new(voice_settings),
            automatic_memory_candidates_enabled: AtomicBool::new(
                memory_settings.automatic_candidates_enabled,
            ),
            secrets,
            active_turn: Mutex::new(None),
            interaction_command: Mutex::new(()),
            active_playback: Mutex::new(None),
            audio_writer: Mutex::new(None),
            audio_connection_id: Mutex::new(None),
            audio_last_pong: Mutex::new(None),
            audio_launch: Mutex::new(()),
            voice_command: Mutex::new(()),
            listening_requested: AtomicBool::new(false),
            audio_recovery_running: AtomicBool::new(false),
            audio_generation: AtomicU64::new(0),
            voice_final_tx,
            audio_recovery_tx,
            screen_observation: RwLock::new(None),
            timer_notify,
            timer_scheduler_started: AtomicBool::new(false),
            next_generation: AtomicU64::new(latest_turn_generation),
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
        let weak_core = Arc::downgrade(&core);
        tauri::async_runtime::spawn(async move {
            while let Some(app) = audio_recovery_rx.recv().await {
                let Some(core) = weak_core.upgrade() else {
                    break;
                };
                core.recover_requested_listening(app).await;
            }
        });
        Ok(core)
    }

    pub fn start_timer_scheduler(self: &Arc<Self>, app: AppHandle) {
        if self.timer_scheduler_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let core = Arc::clone(self);
        let notify = Arc::clone(&self.timer_notify);
        let reconcile_core = Arc::clone(self);
        let reconcile_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let timers = match reconcile_core.storage.list_all_pending_timers() {
                Ok(timers) => timers,
                Err(error) => {
                    let mut runtime = reconcile_core.runtime.write().await;
                    runtime.revision += 1;
                    runtime.last_error = Some(storage_error(error));
                    drop(runtime);
                    let _ = reconcile_core.emit_snapshot(&reconcile_app).await;
                    return;
                }
            };
            if let Err(error) = notifications::reconcile(&reconcile_app, &timers).await {
                let mut runtime = reconcile_core.runtime.write().await;
                runtime.revision += 1;
                runtime.last_error = Some(error);
                drop(runtime);
                let _ = reconcile_core.emit_snapshot(&reconcile_app).await;
            }
        });
        tauri::async_runtime::spawn(async move {
            loop {
                let next = core.storage.next_pending_timer();
                let timer = match next {
                    Ok(timer) => timer,
                    Err(error) => {
                        let mut runtime = core.runtime.write().await;
                        runtime.revision += 1;
                        runtime.last_error = Some(storage_error(error));
                        drop(runtime);
                        let _ = core.emit_snapshot(&app).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };
                match timer {
                    Some(timer) => {
                        let wait_ms = (timer.due_at_ms - now_ms()).max(0) as u64;
                        if wait_ms == 0 {
                            core.fire_due_timer(&app, &timer.id).await;
                            continue;
                        }
                        let wait = Duration::from_millis(wait_ms).min(Duration::from_secs(60));
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = notify.notified() => {}
                        }
                    }
                    None => notify.notified().await,
                }
            }
        });
    }

    pub async fn snapshot(&self) -> Result<AppSnapshot, AppError> {
        let runtime = self.runtime.read().await;
        let provider = self.provider.read().await.clone();
        let pet = self.pet.read().await.clone();
        let character = self.character.read().await.clone();
        let relationship = self
            .storage
            .load_relationship_state(&character.character_id)
            .map_err(storage_error)?;
        let voice_settings = self.voice_settings.read().await.clone();
        let messages = self
            .storage
            .list_messages(&self.conversation_id, 200)
            .map_err(storage_error)?;
        let timers = self
            .storage
            .list_pending_timers(20)
            .map_err(storage_error)?;
        let timer_history = self.storage.list_timer_history(20).map_err(storage_error)?;
        let tool_history = self
            .storage
            .list_tool_runs(&self.conversation_id, 30)
            .map_err(storage_error)?;
        let screen_observation = {
            let mut observation = self.screen_observation.write().await;
            if observation.as_ref().is_some_and(|record| {
                screen_observation_expired(record.metadata.expires_at_ms, now_ms())
            }) {
                *observation = None;
            }
            observation.as_ref().map(|record| record.metadata.clone())
        };
        Ok(AppSnapshot {
            revision: runtime.revision,
            screen_understanding_mode: runtime.screen_understanding_mode,
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
            relationship,
            audio_host_state: runtime.audio_host_state.clone(),
            voice_permissions: runtime.voice_permissions.clone(),
            voice_settings,
            asr_partial: runtime.asr_partial.clone(),
            voice_input_route: runtime.voice_input_route.clone(),
            mouth_level: runtime.mouth_level,
            pending_tool_approval: runtime
                .pending_tool
                .as_ref()
                .map(|tool| tool.approval.clone()),
            timers,
            timer_history,
            tool_history,
            screen_observation,
            last_error: runtime.last_error.clone(),
        })
    }

    pub async fn set_screen_understanding_mode(
        &self,
        app: &AppHandle,
        mode: ScreenUnderstandingMode,
    ) -> Result<ScreenUnderstandingMode, AppError> {
        if matches!(mode, ScreenUnderstandingMode::SessionAllowed)
            && !self.provider.read().await.vision.enabled
        {
            return Err(AppError::new(
                "vision_disabled",
                "请先启用 Vision Provider，再开启本次会话自动理解屏幕。",
                false,
            ));
        }
        let mut runtime = self.runtime.write().await;
        runtime.screen_understanding_mode = mode;
        runtime.revision += 1;
        drop(runtime);
        self.emit_snapshot(app).await?;
        Ok(mode)
    }

    async fn should_auto_approve_screen_action(&self, action: &PendingToolAction) -> bool {
        if !matches!(
            action,
            PendingToolAction::ObserveScreen { .. } | PendingToolAction::AnalyzeScreen { .. }
        ) {
            return false;
        }
        self.screen_automatic_mode_enabled().await
    }

    async fn screen_automatic_mode_enabled(&self) -> bool {
        screen_mode_allows_automatic_analysis(
            self.runtime.read().await.screen_understanding_mode,
            self.provider.read().await.vision.enabled,
        )
    }

    fn schedule_auto_approve_screen_tool(
        self: &Arc<Self>,
        app: AppHandle,
        tool_call_id: String,
        turn_id: String,
        generation: u64,
    ) {
        let weak_core = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            // Let the caller release interaction_command before decide_tool
            // acquires it again. This also keeps the regular approval path
            // identical for explicit and model-suggested screen tools.
            tokio::task::yield_now().await;
            let Some(core) = weak_core.upgrade() else {
                return;
            };
            if !core.screen_automatic_mode_enabled().await {
                core.schedule_tool_approval_expiration(app, tool_call_id, turn_id, generation);
                return;
            }
            if let Err(error) = core.decide_tool(&app, &tool_call_id, true).await {
                let mut runtime = core.runtime.write().await;
                runtime.revision += 1;
                runtime.last_error = Some(error);
                drop(runtime);
                let _ = core.emit_snapshot(&app).await;
            }
        });
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
        let open_url = parse_open_url_request(&text)?;
        let read_webpage = if open_url.is_none() {
            parse_read_webpage_request(&text)?
        } else {
            None
        };
        let timer_request = if open_url.is_none() && read_webpage.is_none() {
            parse_timer_request(&text, now_ms())?
        } else {
            None
        };
        let memory_intent =
            if open_url.is_none() && read_webpage.is_none() && timer_request.is_none() {
                parse_explicit_memory_intent(&text)
                    .map(|intent| {
                        Ok(ExplicitMemoryIntent {
                            content: validate_memory_content(&intent.content)?,
                            scope: intent.scope,
                            kind: intent.kind,
                        })
                    })
                    .transpose()?
            } else {
                None
            };
        let memory_content = memory_intent.as_ref().map(|intent| intent.content.as_str());
        let memory_delete = if open_url.is_none()
            && read_webpage.is_none()
            && timer_request.is_none()
            && memory_content.is_none()
        {
            self.resolve_explicit_memory_delete(&text).await?
        } else {
            None
        };
        let screen_request = if open_url.is_none()
            && read_webpage.is_none()
            && timer_request.is_none()
            && memory_content.is_none()
            && memory_delete.is_none()
        {
            parse_explicit_screen_observe_request(&text)
        } else {
            None
        };
        let timer_cancel = if open_url.is_none()
            && read_webpage.is_none()
            && timer_request.is_none()
            && memory_content.is_none()
            && memory_delete.is_none()
        {
            if screen_request.is_none() {
                self.resolve_explicit_timer_cancel(&text)?
            } else {
                None
            }
        } else {
            None
        };
        let timer_list_request = open_url.is_none()
            && read_webpage.is_none()
            && timer_request.is_none()
            && memory_content.is_none()
            && memory_delete.is_none()
            && screen_request.is_none()
            && timer_cancel.is_none()
            && parse_explicit_timer_list_request(&text);

        let _interaction_guard = self.interaction_command.lock().await;
        self.interrupt_playback().await;
        self.cancel_active(&app, "superseded").await?;
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let turn_id = Uuid::now_v7().to_string();
        let user_message = self
            .storage
            .begin_turn(
                &self.conversation_id,
                &turn_id,
                generation,
                input_source,
                &text,
                now_ms(),
            )
            .map_err(storage_error)?;

        let pending_tool = open_url
            .map(|request| {
                let tool_call_id = Uuid::now_v7().to_string();
                PendingTool {
                    approval: ToolApproval {
                        tool_call_id,
                        tool_name: "system.open_url".into(),
                        display_name: "打开网页".into(),
                        summary: "只会调用系统默认浏览器打开此网址，不会读取或修改网页内容。"
                            .into(),
                        url: Some(request.url.clone()),
                        host: Some(request.host),
                        title: None,
                        due_at_ms: None,
                        expires_at_ms: Self::tool_approval_expiry_ms(),
                    },
                    turn_id: turn_id.clone(),
                    generation,
                    action: PendingToolAction::OpenUrl { url: request.url },
                    origin: PendingToolOrigin::Explicit,
                }
            })
            .or_else(|| {
                read_webpage.map(|request| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "web.read".into(),
                            display_name: "读取网页内容".into(),
                            summary: format!(
                                "只会读取 {} 的公开文字，不执行网页脚本或提交表单。网页内容会作为不可信资料返回。",
                                request.host
                            ),
                            url: Some(request.url.clone()),
                            host: Some(request.host.clone()),
                            title: None,
                            due_at_ms: None,
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::ReadWebpage {
                            url: request.url,
                            host: request.host,
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            })
            .or_else(|| {
                timer_request.map(|request| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "timer.create".into(),
                            display_name: "创建提醒".into(),
                            summary: "提醒会保存在本机，并在到期时通过 macOS 通知提示。".into(),
                            url: None,
                            host: None,
                            title: Some(request.title.clone()),
                            due_at_ms: Some(request.due_at_ms),
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::CreateTimer {
                            title: request.title,
                            due_at_ms: request.due_at_ms,
                            original_request: request.original_request,
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            })
            .or_else(|| {
                memory_intent.clone().map(|intent| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "memory.save".into(),
                            display_name: "保存记忆".into(),
                            summary: format!(
                                "这段{}只会在你确认后保存到本地长期记忆，不会自动发送给模型。",
                                memory_scope_label(intent.scope)
                            ),
                            url: None,
                            host: None,
                            title: Some(intent.content.clone()),
                            due_at_ms: None,
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::SaveMemory {
                            content: intent.content,
                            scope: intent.scope.to_owned(),
                            kind: intent.kind.to_owned(),
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            })
            .or_else(|| {
                memory_delete.map(|memory| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "memory.delete".into(),
                            display_name: "删除记忆".into(),
                            summary: "只会在确认后删除这条本地长期记忆，不会删除其他记忆。".into(),
                            url: None,
                            host: None,
                            title: Some(memory.content.clone()),
                            due_at_ms: None,
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::DeleteMemory {
                            id: memory.id,
                            content: memory.content,
                            version: memory.version,
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            })
            .or_else(|| {
                timer_cancel.map(|timer| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "timer.cancel".into(),
                            display_name: "取消提醒".into(),
                            summary: "只会取消这条已经存在的本地提醒，不会删除其他提醒。".into(),
                            url: None,
                            host: None,
                            title: Some(timer.title.clone()),
                            due_at_ms: Some(timer.due_at_ms),
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::CancelTimer {
                            timer_id: timer.id,
                            title: timer.title,
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            })
            .or_else(|| {
                screen_request.map(|request| {
                    let tool_call_id = Uuid::now_v7().to_string();
                    PendingTool {
                        approval: ToolApproval {
                            tool_call_id,
                            tool_name: "screen.observe".into(),
                            display_name: "读取当前屏幕".into(),
                            summary: format!(
                                "只会在本机生成一次{}临时截图并显示预览；不会上传给任何模型。",
                                request.scope.label()
                            ),
                            url: None,
                            host: None,
                            title: None,
                            due_at_ms: None,
                            expires_at_ms: Self::tool_approval_expiry_ms(),
                        },
                        turn_id: turn_id.clone(),
                        generation,
                        action: PendingToolAction::ObserveScreen {
                            request: request.request,
                            scope: request.scope,
                        },
                        origin: PendingToolOrigin::Explicit,
                    }
                })
            });
        if let Some(tool) = pending_tool.as_ref() {
            let arguments_json = match &tool.action {
                PendingToolAction::OpenUrl { url } => json!({ "url": url }).to_string(),
                PendingToolAction::ReadWebpage { url, host } => {
                    json!({ "url": url, "host": host }).to_string()
                }
                PendingToolAction::CreateTimer {
                    title, due_at_ms, ..
                } => json!({
                    "title": title,
                    "due_at_ms": due_at_ms,
                    "timezone": "local"
                })
                .to_string(),
                PendingToolAction::SaveMemory {
                    content,
                    scope,
                    kind,
                } => json!({ "content": content, "scope": scope, "kind": kind }).to_string(),
                PendingToolAction::DeleteMemory {
                    id,
                    content,
                    version,
                } => json!({
                    "memory_id": id,
                    "content": content,
                    "expected_version": version
                })
                .to_string(),
                PendingToolAction::CancelTimer { timer_id, title } => {
                    json!({ "timer_id": timer_id, "title": title }).to_string()
                }
                PendingToolAction::ObserveScreen { request, scope } => {
                    json!({ "request": request, "scope": scope, "send_to_provider": false })
                        .to_string()
                }
                PendingToolAction::AnalyzeScreen {
                    observation_id,
                    question,
                } => json!({
                    "observation_id": observation_id,
                    "question": question,
                    "send_to_provider": true
                })
                .to_string(),
            };
            self.storage
                .begin_tool_run(
                    ToolRunInput {
                        id: &tool.approval.tool_call_id,
                        conversation_id: &self.conversation_id,
                        turn_id: &turn_id,
                        tool_name: &tool.approval.tool_name,
                        arguments_json: &arguments_json,
                        origin: if matches!(&tool.origin, PendingToolOrigin::Model { .. }) {
                            "model_suggested"
                        } else {
                            "user_explicit"
                        },
                    },
                    now_ms(),
                )
                .map_err(storage_error)?;
            if let PendingToolAction::SaveMemory {
                content,
                scope,
                kind,
            } = &tool.action
            {
                let confidence = if matches!(&tool.origin, PendingToolOrigin::Explicit) {
                    1.0
                } else {
                    0.7
                };
                let sensitivity = classify_memory_sensitivity(content);
                if sensitivity == "restricted" {
                    let _ = self.storage.finish_tool_run(
                        &tool.approval.tool_call_id,
                        "failed",
                        "拒绝把凭据或密钥写入长期记忆。",
                        now_ms(),
                    );
                    return Err(AppError::new(
                        "memory_restricted",
                        "出于安全原因，密码、密钥、Token 等凭据不能写入长期记忆。",
                        false,
                    ));
                }
                let fact_key = derive_memory_fact_key(scope, kind, content);
                if let Err(error) =
                    self.storage
                        .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                            id: &tool.approval.tool_call_id,
                            source_turn_id: &turn_id,
                            source_message_id: Some(&user_message.id),
                            scope,
                            kind,
                            content,
                            confidence,
                            sensitivity,
                            origin: if matches!(&tool.origin, PendingToolOrigin::Explicit) {
                                "user_explicit"
                            } else {
                                "model_suggested"
                            },
                            reason: if matches!(&tool.origin, PendingToolOrigin::Explicit) {
                                "老师明确要求保存这段内容，仍保留审核记录。"
                            } else {
                                "模型依据已验证的用户保存意图提出候选，仍需老师确认。"
                            },
                            fact_key: fact_key.as_deref(),
                            proposed_action: if fact_key.is_some() {
                                "replace"
                            } else {
                                "create"
                            },
                            expires_at_ms: Some(now_ms() + 30 * 24 * 60 * 60 * 1_000),
                            created_at_ms: now_ms(),
                        })
                {
                    let _ = self.storage.finish_tool_run(
                        &tool.approval.tool_call_id,
                        "failed",
                        "无法创建本地记忆候选。",
                        now_ms(),
                    );
                    return Err(storage_error(error));
                }
            }
        }

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
            runtime.pending_tool = pending_tool.clone();
        }
        self.emit_snapshot(&app).await?;

        if let Some(pending) = pending_tool.as_ref() {
            if self
                .should_auto_approve_screen_action(&pending.action)
                .await
            {
                self.schedule_auto_approve_screen_tool(
                    app.clone(),
                    pending.approval.tool_call_id.clone(),
                    pending.turn_id.clone(),
                    pending.generation,
                );
                return Ok(());
            }
            self.schedule_tool_approval_expiration(
                app.clone(),
                pending.approval.tool_call_id.clone(),
                pending.turn_id.clone(),
                pending.generation,
            );
            return Ok(());
        }

        let core = Arc::clone(self);
        let task: Pin<Box<dyn Future<Output = ()> + Send>> = if timer_list_request {
            Box::pin(async move {
                core.run_local_timer_list_turn(app, turn_id, generation, cancel)
                    .await;
            })
        } else {
            Box::pin(async move {
                core.run_text_turn(app, turn_id, generation, text, cancel)
                    .await;
            })
        };
        tauri::async_runtime::spawn(task);
        Ok(())
    }

    async fn run_local_timer_list_turn(
        self: Arc<Self>,
        app: AppHandle,
        turn_id: String,
        generation: u64,
        cancel: CancellationToken,
    ) {
        if cancel.is_cancelled() || !self.is_current(&turn_id, generation).await {
            return;
        }
        let timers = match self.storage.list_all_pending_timers() {
            Ok(timers) => timers,
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
        let response = format_pending_timer_list(&timers, now_ms());
        self.complete_turn(&app, &turn_id, generation, &response)
            .await;
    }

    async fn run_text_turn(
        self: Arc<Self>,
        app: AppHandle,
        turn_id: String,
        generation: u64,
        text: String,
        cancel: CancellationToken,
    ) {
        let history: Vec<_> = match self.storage.list_messages(&self.conversation_id, 24) {
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
            let memories = match self
                .storage
                .search_memories_for_context(&text, &character_id, 6)
            {
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
        let mut messages = history
            .into_iter()
            .map(|message| match message.role {
                crate::contracts::MessageRole::User => LlmMessage::User {
                    content: message.canonical_text,
                },
                crate::contracts::MessageRole::Assistant => LlmMessage::Assistant {
                    content: Some(message.canonical_text),
                    tool_calls: Vec::new(),
                },
            })
            .collect::<Vec<_>>();
        messages.push(LlmMessage::User { content: text });
        self.run_agent_loop(
            app,
            turn_id,
            generation,
            LlmRequest {
                messages,
                memory_context,
                tools: if MODEL_TOOL_CALLS_ENABLED {
                    ToolRegistry::definitions()
                } else {
                    Vec::new()
                },
                tool_steps: 0,
            },
            cancel,
        )
        .await;
    }

    /// Run one conversational Agent Loop.
    ///
    /// A normal turn ends only when the configured LLM emits text and a
    /// completion event. If the LLM emits a tool call, the call is validated,
    /// approved/executed by Rust, and the tool result is fed into the next
    /// loop step. No user text is converted into a canned assistant sentence.
    async fn run_agent_loop(
        self: Arc<Self>,
        app: AppHandle,
        turn_id: String,
        generation: u64,
        request: LlmRequest,
        cancel: CancellationToken,
    ) {
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
        let mut stream = None;
        let mut last_start_error = None;
        for attempt in 0..MAX_PROVIDER_START_ATTEMPTS {
            match tokio::time::timeout(
                Duration::from_secs(15),
                provider.start(request.clone(), cancel.clone()),
            )
            .await
            {
                Ok(Ok(value)) => {
                    stream = Some(value);
                    break;
                }
                Ok(Err(error)) if error.code == "cancelled" => return,
                Ok(Err(error)) => {
                    let should_retry = error.retryable && attempt + 1 < MAX_PROVIDER_START_ATTEMPTS;
                    last_start_error = Some(error);
                    if !should_retry {
                        break;
                    }
                }
                Err(_) => {
                    let error = AppError::new("timeout", "LLM 在 15 秒内未建立连接。", true);
                    let should_retry = attempt + 1 < MAX_PROVIDER_START_ATTEMPTS;
                    last_start_error = Some(error);
                    if !should_retry {
                        break;
                    }
                }
            }
            let backoff_ms = 250_u64.saturating_mul(2_u64.saturating_pow(attempt as u32));
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
            }
        }
        let Some(mut stream) = stream else {
            let error = last_start_error.unwrap_or_else(|| {
                AppError::new("provider_unavailable", "LLM 未能建立连接。", true)
            });
            let failure_code = if error.code == "timeout" {
                "timeout"
            } else {
                "provider_unavailable"
            };
            self.fail_turn(&app, &turn_id, generation, failure_code, error, "")
                .await;
            return;
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
                        Some(Ok(LlmEvent::ToolCalls(calls))) => {
                            self.emit_pending_delta(&app, &turn_id, generation, &mut pending_delta).await;
                            if request.tool_steps >= MAX_TOOL_STEPS {
                                self.fail_turn(
                                    &app,
                                    &turn_id,
                                    generation,
                                    "tool_loop_limit",
                                    AppError::new("tool_loop_limit", "本轮工具调用已达到 3 步上限。", false),
                                    &response,
                                ).await;
                                return;
                            }
                            if calls.len() != 1 {
                                self.fail_turn(
                                    &app,
                                    &turn_id,
                                    generation,
                                    "tool_not_supported",
                                    AppError::new("tool_not_supported", "当前一次只支持一个工具调用。", false),
                                    &response,
                                ).await;
                                return;
                            }
                            let call = calls.into_iter().next().expect("one tool call");
                            if let Err(error) = self
                                .queue_model_tool_call(
                                    ModelToolContext {
                                        app: &app,
                                        turn_id: &turn_id,
                                        generation,
                                        messages: &request.messages,
                                        memory_context: &request.memory_context,
                                        response: &response,
                                        tool_steps: request.tool_steps,
                                    },
                                    call,
                                )
                                .await
                            {
                                self.fail_turn(&app, &turn_id, generation, "tool_invalid", error, &response).await;
                            }
                            return;
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
                                ).await;
                                return;
                            }
                            self.complete_turn(&app, &turn_id, generation, &response).await;
                            return;
                        }
                        Some(Err(error)) if error.code == "cancelled" => return,
                        Some(Err(error)) => {
                            self.fail_turn(&app, &turn_id, generation, "provider_error", error, &response).await;
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

    async fn queue_model_tool_call(
        self: &Arc<Self>,
        context: ModelToolContext<'_>,
        call: ToolCall,
    ) -> Result<(), AppError> {
        let mut candidate = ToolPolicy::validate_model_call(&call, now_ms())?;
        if call.name == "memory.save" {
            let explicit_memory_intent =
                context
                    .messages
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        LlmMessage::User { content } => parse_explicit_memory_intent(content),
                        _ => None,
                    });
            let Some(intent) = explicit_memory_intent else {
                return Err(AppError::new(
                    "memory_confirmation_required",
                    "只有用户明确要求记住或保存信息时，模型才能提出记忆草案。",
                    false,
                ));
            };
            let scope = intent.scope.to_owned();
            let kind = intent.kind.to_owned();
            if let PendingToolAction::SaveMemory {
                scope: candidate_scope,
                kind: candidate_kind,
                ..
            } = &mut candidate.action
            {
                *candidate_scope = scope.clone();
                *candidate_kind = kind.clone();
            }
            candidate.summary = format!(
                "模型请求把这段{}保存到本地长期记忆；执行前需要你的确认。",
                memory_scope_label(&scope)
            );
            if let PendingToolAction::SaveMemory { content, .. } = &candidate.action {
                candidate.arguments_json =
                    json!({ "content": content, "scope": scope, "kind": kind }).to_string();
            }
        }
        if call.name == "screen.observe" && has_explicit_screen_observe_refusal(context.messages) {
            return Err(AppError::new(
                "screen_confirmation_required",
                "用户已明确拒绝读取或上传屏幕内容，未创建屏幕观察请求。",
                false,
            ));
        }
        if let PendingToolAction::ReadWebpage { url, host } = &candidate.action {
            let user_request = context
                .messages
                .iter()
                .rev()
                .find_map(|message| match message {
                    LlmMessage::User { content } => Some(content.as_str()),
                    _ => None,
                })
                .unwrap_or_default();
            if !user_request.contains(url) && !user_request.contains(host) {
                return Err(AppError::new(
                    "web_confirmation_required",
                    "模型只能读取用户当前请求中明确给出的网页地址。",
                    false,
                ));
            }
        }
        if let PendingToolAction::CreateTimer {
            original_request, ..
        } = &mut candidate.action
        {
            if let Some(user_request) =
                context
                    .messages
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        LlmMessage::User { content } => Some(content.trim()),
                        _ => None,
                    })
            {
                *original_request = user_request.to_owned();
            }
        }
        if let PendingToolAction::CancelTimer { title, .. } = &candidate.action {
            let user_request = context
                .messages
                .iter()
                .rev()
                .find_map(|message| match message {
                    LlmMessage::User { content } => Some(content.as_str()),
                    _ => None,
                })
                .unwrap_or_default();
            if !user_request.contains(title) {
                return Err(AppError::new(
                    "timer_confirmation_required",
                    "模型只能取消用户当前请求中明确提到标题的提醒。",
                    false,
                ));
            }
            let timer = self.resolve_pending_timer(title)?;
            candidate.action = PendingToolAction::CancelTimer {
                timer_id: timer.id.clone(),
                title: timer.title.clone(),
            };
            candidate.title = Some(timer.title.clone());
            candidate.due_at_ms = Some(timer.due_at_ms);
            candidate.arguments_json = json!({
                "timer_id": timer.id,
                "title": timer.title,
            })
            .to_string();
        }
        let tool_call_id = Uuid::now_v7().to_string();
        self.storage
            .begin_tool_run(
                ToolRunInput {
                    id: &tool_call_id,
                    conversation_id: &self.conversation_id,
                    turn_id: context.turn_id,
                    tool_name: &call.name,
                    arguments_json: &candidate.arguments_json,
                    origin: "model_suggested",
                },
                now_ms(),
            )
            .map_err(storage_error)?;
        if let PendingToolAction::SaveMemory {
            content,
            scope,
            kind,
        } = &candidate.action
        {
            let sensitivity = classify_memory_sensitivity(content);
            if sensitivity == "restricted" {
                let _ = self.storage.finish_tool_run(
                    &tool_call_id,
                    "failed",
                    "拒绝把凭据或密钥写入长期记忆。",
                    now_ms(),
                );
                return Err(AppError::new(
                    "memory_restricted",
                    "出于安全原因，密码、密钥、Token 等凭据不能写入长期记忆。",
                    false,
                ));
            }
            let fact_key = derive_memory_fact_key(scope, kind, content);
            if let Err(error) =
                self.storage
                    .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                        id: &tool_call_id,
                        source_turn_id: context.turn_id,
                        source_message_id: None,
                        scope,
                        kind,
                        content,
                        confidence: 0.7,
                        sensitivity,
                        origin: "model_suggested",
                        reason: "模型依据已验证的用户保存意图提出候选，仍需老师确认。",
                        fact_key: fact_key.as_deref(),
                        proposed_action: if fact_key.is_some() {
                            "replace"
                        } else {
                            "create"
                        },
                        expires_at_ms: Some(now_ms() + 30 * 24 * 60 * 60 * 1_000),
                        created_at_ms: now_ms(),
                    })
            {
                let _ = self.storage.finish_tool_run(
                    &tool_call_id,
                    "failed",
                    "无法创建本地记忆候选。",
                    now_ms(),
                );
                return Err(storage_error(error));
            }
        }
        let mut continuation = context.messages.to_vec();
        continuation.push(LlmMessage::Assistant {
            content: (!context.response.trim().is_empty()).then(|| context.response.to_owned()),
            tool_calls: vec![call.clone()],
        });
        let pending = PendingTool {
            approval: ToolApproval {
                tool_call_id: tool_call_id.clone(),
                tool_name: call.name.clone(),
                display_name: candidate.display_name,
                summary: candidate.summary,
                url: candidate.url,
                host: candidate.host,
                title: candidate.title,
                due_at_ms: candidate.due_at_ms,
                expires_at_ms: Self::tool_approval_expiry_ms(),
            },
            turn_id: context.turn_id.to_owned(),
            generation: context.generation,
            action: candidate.action,
            origin: PendingToolOrigin::Model {
                call,
                messages: continuation,
                memory_context: context.memory_context.to_vec(),
                tool_steps: context.tool_steps + 1,
            },
        };
        let auto_approve = self
            .should_auto_approve_screen_action(&pending.action)
            .await;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Active;
            runtime.generation_state = GenerationState::Finished;
            runtime.pending_tool = Some(pending);
        }
        self.emit_snapshot(context.app).await?;
        if auto_approve {
            self.schedule_auto_approve_screen_tool(
                context.app.clone(),
                tool_call_id,
                context.turn_id.to_owned(),
                context.generation,
            );
        } else {
            self.schedule_tool_approval_expiration(
                context.app.clone(),
                tool_call_id,
                context.turn_id.to_owned(),
                context.generation,
            );
        }
        Ok(())
    }

    async fn queue_vision_analysis(
        self: &Arc<Self>,
        app: &AppHandle,
        turn_id: &str,
        generation: u64,
        observation_id: String,
        question: String,
        origin: PendingToolOrigin,
    ) -> Result<(), AppError> {
        let question = question.trim().to_owned();
        if question.is_empty() || question.chars().count() > 500 {
            return Err(AppError::new(
                "invalid_request",
                "视觉分析问题必须是 1 到 500 个字符。",
                false,
            ));
        }
        let scope = {
            let observation = self.screen_observation.read().await;
            let Some(record) = observation.as_ref() else {
                return Err(AppError::new(
                    "not_found",
                    "当前没有可发送给视觉模型的屏幕预览。",
                    false,
                ));
            };
            if record.metadata.id != observation_id
                || screen_observation_expired(record.metadata.expires_at_ms, now_ms())
            {
                return Err(AppError::new(
                    "screen_observation_expired",
                    "本地屏幕预览已过期，请重新读取后再分析。",
                    false,
                ));
            }
            record.metadata.scope
        };
        let provider = self.provider.read().await.clone();
        if !provider.vision.enabled {
            return Err(AppError::new(
                "vision_disabled",
                "视觉 Provider 当前未启用，请先在设置中开启。",
                false,
            ));
        }
        let tool_call_id = Uuid::now_v7().to_string();
        let origin_label = if matches!(&origin, PendingToolOrigin::Model { .. }) {
            "model_suggested"
        } else {
            "user_explicit"
        };
        let arguments_json = json!({
            "observation_id": observation_id,
            "question": question,
            "send_to_provider": true
        })
        .to_string();
        self.storage
            .begin_tool_run(
                ToolRunInput {
                    id: &tool_call_id,
                    conversation_id: &self.conversation_id,
                    turn_id,
                    tool_name: "vision.analyze",
                    arguments_json: &arguments_json,
                    origin: origin_label,
                },
                now_ms(),
            )
            .map_err(storage_error)?;
        let pending = PendingTool {
            approval: ToolApproval {
                tool_call_id: tool_call_id.clone(),
                tool_name: "vision.analyze".into(),
                display_name: "发送屏幕给视觉模型".into(),
                summary: format!(
                    "确认后会把{}的当前截图发送到 Vision Provider（{} / {}），只用于回答本次问题；截图不会写入数据库或长期记忆。",
                    scope.label(),
                    vision_endpoint_label(&provider.vision.endpoint),
                    provider.vision.model
                ),
                url: None,
                host: None,
                title: Some(question.clone()),
                due_at_ms: None,
                expires_at_ms: Self::tool_approval_expiry_ms(),
            },
            turn_id: turn_id.to_owned(),
            generation,
            action: PendingToolAction::AnalyzeScreen {
                observation_id,
                question,
            },
            origin,
        };
        let auto_approve = self
            .should_auto_approve_screen_action(&pending.action)
            .await;
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Active;
            runtime.generation_state = GenerationState::Finished;
            runtime.pending_tool = Some(pending);
        }
        self.emit_snapshot(app).await?;
        if auto_approve {
            self.schedule_auto_approve_screen_tool(
                app.clone(),
                tool_call_id,
                turn_id.to_owned(),
                generation,
            );
        } else {
            self.schedule_tool_approval_expiration(
                app.clone(),
                tool_call_id,
                turn_id.to_owned(),
                generation,
            );
        }
        Ok(())
    }

    fn schedule_tool_approval_expiration(
        self: &Arc<Self>,
        app: AppHandle,
        tool_call_id: String,
        turn_id: String,
        generation: u64,
    ) {
        let weak_core = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(TOOL_APPROVAL_TIMEOUT).await;
            let Some(core) = weak_core.upgrade() else {
                return;
            };
            let _interaction_guard = core.interaction_command.lock().await;
            let pending = {
                let mut runtime = core.runtime.write().await;
                if !Self::tool_approval_matches(
                    runtime.pending_tool.as_ref(),
                    &tool_call_id,
                    &turn_id,
                    generation,
                ) {
                    return;
                }
                let pending = runtime.pending_tool.take().expect("pending tool exists");
                runtime.revision += 1;
                runtime.turn_state = TurnState::Active;
                runtime.generation_state = GenerationState::Finished;
                pending
            };
            if matches!(&pending.action, PendingToolAction::SaveMemory { .. }) {
                let _ = core
                    .storage
                    .reject_memory_candidate(&pending.approval.tool_call_id, now_ms());
            }
            let _ = core.storage.finish_tool_run(
                &pending.approval.tool_call_id,
                "cancelled",
                "确认超时，工具未执行。",
                now_ms(),
            );
            let _ = core.emit_snapshot(&app).await;
            if matches!(&pending.origin, PendingToolOrigin::Model { .. }) {
                let _ = core
                    .resume_model_tool(&app, pending, "用户未在 2 分钟内确认，工具未执行。".into())
                    .await;
            } else {
                core.complete_turn(
                    &app,
                    &pending.turn_id,
                    pending.generation,
                    "好的，老师，这次工具确认已超时，动作没有执行。",
                )
                .await;
            }
        });
    }

    fn tool_approval_matches(
        pending: Option<&PendingTool>,
        tool_call_id: &str,
        turn_id: &str,
        generation: u64,
    ) -> bool {
        pending.is_some_and(|tool| {
            tool.approval.tool_call_id == tool_call_id
                && tool.turn_id == turn_id
                && tool.generation == generation
        })
    }

    fn tool_approval_expiry_ms() -> i64 {
        now_ms().saturating_add(TOOL_APPROVAL_TIMEOUT_MS)
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
        let character_id = self.character.read().await.character_id.clone();
        if let Err(error) = self.storage.complete_turn(
            &self.conversation_id,
            &character_id,
            turn_id,
            response,
            now_ms(),
        ) {
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
        if let Ok(Some(user_message)) = self
            .storage
            .user_message_for_turn(&self.conversation_id, turn_id)
        {
            self.record_completed_interaction(&user_message, &character_id);
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

    fn record_completed_interaction(&self, user_message: &ChatMessage, character_id: &str) {
        let text = user_message.canonical_text.trim();
        if self
            .automatic_memory_candidates_enabled
            .load(Ordering::Relaxed)
            && !is_no_memory_request(text)
            && parse_explicit_memory_intent(text).is_none()
        {
            if let Some(candidate) = extract_automatic_memory_candidate(text) {
                let sensitivity = classify_memory_sensitivity(&candidate.content);
                if sensitivity != "restricted" {
                    let fact_key =
                        derive_memory_fact_key(candidate.scope, candidate.kind, &candidate.content);
                    let proposed_action = if fact_key.is_some() {
                        "replace"
                    } else {
                        "create"
                    };
                    match self.storage.has_duplicate_memory_or_candidate(
                        &candidate.content,
                        candidate.scope,
                        character_id,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            let id = Uuid::now_v7().to_string();
                            if let Err(error) = self.storage.create_memory_candidate_with_metadata(
                                MemoryCandidateV2Input {
                                    id: &id,
                                    source_turn_id: &user_message.turn_id,
                                    source_message_id: Some(&user_message.id),
                                    scope: candidate.scope,
                                    kind: candidate.kind,
                                    content: &candidate.content,
                                    confidence: candidate.confidence,
                                    sensitivity,
                                    origin: "automatic_candidate",
                                    reason: "从本轮用户消息中识别出偏好、目标或项目线索；是否长期保存由老师决定。",
                                    fact_key: fact_key.as_deref(),
                                    proposed_action,
                                    expires_at_ms: Some(now_ms() + 30 * 24 * 60 * 60 * 1_000),
                                    created_at_ms: now_ms(),
                                },
                            )
                            {
                                eprintln!(
                                    "automatic memory candidate unavailable [{}]: {error}",
                                    user_message.turn_id
                                );
                            }
                        }
                        Err(error) => eprintln!(
                            "automatic memory duplicate check unavailable [{}]: {error}",
                            user_message.turn_id
                        ),
                    }
                }
            }
        }
        let (affinity_delta, summary) = derive_relationship_signal(text);
        if affinity_delta != 0.0 {
            if let Err(error) = self.storage.apply_relationship_signal(
                character_id,
                affinity_delta,
                summary,
                now_ms(),
            ) {
                eprintln!(
                    "relationship signal unavailable [{}]: {error}",
                    user_message.turn_id
                );
            }
        }
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
        let pending_tool = {
            let mut runtime = self.runtime.write().await;
            if runtime
                .pending_tool
                .as_ref()
                .is_some_and(|tool| tool.turn_id == active.turn_id)
            {
                runtime.pending_tool.take()
            } else {
                None
            }
        };
        if let Some(tool) = pending_tool {
            self.storage
                .finish_tool_run(&tool.approval.tool_call_id, "cancelled", reason, now_ms())
                .map_err(storage_error)?;
        }
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

    pub async fn decide_tool(
        self: &Arc<Self>,
        app: &AppHandle,
        tool_call_id: &str,
        approved: bool,
    ) -> Result<(), AppError> {
        let _interaction_guard = self.interaction_command.lock().await;
        let pending = {
            let mut runtime = self.runtime.write().await;
            let Some(pending) = runtime.pending_tool.as_ref() else {
                return Ok(());
            };
            if pending.approval.tool_call_id != tool_call_id {
                return Err(AppError::new(
                    "conflict",
                    "工具请求已更新，请使用当前确认卡片。",
                    true,
                ));
            }
            let pending = runtime.pending_tool.take().expect("pending tool exists");
            runtime.revision += 1;
            runtime.turn_state = TurnState::Active;
            runtime.generation_state = GenerationState::Finished;
            runtime.last_error = None;
            pending
        };
        self.emit_snapshot(app).await?;

        if !approved {
            let message = match &pending.action {
                PendingToolAction::OpenUrl { url } => format!("好的，老师，已取消打开网页：{url}"),
                PendingToolAction::ReadWebpage { url, .. } => {
                    format!("好的，老师，已取消读取网页：{url}")
                }
                PendingToolAction::CreateTimer { title, .. } => {
                    format!("好的，老师，已取消提醒：{title}")
                }
                PendingToolAction::SaveMemory { content, .. } => {
                    format!("好的，老师，已取消保存这条记忆：{content}")
                }
                PendingToolAction::DeleteMemory { content, .. } => {
                    format!("好的，老师，已保留这条记忆：{content}")
                }
                PendingToolAction::CancelTimer { title, .. } => {
                    format!("好的，老师，已保留提醒：{title}")
                }
                PendingToolAction::ObserveScreen { .. } => {
                    "好的，老师，已取消读取当前屏幕。".into()
                }
                PendingToolAction::AnalyzeScreen { .. } => {
                    "好的，老师，已取消把屏幕发送给视觉模型。".into()
                }
            };
            if matches!(&pending.action, PendingToolAction::SaveMemory { .. }) {
                let _ = self
                    .storage
                    .reject_memory_candidate(&pending.approval.tool_call_id, now_ms());
            }
            self.storage
                .finish_tool_run(
                    &pending.approval.tool_call_id,
                    "rejected",
                    "用户取消了工具请求。",
                    now_ms(),
                )
                .map_err(storage_error)?;
            if matches!(&pending.origin, PendingToolOrigin::Model { .. }) {
                self.resume_model_tool(
                    app,
                    pending,
                    "用户拒绝了这次工具调用，工具没有执行。".into(),
                )
                .await?;
            } else {
                self.complete_turn(app, &pending.turn_id, pending.generation, &message)
                    .await;
            }
            return Ok(());
        }

        self.storage
            .finish_tool_run(
                &pending.approval.tool_call_id,
                "approved",
                "用户允许执行工具。",
                now_ms(),
            )
            .map_err(storage_error)?;
        let pending_tool_id = pending.approval.tool_call_id.clone();
        let execution = match pending.action.clone() {
            PendingToolAction::OpenUrl { url } => {
                let result = open_url_in_default_browser(&url).await;
                match result {
                    Ok(()) => Ok(format!("已交给系统默认浏览器打开网页：{url}")),
                    Err(error) => Err(error),
                }
            }
            PendingToolAction::ReadWebpage { url, .. } => {
                let cancel = self
                    .active_turn
                    .lock()
                    .await
                    .as_ref()
                    .map(|active| active.cancel.clone())
                    .unwrap_or_else(CancellationToken::new);
                read_webpage(&url, cancel).await
            }
            PendingToolAction::CreateTimer {
                title,
                due_at_ms,
                original_request,
            } => {
                let timer_id = Uuid::now_v7().to_string();
                match self.storage.create_timer(
                    TimerInput {
                        id: &timer_id,
                        conversation_id: &self.conversation_id,
                        tool_run_id: &pending.approval.tool_call_id,
                        title: &title,
                        due_at_ms,
                        timezone: "local",
                        original_request: &original_request,
                    },
                    now_ms(),
                ) {
                    Ok(timer) => {
                        if let Err(error) = notifications::schedule_timer(app, &timer).await {
                            let _ = self.storage.cancel_timer(&timer.id, now_ms());
                            self.timer_notify.notify_one();
                            Err(error)
                        } else {
                            self.timer_notify.notify_one();
                            Ok(format!(
                                "已创建一次性提醒：{title}，约{}后触发。",
                                format_timer_delay(due_at_ms - now_ms())
                            ))
                        }
                    }
                    Err(error) => Err(AppError::new(
                        "tool_execution_failed",
                        format!("本地提醒创建失败：{error}"),
                        true,
                    )),
                }
            }
            PendingToolAction::SaveMemory { content, kind, .. } => {
                let character_id = self.character.read().await.character_id.clone();
                let (valid_from_ms, valid_until_ms) =
                    derive_memory_validity(&kind, &content, now_ms());
                match self.storage.confirm_memory_candidate_with_validity(
                    &pending_tool_id,
                    &character_id,
                    None,
                    valid_from_ms,
                    valid_until_ms,
                    now_ms(),
                ) {
                    Ok(
                        MemoryConfirmResult::Saved(memory) | MemoryConfirmResult::Duplicate(memory),
                    ) => Ok(format!(
                        "已保存本地长期记忆：{}（记忆 ID：{}）",
                        memory.content, memory.id
                    )),
                    Ok(MemoryConfirmResult::Conflict(existing)) => Err(AppError::new(
                        "conflict",
                        format!("这条记忆与现有记忆冲突：{}", existing.content),
                        true,
                    )),
                    Err(error) => Err(AppError::new(
                        "tool_execution_failed",
                        format!("本地记忆保存失败：{error}"),
                        true,
                    )),
                }
            }
            PendingToolAction::DeleteMemory {
                id,
                content,
                version,
            } => match self.storage.delete_memory(&id, version, now_ms()) {
                Ok(true) => Ok(format!("已删除本地长期记忆：{content}")),
                Ok(false) => Err(AppError::new(
                    "conflict",
                    "这条记忆已被修改或删除，请刷新记忆列表后重试。",
                    true,
                )),
                Err(error) => Err(AppError::new(
                    "tool_execution_failed",
                    format!("本地记忆删除失败：{error}"),
                    true,
                )),
            },
            PendingToolAction::CancelTimer { timer_id, title } => {
                match self.storage.cancel_timer(&timer_id, now_ms()) {
                    Ok(true) => match notifications::cancel_timer(app, &timer_id).await {
                        Ok(()) => {
                            self.timer_notify.notify_one();
                            Ok(format!("已取消本地提醒：{title}"))
                        }
                        Err(error) => Err(error),
                    },
                    Ok(false) => Err(AppError::new(
                        "not_found",
                        "这条提醒不存在、已经触发或已经取消。",
                        false,
                    )),
                    Err(error) => Err(storage_error(error)),
                }
            }
            PendingToolAction::ObserveScreen { scope, .. } => self
                .capture_screen_observation(app, scope)
                .await
                .map(|observation| {
                    format!(
                        "已读取{}，并生成了本机临时预览（观察 ID：{}）。截图尚未发送给任何模型。",
                        observation.scope.label(),
                        observation.id,
                    )
                }),
            PendingToolAction::AnalyzeScreen {
                observation_id,
                question,
            } => {
                let png = {
                    let observation = self.screen_observation.read().await;
                    if observation.as_ref().is_some_and(|record| {
                        screen_observation_expired(record.metadata.expires_at_ms, now_ms())
                    }) {
                        return Err(AppError::new(
                            "screen_observation_expired",
                            "本地屏幕预览已过期，请重新读取后再分析。",
                            false,
                        ));
                    }
                    observation
                        .as_ref()
                        .filter(|record| record.metadata.id == observation_id)
                        .map(|record| record.png.clone())
                        .ok_or_else(|| {
                            AppError::new(
                                "not_found",
                                "当前没有可发送给视觉模型的屏幕预览。",
                                false,
                            )
                        })?
                };
                if png.len() > 8 * 1024 * 1024 {
                    return Err(AppError::new(
                        "vision_image_too_large",
                        "屏幕截图超过 8 MB，未发送给视觉模型。",
                        false,
                    ));
                }
                let provider = self.provider.read().await.clone();
                if !provider.vision.enabled {
                    return Err(AppError::new(
                        "vision_disabled",
                        "视觉 Provider 当前未启用，请先在设置中开启。",
                        false,
                    ));
                }
                let api_key = self.resolve_vision_api_key().await?;
                let cancel = self
                    .active_turn
                    .lock()
                    .await
                    .as_ref()
                    .map(|active| active.cancel.clone())
                    .unwrap_or_else(CancellationToken::new);
                analyze_image(&provider.vision, api_key, &png, &question, cancel).await
            }
        };
        match execution {
            Ok(result) => {
                self.storage
                    .finish_tool_run(
                        &pending.approval.tool_call_id,
                        "succeeded",
                        &result,
                        now_ms(),
                    )
                    .map_err(storage_error)?;
                if let PendingToolAction::ObserveScreen { request, .. } = &pending.action {
                    let vision_enabled = self.provider.read().await.vision.enabled;
                    if vision_enabled {
                        let observation_id = self
                            .screen_observation
                            .read()
                            .await
                            .as_ref()
                            .map(|record| record.metadata.id.clone());
                        if let Some(observation_id) = observation_id {
                            let origin = pending.origin.clone();
                            if let Err(error) = self
                                .queue_vision_analysis(
                                    app,
                                    &pending.turn_id,
                                    pending.generation,
                                    observation_id,
                                    request.clone(),
                                    origin,
                                )
                                .await
                            {
                                if matches!(&pending.origin, PendingToolOrigin::Model { .. }) {
                                    self.resume_model_tool(
                                        app,
                                        pending,
                                        format!(
                                            "{result}（无法准备视觉模型确认：{}）",
                                            error.message
                                        ),
                                    )
                                    .await?;
                                } else {
                                    self.complete_turn(
                                        app,
                                        &pending.turn_id,
                                        pending.generation,
                                        &format!(
                                            "{result}（无法准备视觉模型确认：{}）",
                                            error.message
                                        ),
                                    )
                                    .await;
                                }
                            }
                            return Ok(());
                        }
                    }
                }
                let is_analyze = matches!(&pending.action, PendingToolAction::AnalyzeScreen { .. });
                if is_analyze {
                    if let PendingToolAction::AnalyzeScreen { observation_id, .. } = &pending.action
                    {
                        self.mark_screen_observation_sent(observation_id, app).await;
                    }
                }
                if matches!(&pending.origin, PendingToolOrigin::Model { .. }) {
                    self.resume_model_tool(app, pending, result).await?;
                } else if is_analyze {
                    self.complete_turn(app, &pending.turn_id, pending.generation, &result)
                        .await;
                } else {
                    let message = match &pending.action {
                        PendingToolAction::OpenUrl { url } => format!("网页已打开，老师：{url}"),
                        PendingToolAction::ReadWebpage { .. } => result,
                        PendingToolAction::CreateTimer {
                            title, due_at_ms, ..
                        } => format!(
                            "已设置提醒：{title}（约{}后提醒）",
                            format_timer_delay(*due_at_ms - now_ms())
                        ),
                        PendingToolAction::SaveMemory { content, .. } => {
                            format!("已保存记忆：{content}")
                        }
                        PendingToolAction::DeleteMemory { content, .. } => {
                            format!("已删除记忆：{content}")
                        }
                        PendingToolAction::CancelTimer { title, .. } => {
                            format!("已取消提醒：{title}")
                        }
                        PendingToolAction::ObserveScreen { .. } => {
                            "已读取当前屏幕，老师可以在本地预览区查看截图。".into()
                        }
                        PendingToolAction::AnalyzeScreen { .. } => "已完成视觉模型分析。".into(),
                    };
                    self.complete_turn(app, &pending.turn_id, pending.generation, &message)
                        .await;
                }
            }
            Err(error) => {
                let error_message = error.message.clone();
                let _ = self.storage.finish_tool_run(
                    &pending.approval.tool_call_id,
                    "failed",
                    &error_message,
                    now_ms(),
                );
                if matches!(&pending.origin, PendingToolOrigin::Model { .. }) {
                    self.resume_model_tool(app, pending, format!("工具执行失败：{error_message}"))
                        .await?;
                } else {
                    self.complete_turn(
                        app,
                        &pending.turn_id,
                        pending.generation,
                        &format!("工具执行失败：{error_message}"),
                    )
                    .await;
                }
            }
        }
        Ok(())
    }

    async fn resume_model_tool(
        self: &Arc<Self>,
        app: &AppHandle,
        pending: PendingTool,
        tool_result: String,
    ) -> Result<(), AppError> {
        let PendingToolOrigin::Model {
            call,
            mut messages,
            memory_context,
            tool_steps,
        } = pending.origin
        else {
            return Err(AppError::new(
                "internal",
                "显式工具请求不能恢复模型工具循环。",
                false,
            ));
        };
        messages.push(LlmMessage::Tool {
            tool_call_id: call.id,
            content: tool_result,
        });
        let cancel = self
            .active_turn
            .lock()
            .await
            .as_ref()
            .filter(|active| {
                active.turn_id == pending.turn_id && active.generation == pending.generation
            })
            .map(|active| active.cancel.clone())
            .ok_or_else(|| {
                AppError::new("conflict", "当前对话已经结束，无法继续工具请求。", true)
            })?;
        let core = Arc::clone(self);
        let turn_id = pending.turn_id;
        let generation = pending.generation;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            core.run_agent_loop(
                app,
                turn_id,
                generation,
                LlmRequest {
                    messages,
                    memory_context,
                    tools: ToolRegistry::definitions(),
                    tool_steps,
                },
                cancel,
            )
            .await;
        });
        Ok(())
    }

    fn resolve_explicit_timer_cancel(&self, text: &str) -> Result<Option<TimerItem>, AppError> {
        let Some(query) = parse_timer_cancel_query(text) else {
            return Ok(None);
        };
        self.resolve_pending_timer(&query).map(Some)
    }

    async fn resolve_explicit_memory_delete(
        &self,
        text: &str,
    ) -> Result<Option<MemoryItem>, AppError> {
        let Some(query) = parse_explicit_memory_delete_query(text) else {
            return Ok(None);
        };
        let character_id = self.character.read().await.character_id.clone();
        let candidates = self
            .storage
            .search_memories(&query, &character_id, 20)
            .map_err(storage_error)?;
        resolve_unique_memory_match(candidates, &query).map(Some)
    }

    fn resolve_pending_timer(&self, query: &str) -> Result<TimerItem, AppError> {
        let timers = self
            .storage
            .list_pending_timers(50)
            .map_err(storage_error)?;
        let query = query.trim();
        let matches = timers
            .into_iter()
            .filter(|timer| query.is_empty() || timer.title == query || timer.title.contains(query))
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(AppError::new(
                "timer_not_found",
                "没有找到能唯一匹配的待处理提醒，请在提醒列表中选择。",
                false,
            )),
            1 => Ok(matches.into_iter().next().expect("one timer match")),
            _ => Err(AppError::new(
                "timer_ambiguous",
                "有多条提醒可能匹配，请在提醒列表中选择要取消的那一条。",
                false,
            )),
        }
    }

    async fn capture_screen_observation(
        self: &Arc<Self>,
        app: &AppHandle,
        scope: ScreenCaptureScope,
    ) -> Result<ScreenObservation, AppError> {
        let png = capture_screen_png(scope).await?;
        if png.len() > 32 * 1024 * 1024 {
            return Err(AppError::new(
                "screen_capture_too_large",
                "当前屏幕截图超过 32 MB，未保存到本地预览。",
                false,
            ));
        }
        if png.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
            return Err(AppError::new(
                "screen_capture_invalid",
                "macOS 返回的屏幕截图格式无法识别。",
                false,
            ));
        }
        let ocr_text = match run_local_screen_ocr(app, &png).await {
            Ok(text) => text,
            Err(error) => {
                eprintln!(
                    "local screen OCR unavailable [{}]: {}",
                    error.diagnostic_id, error.message
                );
                None
            }
        };
        let captured_at_ms = now_ms();
        let metadata = ScreenObservation {
            id: Uuid::now_v7().to_string(),
            captured_at_ms,
            expires_at_ms: captured_at_ms.saturating_add(SCREEN_OBSERVATION_TTL_MS),
            scope,
            sent_to_vision: false,
        };
        *self.screen_observation.write().await = Some(ScreenObservationRecord {
            metadata: metadata.clone(),
            png,
            ocr_text,
        });
        self.runtime.write().await.revision += 1;
        if let Err(error) = self.emit_snapshot(app).await {
            *self.screen_observation.write().await = None;
            return Err(error);
        }
        let weak_core = Arc::downgrade(self);
        let observation_id = metadata.id.clone();
        let expiration_app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(SCREEN_OBSERVATION_TTL_MS as u64)).await;
            let Some(core) = weak_core.upgrade() else {
                return;
            };
            let removed = {
                let mut observation = core.screen_observation.write().await;
                let should_remove = observation.as_ref().is_some_and(|record| {
                    record.metadata.id == observation_id
                        && screen_observation_expired(record.metadata.expires_at_ms, now_ms())
                });
                if should_remove {
                    *observation = None;
                }
                should_remove
            };
            if removed {
                core.runtime.write().await.revision += 1;
                let _ = core.emit_snapshot(&expiration_app).await;
            }
        });
        Ok(metadata)
    }

    async fn mark_screen_observation_sent(&self, observation_id: &str, app: &AppHandle) {
        let marked = {
            let mut observation = self.screen_observation.write().await;
            let Some(record) = observation.as_mut() else {
                return;
            };
            if record.metadata.id != observation_id {
                return;
            }
            record.metadata.sent_to_vision = true;
            true
        };
        if marked {
            self.runtime.write().await.revision += 1;
            let _ = self.emit_snapshot(app).await;
        }
    }

    pub async fn get_screen_observation_preview(
        &self,
        observation_id: &str,
    ) -> Result<ScreenObservationPreview, AppError> {
        if observation_id.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "观察 ID 不能为空。",
                false,
            ));
        }
        let mut observation = self.screen_observation.write().await;
        if observation.as_ref().is_some_and(|record| {
            screen_observation_expired(record.metadata.expires_at_ms, now_ms())
        }) {
            *observation = None;
            return Err(AppError::new(
                "screen_observation_expired",
                "本地屏幕预览已自动过期，请重新读取。",
                false,
            ));
        }
        let Some(record) = observation.as_ref() else {
            return Err(AppError::new(
                "not_found",
                "当前没有可预览的屏幕观察。",
                false,
            ));
        };
        if record.metadata.id != observation_id {
            return Err(AppError::new(
                "conflict",
                "屏幕观察已更新，请重新打开当前预览。",
                true,
            ));
        }
        Ok(ScreenObservationPreview {
            id: record.metadata.id.clone(),
            captured_at_ms: record.metadata.captured_at_ms,
            data_url: format!(
                "data:image/png;base64,{}",
                BASE64_STANDARD.encode(&record.png)
            ),
            ocr_text: record.ocr_text.clone(),
        })
    }

    pub async fn clear_screen_observation(&self, app: &AppHandle) -> Result<(), AppError> {
        *self.screen_observation.write().await = None;
        self.runtime.write().await.revision += 1;
        self.emit_snapshot(app).await
    }

    pub async fn cancel_timer(&self, app: &AppHandle, timer_id: &str) -> Result<(), AppError> {
        if timer_id.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "提醒 ID 不能为空。",
                false,
            ));
        }
        let cancelled = self
            .storage
            .cancel_timer(timer_id, now_ms())
            .map_err(storage_error)?;
        if !cancelled {
            return Err(AppError::new(
                "not_found",
                "提醒不存在、已经触发或已经取消。",
                false,
            ));
        }
        notifications::cancel_timer(app, timer_id).await?;
        self.timer_notify.notify_one();
        self.emit_snapshot(app).await
    }

    async fn fire_due_timer(&self, app: &AppHandle, timer_id: &str) {
        match self.storage.claim_due_timer(timer_id, now_ms()) {
            Ok(Some(timer)) => {
                let should_notify = notifications::claim_notification(app, &timer.id)
                    .await
                    .unwrap_or(true);
                let _ = notifications::cleanup_fired_job(app, &timer.id).await;
                if should_notify {
                    let notification = app
                        .notification()
                        .builder()
                        .title("爱丽丝的提醒")
                        .body(&timer.title)
                        .show();
                    if let Err(error) = notification {
                        let mut runtime = self.runtime.write().await;
                        runtime.revision += 1;
                        runtime.last_error = Some(AppError::new(
                            "notification_failed",
                            format!("提醒已到期，但 macOS 通知发送失败：{error}"),
                            true,
                        ));
                    }
                }
                let _ = self.emit_snapshot(app).await;
            }
            Ok(None) => {}
            Err(error) => {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.last_error = Some(storage_error(error));
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
        }
    }

    pub async fn update_provider(
        &self,
        value: ProviderSettings,
        expected_revision: u64,
        api_key: Option<String>,
        clear_api_key: bool,
        vision_api_key: Option<String>,
        clear_vision_api_key: bool,
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
        let previous_vision_secret = self.get_stored_vision_api_key().await?;
        if clear_api_key {
            self.delete_llm_api_key().await?;
        } else if let Some(secret) = api_key
            .as_deref()
            .filter(|secret| !secret.trim().is_empty())
        {
            self.set_llm_api_key(secret.to_owned()).await?;
        }
        if clear_vision_api_key {
            if let Err(error) = self.delete_vision_api_key().await {
                self.restore_secrets(previous_secret.clone(), previous_vision_secret.clone())
                    .await;
                return Err(error);
            }
        } else if let Some(secret) = vision_api_key
            .as_deref()
            .filter(|secret| !secret.trim().is_empty())
        {
            if let Err(error) = self.set_vision_api_key(secret.to_owned()).await {
                self.restore_secrets(previous_secret.clone(), previous_vision_secret.clone())
                    .await;
                return Err(error);
            }
        }
        let has_api_key = self.resolve_llm_api_key().await?.is_some();
        let has_vision_api_key = self.resolve_vision_api_key().await?.is_some();
        let mut updated = value;
        updated.revision = provider.revision + 1;
        updated.has_api_key = has_api_key;
        updated.vision.has_api_key = has_vision_api_key;
        if let Err(error) = self.storage.save_provider_settings(&updated, now_ms()) {
            self.restore_secrets(previous_secret, previous_vision_secret)
                .await;
            return Err(storage_error(error));
        }
        *provider = updated.clone();
        Ok(updated)
    }

    pub async fn test_provider(&self) -> Result<String, AppError> {
        let settings = self.provider.read().await.clone();
        let api_key = self.resolve_llm_api_key().await?;
        let provider = create_provider(&settings, api_key)?;
        let mut stream = match tokio::time::timeout(
            Duration::from_secs(15),
            provider.start(
                LlmRequest {
                    messages: vec![LlmMessage::User {
                        content: "请只回复：连接正常".into(),
                    }],
                    memory_context: Vec::new(),
                    tools: Vec::new(),
                    tool_steps: 0,
                },
                CancellationToken::new(),
            ),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(AppError::new(
                    "provider_test_timeout",
                    "Provider 在 15 秒内没有建立连接。",
                    true,
                ));
            }
        };
        let response = tokio::time::timeout(Duration::from_secs(30), async {
            let mut response = String::new();
            while let Some(event) = stream.next().await {
                match event? {
                    LlmEvent::TextDelta(delta) => response.push_str(&delta),
                    LlmEvent::Completed => break,
                    LlmEvent::ToolCalls(_) => {
                        return Err(AppError::new(
                            "provider_test_failed",
                            "连接测试收到工具调用，而不是文本响应。",
                            false,
                        ));
                    }
                }
            }
            Ok::<String, AppError>(response)
        })
        .await
        .map_err(|_| {
            AppError::new(
                "provider_test_timeout",
                "Provider 连接测试超过 30 秒未完成。",
                true,
            )
        })??;
        if response.trim().is_empty() {
            return Err(AppError::new(
                "provider_test_failed",
                "Provider 没有返回可显示的文本。",
                true,
            ));
        }
        Ok(response.chars().take(1_000).collect())
    }

    pub async fn search_memories(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, AppError> {
        let character_id = self.character.read().await.character_id.clone();
        self.storage
            .search_memories_for_management(query, &character_id, limit)
            .map_err(storage_error)
    }

    pub fn list_memory_candidates(&self, limit: usize) -> Result<Vec<MemoryCandidate>, AppError> {
        self.storage
            .list_pending_memory_candidates(limit)
            .map_err(storage_error)
    }

    pub fn memory_source(&self, id: &str) -> Result<Option<MemorySource>, AppError> {
        self.storage.memory_source(id).map_err(storage_error)
    }

    pub async fn export_memories(&self) -> Result<String, AppError> {
        let character_id = self.character.read().await.character_id.clone();
        let memories = self
            .storage
            .list_memories_for_export(&character_id)
            .map_err(storage_error)?;
        serde_json::to_string_pretty(&json!({
            "schema_version": "memory-v2",
            "exported_at_ms": now_ms(),
            "character_id": character_id,
            "memories": memories,
        }))
        .map_err(|error| {
            AppError::new(
                "memory_export",
                format!("记忆导出序列化失败：{error}"),
                false,
            )
        })
    }

    pub fn memory_settings(&self) -> Result<MemorySettings, AppError> {
        self.storage.load_memory_settings().map_err(storage_error)
    }

    pub fn update_memory_settings(
        &self,
        automatic_candidates_enabled: bool,
        expected_revision: u64,
    ) -> Result<MemorySettings, AppError> {
        let mut settings = self.storage.load_memory_settings().map_err(storage_error)?;
        if settings.revision != expected_revision {
            return Err(AppError::new(
                "conflict",
                "记忆设置已在其他窗口更新，请刷新后重试。",
                true,
            ));
        }
        settings.automatic_candidates_enabled = automatic_candidates_enabled;
        settings.revision += 1;
        self.storage
            .save_memory_settings(&settings, now_ms())
            .map_err(storage_error)?;
        self.automatic_memory_candidates_enabled
            .store(settings.automatic_candidates_enabled, Ordering::Relaxed);
        Ok(settings)
    }

    pub async fn review_memory_candidate(
        &self,
        candidate_id: &str,
        approved: bool,
        resolution: Option<&str>,
    ) -> Result<Option<MemoryItem>, AppError> {
        if candidate_id.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "记忆候选 ID 不能为空。",
                false,
            ));
        }
        let _interaction_guard = self.interaction_command.lock().await;
        if self
            .runtime
            .read()
            .await
            .pending_tool
            .as_ref()
            .is_some_and(|tool| tool.approval.tool_call_id == candidate_id)
        {
            return Err(AppError::new(
                "conflict",
                "请先处理上方的工具确认卡片，再单独审核这条候选。",
                true,
            ));
        }
        if approved {
            let character_id = self.character.read().await.character_id.clone();
            let resolution = match resolution {
                None => None,
                Some("replace") => Some(MemoryConflictResolution::Replace),
                Some("keep_both") => Some(MemoryConflictResolution::KeepBoth),
                Some(_) => {
                    return Err(AppError::new(
                        "invalid_request",
                        "记忆冲突处理方式无效。",
                        false,
                    ));
                }
            };
            let (valid_from_ms, valid_until_ms) = self
                .storage
                .list_pending_memory_candidates(50)
                .map_err(storage_error)?
                .into_iter()
                .find(|candidate| candidate.id == candidate_id)
                .map(|candidate| {
                    derive_memory_validity(&candidate.kind, &candidate.content, now_ms())
                })
                .unwrap_or((None, None));
            match self
                .storage
                .confirm_memory_candidate_with_validity(
                    candidate_id,
                    &character_id,
                    resolution,
                    valid_from_ms,
                    valid_until_ms,
                    now_ms(),
                )
                .map_err(storage_error)?
            {
                MemoryConfirmResult::Saved(memory) | MemoryConfirmResult::Duplicate(memory) => {
                    Ok(Some(memory))
                }
                MemoryConfirmResult::Conflict(existing) => Err(AppError::new(
                    "memory_conflict",
                    format!(
                        "这条候选与现有记忆冲突：{}。请选择替换旧记忆或保留两条。",
                        existing.content
                    ),
                    true,
                )),
            }
        } else if self
            .storage
            .reject_memory_candidate(candidate_id, now_ms())
            .map_err(storage_error)?
        {
            Ok(None)
        } else {
            Err(AppError::new(
                "conflict",
                "这条记忆候选已被处理，请刷新后重试。",
                true,
            ))
        }
    }

    pub async fn update_memory_candidate(
        &self,
        candidate_id: &str,
        content: &str,
    ) -> Result<MemoryCandidate, AppError> {
        if candidate_id.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "记忆候选 ID 不能为空。",
                false,
            ));
        }
        let content = validate_memory_content(content)?;
        let sensitivity = classify_memory_sensitivity(&content);
        if sensitivity == "restricted" {
            return Err(AppError::new(
                "memory_restricted",
                "出于安全原因，密码、密钥、Token 等凭据不能写入长期记忆。",
                false,
            ));
        }
        let fact_key = self
            .storage
            .list_pending_memory_candidates(50)
            .map_err(storage_error)?
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .and_then(|candidate| {
                derive_memory_fact_key(&candidate.scope, &candidate.kind, &content)
            });
        let _interaction_guard = self.interaction_command.lock().await;
        if self
            .runtime
            .read()
            .await
            .pending_tool
            .as_ref()
            .is_some_and(|tool| tool.approval.tool_call_id == candidate_id)
        {
            return Err(AppError::new(
                "conflict",
                "请先处理上方的工具确认卡片，再编辑这条候选。",
                true,
            ));
        }
        self.storage
            .update_memory_candidate_with_metadata(
                candidate_id,
                &content,
                fact_key.as_deref(),
                Some(sensitivity),
            )
            .map_err(storage_error)?
            .ok_or_else(|| AppError::new("conflict", "这条记忆候选已被处理，请刷新后重试。", true))
    }

    pub fn create_memory(&self, content: &str) -> Result<MemoryItem, AppError> {
        let content = validate_memory_content(content)?;
        let sensitivity = classify_memory_sensitivity(&content);
        if sensitivity == "restricted" {
            return Err(AppError::new(
                "memory_restricted",
                "出于安全原因，密码、密钥、Token 等凭据不能写入长期记忆。",
                false,
            ));
        }
        let kind = infer_memory_kind(&content, "note");
        let fact_key = derive_memory_fact_key("user", kind, &content);
        if let Some(existing) = self
            .storage
            .find_active_memory_conflict("user", None, fact_key.as_deref(), &content, now_ms())
            .map_err(storage_error)?
        {
            return Err(AppError::new(
                "memory_conflict",
                format!(
                    "这条内容与已有记忆冲突：{}。请先编辑旧记忆，或通过对话提交替换候选。",
                    existing.content
                ),
                true,
            ));
        }
        let created_at_ms = now_ms();
        let (valid_from_ms, valid_until_ms) = derive_memory_validity(kind, &content, created_at_ms);
        self.storage
            .create_user_memory_with_validity(
                &content,
                kind,
                sensitivity,
                fact_key.as_deref(),
                valid_from_ms,
                valid_until_ms,
                created_at_ms,
            )
            .map_err(storage_error)
    }

    pub fn update_memory(
        &self,
        id: &str,
        content: &str,
        expected_version: u64,
    ) -> Result<MemoryItem, AppError> {
        let content = validate_memory_content(content)?;
        if classify_memory_sensitivity(&content) == "restricted" {
            return Err(AppError::new(
                "memory_restricted",
                "出于安全原因，密码、密钥、Token 等凭据不能写入长期记忆。",
                false,
            ));
        }
        let current = self
            .storage
            .memory_by_id(id)
            .map_err(storage_error)?
            .ok_or_else(|| AppError::new("conflict", "记忆已被修改或删除，请刷新后重试。", true))?;
        let sensitivity = classify_memory_sensitivity(&content);
        let fact_key = derive_memory_fact_key(&current.scope, &current.kind, &content);
        if current.scope == "user"
            && self
                .storage
                .has_duplicate_user_memory(&content, id)
                .map_err(storage_error)?
        {
            return Err(AppError::new(
                "duplicate_memory",
                "已有一条完全相同的长期记忆，请编辑现有记忆或修改这条内容。",
                true,
            ));
        }
        let updated_at_ms = now_ms();
        let (valid_from_ms, valid_until_ms) =
            derive_memory_validity(&current.kind, &content, updated_at_ms);
        self.storage
            .update_memory_with_validity(
                id,
                &content,
                expected_version,
                fact_key.as_deref(),
                Some(sensitivity),
                valid_from_ms,
                valid_until_ms,
                updated_at_ms,
            )
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

    pub fn data_summary(&self) -> Result<DataSummary, AppError> {
        self.storage
            .data_summary(&self.conversation_id)
            .map_err(storage_error)
    }

    pub async fn clear_conversation_history(
        self: &Arc<Self>,
        app: &AppHandle,
    ) -> Result<DataSummary, AppError> {
        let _interaction_guard = self.interaction_command.lock().await;
        self.interrupt_playback().await;
        self.cancel_active(app, "data_cleared").await?;

        let has_voice_capture = self.runtime.read().await.active_voice_source.is_some();
        if has_voice_capture {
            if let Some(writer) = self.audio_writer.lock().await.clone() {
                let _ = audio::send(&writer, json!({"type": "asr.stop"})).await;
            }
        }
        self.storage
            .clear_conversation_history(&self.conversation_id, now_ms())
            .map_err(storage_error)?;
        self.timer_notify.notify_one();
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.turn_state = TurnState::Idle;
            runtime.generation_state = GenerationState::NotStarted;
            runtime.active_turn_id = None;
            runtime.input_state = if runtime.listening_mode == ListeningMode::Armed {
                InputState::WakeListening
            } else {
                InputState::Stopped
            };
            runtime.active_voice_capture_id = None;
            runtime.active_voice_source = None;
            runtime.asr_partial.clear();
            runtime.voice_input_route = "unknown".into();
            runtime.last_error = None;
        }
        self.emit_snapshot(app).await?;
        self.data_summary()
    }

    pub fn clear_all_memories(&self) -> Result<DataSummary, AppError> {
        self.storage.clear_all_memories().map_err(storage_error)?;
        self.data_summary()
    }

    pub async fn request_voice_permissions(
        self: &Arc<Self>,
        app: &AppHandle,
    ) -> Result<(), AppError> {
        let writer = self.ensure_audio_host(app).await?;
        audio::send(&writer, json!({"type": "permissions.request"})).await
    }

    pub async fn open_voice_privacy_settings(
        self: &Arc<Self>,
        app: &AppHandle,
    ) -> Result<(), AppError> {
        let writer = self.ensure_audio_host(app).await?;
        audio::send(&writer, json!({"type": "settings.open_privacy"})).await
    }

    pub async fn toggle_listening(self: &Arc<Self>, app: &AppHandle) -> Result<(), AppError> {
        let enabled = self.runtime.read().await.listening_mode != ListeningMode::Armed;
        self.set_listening(app, enabled).await
    }

    pub async fn set_listening(
        self: &Arc<Self>,
        app: &AppHandle,
        enabled: bool,
    ) -> Result<(), AppError> {
        let _voice_guard = self.voice_command.lock().await;
        if !enabled {
            self.listening_requested.store(false, Ordering::SeqCst);
            let send_result = if let Some(writer) = self.audio_writer.lock().await.clone() {
                audio::send(&writer, json!({"type": "listening.set", "enabled": false})).await
            } else {
                Ok(())
            };
            {
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.listening_mode = ListeningMode::Disabled;
                runtime.input_state = InputState::Stopped;
                runtime.active_voice_capture_id = None;
                runtime.active_voice_source = None;
                runtime.asr_partial.clear();
                runtime.voice_input_route = "unknown".into();
            }
            self.emit_snapshot(app).await?;
            return send_result;
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
        self.listening_requested.store(true, Ordering::SeqCst);
        if let Err(error) =
            audio::send(&writer, json!({"type": "listening.set", "enabled": true})).await
        {
            self.listening_requested.store(false, Ordering::SeqCst);
            return Err(error);
        }
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
        {
            let runtime = self.runtime.read().await;
            if runtime.active_voice_source.as_deref() == Some("push_to_talk")
                && runtime.active_voice_capture_id.is_some()
            {
                return Ok(());
            }
        }
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
            runtime.mouth_level = 0.0;
            runtime.active_voice_capture_id = Some(capture_id);
            runtime.active_voice_source = Some("push_to_talk".into());
            runtime.asr_partial.clear();
            runtime.voice_input_route = "unknown".into();
            runtime.last_error = None;
        }
        self.emit_snapshot(app).await
    }

    pub async fn finish_push_to_talk(&self, app: &AppHandle) -> Result<(), AppError> {
        let _voice_guard = self.voice_command.lock().await;
        {
            let runtime = self.runtime.read().await;
            if runtime.active_voice_source.as_deref() != Some("push_to_talk")
                || runtime.active_voice_capture_id.is_none()
            {
                return Ok(());
            }
        }
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
            runtime.mouth_level = 0.0;
        }
        self.emit_snapshot(app).await
    }

    pub async fn shutdown_audio_host(&self) {
        self.listening_requested.store(false, Ordering::SeqCst);
        let writer = self.audio_writer.lock().await.take();
        *self.audio_connection_id.lock().await = None;
        *self.audio_last_pong.lock().await = None;
        if let Some(writer) = writer {
            let _ = audio::send(&writer, json!({"type": "shutdown"})).await;
        }
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
        let connection_id = connection.connection_id.clone();
        *self.audio_writer.lock().await = Some(connection.writer.clone());
        *self.audio_connection_id.lock().await = Some(connection_id.clone());
        *self.audio_last_pong.lock().await = Some((connection_id.clone(), Instant::now()));
        {
            let mut runtime = self.runtime.write().await;
            runtime.revision += 1;
            runtime.audio_host_state = "handshaking".into();
        }
        self.emit_snapshot(app).await?;
        let heartbeat_core = Arc::clone(self);
        let heartbeat_app = app.clone();
        let heartbeat_writer = connection.writer.clone();
        let heartbeat_connection_id = connection_id.clone();
        tauri::async_runtime::spawn(async move {
            heartbeat_core
                .run_audio_heartbeat(heartbeat_app, heartbeat_writer, heartbeat_connection_id)
                .await;
        });
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

    async fn run_audio_heartbeat(
        self: Arc<Self>,
        app: AppHandle,
        writer: AudioWriter,
        connection_id: String,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = 0_u64;
        loop {
            interval.tick().await;
            let current = self.audio_connection_id.lock().await.clone();
            if current.as_deref() != Some(connection_id.as_str()) {
                return;
            }
            let stale = self
                .audio_last_pong
                .lock()
                .await
                .as_ref()
                .filter(|(id, _)| id == &connection_id)
                .is_none_or(|(_, last_pong)| last_pong.elapsed() > Duration::from_secs(6));
            if stale {
                self.handle_audio_event(
                    &app,
                    json!({
                        "type": "host.disconnected",
                        "_connection_id": connection_id,
                        "reason": "heartbeat_timeout"
                    }),
                )
                .await;
                return;
            }
            sequence = sequence.wrapping_add(1);
            if audio::send(&writer, json!({"type": "core.ping", "seq": sequence}))
                .await
                .is_err()
            {
                self.handle_audio_event(
                    &app,
                    json!({
                        "type": "host.disconnected",
                        "_connection_id": connection_id,
                        "reason": "heartbeat_write_failed"
                    }),
                )
                .await;
                return;
            }
        }
    }

    async fn handle_audio_event(self: &Arc<Self>, app: &AppHandle, event: Value) {
        let event_connection_id = event.get("_connection_id").and_then(Value::as_str);
        if let Some(event_connection_id) = event_connection_id {
            let current = self.audio_connection_id.lock().await;
            if current.as_deref() != Some(event_connection_id) {
                return;
            }
        }
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "host.pong" => {
                if let Some(connection_id) = event_connection_id {
                    *self.audio_last_pong.lock().await =
                        Some((connection_id.to_owned(), Instant::now()));
                }
            }
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
                let permissions_authorized = runtime.voice_permissions.microphone == "authorized"
                    && runtime.voice_permissions.speech_recognition == "authorized";
                if !permissions_authorized {
                    self.listening_requested.store(false, Ordering::SeqCst);
                    runtime.listening_mode = ListeningMode::Disabled;
                    runtime.input_state = InputState::Stopped;
                    runtime.active_voice_capture_id = None;
                    runtime.active_voice_source = None;
                    runtime.asr_partial.clear();
                    runtime.voice_input_route = "unknown".into();
                }
                drop(runtime);
                if !permissions_authorized {
                    if let Some(writer) = self.audio_writer.lock().await.clone() {
                        let _ = audio::send(
                            &writer,
                            json!({"type": "listening.set", "enabled": false}),
                        )
                        .await;
                    }
                }
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
                runtime.voice_input_route = if event
                    .get("on_device")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "on_device".into()
                } else {
                    "system_service".into()
                };
                runtime.input_state = if mode == "wake" {
                    runtime.active_voice_source = None;
                    runtime.asr_partial.clear();
                    InputState::WakeListening
                } else {
                    InputState::Capturing
                };
                runtime.last_error = None;
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
                    runtime.voice_input_route = "unknown".into();
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
            "tts.level" => {
                let turn_id = event
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let generation = event.get("generation").and_then(Value::as_u64);
                let level = event.get("level").and_then(Value::as_f64).unwrap_or(0.0);
                if !level.is_finite() {
                    return;
                }
                let active = self.active_playback.lock().await;
                if active
                    .as_ref()
                    .is_none_or(|(active_turn, active_generation)| {
                        Some(active_turn.as_str()) != turn_id.as_deref()
                            || Some(*active_generation) != generation
                    })
                {
                    return;
                }
                drop(active);
                self.runtime.write().await.mouth_level = level.clamp(0.0, 1.0);
                let _ = self
                    .emit(
                        app,
                        "avatar:mouth-level",
                        turn_id,
                        generation,
                        MouthLevelPayload {
                            level: level.clamp(0.0, 1.0),
                        },
                    )
                    .await;
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
                runtime.mouth_level = 0.0;
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
                runtime.voice_input_route = "unknown".into();
                runtime.mouth_level = 0.0;
                if matches!(
                    code,
                    "on_device_wake_unavailable"
                        | "wake_recognition_unavailable"
                        | "system_speech_disabled"
                        | "permission_denied"
                ) {
                    self.listening_requested.store(false, Ordering::SeqCst);
                    runtime.listening_mode = ListeningMode::Disabled;
                }
                runtime.last_error = Some(AppError::new(code, message, true));
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
            }
            "host.disconnected" => {
                *self.audio_connection_id.lock().await = None;
                *self.audio_last_pong.lock().await = None;
                *self.audio_writer.lock().await = None;
                *self.active_playback.lock().await = None;
                let should_recover = self.listening_requested.load(Ordering::SeqCst);
                let mut runtime = self.runtime.write().await;
                runtime.revision += 1;
                runtime.audio_host_state = "failed".into();
                runtime.input_state = if should_recover {
                    InputState::Recovering
                } else {
                    InputState::Stopped
                };
                runtime.playback_state = PlaybackState::Idle;
                runtime.mouth_level = 0.0;
                runtime.listening_mode = if should_recover {
                    ListeningMode::Armed
                } else {
                    ListeningMode::Disabled
                };
                runtime.active_voice_capture_id = None;
                runtime.active_voice_source = None;
                runtime.asr_partial.clear();
                runtime.voice_input_route = "unknown".into();
                runtime.last_error = Some(AppError::new(
                    "audio_host_disconnected",
                    if should_recover {
                        "AudioHost 已断开，正在尝试恢复持续监听。"
                    } else {
                        "AudioHost 已断开；文字对话仍可使用。"
                    },
                    true,
                ));
                drop(runtime);
                let _ = self.emit_snapshot(app).await;
                if should_recover {
                    let _ = self.audio_recovery_tx.send(app.clone());
                }
            }
            _ => {}
        }
    }

    async fn recover_requested_listening(self: Arc<Self>, app: AppHandle) {
        if self.audio_recovery_running.swap(true, Ordering::SeqCst) {
            return;
        }
        self.recover_requested_listening_inner(&app).await;
        self.audio_recovery_running.store(false, Ordering::SeqCst);
    }

    async fn recover_requested_listening_inner(self: &Arc<Self>, app: &AppHandle) {
        if !self.listening_requested.load(Ordering::SeqCst) {
            return;
        }
        let connection_present = self.audio_writer.lock().await.is_some();
        let already_recovered = {
            let runtime = self.runtime.read().await;
            connection_present
                && runtime.audio_host_state != "failed"
                && runtime.input_state == InputState::WakeListening
                && runtime.listening_mode == ListeningMode::Armed
        };
        if already_recovered {
            return;
        }

        let retry_delays = [
            Duration::from_millis(250),
            Duration::from_secs(1),
            Duration::from_secs(3),
        ];
        let mut last_error = None;

        for delay in retry_delays {
            if !self.listening_requested.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(delay).await;
            if !self.listening_requested.load(Ordering::SeqCst) {
                return;
            }

            match self.ensure_audio_host(app).await {
                Ok(writer) => {
                    match audio::send(&writer, json!({"type": "listening.set", "enabled": true}))
                        .await
                    {
                        Ok(()) => {
                            if !self.listening_requested.load(Ordering::SeqCst) {
                                let _ = audio::send(
                                    &writer,
                                    json!({"type": "listening.set", "enabled": false}),
                                )
                                .await;
                                return;
                            }
                            let mut runtime = self.runtime.write().await;
                            runtime.revision += 1;
                            runtime.listening_mode = ListeningMode::Armed;
                            runtime.input_state = InputState::WakeListening;
                            runtime.last_error = None;
                            drop(runtime);
                            let _ = self.emit_snapshot(app).await;
                            return;
                        }
                        Err(error) => {
                            last_error = Some(error);
                            for _ in 0..20 {
                                if self.audio_writer.lock().await.is_none() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                            if self.audio_writer.lock().await.is_some() {
                                break;
                            }
                        }
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }

        self.listening_requested.store(false, Ordering::SeqCst);
        let mut runtime = self.runtime.write().await;
        runtime.revision += 1;
        runtime.listening_mode = ListeningMode::Disabled;
        runtime.input_state = InputState::Stopped;
        let detail = last_error
            .map(|error| error.message)
            .unwrap_or_else(|| "未获得可用连接。".into());
        runtime.last_error = Some(AppError::new(
            "audio_host_recovery_failed",
            format!("AudioHost 恢复失败，已关闭持续监听；文字对话仍可使用。详情：{detail}"),
            true,
        ));
        drop(runtime);
        let _ = self.emit_snapshot(app).await;
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
            runtime.mouth_level = 0.0;
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
            runtime.mouth_level = 0.0;
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
        runtime.mouth_level = 0.0;
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

    async fn resolve_vision_api_key(&self) -> Result<Option<String>, AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.resolve_vision_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn get_stored_vision_api_key(&self) -> Result<Option<String>, AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.get_stored_vision_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn set_vision_api_key(&self, value: String) -> Result<(), AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.set_vision_api_key(&value))
            .await
            .map_err(secret_task_error)?
    }

    async fn delete_vision_api_key(&self) -> Result<(), AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.delete_vision_api_key())
            .await
            .map_err(secret_task_error)?
    }

    async fn restore_secrets(&self, llm: Option<String>, vision: Option<String>) {
        match llm {
            Some(secret) => {
                let _ = self.set_llm_api_key(secret).await;
            }
            None => {
                let _ = self.delete_llm_api_key().await;
            }
        }
        match vision {
            Some(secret) => {
                let _ = self.set_vision_api_key(secret).await;
            }
            None => {
                let _ = self.delete_vision_api_key().await;
            }
        }
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

fn screen_observation_expired(expires_at_ms: i64, now_ms: i64) -> bool {
    now_ms >= expires_at_ms
}

fn vision_endpoint_label(endpoint: &str) -> String {
    reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "已配置地址".into())
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
    if value.vision.enabled {
        let endpoint = reqwest::Url::parse(&value.vision.endpoint).map_err(|_| {
            AppError::new(
                "invalid_request",
                "Vision Provider endpoint 不是有效 URL。",
                false,
            )
        })?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(AppError::new(
                "invalid_request",
                "Vision Provider endpoint 只允许 HTTP 或 HTTPS。",
                false,
            ));
        }
        if value.vision.model.trim().is_empty() {
            return Err(AppError::new(
                "invalid_request",
                "Vision Provider 模型名称不能为空。",
                false,
            ));
        }
    }
    Ok(())
}

fn validate_pet_settings(value: &PetSettings) -> Result<(), AppError> {
    if !value.scale.is_finite() || !(0.5..=1.5).contains(&value.scale) {
        return Err(AppError::new(
            "invalid_request",
            "桌宠缩放比例必须在 50% 到 150% 之间。",
            false,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExplicitMemoryIntent {
    content: String,
    scope: &'static str,
    kind: &'static str,
}

struct AutomaticMemoryCandidate {
    content: String,
    scope: &'static str,
    kind: &'static str,
    confidence: f64,
}

fn extract_automatic_memory_candidate(text: &str) -> Option<AutomaticMemoryCandidate> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 4
        || trimmed.chars().count() > 1_000
        || trimmed.ends_with('?')
        || trimmed.ends_with('？')
        || trimmed.ends_with('吗')
        || [
            "密码",
            "口令",
            "密钥",
            "api key",
            "apikey",
            "token",
            "sk-",
            "身份证",
            "银行卡",
            "验证码",
            "手机号",
            "电话号码",
            "住址",
            "家庭住址",
        ]
        .iter()
        .any(|marker| trimmed.to_ascii_lowercase().contains(marker))
    {
        return None;
    }
    let without_address = trimmed
        .strip_prefix("老师")
        .map(|value| value.trim_start_matches(['，', ',', '：', ':', ' ']))
        .unwrap_or(trimmed);
    let markers = [
        ("我喜欢", "preference", "user", 0.66),
        ("我不喜欢", "preference", "user", 0.66),
        ("我偏好", "preference", "user", 0.66),
        ("我习惯", "preference", "user", 0.64),
        ("我的爱好是", "preference", "user", 0.68),
        ("我的目标是", "goal", "user", 0.64),
        ("我希望以后", "preference", "user", 0.64),
        ("我目前在做", "project", "user", 0.60),
        ("我正在做", "project", "user", 0.60),
    ];
    let (marker, kind, scope, confidence) = markers
        .iter()
        .find(|(marker, ..)| without_address.starts_with(marker))
        .copied()?;
    let content = validate_memory_content(trimmed).ok()?;
    let remainder = without_address
        .strip_prefix(marker)
        .unwrap_or_default()
        .trim_matches(|character: char| {
            matches!(character, '：' | ':' | '，' | ',' | '。' | '！' | '!')
        });
    if remainder.is_empty() || remainder.chars().count() > 700 {
        return None;
    }
    Some(AutomaticMemoryCandidate {
        content,
        scope,
        kind,
        confidence,
    })
}

fn is_no_memory_request(text: &str) -> bool {
    [
        "不要记住",
        "别记住",
        "这次不要记",
        "这次别记",
        "不需要记住",
        "无需记住",
        "不要保存",
        "别保存",
        "不要记录",
        "别记录",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn classify_memory_sensitivity(content: &str) -> &'static str {
    let lowered = content.to_ascii_lowercase();
    if [
        "密码",
        "口令",
        "密钥",
        "api key",
        "apikey",
        "token",
        "sk-",
        "secret",
        "验证码",
        "私钥",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "restricted";
    }
    if [
        "身份证",
        "银行卡",
        "信用卡",
        "手机号",
        "电话号码",
        "住址",
        "家庭住址",
        "地址",
        "邮箱",
        "电子邮件",
        "病史",
        "疾病",
        "用药",
        "诊断",
        "工资",
        "收入",
        "债务",
        "财务",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "sensitive";
    }
    "normal"
}

fn derive_memory_fact_key(scope: &str, kind: &str, content: &str) -> Option<String> {
    if scope != "user" || kind != "preference" {
        return None;
    }
    let prefixes = [
        "我喜欢",
        "我不喜欢",
        "我偏好",
        "我习惯",
        "我的爱好是",
        "我希望以后",
    ];
    let remainder = prefixes
        .iter()
        .find_map(|prefix| content.trim().strip_prefix(prefix))?
        .trim_matches(|character: char| {
            matches!(character, '：' | ':' | '，' | ',' | '。' | '！' | '!')
        })
        .trim();
    if remainder.is_empty() {
        return None;
    }
    let normalized = remainder
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || !character.is_ascii_punctuation() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then(|| format!("{scope}:{kind}:{normalized}"))
}

fn derive_memory_validity(kind: &str, content: &str, now_ms: i64) -> (Option<i64>, Option<i64>) {
    let valid_from_ms = Some(now_ms);
    let default_valid_until_ms = || match kind {
        "goal" => Some(now_ms.saturating_add(90 * 24 * 60 * 60 * 1_000)),
        "project" => Some(now_ms.saturating_add(180 * 24 * 60 * 60 * 1_000)),
        _ => None,
    };
    let Some(now) = Local.timestamp_millis_opt(now_ms).single() else {
        return (valid_from_ms, default_valid_until_ms());
    };
    let Ok((Some(date), _)) = parse_absolute_date(content, now.date_naive()) else {
        return (valid_from_ms, default_valid_until_ms());
    };
    let Some(next_date) = date.checked_add_signed(ChronoDuration::days(1)) else {
        return (valid_from_ms, None);
    };
    let Some(naive) = next_date.and_hms_opt(0, 0, 0) else {
        return (valid_from_ms, None);
    };
    let valid_until_ms = match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Some(value.timestamp_millis()),
        LocalResult::Ambiguous(first, second) => Some(first.min(second).timestamp_millis()),
        LocalResult::None => None,
    };
    (
        valid_from_ms,
        valid_until_ms.or_else(default_valid_until_ms),
    )
}

fn derive_relationship_signal(text: &str) -> (f64, &'static str) {
    if ["讨厌", "失望", "生气", "别烦", "糟糕", "不想理"]
        .iter()
        .any(|marker| text.contains(marker))
    {
        return (
            -0.04,
            "近期互动出现了负面情绪，关系状态保持可修复。熟悉度不会因此下降。",
        );
    }
    if ["谢谢", "太好了", "开心", "喜欢", "厉害", "帮到我"]
        .iter()
        .any(|marker| text.contains(marker))
    {
        if ["我们一起", "一起完成", "共同", "并肩", "一起做"]
            .iter()
            .any(|marker| text.contains(marker))
        {
            return (0.06, "最近与老师有共同完成事项的积极互动。");
        }
        return (0.03, "最近互动氛围偏积极。");
    }
    (0.0, "")
}

fn parse_explicit_memory_intent(text: &str) -> Option<ExplicitMemoryIntent> {
    if is_no_memory_request(text) {
        return None;
    }
    let (markers, scope, kind) = if [
        "记住我们",
        "记下我们",
        "记录我们",
        "记住这次",
        "记下这次",
        "记录这次",
        "记住这段经历",
        "记下这段经历",
        "记录这段经历",
    ]
    .iter()
    .any(|marker| text.contains(marker))
    {
        (
            [
                "记住我们",
                "记下我们",
                "记录我们",
                "记住这次",
                "记下这次",
                "记录这次",
                "记住这段经历",
                "记下这段经历",
                "记录这段经历",
            ]
            .as_slice(),
            "relationship",
            "event",
        )
    } else {
        (
            ["记住", "记下", "别忘了", "不要忘记", "保存这条", "存进记忆"].as_slice(),
            "user",
            "note",
        )
    };
    markers
        .iter()
        .find_map(|marker| {
            text.find(marker)
                .map(|start| (start + marker.len(), *marker))
        })
        .and_then(|(start, marker)| {
            let mut content = text[start..].trim_matches(|character: char| {
                matches!(character, '：' | ':' | '，' | ',' | '。' | '！' | '!')
            });
            if scope == "relationship" && marker.ends_with('们') {
                content = content.strip_prefix('的').unwrap_or(content).trim();
            }
            (!content.is_empty()).then(|| ExplicitMemoryIntent {
                content: content.to_owned(),
                scope,
                kind: infer_memory_kind(content, kind),
            })
        })
}

fn infer_memory_kind(content: &str, fallback: &'static str) -> &'static str {
    if [
        "我喜欢",
        "我不喜欢",
        "我偏好",
        "我习惯",
        "我的爱好是",
        "我希望以后",
    ]
    .iter()
    .any(|marker| content.starts_with(marker))
    {
        "preference"
    } else if content.starts_with("我的目标是") {
        "goal"
    } else if content.starts_with("我目前在做") || content.starts_with("我正在做") {
        "project"
    } else {
        fallback
    }
}

fn memory_scope_label(scope: &str) -> &'static str {
    if scope == "relationship" {
        "关系记忆"
    } else {
        "用户记忆"
    }
}

fn parse_explicit_memory_delete_query(text: &str) -> Option<String> {
    if [
        "不要忘记",
        "别忘记",
        "别忘了",
        "不要删除",
        "别删除",
        "不要清除",
        "别清除",
        "取消删除",
        "不用删除",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
    {
        return None;
    }

    let trimmed = text.trim();
    let direct_forget = ["忘记", "忘掉"]
        .iter()
        .any(|marker| trimmed.starts_with(marker))
        && !["忘记了", "忘记过", "忘掉了", "忘掉过"]
            .iter()
            .any(|marker| trimmed.starts_with(marker));
    let has_forget_request = [
        "请帮我忘记",
        "请帮我忘掉",
        "帮我忘记",
        "帮我忘掉",
        "请忘记",
        "请忘掉",
        "让我忘记",
        "让我忘掉",
    ]
    .iter()
    .any(|marker| text.contains(marker));
    let has_memory_delete_verb = ["删除", "移除", "清除", "删掉", "忘记", "忘掉"]
        .iter()
        .any(|verb| text.contains(verb));
    let has_memory_target_marker = ["删除记忆", "移除记忆", "清除记忆", "删掉记忆"]
        .iter()
        .any(|marker| text.contains(marker));
    if !direct_forget
        && !has_forget_request
        && !(has_memory_delete_verb && (text.contains("记忆") || text.contains("这条")))
        && !has_memory_target_marker
    {
        return None;
    }

    let markers = [
        "请帮我忘记",
        "请帮我忘掉",
        "帮我忘记",
        "帮我忘掉",
        "请忘记",
        "请忘掉",
        "让我忘记",
        "让我忘掉",
        "删除这条记忆",
        "移除这条记忆",
        "清除这条记忆",
        "删掉这条记忆",
        "删除记忆",
        "移除记忆",
        "清除记忆",
        "删掉记忆",
        "忘记",
        "忘掉",
        "删除",
        "移除",
        "清除",
        "删掉",
    ];
    let marker = markers
        .iter()
        .filter_map(|marker| {
            text.find(marker)
                .map(|start| (start, marker.len(), *marker))
        })
        .min_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    let Some((start, length, _)) = marker else {
        return Some(String::new());
    };
    let mut query = text[start + length..]
        .trim_matches(|character: char| {
            matches!(
                character,
                '：' | ':' | '，' | ',' | '。' | '！' | '!' | '？' | '?' | ' '
            )
        })
        .trim()
        .to_owned();
    for prefix in ["这条记忆", "这条", "记忆"] {
        if let Some(rest) = query.strip_prefix(prefix) {
            query = rest
                .trim_matches(|character: char| matches!(character, '：' | ':' | '，' | ',' | ' '))
                .trim()
                .to_owned();
            break;
        }
    }
    Some(query)
}

fn resolve_unique_memory_match(
    candidates: Vec<MemoryItem>,
    query: &str,
) -> Result<MemoryItem, AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::new(
            "memory_delete_target_required",
            "请明确说出要忘记的记忆内容；暂不支持通过聊天清空全部记忆。",
            false,
        ));
    }
    let exact = candidates
        .iter()
        .filter(|memory| memory.content.trim() == query)
        .cloned()
        .collect::<Vec<_>>();
    let matches = if exact.is_empty() { candidates } else { exact };
    match matches.len() {
        0 => Err(AppError::new(
            "memory_not_found",
            "没有找到能唯一匹配的本地记忆，请在记忆面板中选择。",
            false,
        )),
        1 => Ok(matches.into_iter().next().expect("one memory match")),
        _ => Err(AppError::new(
            "memory_ambiguous",
            "有多条记忆可能匹配，请在记忆面板中选择要删除的那一条。",
            false,
        )),
    }
}

fn parse_open_url_request(text: &str) -> Result<Option<OpenUrlRequest>, AppError> {
    let has_open_intent = ["打开", "访问", "进入", "浏览"]
        .iter()
        .any(|verb| text.contains(verb));
    if !has_open_intent || text.contains("不要打开") || text.contains("别打开") {
        return Ok(None);
    }

    let candidate = text.split_whitespace().find_map(|token| {
        let trimmed = token.trim_matches(|character: char| {
            matches!(
                character,
                '，' | '。'
                    | '！'
                    | '？'
                    | '；'
                    | ','
                    | '.'
                    | '!'
                    | '?'
                    | ';'
                    | ':'
                    | '：'
                    | '“'
                    | '”'
                    | '"'
                    | '\''
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
            )
        });
        let lower = trimmed.to_ascii_lowercase();
        (lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("www.")
            || (trimmed.contains('.') && !trimmed.contains(' ')))
        .then_some(trimmed.to_owned())
    });
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    Ok(Some(normalize_open_url(&candidate)?))
}

fn parse_read_webpage_request(text: &str) -> Result<Option<OpenUrlRequest>, AppError> {
    let has_read_intent = ["网页", "页面", "网站"]
        .iter()
        .any(|target| text.contains(target))
        && ["读取", "阅读", "查看", "看看", "分析", "获取"]
            .iter()
            .any(|verb| text.contains(verb));
    if !has_read_intent
        || ["不要读取", "别读取", "不要查看", "别查看", "不需要读取"]
            .iter()
            .any(|phrase| text.contains(phrase))
    {
        return Ok(None);
    }

    let candidate = text.split_whitespace().find_map(|token| {
        let trimmed = token.trim_matches(|character: char| {
            matches!(
                character,
                '，' | '。'
                    | '！'
                    | '？'
                    | '；'
                    | ','
                    | '.'
                    | '!'
                    | '?'
                    | ';'
                    | ':'
                    | '：'
                    | '“'
                    | '”'
                    | '"'
                    | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
            )
        });
        let lower = trimmed.to_ascii_lowercase();
        (lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("www.")
            || (trimmed.contains('.') && !trimmed.contains(' ')))
        .then_some(trimmed.to_owned())
    });
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    Ok(Some(normalize_open_url(&candidate)?))
}

struct TimerRequest {
    title: String,
    due_at_ms: i64,
    original_request: String,
}

fn parse_timer_request(text: &str, now_ms: i64) -> Result<Option<TimerRequest>, AppError> {
    let has_timer_intent = ["提醒", "叫我", "闹钟", "定时", "计时"]
        .iter()
        .any(|keyword| text.contains(keyword));
    if !has_timer_intent
        || (text.contains("取消")
            && ["提醒", "闹钟", "定时", "计时"]
                .iter()
                .any(|keyword| text.contains(keyword)))
    {
        return Ok(None);
    }
    if ["每天", "每日", "每周", "每星期", "每月", "工作日", "重复"]
        .iter()
        .any(|marker| text.contains(marker))
    {
        return Err(AppError::new(
            "unsupported_timer_schedule",
            "当前只支持一次性提醒，暂不支持每天、每周或其他重复规则。",
            false,
        ));
    }
    let due_at_ms = if let Some(delay_ms) = parse_relative_delay_ms(text) {
        validate_timer_delay_ms(delay_ms)?;
        now_ms.checked_add(delay_ms).ok_or_else(|| {
            AppError::new(
                "invalid_tool_arguments",
                "提醒时间超出系统可支持范围。",
                false,
            )
        })?
    } else {
        let Some(due_at_ms) = parse_absolute_due_at_ms(text, now_ms)? else {
            return Ok(None);
        };
        validate_timer_delay_ms(due_at_ms.saturating_sub(now_ms))?;
        due_at_ms
    };
    Ok(Some(TimerRequest {
        title: text.trim().to_owned(),
        due_at_ms,
        original_request: text.trim().to_owned(),
    }))
}

fn validate_timer_delay_ms(delay_ms: i64) -> Result<(), AppError> {
    if !(1_000..=7 * 24 * 60 * 60 * 1_000).contains(&delay_ms) {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "一次性提醒只支持 1 秒到 7 天后的时间。",
            false,
        ));
    }
    Ok(())
}

fn parse_absolute_due_at_ms(text: &str, now_ms: i64) -> Result<Option<i64>, AppError> {
    let now = Local.timestamp_millis_opt(now_ms).single().ok_or_else(|| {
        AppError::new(
            "invalid_tool_arguments",
            "无法根据当前系统时区解析提醒时间。",
            false,
        )
    })?;
    let (date, has_date_marker) = parse_absolute_date(text, now.date_naive())?;
    let Some((hour, minute)) = parse_absolute_clock(text)? else {
        if has_date_marker {
            return Err(AppError::new(
                "invalid_tool_arguments",
                "请提供明确的提醒时刻，例如“明天 10 点”或“8 月 30 日 10:30”。",
                false,
            ));
        }
        return Ok(None);
    };
    let date = date.unwrap_or_else(|| now.date_naive());
    let naive = date.and_hms_opt(hour, minute, 0).ok_or_else(|| {
        AppError::new(
            "invalid_tool_arguments",
            "提醒时间不是有效的日期或时刻。",
            false,
        )
    })?;
    let local = match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(_, _) => {
            return Err(AppError::new(
                "invalid_tool_arguments",
                "该时刻在本地时区存在歧义，请改用其他时间。",
                false,
            ));
        }
        LocalResult::None => {
            return Err(AppError::new(
                "invalid_tool_arguments",
                "该时刻在本地时区不存在，请改用其他时间。",
                false,
            ));
        }
    };
    let due_at_ms = local.timestamp_millis();
    if due_at_ms <= now_ms {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "提醒时间必须在未来，请改用更晚的日期或时刻。",
            false,
        ));
    }
    Ok(Some(due_at_ms))
}

fn parse_absolute_date(
    text: &str,
    today: chrono::NaiveDate,
) -> Result<(Option<chrono::NaiveDate>, bool), AppError> {
    if text.contains("后天") {
        return Ok((today.checked_add_signed(ChronoDuration::days(2)), true));
    }
    if text.contains("明天") {
        return Ok((today.checked_add_signed(ChronoDuration::days(1)), true));
    }
    if text.contains("今天") {
        return Ok((Some(today), true));
    }

    if let Some((year, month, day)) = parse_numeric_date_parts(text) {
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
            AppError::new(
                "invalid_tool_arguments",
                "提醒日期不是有效的年月日。",
                false,
            )
        })?;
        return Ok((Some(date), true));
    }

    let Some(month_start) = text.find('月') else {
        let has_date_marker = text.contains('年');
        return Ok((None, has_date_marker));
    };
    let day_start = text[month_start + '月'.len_utf8()..]
        .char_indices()
        .find_map(|(offset, character)| {
            matches!(character, '日' | '号').then_some(month_start + '月'.len_utf8() + offset)
        });
    let Some(day_start) = day_start else {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "请提供完整的提醒日期，例如“8 月 30 日 10 点”。",
            false,
        ));
    };
    let month = parse_number_before_marker(text, month_start).ok_or_else(|| {
        AppError::new(
            "invalid_tool_arguments",
            "提醒日期中的月份无法识别。",
            false,
        )
    })?;
    let day = parse_number_before_marker(text, day_start).ok_or_else(|| {
        AppError::new(
            "invalid_tool_arguments",
            "提醒日期中的日期无法识别。",
            false,
        )
    })?;
    let year = text[..month_start]
        .rfind('年')
        .and_then(|year_marker| parse_number_before_marker(text, year_marker))
        .map(|value| value as i32)
        .unwrap_or_else(|| today.year());
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
        AppError::new(
            "invalid_tool_arguments",
            "提醒日期不是有效的年月日。",
            false,
        )
    })?;
    Ok((Some(date), true))
}

fn parse_absolute_clock(text: &str) -> Result<Option<(u32, u32)>, AppError> {
    for (colon_start, character) in text.char_indices() {
        if character != ':' {
            continue;
        }
        let Some(hour) = trailing_ascii_number(&text[..colon_start]) else {
            continue;
        };
        let minute_start = colon_start + character.len_utf8();
        let Some(minute) = leading_ascii_number(&text[minute_start..]) else {
            continue;
        };
        return validate_clock(hour, minute);
    }

    let Some((marker_start, marker_length)) =
        text.char_indices().find_map(|(offset, character)| {
            matches!(character, '点' | '时').then_some((offset, character.len_utf8()))
        })
    else {
        return Ok(None);
    };
    let Some(mut hour) = parse_number_before_marker(text, marker_start) else {
        return Ok(None);
    };
    let after = text[marker_start + marker_length..].trim_start();
    let minute = if after.starts_with('半') {
        30
    } else if let Some((value, consumed)) = parse_number_prefix(after) {
        let rest = after[consumed..].trim_start();
        if rest.starts_with('分') {
            value
        } else {
            0
        }
    } else {
        0
    };
    let is_pm = ["下午", "晚上", "傍晚", "夜里", "夜间"]
        .iter()
        .any(|marker| text.contains(marker));
    let is_noon = text.contains("中午") && hour < 11;
    if (is_pm || is_noon) && hour < 12 {
        hour += 12;
    } else if text.contains("凌晨") && hour == 12 {
        hour = 0;
    }
    validate_clock(hour, minute)
}

fn validate_clock(hour: u32, minute: u32) -> Result<Option<(u32, u32)>, AppError> {
    if hour > 23 || minute > 59 {
        return Err(AppError::new(
            "invalid_tool_arguments",
            "提醒时刻不是有效的 24 小时时间。",
            false,
        ));
    }
    Ok(Some((hour, minute)))
}

fn parse_numeric_date_parts(text: &str) -> Option<(i32, u32, u32)> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let mut year_end = start;
        while year_end < bytes.len() && bytes[year_end].is_ascii_digit() {
            year_end += 1;
        }
        if year_end - start != 4
            || year_end >= bytes.len()
            || !matches!(bytes[year_end], b'-' | b'/')
        {
            continue;
        }
        let month_start = year_end + 1;
        let mut month_end = month_start;
        while month_end < bytes.len() && bytes[month_end].is_ascii_digit() {
            month_end += 1;
        }
        if month_end == month_start
            || month_end >= bytes.len()
            || !matches!(bytes[month_end], b'-' | b'/')
        {
            continue;
        }
        let day_start = month_end + 1;
        let mut day_end = day_start;
        while day_end < bytes.len() && bytes[day_end].is_ascii_digit() {
            day_end += 1;
        }
        if day_end == day_start {
            continue;
        }
        return Some((
            text[start..year_end].parse().ok()?,
            text[month_start..month_end].parse().ok()?,
            text[day_start..day_end].parse().ok()?,
        ));
    }
    None
}

fn parse_number_before_marker(text: &str, marker_start: usize) -> Option<u32> {
    let before = text[..marker_start].trim_end();
    let mut number_start = before.len();
    while let Some((offset, character)) = before[..number_start].char_indices().next_back() {
        if is_number_character(character) {
            number_start = offset;
        } else {
            break;
        }
    }
    (number_start < before.len()).then(|| parse_number_token(&before[number_start..]))?
}

fn parse_number_prefix(text: &str) -> Option<(u32, usize)> {
    let mut end = 0;
    for (offset, character) in text.char_indices() {
        if is_number_character(character) {
            end = offset + character.len_utf8();
        } else {
            break;
        }
    }
    (end > 0)
        .then(|| Some((parse_number_token(&text[..end])?, end)))
        .flatten()
}

fn parse_number_token(token: &str) -> Option<u32> {
    if token.chars().all(|character| character.is_ascii_digit()) {
        return token.parse().ok();
    }
    if token
        .chars()
        .all(|character| chinese_digit(character).is_some())
    {
        let mut value = 0_u32;
        for character in token.chars() {
            value = value
                .saturating_mul(10)
                .saturating_add(chinese_digit(character)?);
        }
        return Some(value);
    }
    let mut total = 0_u32;
    let mut current = 0_u32;
    for character in token.chars() {
        if let Some(value) = chinese_digit(character) {
            current = value;
        } else {
            let unit = match character {
                '十' => 10,
                '百' => 100,
                '千' => 1_000,
                _ => return None,
            };
            total = total.saturating_add(if current == 0 { unit } else { current * unit });
            current = 0;
        }
    }
    Some(total.saturating_add(current))
}

fn is_number_character(character: char) -> bool {
    character.is_ascii_digit()
        || chinese_digit(character).is_some()
        || matches!(character, '十' | '百' | '千')
}

fn chinese_digit(character: char) -> Option<u32> {
    match character {
        '零' | '〇' | '○' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

fn trailing_ascii_number(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let mut start = bytes.len();
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < bytes.len())
        .then(|| text[start..].parse().ok())
        .flatten()
}

fn leading_ascii_number(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    (end > 0).then(|| text[..end].parse().ok()).flatten()
}

fn parse_timer_cancel_query(text: &str) -> Option<String> {
    if !text.contains("取消") && !text.contains("撤销") {
        return None;
    }
    if !["提醒", "闹钟", "定时", "计时"]
        .iter()
        .any(|keyword| text.contains(keyword))
    {
        return None;
    }

    let query = [
        "请帮我",
        "帮我",
        "麻烦",
        "请",
        "取消",
        "撤销",
        "提醒",
        "闹钟",
        "定时",
        "计时",
        "叫我",
        "刚才",
        "那个",
        "一下",
        "掉",
    ]
    .into_iter()
    .fold(text.to_owned(), |value, marker| value.replace(marker, " "));
    Some(
        query
            .trim_matches(|character: char| {
                matches!(
                    character,
                    '：' | ':' | '，' | ',' | '。' | '！' | '!' | '？' | '?' | ' '
                )
            })
            .trim()
            .to_owned(),
    )
}

fn parse_explicit_timer_list_request(text: &str) -> bool {
    let compact = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if !compact.contains("提醒") && !compact.contains("闹钟") && !compact.contains("定时") {
        return false;
    }
    if [
        "取消", "撤销", "设置", "创建", "添加", "删除", "不要", "别", "不用", "无需",
    ]
    .iter()
    .any(|keyword| compact.contains(keyword))
    {
        return false;
    }
    [
        "有哪些提醒",
        "查看提醒",
        "看看提醒",
        "显示提醒",
        "列出提醒",
        "提醒列表",
        "待处理提醒",
        "目前有几条提醒",
        "还剩几条提醒",
    ]
    .iter()
    .any(|keyword| compact.contains(keyword))
}

fn parse_explicit_screen_observe_request(text: &str) -> Option<ExplicitScreenObserveRequest> {
    let has_screen_target = ["屏幕", "桌面", "画面", "显示器"]
        .iter()
        .any(|keyword| text.contains(keyword));
    let has_observe_intent = ["看", "查看", "观察", "分析", "识别", "读取", "截图", "截屏"]
        .iter()
        .any(|keyword| text.contains(keyword));
    if !has_screen_target || !has_observe_intent {
        return None;
    }
    if ["别看", "不要看", "不看"]
        .iter()
        .any(|keyword| text.contains(keyword))
    {
        return None;
    }
    if ["取消屏幕", "停止观察", "不要读取", "不要截图", "不需要看"]
        .iter()
        .any(|keyword| text.contains(keyword))
    {
        return None;
    }
    let scope = if ["主屏", "主显示器", "主屏幕"]
        .iter()
        .any(|keyword| text.contains(keyword))
    {
        ScreenCaptureScope::MainDisplay
    } else {
        ScreenCaptureScope::AllDisplays
    };
    Some(ExplicitScreenObserveRequest {
        request: text.trim().to_owned(),
        scope,
    })
}

fn has_explicit_screen_observe_refusal(messages: &[LlmMessage]) -> bool {
    let Some(content) = messages.iter().rev().find_map(|message| match message {
        LlmMessage::User { content } => Some(content.as_str()),
        _ => None,
    }) else {
        return false;
    };
    [
        "不要看屏幕",
        "别看屏幕",
        "不要读取屏幕",
        "别读取屏幕",
        "不要截图",
        "别截图",
        "不要上传截图",
        "别上传截图",
        "不需要读取屏幕",
        "取消屏幕观察",
        "停止屏幕观察",
        "不要看当前页面",
        "别看当前页面",
        "不用看当前页面",
    ]
    .iter()
    .any(|phrase| content.contains(phrase))
}

fn screen_mode_allows_automatic_analysis(
    mode: ScreenUnderstandingMode,
    vision_enabled: bool,
) -> bool {
    matches!(mode, ScreenUnderstandingMode::SessionAllowed) && vision_enabled
}

fn parse_relative_delay_ms(text: &str) -> Option<i64> {
    let common_phrases = [
        ("半小时", 30 * 60 * 1_000_i64),
        ("一小时", 60 * 60 * 1_000_i64),
        ("一个小时", 60 * 60 * 1_000_i64),
        ("两小时", 2 * 60 * 60 * 1_000_i64),
        ("二小时", 2 * 60 * 60 * 1_000_i64),
        ("三小时", 3 * 60 * 60 * 1_000_i64),
    ];
    if text.contains("后") || text.contains("以后") {
        for (phrase, delay) in common_phrases {
            if text.contains(&format!("{phrase}后")) || text.contains(&format!("{phrase}以后")) {
                return Some(delay);
            }
        }
    }

    let marker = text.rfind("以后").or_else(|| text.rfind('后'))?;
    let before = text[..marker].trim_end();
    let (unit_length, multiplier) = if before.ends_with("分钟") {
        ("分钟".len(), 60 * 1_000)
    } else if before.ends_with("小时") {
        ("小时".len(), 60 * 60 * 1_000)
    } else if before.ends_with('秒') {
        ('秒'.len_utf8(), 1_000)
    } else if before.ends_with('分') {
        ('分'.len_utf8(), 60 * 1_000)
    } else if before.ends_with('时') {
        ('时'.len_utf8(), 60 * 60 * 1_000)
    } else if before.ends_with('天') {
        ('天'.len_utf8(), 24 * 60 * 60 * 1_000)
    } else {
        return None;
    };
    let number_text = before[..before.len() - unit_length].trim_end();
    let end = number_text.len();
    let bytes = number_text.as_bytes();
    let mut number_start = end;
    while number_start > 0 && bytes[number_start - 1].is_ascii_digit() {
        number_start -= 1;
    }
    if number_start == end {
        return None;
    }
    let amount = number_text[number_start..end].parse::<i64>().ok()?;
    amount.checked_mul(multiplier)
}

fn format_timer_delay(delay_ms: i64) -> String {
    let total_seconds = (delay_ms.max(1) / 1_000) as u64;
    if total_seconds < 60 {
        return format!("{total_seconds}秒");
    }
    let total_minutes = total_seconds / 60;
    if total_minutes < 60 {
        return format!("{total_minutes}分钟");
    }
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    if hours < 24 {
        return if minutes == 0 {
            format!("{hours}小时")
        } else {
            format!("{hours}小时{minutes}分钟")
        };
    }
    let days = hours / 24;
    let remaining_hours = hours % 24;
    if remaining_hours == 0 {
        format!("{days}天")
    } else {
        format!("{days}天{remaining_hours}小时")
    }
}

fn format_pending_timer_list(timers: &[TimerItem], now_ms: i64) -> String {
    if timers.is_empty() {
        return "老师，目前没有待处理的提醒。".into();
    }
    let details = timers
        .iter()
        .enumerate()
        .map(|(index, timer)| {
            format!(
                "{}. {}（约{}后）",
                index + 1,
                timer.title,
                format_timer_delay(timer.due_at_ms.saturating_sub(now_ms))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("老师，目前有 {} 条待处理提醒：\n{details}", timers.len())
}

async fn open_url_in_default_browser(url: &str) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let status = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("/usr/bin/open")
                .arg(url)
                .status(),
        )
        .await
        .map_err(|_| AppError::new("tool_execution_timeout", "打开网页超过 5 秒未完成。", true))?
        .map_err(|error| {
            AppError::new(
                "tool_execution_failed",
                format!("无法启动默认浏览器：{error}"),
                true,
            )
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(AppError::new(
                "tool_execution_failed",
                format!(
                    "默认浏览器拒绝打开网页（退出码：{}）。",
                    status.code().unwrap_or(-1)
                ),
                true,
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Err(AppError::new(
            "tool_unavailable",
            "当前版本的打开网页工具仅支持 macOS。",
            false,
        ))
    }
}

async fn capture_screen_png(scope: ScreenCaptureScope) -> Result<Vec<u8>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let capture_id = Uuid::now_v7().to_string();
        let directory =
            std::env::temp_dir().join(format!("desktop-ai-companion-screen-{capture_id}"));
        std::fs::create_dir(&directory).map_err(|error| {
            AppError::new(
                "screen_capture_failed",
                format!("无法创建屏幕截图临时目录：{error}"),
                true,
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
        }
        let path = directory.join("capture.png");
        let mut command = tokio::process::Command::new("/usr/sbin/screencapture");
        command.kill_on_drop(true).args(["-x", "-t", "png"]);
        if scope == ScreenCaptureScope::MainDisplay {
            command.arg("-m");
        }
        command.arg(&path);
        let status = match tokio::time::timeout(Duration::from_secs(15), command.status()).await {
            Ok(result) => result.map_err(|error| {
                AppError::new(
                    "screen_capture_failed",
                    format!("无法启动 macOS 屏幕截图服务：{error}"),
                    true,
                )
            }),
            Err(_) => Err(AppError::new(
                "screen_capture_timeout",
                "读取当前屏幕超过 15 秒未完成。",
                true,
            )),
        };
        let bytes = match status {
            Ok(status) if status.success() => tokio::fs::read(&path).await.map_err(|error| {
                AppError::new(
                    "screen_capture_failed",
                    format!("无法读取本机临时屏幕截图：{error}"),
                    true,
                )
            }),
            Ok(status) => Err(AppError::new(
                "screen_capture_permission",
                format!(
                    "macOS 没有允许读取屏幕（退出码：{}）。请在系统设置中授予屏幕录制权限。",
                    status.code().unwrap_or(-1)
                ),
                false,
            )),
            Err(error) => Err(error),
        };
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_dir(&directory).await;
        bytes
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(AppError::new(
            "tool_unavailable",
            "当前版本的屏幕观察仅支持 macOS。",
            false,
        ))
    }
}

#[cfg(target_os = "macos")]
fn local_screen_ocr_executable(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let packaged = resource_dir.join("ScreenOCR");
        if packaged.is_file() {
            return Some(packaged);
        }
    }
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/ScreenOCR");
    development.is_file().then_some(development)
}

#[derive(Debug, Deserialize)]
struct LocalOcrResult {
    text: String,
}

async fn run_local_screen_ocr(app: &AppHandle, png: &[u8]) -> Result<Option<String>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let Some(executable) = local_screen_ocr_executable(app) else {
            return Ok(None);
        };
        let ocr_id = Uuid::now_v7().to_string();
        let directory = std::env::temp_dir().join(format!("desktop-ai-companion-ocr-{ocr_id}"));
        tokio::fs::create_dir(&directory).await.map_err(|error| {
            AppError::new(
                "screen_ocr_failed",
                format!("无法创建本地 OCR 临时目录：{error}"),
                true,
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
        }
        let path = directory.join("capture.png");
        let result = async {
            tokio::fs::write(&path, png).await.map_err(|error| {
                AppError::new(
                    "screen_ocr_failed",
                    format!("无法准备本地 OCR 图片：{error}"),
                    true,
                )
            })?;
            let output = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::process::Command::new(&executable)
                    .kill_on_drop(true)
                    .args(["--input", path.to_string_lossy().as_ref()])
                    .output(),
            )
            .await
            .map_err(|_| AppError::new("screen_ocr_timeout", "本地 OCR 超过 15 秒未完成。", true))?
            .map_err(|error| {
                AppError::new(
                    "screen_ocr_failed",
                    format!("无法启动本地 OCR：{error}"),
                    true,
                )
            })?;
            if !output.status.success() {
                return Err(AppError::new(
                    "screen_ocr_failed",
                    format!(
                        "本地 OCR 执行失败（退出码：{}）。",
                        output.status.code().unwrap_or(-1)
                    ),
                    true,
                ));
            }
            if output.stdout.len() > 256 * 1024 {
                return Err(AppError::new(
                    "screen_ocr_output_too_large",
                    "本地 OCR 返回内容过大，已丢弃。",
                    false,
                ));
            }
            let parsed: LocalOcrResult =
                serde_json::from_slice(&output.stdout).map_err(|error| {
                    AppError::new(
                        "screen_ocr_failed",
                        format!("本地 OCR 返回格式无法识别：{error}"),
                        true,
                    )
                })?;
            let text = parsed.text.trim();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text.chars().take(6_000).collect()))
            }
        }
        .await;
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_dir(&directory).await;
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, png);
        Ok(None)
    }
}

fn memory_context_with_budget(memories: Vec<MemoryItem>, budget: usize) -> Vec<MemoryContextEntry> {
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
            (!content.is_empty()).then_some(MemoryContextEntry {
                id: memory.id,
                scope: memory.scope,
                kind: memory.kind,
                content,
                sensitivity: memory.sensitivity,
                updated_at_ms: memory.updated_at_ms,
            })
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
            vision: Default::default(),
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

        let vision_valid = ProviderSettings {
            vision: crate::contracts::VisionProviderSettings {
                enabled: true,
                endpoint: "https://vision.example.com/v1".into(),
                model: "vision-model".into(),
                has_api_key: false,
            },
            ..ProviderSettings::default()
        };
        assert!(validate_provider_settings(&vision_valid).is_ok());
        let vision_invalid = ProviderSettings {
            vision: crate::contracts::VisionProviderSettings {
                endpoint: "file:///tmp/vision".into(),
                ..vision_valid.vision.clone()
            },
            ..vision_valid
        };
        assert!(validate_provider_settings(&vision_invalid).is_err());
    }

    #[test]
    fn validates_pet_scale() {
        assert!(validate_pet_settings(&PetSettings::default()).is_ok());
        assert!(validate_pet_settings(&PetSettings {
            scale: 0.4,
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
    fn matches_tool_approval_identity_and_generation() {
        let pending = PendingTool {
            approval: ToolApproval {
                tool_call_id: "tool-1".into(),
                tool_name: "system.open_url".into(),
                display_name: "打开网页".into(),
                summary: "打开 example.com".into(),
                url: Some("https://example.com".into()),
                host: Some("example.com".into()),
                title: None,
                due_at_ms: None,
                expires_at_ms: 2_000,
            },
            turn_id: "turn-1".into(),
            generation: 3,
            action: PendingToolAction::OpenUrl {
                url: "https://example.com".into(),
            },
            origin: PendingToolOrigin::Explicit,
        };

        assert!(AppCore::tool_approval_matches(
            Some(&pending),
            "tool-1",
            "turn-1",
            3
        ));
        assert!(!AppCore::tool_approval_matches(
            Some(&pending),
            "tool-1",
            "turn-1",
            4
        ));
        assert!(!AppCore::tool_approval_matches(
            Some(&pending),
            "tool-2",
            "turn-1",
            3
        ));
        assert!(!AppCore::tool_approval_matches(None, "tool-1", "turn-1", 3));
    }

    #[test]
    fn parses_explicit_open_url_request() {
        let request =
            parse_open_url_request("老师，帮我打开 https://github.com/mingyuan.wang/demo。")
                .unwrap()
                .unwrap();
        assert_eq!(request.url, "https://github.com/mingyuan.wang/demo");
        assert_eq!(request.host, "github.com");

        let bare_host = parse_open_url_request("访问 www.example.com")
            .unwrap()
            .unwrap();
        assert_eq!(bare_host.url, "https://www.example.com/");
    }

    #[test]
    fn parses_explicit_read_webpage_request() {
        let request =
            parse_read_webpage_request("老师，请读取 https://example.com/docs 页面内容。")
                .unwrap()
                .unwrap();
        assert_eq!(request.url, "https://example.com/docs");
        assert_eq!(request.host, "example.com");
        assert!(parse_read_webpage_request("不要读取 https://example.com")
            .unwrap()
            .is_none());
        assert!(parse_read_webpage_request("请读取网页但没有网址")
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_unsafe_open_url_request() {
        assert!(parse_open_url_request("打开 file:///tmp/secret.txt").is_err());
        assert!(parse_open_url_request("打开 https://user:password@example.com").is_err());
        assert!(parse_open_url_request("打开网页但没有提供网址")
            .unwrap()
            .is_none());
    }

    #[test]
    fn parses_relative_timer_request() {
        let timer = parse_timer_request("半小时后提醒我喝水", 1_000)
            .unwrap()
            .unwrap();
        assert_eq!(timer.due_at_ms, 1_801_000);
        assert_eq!(timer.title, "半小时后提醒我喝水");

        let timer = parse_timer_request("提醒我 30 分钟后开会", 2_000)
            .unwrap()
            .unwrap();
        assert_eq!(timer.due_at_ms, 1_802_000);
    }

    #[test]
    fn parses_absolute_timer_request_in_local_timezone() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 30, 9, 0, 0)
            .single()
            .unwrap();
        let now_ms = now.timestamp_millis();

        let timer = parse_timer_request("明天下午三点半提醒我开会", now_ms)
            .unwrap()
            .unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 8, 31, 15, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(timer.due_at_ms, expected);

        let timer = parse_timer_request("2026-08-31 16:05 提醒我提交报告", now_ms)
            .unwrap()
            .unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 8, 31, 16, 5, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(timer.due_at_ms, expected);

        let timer = parse_timer_request("8月30日 10:15提醒我喝水", now_ms)
            .unwrap()
            .unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 8, 30, 10, 15, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(timer.due_at_ms, expected);

        let timer = parse_timer_request("二〇二六年八月三十一日晚上八点提醒我回家", now_ms)
            .unwrap()
            .unwrap();
        let expected = Local
            .with_ymd_and_hms(2026, 8, 31, 20, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(timer.due_at_ms, expected);
    }

    #[test]
    fn ignores_non_relative_timer_request() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 30, 9, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert!(parse_timer_request("明天下午提醒我开会", now).is_err());
        assert!(parse_timer_request("取消 30 分钟后的提醒", 1_000)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rejects_ambiguous_or_past_absolute_timer_requests() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 30, 9, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert!(parse_timer_request("今天八点提醒我吃药", now).is_err());
        assert!(parse_timer_request("2026-08-30 10:00 提醒我开会", now)
            .unwrap()
            .is_some());
        assert!(parse_timer_request("2026-09-10 10:00 提醒我开会", now).is_err());
        assert!(parse_timer_request("明天 25 点提醒我开会", now).is_err());
        assert!(parse_timer_request("每天 10 点提醒我吃药", now).is_err());
    }

    #[test]
    fn parses_explicit_timer_cancel_query() {
        assert_eq!(
            parse_timer_cancel_query("请取消提醒：喝水"),
            Some("喝水".into())
        );
        assert_eq!(parse_timer_cancel_query("取消提醒"), Some(String::new()));
        assert_eq!(parse_timer_cancel_query("请打开网页"), None);
    }

    #[test]
    fn recognizes_only_read_only_timer_list_requests() {
        assert!(parse_explicit_timer_list_request("老师，我有哪些提醒？"));
        assert!(parse_explicit_timer_list_request("查看一下待处理提醒"));
        assert!(parse_explicit_timer_list_request("显示提醒列表"));
        assert!(!parse_explicit_timer_list_request("请取消提醒：喝水"));
        assert!(!parse_explicit_timer_list_request("半小时后提醒我喝水"));
        assert!(!parse_explicit_timer_list_request("不要查看提醒"));
    }

    #[test]
    fn formats_local_timer_list_without_provider_context() {
        let timers = vec![TimerItem {
            id: "timer-1".into(),
            title: "喝水".into(),
            due_at_ms: 61_000,
            timezone: "local".into(),
            original_request: "一分钟后提醒我喝水".into(),
            status: "pending".into(),
            created_at_ms: 1_000,
        }];
        let response = format_pending_timer_list(&timers, 1_000);
        assert!(response.contains("目前有 1 条待处理提醒"));
        assert!(response.contains("喝水"));
        assert!(response.contains("1分钟后"));
        assert_eq!(
            format_pending_timer_list(&[], 1_000),
            "老师，目前没有待处理的提醒。"
        );
    }

    #[test]
    fn recognizes_explicit_screen_observe_request() {
        let request =
            parse_explicit_screen_observe_request("请帮我看一下当前屏幕").expect("screen request");
        assert_eq!(request.request, "请帮我看一下当前屏幕");
        assert_eq!(request.scope, ScreenCaptureScope::AllDisplays);

        let request = parse_explicit_screen_observe_request("请看一下主显示器")
            .expect("main display request");
        assert_eq!(request.scope, ScreenCaptureScope::MainDisplay);
        assert!(parse_explicit_screen_observe_request("不要看我的屏幕").is_none());
        assert!(parse_explicit_screen_observe_request("取消屏幕观察").is_none());
        assert!(parse_explicit_screen_observe_request("你好吗").is_none());
    }

    #[test]
    fn model_screen_observe_honors_latest_user_refusal() {
        assert!(!has_explicit_screen_observe_refusal(&[LlmMessage::User {
            content: "请评价我当前页面的观点".into(),
        },]));
        assert!(has_explicit_screen_observe_refusal(&[LlmMessage::User {
            content: "不要读取屏幕".into(),
        },]));
        assert!(has_explicit_screen_observe_refusal(&[
            LlmMessage::User {
                content: "请查看当前屏幕".into(),
            },
            LlmMessage::Assistant {
                content: Some("好的".into()),
                tool_calls: Vec::new(),
            },
            LlmMessage::User {
                content: "不要上传截图".into(),
            },
        ]));
    }

    #[test]
    fn screen_automatic_mode_requires_session_consent_and_vision() {
        assert!(!screen_mode_allows_automatic_analysis(
            ScreenUnderstandingMode::AskEachTime,
            true
        ));
        assert!(!screen_mode_allows_automatic_analysis(
            ScreenUnderstandingMode::SessionAllowed,
            false
        ));
        assert!(screen_mode_allows_automatic_analysis(
            ScreenUnderstandingMode::SessionAllowed,
            true
        ));
        assert_eq!(
            RuntimeState::default().screen_understanding_mode,
            ScreenUnderstandingMode::AskEachTime
        );
    }

    #[test]
    fn screen_observation_expires_at_ttl_boundary() {
        let captured_at_ms = 100;
        let expires_at_ms = captured_at_ms + SCREEN_OBSERVATION_TTL_MS;
        assert!(!screen_observation_expired(
            expires_at_ms,
            expires_at_ms - 1
        ));
        assert!(screen_observation_expired(expires_at_ms, expires_at_ms));
    }

    #[test]
    fn validates_model_tool_candidates_before_approval() {
        let url = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-url".into(),
                name: "system.open_url".into(),
                arguments: r#"{"url":"https://example.com"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert_eq!(url.host.as_deref(), Some("example.com"));

        let web = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-web-read".into(),
                name: "web.read".into(),
                arguments: r#"{"url":"https://example.com/docs"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert_eq!(web.url.as_deref(), Some("https://example.com/docs"));
        assert_eq!(web.host.as_deref(), Some("example.com"));

        let timer = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-timer".into(),
                name: "timer.create".into(),
                arguments: r#"{"title":"喝水","delay_seconds":30}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert_eq!(timer.due_at_ms, Some(31_000));

        let cancel_timer = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-timer-cancel".into(),
                name: "timer.cancel".into(),
                arguments: r#"{"title":"喝水"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert_eq!(cancel_timer.title.as_deref(), Some("喝水"));
        assert!(ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-timer-cancel-invalid".into(),
                name: "timer.cancel".into(),
                arguments: r#"{}"#.into(),
            },
            1_000,
        )
        .is_err());

        let memory = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-memory".into(),
                name: "memory.save".into(),
                arguments: r#"{"content":"老师喜欢蓝色"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert_eq!(memory.title.as_deref(), Some("老师喜欢蓝色"));

        let screen = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-screen".into(),
                name: "screen.observe".into(),
                arguments: r#"{"request":"请查看当前屏幕"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert!(matches!(
            screen.action,
            ToolAction::ObserveScreen { request, scope }
                if request == "请查看当前屏幕" && scope == ScreenCaptureScope::AllDisplays
        ));

        let main_screen = ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-main-screen".into(),
                name: "screen.observe".into(),
                arguments: r#"{"request":"请查看主显示器","scope":"main_display"}"#.into(),
            },
            1_000,
        )
        .unwrap();
        assert!(matches!(
            main_screen.action,
            ToolAction::ObserveScreen { scope, .. }
                if scope == ScreenCaptureScope::MainDisplay
        ));

        assert!(ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-invalid-screen-scope".into(),
                name: "screen.observe".into(),
                arguments: r#"{"request":"请查看当前屏幕","scope":"window"}"#.into(),
            },
            1_000,
        )
        .is_err());

        assert!(ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-unsafe".into(),
                name: "system.open_url".into(),
                arguments: r#"{"url":"file:///tmp/secret"}"#.into(),
            },
            1_000,
        )
        .is_err());
        assert!(ToolPolicy::validate_model_call(
            &ToolCall {
                id: "call-unknown".into(),
                name: "system.run_shell".into(),
                arguments: "{}".into(),
            },
            1_000,
        )
        .is_err());
    }

    #[test]
    fn recognizes_only_explicit_memory_intent() {
        assert!(parse_explicit_memory_intent("请记住我喜欢蓝色").is_some());
        assert!(parse_explicit_memory_intent("别忘了我周五要开会").is_some());
        assert!(parse_explicit_memory_intent("我喜欢蓝色").is_none());
        assert!(parse_explicit_memory_intent("不要记住这句话").is_none());
        assert_eq!(
            parse_explicit_memory_intent("请记住：我喜欢蓝色。").map(|intent| intent.content),
            Some("我喜欢蓝色".into())
        );
        assert!(parse_explicit_memory_intent("请记住").is_none());
        let relationship =
            parse_explicit_memory_intent("请记住我们：第一次并肩完成任务。").unwrap();
        assert_eq!(relationship.scope, "relationship");
        assert_eq!(relationship.kind, "event");
        assert_eq!(relationship.content, "第一次并肩完成任务");
        assert_eq!(
            parse_explicit_memory_intent("请记住这次：我们一起完成了任务。")
                .map(|intent| intent.content),
            Some("我们一起完成了任务".into())
        );
    }

    #[test]
    fn extracts_only_conservative_automatic_memory_candidates() {
        let candidate = extract_automatic_memory_candidate("老师，我喜欢蓝色。为我记住这个")
            .expect("stable preference should be a candidate");
        assert_eq!(candidate.scope, "user");
        assert_eq!(candidate.kind, "preference");
        assert!(candidate.confidence >= 0.6);
        assert!(extract_automatic_memory_candidate("我喜欢什么颜色？").is_none());
        assert!(extract_automatic_memory_candidate("我的 API key 是 sk-secret").is_none());
        assert!(extract_automatic_memory_candidate("请记住我喜欢蓝色").is_none());
    }

    #[test]
    fn memory_policy_detects_no_memory_and_sensitive_content() {
        assert!(is_no_memory_request("这次不要记住我喜欢蓝色"));
        assert!(is_no_memory_request("别记录这段信息"));
        assert!(!is_no_memory_request("不要忘记我喜欢蓝色"));
        assert_eq!(classify_memory_sensitivity("我的住址是某地"), "sensitive");
        assert_eq!(
            classify_memory_sensitivity("API key 是 sk-secret"),
            "restricted"
        );
        assert_eq!(classify_memory_sensitivity("我喜欢蓝色"), "normal");
    }

    #[test]
    fn preference_fact_key_normalizes_opposite_statements() {
        assert_eq!(
            derive_memory_fact_key("user", "preference", "我喜欢  咖啡"),
            derive_memory_fact_key("user", "preference", "我不喜欢咖啡")
        );
        assert!(derive_memory_fact_key("user", "note", "我喜欢咖啡").is_none());
        assert!(parse_explicit_memory_intent("请记住我喜欢咖啡")
            .is_some_and(|intent| intent.kind == "preference"));
    }

    #[test]
    fn memory_validity_uses_an_explicit_date_before_default_ttl() {
        let now = Local
            .with_ymd_and_hms(2026, 9, 1, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let expected_until = Local
            .with_ymd_and_hms(2099, 1, 1, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let (valid_from_ms, valid_until_ms) =
            derive_memory_validity("project", "我的项目截止 2098年12月31日完成", now);
        assert_eq!(valid_from_ms, Some(now));
        assert_eq!(valid_until_ms, Some(expected_until));
    }

    #[test]
    fn derives_bounded_relationship_signals_without_lowering_familiarity() {
        assert_eq!(
            derive_relationship_signal("谢谢老师，我们一起完成任务了"),
            (0.06, "最近与老师有共同完成事项的积极互动。")
        );
        assert_eq!(
            derive_relationship_signal("我有点失望"),
            (
                -0.04,
                "近期互动出现了负面情绪，关系状态保持可修复。熟悉度不会因此下降。"
            )
        );
        assert_eq!(derive_relationship_signal("今天天气不错"), (0.0, ""));
    }

    #[test]
    fn parses_explicit_memory_delete_queries_without_guessing() {
        assert_eq!(
            parse_explicit_memory_delete_query("请帮我忘记：我喜欢蓝色"),
            Some("我喜欢蓝色".into())
        );
        assert_eq!(
            parse_explicit_memory_delete_query("删除记忆：老师喜欢蓝色"),
            Some("老师喜欢蓝色".into())
        );
        assert_eq!(
            parse_explicit_memory_delete_query("删除这条记忆"),
            Some(String::new())
        );
        assert!(parse_explicit_memory_delete_query("不要忘记我喜欢蓝色").is_none());
        assert!(parse_explicit_memory_delete_query("我忘记了开会时间").is_none());
        assert!(parse_explicit_memory_delete_query("你好吗").is_none());
    }

    #[test]
    fn resolves_only_one_memory_candidate_for_chat_deletion() {
        let memory = |id: &str, content: &str| MemoryItem {
            id: id.into(),
            scope: "user".into(),
            character_id: None,
            kind: "note".into(),
            content: content.into(),
            importance: 0.7,
            confidence: 1.0,
            sensitivity: "normal".into(),
            origin: "user_explicit".into(),
            fact_key: None,
            source_turn_id: None,
            source_message_id: None,
            valid_from_ms: None,
            valid_until_ms: None,
            last_confirmed_at_ms: Some(1),
            version: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let candidate = memory("memory-1", "我喜欢蓝色");
        assert_eq!(
            resolve_unique_memory_match(vec![candidate.clone()], "我喜欢蓝色")
                .unwrap()
                .id,
            candidate.id
        );
        assert_eq!(
            resolve_unique_memory_match(Vec::new(), "我喜欢蓝色")
                .unwrap_err()
                .code,
            "memory_not_found"
        );
        assert_eq!(
            resolve_unique_memory_match(
                vec![candidate.clone(), memory("memory-2", "我也喜欢蓝色")],
                "蓝色"
            )
            .unwrap_err()
            .code,
            "memory_ambiguous"
        );
        assert_eq!(
            resolve_unique_memory_match(vec![candidate], "")
                .unwrap_err()
                .code,
            "memory_delete_target_required"
        );
    }

    #[tokio::test]
    async fn explicit_memory_is_retrieved_used_and_deleted() {
        let storage = Storage::open_in_memory().unwrap();
        let memory = storage
            .create_user_memory("老师最喜欢的颜色是蓝色", 10)
            .unwrap();
        storage
            .create_user_memory("老师明天下午要开会", 10)
            .unwrap();
        let matched = storage
            .search_memories("老师喜欢什么颜色", "aris", 6)
            .unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, memory.id);

        let provider = create_provider(&ProviderSettings::default(), None).unwrap();
        let mut stream = provider
            .start(
                LlmRequest {
                    messages: vec![LlmMessage::User {
                        content: "老师喜欢什么颜色？".into(),
                    }],
                    memory_context: memory_context_with_budget(matched, 2_400),
                    tools: ToolRegistry::definitions(),
                    tool_steps: 0,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        while let Some(event) = stream.next().await {
            if let LlmEvent::TextDelta(delta) = event.unwrap() {
                response.push_str(&delta);
            }
        }
        assert!(response.contains("老师最喜欢的颜色是蓝色"));

        assert!(storage
            .delete_memory(&memory.id, memory.version, 11)
            .unwrap());
        assert!(storage
            .search_memories("老师喜欢什么颜色", "aris", 6)
            .unwrap()
            .is_empty());
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
