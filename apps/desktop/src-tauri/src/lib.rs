mod audio;
mod commands;
mod contracts;
mod core;
mod desktop;
mod notifications;
mod provider;
mod secret;
mod storage;
mod tools;

use std::{fs, path::Path};

use commands::CoreState;
use core::AppCore;
use storage::Storage;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_local_environment();
    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts([
            "CommandOrControl+Shift+Space",
            "CommandOrControl+Shift+L",
            "CommandOrControl+Shift+M",
        ])
        .expect("valid companion global shortcuts")
        .with_handler(|app, shortcut, event| {
            let primary = if cfg!(target_os = "macos") {
                Modifiers::SUPER
            } else {
                Modifiers::CONTROL
            };
            if shortcut.matches(primary | Modifiers::SHIFT, Code::Space)
                && event.state == ShortcutState::Pressed
            {
                let _ = desktop::toggle_main_window(app);
            } else if shortcut.matches(primary | Modifiers::SHIFT, Code::KeyL)
                && event.state == ShortcutState::Pressed
            {
                let core = std::sync::Arc::clone(&app.state::<CoreState>().0);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if core.toggle_listening(&app).await.is_err() {
                        let _ = desktop::show_main_window(&app);
                    }
                });
            } else if shortcut.matches(primary | Modifiers::SHIFT, Code::KeyM) {
                let core = std::sync::Arc::clone(&app.state::<CoreState>().0);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if event.state == ShortcutState::Pressed {
                        let _ = core.start_push_to_talk(&app).await;
                    } else {
                        let _ = core.finish_push_to_talk(&app).await;
                    }
                });
            }
        })
        .build();
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(shortcut_plugin)
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Storage::open(&data_dir.join("companion.sqlite3"))?;
            let core = AppCore::new(storage)?;
            core.start_timer_scheduler(app.handle().clone());
            app.manage(CoreState(core));
            desktop::setup_from_state(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_get_snapshot,
            commands::chat_submit_text,
            commands::turn_cancel,
            commands::tool_decide,
            commands::timer_cancel,
            commands::screen_get_observation_preview,
            commands::screen_clear_observation,
            commands::screen_set_understanding_mode,
            commands::settings_update_provider,
            commands::settings_test_provider,
            commands::pet_open_panel,
            commands::pet_update_settings,
            commands::character_import_vrm,
            commands::memory_search,
            commands::memory_list_candidates,
            commands::memory_get_source,
            commands::memory_export,
            commands::memory_get_settings,
            commands::memory_update_settings,
            commands::memory_review_candidate,
            commands::memory_update_candidate,
            commands::memory_create,
            commands::memory_update,
            commands::memory_delete,
            commands::data_get_summary,
            commands::data_clear_conversation,
            commands::data_clear_memories,
            commands::voice_request_permissions,
            commands::voice_open_privacy_settings,
            commands::voice_set_listening_mode,
            commands::voice_ptt_start,
            commands::voice_ptt_finish,
            commands::playback_stop,
            commands::voice_update_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running desktop AI companion");
}

#[cfg(debug_assertions)]
fn load_local_environment() {
    let manifest_directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    for directory in manifest_directory.ancestors() {
        let environment_path = directory.join(".env");
        if environment_path.is_file() {
            let _ = dotenvy::from_path(environment_path);
            break;
        }
    }
}

#[cfg(not(debug_assertions))]
fn load_local_environment() {}
