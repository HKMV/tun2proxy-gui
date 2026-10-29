#![cfg_attr(windows, windows_subsystem = "windows")]
//! tun2proxy 的跨平台桌面 GUI。
//!
//! # 目录结构
//!
//! ```text
//! src/
//! ├─ main.rs      启动流程：权限 → DPI → 配置 → 引擎 → 窗口
//! ├─ core/        与界面无关的内核（不依赖 zsui，可单独测试）
//! │   ├─ mod.rs     模块说明与再导出
//! │   ├─ paths.rs   配置 / 下载 / 日志的存放位置
//! │   ├─ config.rs  用户配置的读写、校验、转成命令行参数
//! │   ├─ binary.rs  下载 / 校验 / 解压 tun2proxy 官方二进制
//! │   ├─ engine.rs  拉起子进程、收日志、维护状态与更新进度
//! │   ├─ elevate.rs 管理员 / root 权限检测与 Windows 自动提权
//! │   ├─ watchdog.rs 界面线程卡顿探针（假死时记录卡在哪一段）
//! │   └─ platform.rs 进程级初始化（Windows DPI 感知）
//! └─ ui/          界面层（zsui）
//!     ├─ state.rs   应用状态与消息枚举
//!     ├─ view.rs    整棵视图树
//!     ├─ cards.rs   各张设置卡片
//!     ├─ widgets.rs 可复用的文字样式与小部件
//!     └─ update.rs  消息处理
//! ```
//!
//! 启动流程：
//!
//! 1. 检查权限（Windows 上没有管理员权限时会询问并 UAC 重新拉起自己）；
//! 2. 声明 DPI 感知，拿到显示器缩放比例；
//! 3. 读取配置，创建内核引擎；
//! 4. 把「请求重绘」的回调交给引擎，后台线程改状态时界面会自动刷新；
//! 5. 打开窗口并进入事件循环，窗口关闭后确保 tun2proxy 子进程已退出。
//!
//! # 为什么是 `windows_subsystem = "windows"`
//!
//! 不加这行，Windows 会把程序当成控制台程序：双击启动时先弹一个黑窗口，
//! 并且它会在整个运行期间挂在后面。加上之后就没有控制台，代价是我们自己的
//! `println!` / `eprintln!` 也看不见了——调试信息走界面里的运行日志
//! （内核已经把 tun2proxy 子进程的输出收集进去了）。
//!
//! 子进程不会跟着变：`tun2proxy` 本身是控制台程序，如果不管，从 GUI 里拉
//! 起来同样会闪一个黑窗口，所以 `core/engine.rs` 与 `core/binary.rs` 都带了
//! `CREATE_NO_WINDOW`。

mod core;
mod ui;

use std::sync::Arc;

use crate::core::{elevate, platform, Config, Engine};
use zsui::{native_window, UiInvalidationHandle};

use ui::App;

/// 标题栏的叉被按下时发出的应用命令。
const CLOSE_COMMAND: &str = "close-window";

fn main() {
    // 0. 卡顿探针：之后界面线程无论卡在哪一段，都会写进 stall.log。
    core::watchdog::install();

    // 1. 权限：不足时可能直接重新拉起自己（Windows 会弹 UAC 对话框）。
    elevate::ensure_elevated();

    // 2. 声明 DPI 感知并拿到缩放比例（必须在建窗口之前）。
    let scale = platform::init();
    let logical_size = |w: f32, h: f32| ((w * scale) as u32, (h * scale) as u32);

    // 3. 配置 + 引擎。Engine::new 会探测已安装的 tun2proxy 版本。
    let config = Config::load();
    // 语言要在建任何界面元素之前定下来：视图是「读全局语言」渲染的，
    // 先建窗口再切语言会闪一下另一门语言。
    core::i18n::apply(config.language);
    let engine = Engine::new();

    // 4. 引擎 → 界面的重绘通道。core 只认「回调」，不认 zsui。
    let invalidation = UiInvalidationHandle::new();
    let handle = invalidation.clone();
    engine.set_rebuild_hook(Arc::new(move || {
        handle.request_rebuild();
    }));

    // 5. 启动时静默检查一次更新（只查询，不下载）。
    //
    // 延后 0.8 秒再查：窗口刚建好时「请求重绘」还不生效，过早拿到的结果
    // 会一直看不见，直到用户随手点一下界面。顺便在查之前先唤醒一次界面，
    // 把这段时间里攒下的状态（版本号等）画出来。
    if config.auto_check_update {
        let engine = engine.clone();
        let repo = config.release_repo.clone();
        let wake = invalidation.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            wake.request_rebuild();
            engine.check_update(repo);
        });
    }

    let app = App::new(config, engine.clone(), engine.shared());

    // 系统托盘：装不上（非 Windows / 资源管理器拒绝）不影响主流程。
    let _ = core::tray::install(engine.clone());

    let (width, height) = logical_size(900.0, 650.0);
    let (min_width, min_height) = logical_size(ui::MIN_WINDOW_WIDTH, ui::MIN_WINDOW_HEIGHT);
    let mut window = native_window("Tun2Proxy GUI")
        .size(width, height)
        .min_size(min_width, min_height)
        .invalidation_handle(invalidation);

    // 标题栏 / 任务栏图标。exe 资源里的那个图标负责资源管理器与任务栏，
    // 这里再显式设一次：非 Windows 平台没有 exe 资源，只能走这条路。
    if let Some(icon) = core::paths::install_icon() {
        window = window.icon_path(icon.to_string_lossy().into_owned());
    }

    // 关窗走一道确认：tun2proxy 是常驻服务，点叉直接把隧道停了很容易误操作。
    // `on_close_requested` 把原生 WM_CLOSE 转成一条应用命令，命令映射成
    // `Msg::CloseRequested`；update 里不调 `cx.quit()` 就表示「否决关闭」，
    // 于是窗口继续活着（可以只把它藏进托盘）。
    let result = window
        .stateful_view_with_app_commands(app, ui::view, ui::update, |command| match command {
            zsui::Command::Custom { id, .. } if id == CLOSE_COMMAND => {
                Some(ui::Msg::CloseRequested)
            }
            _ => None,
        })
        .on_close_requested(zsui::Command::custom(CLOSE_COMMAND))
        .run();

    // 6. 退出前确保子进程结束，避免残留 TUN 网卡和路由。
    engine.shutdown_blocking();

    if let Err(err) = result {
        eprintln!(
            "{}",
            tr_format!("窗口运行失败：{err}", "The window failed: {err}")
        );
        std::process::exit(1);
    }
}
