//! Conceit 漫画阅读器 — Tauri 应用库入口。

mod commands;
pub mod db;
mod library;
mod mobile_import;

use std::sync::Mutex;
use tauri::{Emitter, Manager};

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

            // 启动时后台补生成缺失的封面缩略图（历史数据、手动删过的文件）。
            // 解码原图较慢，放后台线程做；完成后通知前端刷新，书架立即用上缩略图。
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let state = handle.state::<Mutex<commands::AppState>>();
                match commands::backfill_thumbnails(&state) {
                    Ok(0) => {}
                    Ok(n) => {
                        let _ = handle.emit("library:thumbnails-ready", n);
                    }
                    Err(e) => eprintln!("补生成封面缩略图失败: {e}"),
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        // Android 导入桥（SAF 目录 / zip 选择）；桌面端为空插件
        .plugin(mobile_import::init())
        .invoke_handler(tauri::generate_handler![
            commands::library_snapshot,
            commands::create_folder,
            commands::create_empty_manga,
            commands::rename_item,
            commands::delete_items,
            commands::move_item,
            commands::reorder,
            commands::save_progress,
            commands::library_health,
            commands::library_health_clean,
            commands::adopt_manga_dir,
            commands::rescan_manga_pages,
            commands::manga_pages,
            commands::import_sources,
            commands::update_item_settings,
            commands::add_manga_pages,
            commands::delete_manga_pages,
            commands::reorder_manga_pages,
            commands::app_platform,
            mobile_import::pick_directory,
            mobile_import::pick_zip_files,
            mobile_import::pick_images,
            mobile_import::clear_import_cache,
            mobile_import::exit_app,
            mobile_import::system_bar_insets,
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用时出错");
}
