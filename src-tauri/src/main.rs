// Conceit 漫画阅读器入口。
// 桌面平台使用默认窗口装饰；Windows 发布版本下不弹控制台。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    conceit_lib::run()
}
