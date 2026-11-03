#![cfg_attr(windows, windows_subsystem = "windows")]
//! tun2proxy 的跨平台桌面 GUI。
//!
//! # 目录结构
//!
//! ```text
//! src/
//! ├─ main.rs      启动流程：权限 → 配置 → 引擎 → 托盘 → 窗口
//! ├─ core/        与界面无关的内核（不依赖 GPUI，可单独测试）
//! │   ├─ mod.rs     模块说明与再导出
//! │   ├─ paths.rs   配置 / 下载 / 日志的存放位置
//! │   ├─ config.rs  用户配置的读写、校验、转成命令行参数
//! │   ├─ binary.rs  下载 / 校验 / 解压 tun2proxy 官方二进制
//! │   ├─ engine.rs  拉起子进程、收日志、维护状态与更新进度
//! │   ├─ elevate.rs 管理员 / root 权限检测与 Windows 自动提权
//! │   ├─ watchdog.rs 界面线程卡顿探针（假死时记录卡在哪一段）
//! │   └─ platform.rs 进程级初始化（Windows DPI 感知）
//! └─ ui/          界面层（GPUI + gpuikit）
//!     ├─ state.rs   应用状态与消息枚举（`App` 同时是根视图实体）
//!     ├─ view.rs    `impl Render for App`
//!     ├─ theme.rs   gpuikit 主题的装载与换肤
//!     ├─ sidebar.rs 左侧边栏
//!     ├─ pages.rs   右侧内容区
//!     ├─ cards.rs   各张设置卡片
//!     ├─ chart.rs   流量折线图
//!     ├─ widgets.rs 可复用的文字样式与小部件
//!     └─ update.rs  消息处理
//! ```
//!
//! 启动流程：
//!
//! 1. 检查权限（Windows 上没有管理员权限时会询问并 UAC 重新拉起自己）；
//! 2. 读取配置，创建内核引擎；
//! 3. 装托盘图标（装不上不影响主流程）；
//! 4. 建 GPUI 应用：装组件库、装主题、开窗、挂关窗拦截；
//! 5. 事件循环结束后确保 tun2proxy 子进程已退出。
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

use gpui::{AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_platform::current_platform;

use crate::core::{Config, Engine, elevate};

/// 窗口标题。
///
/// Windows 上它有两个用途，都不能省：
///
/// - 任务栏 / Alt-Tab 里显示的名字；
/// - 托盘「显示窗口」靠 `core::window::find_main()` 按「本进程里**有标题**的
///   顶层窗口」反查主窗口，没有标题就找不到（点了没反应）。
const WINDOW_TITLE: &str = "Tun2Proxy GUI";

fn main() {
    // 0. 卡顿探针：之后界面线程无论卡在哪一段，都会写进 stall.log。
    core::watchdog::install();

    // 1. 权限：不足时可能直接重新拉起自己（Windows 会弹 UAC 对话框）。
    elevate::ensure_elevated();

    // 2. 声明 DPI 感知（必须在建窗口之前）。
    core::platform::init();

    // 3. 配置 + 引擎。Engine::new 会探测已安装的 tun2proxy 版本。
    let config = Config::load();
    // 语言要在建任何界面元素之前定下来：视图是「读全局语言」渲染的，
    // 先建窗口再切语言会闪一下另一门语言。
    core::i18n::apply(config.language);
    // 主题同理：开窗之前就要装好，否则第一帧是默认配色，闪一下才变色。
    let theme_setting = config.theme;
    let engine = Engine::new();
    let shared = engine.shared();

    // 4. 系统托盘：装不上（非 Windows / 资源管理器拒绝）不影响主流程。
    let _ = core::tray::install(engine.clone());

    // 5. GPUI 应用。`with_assets` 把 gpuikit 的图标 / 字体资源挂进应用，
    //    没有它所有 `Svg` 都会画成空白方块。
    let window_engine = engine.clone();
    Application::with_platform(current_platform(false))
        .with_assets(gpuikit::assets())
        .run(move |cx| {
            // 组件库初始化（主题、输入键位、元素管理器）必须先于任何窗口。
            gpuikit::init(cx);
            // 再按「环境变量 > 设置项 > 系统」装主题。整套配色与控件尺度
            // 都由组件库决定，这里只挑明暗并做层次修正。
            ui::theme::install(cx, theme_setting);

            let window_size = size(px(900.0), px(650.0));
            let options = WindowOptions {
                // 居中：多显示器时落在主显示器中央，而不是左上角。
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    window_size,
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some(WINDOW_TITLE.into()),
                    // 用系统原生标题栏：最小化 / 最大化 / 关闭交给 Windows，
                    // 我们只在「关闭」这一路上插一道确认。
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(
                    px(ui::MIN_WINDOW_WIDTH),
                    px(ui::MIN_WINDOW_HEIGHT),
                )),
                ..Default::default()
            };

            let engine = window_engine.clone();
            let shared = shared.clone();
            cx.open_window(options, move |window, cx| {
                // 窗口标题由 `titlebar.title` 决定（Windows 上 gpui 直接把它
                // 作为 `CreateWindowExW` 的窗口名），托盘「显示窗口」全指着它。
                let app = cx.new(|cx| ui::App::new(config, engine, shared, window, cx));
                // 全局句柄 + 关窗拦截（拦截里要判断托盘「退出」是否已确认）。
                ui::view::install_window_hooks(app.clone(), window, cx);
                app
            })
            .expect("failed to open the main window");

            cx.activate(true);
        });

    // 6. 退出前确保子进程结束，避免残留 TUN 网卡和路由。
    engine.shutdown_blocking();
}
