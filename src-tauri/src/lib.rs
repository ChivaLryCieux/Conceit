//! Conceit 漫画阅读器 — Tauri 应用库入口。

mod commands;
mod library;

use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // 书库放在 <app_data_dir>/library，随应用自维护
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("无法定位应用数据目录: {e}"))?;
            let root = data_dir.join("library");
            let state = commands::AppState::new(root)?;
            app.manage(Mutex::new(state));
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::library_snapshot,
            commands::create_folder,
            commands::rename_item,
            commands::delete_items,
            commands::move_item,
            commands::reorder,
            commands::save_progress,
            commands::manga_pages,
            commands::manga_cover,
            commands::suggest_title,
            commands::import_sources,
            commands::update_item_settings,
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用时出错");
}
