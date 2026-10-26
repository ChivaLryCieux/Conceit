//! 桌面端存根：导入桥的命令只在 Android 上有意义（见 [super] 的模块文档）。
//! 前端按平台分流，正常不会调用到这里；留一份同签名实现，
//! 既让 `invoke_handler` 的注册与平台无关，也在误用时给出可读的错误。

use super::InsetsResponse;
use tauri::plugin::{Builder, TauriPlugin};

/// 与 Android 端一致的空插件（无 setup 行为）；名字仅用于标识，无实际作用
pub fn init<R: tauri::Runtime>() -> TauriPlugin<R> {
    Builder::new("conceit-android").build()
}

/// `pick_directory` 的桌面存根
#[tauri::command]
pub async fn pick_directory(_channel: tauri::ipc::Channel) -> Result<Option<String>, String> {
    Err("目录选择器仅在 Android 上可用".to_string())
}

/// `pick_zip_files` 的桌面存根
#[tauri::command]
pub async fn pick_zip_files(_channel: tauri::ipc::Channel) -> Result<Vec<String>, String> {
    Err("zip 选择器仅在 Android 上可用".to_string())
}

/// `pick_images` 的桌面存根（设置页「增加图片」在桌面走插件对话框，不会调到这里）
#[tauri::command]
pub async fn pick_images() -> Result<Vec<String>, String> {
    Err("图片选择器仅在 Android 上可用".to_string())
}

/// `clear_import_cache` 的桌面存根（无缓存可清，直接成功）
#[tauri::command]
pub async fn clear_import_cache() -> Result<(), String> {
    Ok(())
}

/// `exit_app` 的桌面存根（系统返回手势仅安卓有；正常不会触达）
#[tauri::command]
pub async fn exit_app() -> Result<(), String> {
    Err("退出命令仅在 Android 上可用".to_string())
}

/// `system_bar_insets` 的桌面存根（桌面无系统栏遮挡，前端回退 env()）
#[tauri::command]
pub async fn system_bar_insets() -> Result<InsetsResponse, String> {
    Err("系统栏高度查询仅在 Android 上可用".to_string())
}

/// `set_status_bar_style` 的桌面存根：桌面窗口没有系统栏图标配色问题，
/// 前端按平台分流，正常不会调到这里
#[tauri::command]
pub async fn set_status_bar_style(_light: bool) -> Result<(), String> {
    Err("状态栏配色设置仅在 Android 上可用".to_string())
}

/// `save_to_downloads` 的桌面存根：桌面端由
/// [`crate::commands::save_to_download_dir`] 直接写 `download_dir()`，
/// 走不到这里；留一份同签名实现只为让 `mobile_import::*` 的再导出
/// 在两个平台上形状一致。
#[allow(dead_code)]
pub async fn save_to_downloads(
    _app: &tauri::AppHandle,
    _zip_path: &std::path::Path,
    _file_name: &str,
    _channel: &tauri::ipc::Channel,
) -> Result<String, String> {
    Err("写入下载目录的原生桥仅在 Android 上可用".to_string())
}
