use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::{
    contracts::{
        AppError, AppSnapshot, CancelTurnRequest, CharacterSettings, PetSettings, ProviderSettings,
        SubmitTextRequest, UpdatePetSettingsRequest, UpdateProviderRequest,
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
                revision: request.expected_revision,
            },
            request.expected_revision,
            request.api_key,
            request.clear_api_key,
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
