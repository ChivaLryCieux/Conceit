//! Android 原生桥的实现。
//!
//! 通过 tauri 插件的 setup 钩子把 Kotlin 端 `ConceitPlugin` 注册进
//! PluginManager，句柄存入应用状态；下面的命令再把调用转给它。
//! 注册失败（例如 Kotlin 类没被打进包）只记日志不中断启动——
//! 此时命令会因状态缺失而报错，导入功能不可用但应用仍可正常使用。

use serde::Deserialize;
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime, State, Wry,
};

use super::InsetsResponse;

/// Kotlin 插件在 PluginManager 中的注册名（前后端必须一致）
pub const PLUGIN_NAME: &str = "conceit-android";
/// Kotlin 类所在的包名（`register_android_plugin` 用它拼 JNI 类路径）
const PLUGIN_IDENTIFIER: &str = "com.conceit.mangareader";
const PLUGIN_CLASS: &str = "ConceitPlugin";

/// 保存在 Tauri 状态里的 Kotlin 插件句柄
pub struct ImportHandle<R: Runtime>(PluginHandle<R>);

/// 注册插件；在 `lib.rs` 里以 `.plugin(mobile_import::init())` 接入
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new(PLUGIN_NAME)
        .setup(|app, api| {
            // 注册失败（例如 Kotlin 类没被打进包）只记日志不中断启动：
            // 此时命令会因状态缺失而报错，导入不可用但应用仍能正常使用
            match api.register_android_plugin(PLUGIN_IDENTIFIER, PLUGIN_CLASS) {
                Ok(handle) => {
                    app.manage(ImportHandle(handle));
                }
                Err(e) => eprintln!("注册 Android 导入插件失败（导入功能不可用）: {e}"),
            }
            Ok(())
        })
        .build()
}

/// Kotlin `pickDirectory` 的响应：缺失 path 字段 = 用户取消
#[derive(Deserialize, Default)]
struct PathResponse {
    #[serde(default)]
    path: Option<String>,
}

/// Kotlin `pickZipFiles` 的响应：空列表 = 用户取消
#[derive(Deserialize, Default)]
struct PathsResponse {
    #[serde(default)]
    paths: Vec<String>,
}

/// 打开系统目录选择器，返回缓存中的真实目录路径；取消返回 `None`
/// （运行时固定为 Wry：泛型命令无法在 app 级 generate_handler! 里推断 R）。
///
/// `channel` 透传给 Kotlin 侧：SAF 把整棵树复制进缓存的过程往往比之后的
/// 书库复制更久，进度也由它经同一 Channel 回报（见 ImportProgress）。
#[tauri::command]
pub async fn pick_directory(
    handle: State<'_, ImportHandle<Wry>>,
    channel: tauri::ipc::Channel,
) -> Result<Option<String>, String> {
    let res: PathResponse = handle
        .0
        .run_mobile_plugin_async(
            "pickDirectory",
            serde_json::json!({ "channel": channel }),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.path)
}

/// 打开系统文件选择器（zip，可多选），返回缓存中的真实文件路径；取消返回空列表
#[tauri::command]
pub async fn pick_zip_files(
    handle: State<'_, ImportHandle<Wry>>,
    channel: tauri::ipc::Channel,
) -> Result<Vec<String>, String> {
    let res: PathsResponse = handle
        .0
        .run_mobile_plugin_async("pickZipFiles", serde_json::json!({ "channel": channel }))
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.paths)
}

/// 打开系统图片选择器（可多选），返回缓存中的真实文件路径；取消返回空列表。
/// 给设置页「增加图片」用，选完由 `add_manga_pages` 复制进书库
#[tauri::command]
pub async fn pick_images(
    handle: State<'_, ImportHandle<Wry>>,
) -> Result<Vec<String>, String> {
    let res: PathsResponse = handle
        .0
        .run_mobile_plugin_async("pickImages", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.paths)
}

/// 清理导入桥复制进缓存的临时文件（`picked-import` / `tree-import` / `picked-images`）
#[tauri::command]
pub async fn clear_import_cache(
    handle: State<'_, ImportHandle<Wry>>,
) -> Result<(), String> {
    let _: serde_json::Value = handle
        .0
        .run_mobile_plugin_async("clearImportCache", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 退出应用（安卓系统返回在书架根目录时由前端调用；Kotlin 侧 finish Activity）
#[tauri::command]
pub async fn exit_app(handle: State<'_, ImportHandle<Wry>>) -> Result<(), String> {
    let _: serde_json::Value = handle
        .0
        .run_mobile_plugin_async("exitApp", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 查询系统栏高度（px）：状态栏 top / 导航栏 bottom，供顶栏与底栏避让
#[tauri::command]
pub async fn system_bar_insets(
    handle: State<'_, ImportHandle<Wry>>,
) -> Result<InsetsResponse, String> {
    handle
        .0
        .run_mobile_plugin_async("systemBarInsets", serde_json::json!({}))
        .await
        .map_err(|e| e.to_string())
}
