use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::{
    contracts::{
        ActionRequest, AppError, AppSnapshot, CancelTimerRequest, CancelTurnRequest,
        CharacterSettings, CreateMemoryRequest, DataSummary, DeleteMemoryRequest, MemoryItem,
        PetSettings, ProviderSettings, ScreenObservationPreview, ScreenObservationRequest,
        SearchMemoriesRequest, SetListeningRequest, SubmitTextRequest, ToolDecisionRequest,
        UpdateMemoryRequest, UpdatePetSettingsRequest, UpdateProviderRequest,
        UpdateVoiceSettingsRequest, VisionProviderSettings, VoiceSettings,
    },
    core::AppCore,
    desktop,
};

pub struct CoreState(pub Arc<AppCore>);

#[tauri::command]
pub async fn app_get_snapshot(state: State<'_, CoreState>) -> Result<AppSnapshot, AppError> {
    state.0.snapshot().await
}

#[tauri::command]
pub async fn chat_submit_text(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: SubmitTextRequest,
) -> Result<(), AppError> {
    if request.request_id.trim().is_empty() {
        return Err(AppError::new(
            "invalid_request",
            "request_id 不能为空。",
            false,
        ));
    }
    state.0.start_text_turn(app, request.text).await
}

#[tauri::command]
pub async fn turn_cancel(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: CancelTurnRequest,
) -> Result<(), AppError> {
    if request.request_id.trim().is_empty() {
        return Err(AppError::new(
            "invalid_request",
            "request_id 不能为空。",
            false,
        ));
    }
    state
        .0
        .cancel_turn(&app, &request.turn_id, &request.reason)
        .await
}

#[tauri::command]
pub async fn tool_decide(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ToolDecisionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state
        .0
        .decide_tool(&app, &request.tool_call_id, request.approved)
        .await
}

#[tauri::command]
pub async fn timer_cancel(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: CancelTimerRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.cancel_timer(&app, &request.timer_id).await
}

#[tauri::command]
pub async fn screen_get_observation_preview(
    state: State<'_, CoreState>,
    request: ScreenObservationRequest,
) -> Result<ScreenObservationPreview, AppError> {
    validate_request_id(&request.request_id)?;
    state
        .0
        .get_screen_observation_preview(&request.observation_id)
        .await
}

#[tauri::command]
pub async fn screen_clear_observation(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.clear_screen_observation(&app).await
}

#[tauri::command]
pub async fn settings_update_provider(
    state: State<'_, CoreState>,
    request: UpdateProviderRequest,
) -> Result<ProviderSettings, AppError> {
    if request.request_id.trim().is_empty() {
        return Err(AppError::new(
            "invalid_request",
            "request_id 不能为空。",
            false,
        ));
    }
    state
        .0
        .update_provider(
            ProviderSettings {
                provider: request.provider,
                endpoint: request.endpoint,
                model: request.model,
                has_api_key: false,
                allow_memory_context: request.allow_memory_context,
                vision: VisionProviderSettings {
                    enabled: request.vision.enabled,
                    endpoint: request.vision.endpoint,
                    model: request.vision.model,
                    has_api_key: false,
                },
                revision: request.expected_revision,
            },
            request.expected_revision,
            request.api_key,
            request.clear_api_key,
            request.vision.api_key,
            request.vision.clear_api_key,
        )
        .await
}

#[tauri::command]
pub fn pet_open_panel(app: AppHandle) -> Result<(), AppError> {
    desktop::show_main_window(&app)
}

#[tauri::command]
pub async fn pet_update_settings(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: UpdatePetSettingsRequest,
) -> Result<PetSettings, AppError> {
    if request.request_id.trim().is_empty() {
        return Err(AppError::new(
            "invalid_request",
            "request_id 不能为空。",
            false,
        ));
    }
    state
        .0
        .update_pet_settings(
            &app,
            PetSettings {
                visible: request.visible,
                click_through: request.click_through,
                scale: request.scale,
                revision: request.expected_revision,
            },
            request.expected_revision,
        )
        .await
}

#[tauri::command]
pub async fn character_import_vrm(
    app: AppHandle,
    state: State<'_, CoreState>,
) -> Result<Option<CharacterSettings>, AppError> {
    let selected = app
        .dialog()
        .file()
        .set_title("导入 VRM 角色")
        .add_filter("VRM 角色", &["vrm"])
        .blocking_pick_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected.into_path().map_err(|error| {
        AppError::new(
            "character_import",
            format!("无法读取所选文件路径：{error}"),
            false,
        )
    })?;
    state.0.import_character_vrm(&app, path).await.map(Some)
}

#[tauri::command]
pub async fn memory_search(
    state: State<'_, CoreState>,
    request: SearchMemoriesRequest,
) -> Result<Vec<MemoryItem>, AppError> {
    validate_request_id(&request.request_id)?;
    state.0.search_memories(&request.query, request.limit).await
}

#[tauri::command]
pub fn memory_create(
    state: State<'_, CoreState>,
    request: CreateMemoryRequest,
) -> Result<MemoryItem, AppError> {
    validate_request_id(&request.request_id)?;
    state.0.create_memory(&request.content)
}

#[tauri::command]
pub fn memory_update(
    state: State<'_, CoreState>,
    request: UpdateMemoryRequest,
) -> Result<MemoryItem, AppError> {
    validate_request_id(&request.request_id)?;
    state
        .0
        .update_memory(&request.id, &request.content, request.expected_version)
}

#[tauri::command]
pub fn memory_delete(
    state: State<'_, CoreState>,
    request: DeleteMemoryRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.delete_memory(&request.id, request.expected_version)
}

#[tauri::command]
pub fn data_get_summary(
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<DataSummary, AppError> {
    validate_request_id(&request.request_id)?;
    state.0.data_summary()
}

#[tauri::command]
pub async fn data_clear_conversation(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<DataSummary, AppError> {
    validate_request_id(&request.request_id)?;
    state.0.clear_conversation_history(&app).await
}

#[tauri::command]
pub fn data_clear_memories(
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<DataSummary, AppError> {
    validate_request_id(&request.request_id)?;
    state.0.clear_all_memories()
}

fn validate_request_id(request_id: &str) -> Result<(), AppError> {
    if request_id.trim().is_empty() {
        return Err(AppError::new(
            "invalid_request",
            "request_id 不能为空。",
            false,
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn voice_request_permissions(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.request_voice_permissions(&app).await
}

#[tauri::command]
pub async fn voice_open_privacy_settings(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.open_voice_privacy_settings(&app).await
}

#[tauri::command]
pub async fn voice_set_listening_mode(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: SetListeningRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.set_listening(&app, request.enabled).await
}

#[tauri::command]
pub async fn voice_ptt_start(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.start_push_to_talk(&app).await
}

#[tauri::command]
pub async fn voice_ptt_finish(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.finish_push_to_talk(&app).await
}

#[tauri::command]
pub async fn playback_stop(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: ActionRequest,
) -> Result<(), AppError> {
    validate_request_id(&request.request_id)?;
    state.0.stop_playback(&app).await
}

#[tauri::command]
pub async fn voice_update_settings(
    app: AppHandle,
    state: State<'_, CoreState>,
    request: UpdateVoiceSettingsRequest,
) -> Result<VoiceSettings, AppError> {
    validate_request_id(&request.request_id)?;
    state
        .0
        .update_voice_settings(&app, request.tts_enabled, request.expected_revision)
        .await
}
