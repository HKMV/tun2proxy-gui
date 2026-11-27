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
// prelude 里也有个 `window()`（返回的是窗口尺寸 `Size`），名字撞了：
// 显式导入元素版，声明窗口的那个才是这里要用的。
use nsh_core::element::desktop::window;

/// 本 crate 对外暴露给 TSX 的那一套（`nsh.json` 的 `"rust"`）。
///
/// 目前界面全在 Rust 里，这里只为将来的 TSX 入口留一个注册点。
#[native_module("tun2proxy/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// 窗口标题。
///
/// Windows 上它有两个用途，都不能省：
///
/// - 任务栏 / Alt-Tab 里显示的名字；
/// - 系统右上角那三个按钮与「最小化到任务栏」都依附在一个有标题的顶层窗口上。
///
/// nsh 0.7 起窗口模型是 v2：宿主窗口永不显示，窗口由 app 自己声明，
/// 标题也就跟着窗口一起声明（`main.rs` 只用它填宿主窗口的选项）。
pub const WINDOW_TITLE: &str = "Tun2Proxy GUI";

/// 应用根组件。
///
/// nsh 0.7 的窗口模型 v2 里**宿主窗口永不显示**：程序要自己声明窗口
/// （`window()`），界面挂在它的 content 里。少这一步，窗口会建出来但一直
/// 是隐藏的（`main_window().show()` 在 v2 里是空操作）。
///
/// 声明出来的窗口就是 `current_window()`，所以关窗按钮、最小化到托盘、
/// 托盘点击回前台、单实例唤出这些原来写在 `ui::shell` 里的逻辑一行不用改。
#[component]
pub fn App() -> impl IntoElement {
    window()
        .id("main")
        .title(WINDOW_TITLE)
        .default_size(size(px(900.), px(650.)))
        .content(|| {
            view()
                .flex_1()
                .bg(move || theme().background)
                .text_color(move || theme().foreground)
                .child(ui::shell::shell())
        })
}
