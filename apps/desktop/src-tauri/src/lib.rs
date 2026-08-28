mod commands;
mod contracts;
mod core;
mod desktop;
mod provider;
mod secret;
mod storage;

use std::fs;

use commands::CoreState;
use core::AppCore;
use storage::Storage;
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["CommandOrControl+Shift+Space"])
        .expect("valid companion global shortcuts")
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let _ = desktop::toggle_main_window(app);
            }
        })
        .build();
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(shortcut_plugin)
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Storage::open(&data_dir.join("companion.sqlite3"))?;
            let core = AppCore::new(storage)?;
            app.manage(CoreState(core));
            desktop::setup_from_state(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_get_snapshot,
            commands::chat_submit_text,
            commands::turn_cancel,
            commands::settings_update_provider,
            commands::pet_open_panel,
            commands::pet_update_settings,
            commands::character_import_vrm,
            commands::memory_search,
            commands::memory_create,
            commands::memory_update,
            commands::memory_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running desktop AI companion");
}
