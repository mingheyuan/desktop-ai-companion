use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, LogicalSize, Manager, Size, WindowEvent,
};

use crate::{
    commands::CoreState,
    contracts::{AppError, PetSettings},
    core::AppCore,
};

const PET_BASE_WIDTH: f64 = 360.0;
const PET_BASE_HEIGHT: f64 = 520.0;

pub fn show_main_window(app: &AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::new("internal", "主窗口不存在。", false))?;
    window
        .show()
        .map_err(|error| desktop_error("无法显示主窗口", error))?;
    window
        .set_focus()
        .map_err(|error| desktop_error("无法聚焦主窗口", error))
}

pub fn toggle_main_window(app: &AppHandle) -> Result<(), AppError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::new("internal", "主窗口不存在。", false))?;
    let visible = window
        .is_visible()
        .map_err(|error| desktop_error("无法读取主窗口状态", error))?;
    if visible {
        window
            .hide()
            .map_err(|error| desktop_error("无法隐藏主窗口", error))
    } else {
        show_main_window(app)
    }
}

pub fn apply_pet_window_settings(app: &AppHandle, settings: &PetSettings) -> Result<(), AppError> {
    let window = app
        .get_webview_window("pet")
        .ok_or_else(|| AppError::new("internal", "桌宠窗口不存在。", false))?;
    window
        .set_size(Size::Logical(LogicalSize::new(
            PET_BASE_WIDTH * settings.scale,
            PET_BASE_HEIGHT * settings.scale,
        )))
        .map_err(|error| desktop_error("无法缩放桌宠窗口", error))?;
    window
        .set_ignore_cursor_events(settings.click_through)
        .map_err(|error| desktop_error("无法切换桌宠点击穿透", error))?;
    if settings.visible {
        window
            .show()
            .map_err(|error| desktop_error("无法显示桌宠窗口", error))?;
    } else {
        window
            .hide()
            .map_err(|error| desktop_error("无法隐藏桌宠窗口", error))?;
    }
    Ok(())
}

pub fn setup(app: &mut App, core: Arc<AppCore>) -> tauri::Result<()> {
    let open_main = MenuItem::with_id(app, "open-main", "打开聊天面板", true, None::<&str>)?;
    let toggle_pet = MenuItem::with_id(app, "toggle-pet", "显示 / 隐藏桌宠", true, None::<&str>)?;
    let toggle_click_through = MenuItem::with_id(
        app,
        "toggle-click-through",
        "切换桌宠点击穿透",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_main,
            &toggle_pet,
            &toggle_click_through,
            &separator,
            &quit,
        ],
    )?;

    let tray_core = Arc::clone(&core);
    let mut tray = TrayIconBuilder::with_id("companion-tray")
        .tooltip("Desktop AI Companion")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open-main" => {
                let _ = show_main_window(app);
            }
            "toggle-pet" => {
                spawn_pet_toggle(app.clone(), Arc::clone(&tray_core), false);
            }
            "toggle-click-through" => {
                spawn_pet_toggle(app.clone(), Arc::clone(&tray_core), true);
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone()).icon_as_template(true);
    }
    tray.build(app)?;

    if let Some(main) = app.get_webview_window("main") {
        let main_for_event = main.clone();
        main.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = main_for_event.hide();
            }
        });
    }

    let app_handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let _ = core.apply_current_pet_settings(&app_handle).await;
    });
    Ok(())
}

fn spawn_pet_toggle(app: AppHandle, core: Arc<AppCore>, click_through: bool) {
    tauri::async_runtime::spawn(async move {
        let result = if click_through {
            core.toggle_pet_click_through(&app).await
        } else {
            core.toggle_pet_visibility(&app).await
        };
        if let Err(error) = result {
            eprintln!(
                "desktop action failed [{}]: {}",
                error.diagnostic_id, error.message
            );
        }
    });
}

pub fn setup_from_state(app: &mut App) -> tauri::Result<()> {
    let core = Arc::clone(&app.state::<CoreState>().0);
    setup(app, core)
}

fn desktop_error(context: &str, error: tauri::Error) -> AppError {
    AppError::new("desktop_error", format!("{context}：{error}"), true)
}
