//! tun2proxy 的跨平台桌面 GUI。
//!
//! # 目录结构
//!
//! ```text
//! src/
//! ├─ lib.rs        根组件 `App`（nsh 的 `#[native_module]` + `#[component]`）
//! ├─ main.rs       启动流程：权限 → 配置 → 引擎 → 托盘 → 窗口
//! ├─ core/         与界面无关的内核（不依赖任何 UI 框架，可单独测试）
//! │   ├─ paths.rs   配置 / 下载 / 日志的存放位置
//! │   ├─ config.rs  用户配置的读写、校验、转成命令行参数
//! │   ├─ binary.rs  下载 / 校验 / 解压 tun2proxy 官方二进制
//! │   ├─ engine.rs  拉起子进程、收日志、维护状态与更新进度
//! │   ├─ elevate.rs 管理员 / root 权限检测与 Windows 自动提权
//! │   └─ ...
//! └─ ui/           界面层（nsh）：主题 token、样式化控件、各页
//!     ├─ page.rs    导航页枚举（名字与说明）
//!     ├─ shell.rs   整窗骨架：侧边栏 + 内容区
//!     └─ ...
//! ```
//!
//! 这一版是从 GPUI + gpuikit 迁到 Native Shell（nsh）的开始：内核一行未改，
//! 界面按 nsh 的「无样式基础组件 + 应用自己画的 ui 层」重建。

pub mod core;
pub mod ui;

use nsh_base::theme;
use nsh_core::prelude::*;

/// 本 crate 对外暴露给 TSX 的那一套（`nsh.json` 的 `"rust"`）。
///
/// 目前界面全在 Rust 里，这里只为将来的 TSX 入口留一个注册点。
#[native_module("tun2proxy/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// 应用根组件。
///
/// 只做一件事：把整窗骨架铺满窗口，其余交给 [`ui`]。这里是**第一帧**，
/// 也是状态树的所有者——`ui::shell` 里的信号都挂在这个 owner 下。
#[component]
pub fn App() -> impl IntoElement {
    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(ui::shell::shell())
}
