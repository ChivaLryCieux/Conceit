//! Android 原生桥。
//!
//! 背景：`tauri-plugin-dialog` 在 Android 上有两个硬伤——
//! 1. **没有目录选择器**：JS 传 `directory: true` 时，插件 Rust 端直接返回
//!    `FolderPickerNotImplemented`（前端表现为「打开选择对话框失败」）；
//! 2. **文件选择器返回的不是路径**：Android 端 `createPickFilesResult` 返回的是
//!    `content://` URI 字符串，`std::fs` 无法读取（前端表现为「路径不存在」）。
//!
//! 解决办法：应用内置一个 Kotlin 插件（`src-tauri/android/ConceitPlugin.kt`，
//! 构建时由 `scripts/build-android.mjs` 同步进 `gen/android`），用系统文档选择器
//! （SAF）完成「选择 → 复制到应用缓存目录 → 返回真实路径」。之后
//! `import_sources` 按普通文件夹 / zip 处理即可，书库与源目录保持独立。
//!
//! 命令统一注册在应用的 invoke_handler 里（不占用 `plugin:` 命名空间），
//! 桌面端由 [stub] 提供同签名的错误返回——前端只在 Android 上调用它们。

use serde::{Deserialize, Serialize};

/// Kotlin `systemBarInsets` 的响应：系统栏高度（px）
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct InsetsResponse {
    /// 状态栏高度
    pub top: u32,
    /// 导航栏高度
    pub bottom: u32,
}

#[cfg(target_os = "android")]
mod android;

#[cfg(not(target_os = "android"))]
mod stub;

// 注意：必须用 glob 再导出。`#[tauri::command]` 除了函数本身还会生成
// `__cmd__*` / `__tauri_command_name_*` 两个宏并通过 `pub use` 与原函数同路径
// 导出，`generate_handler!` 引用的是这些宏；按名再导出函数会漏掉它们。
#[cfg(target_os = "android")]
pub use android::*;

#[cfg(not(target_os = "android"))]
pub use stub::*;
