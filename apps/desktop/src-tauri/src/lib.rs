mod commands;
mod contracts;
mod core;
mod provider;
mod secret;
mod storage;

use std::fs;

use commands::CoreState;
use core::AppCore;
use storage::Storage;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&data_dir)?;
            let storage = Storage::open(&data_dir.join("companion.sqlite3"))?;
            let core = AppCore::new(storage)?;
            app.manage(CoreState(core));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_get_snapshot,
            commands::chat_submit_text,
            commands::turn_cancel,
            commands::settings_update_provider,
            commands::pet_open_panel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running desktop AI companion");
}
