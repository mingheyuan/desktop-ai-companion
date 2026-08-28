use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::{
    contracts::{
        AppError, AppSnapshot, CancelTurnRequest, ProviderSettings, SubmitTextRequest,
        UpdateProviderRequest,
    },
    core::AppCore,
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
                has_api_key: std::env::var("DESKTOP_AI_LLM_API_KEY").is_ok(),
                revision: request.expected_revision,
            },
            request.expected_revision,
        )
        .await
}

#[tauri::command]
pub fn pet_open_panel(app: AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::new("internal", "主窗口不存在。", false))?;
    window
        .show()
        .map_err(|error| AppError::new("internal", format!("无法显示主窗口：{error}"), true))?;
    window
        .set_focus()
        .map_err(|error| AppError::new("internal", format!("无法聚焦主窗口：{error}"), true))
}
