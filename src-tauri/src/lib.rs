// Conceit 漫画阅读器 — Tauri 应用库入口。

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用时出错");
}
