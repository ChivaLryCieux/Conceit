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
pub async fn pick_directory() -> Result<Option<String>, String> {
    Err("目录选择器仅在 Android 上可用".to_string())
}

/// `pick_zip_files` 的桌面存根
#[tauri::command]
pub async fn pick_zip_files() -> Result<Vec<String>, String> {
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
