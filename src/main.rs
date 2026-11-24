#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
//! 启动流程：单实例 → 权限 → 配置 → 引擎 → 托盘 → 窗口。
//!
//! `cargo run` / 发布版 exe 都从这里进；窗口和绘制交给 nsh 的平台层
//! （Windows 上是 Win32 + Direct2D），界面本身在 `lib.rs` 的根组件里。
//!
//! 顺序与 GPUI 时代完全一致，只有「建 GPUI 应用」那一步换成了
//! `platform::run_app` + 一个带主题 token 的 `Root`：
//!
//! 0. 单实例：已经有一个在跑，就把它的窗口叫到前台，自己直接退出；
//! 1. 检查权限（Windows 上没有管理员权限时会询问并 UAC 重新拉起自己）；
//! 2. 读取配置，创建内核引擎；
//! 3. 装托盘图标（装不上不影响主流程）；
//! 4. 开窗口跑根组件；
//! 5. 事件循环结束后确保 tun2proxy 子进程已退出。
//!
//! # 为什么是 `windows_subsystem = "windows"`
//!
//! 不加这行，Windows 会把程序当成控制台程序：双击启动时先弹一个黑窗口，
//! 并且它会在整个运行期间挂在后面。子进程 `tun2proxy` 的 `CREATE_NO_WINDOW`
//! 在 `core/engine.rs` 与 `core/binary.rs` 里各自带着。

use tun2proxy_gui::core::{Config, Engine, elevate};
use tun2proxy_gui::ui;

#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

/// 窗口标题。
///
/// Windows 上它有两个用途，都不能省：
///
/// - 任务栏 / Alt-Tab 里显示的名字；
/// - 托盘「显示窗口」靠 `core::window::find_main()` 按「本进程里**有标题**的
///   顶层窗口」反查主窗口，没有标题就找不到（点了没反应）。
const WINDOW_TITLE: &str = "Tun2Proxy GUI";

/// 应用 id：nsh 用它给窗口、托盘和数据目录定位。
const APP_ID: &str = "com.hkmv.tun2proxy-gui";

fn main() {
    // 0. 卡顿探针：之后界面线程无论卡在哪一段，都会写进 stall.log。
    tun2proxy_gui::core::watchdog::install();

    // 0.5 单实例：已经开着一个就请它自己把窗口显示出来，本进程直接退出。
    // 放在提权**之前**：第二次双击不该再弹一次 UAC。
    if tun2proxy_gui::core::single_instance::request_show() {
        return;
    }

    // 1. 权限：不足时可能直接重新拉起自己（Windows 会弹 UAC 对话框）。
    elevate::ensure_elevated();

    // 1.5 认领「本会话唯一实例」。
    if !tun2proxy_gui::core::single_instance::claim() {
        if !tun2proxy_gui::core::single_instance::request_show() {
            tun2proxy_gui::core::single_instance::focus_window(WINDOW_TITLE);
        }
        return;
    }
    tun2proxy_gui::core::single_instance::create_show_event();

    // 2. 声明 DPI 感知（必须在建窗口之前）。
    tun2proxy_gui::core::platform::init();

    // 3. 配置 + 引擎。Engine::new 会探测已安装的 tun2proxy 版本。
    let config = Config::load();
    // 语言要在建任何界面元素之前定下来。
    tun2proxy_gui::core::i18n::apply(config.language);
    let engine = Engine::new();

    // 3.5 上一次被强杀留下的隧道网卡：开界面之前先清掉。
    engine.clean_stale_adapters();

    // 4. 系统托盘：装不上（非 Windows / 资源管理器拒绝）不影响主流程。
    let _ = tun2proxy_gui::core::tray::install(engine.clone());

    // 界面层要读配置与引擎，交给 `ui::state` 保管。
    ui::state::install(config.clone(), engine.clone());

    // 5. 开窗口跑根组件，直到用户退出。
    //
    // 主题在这里装进 `Root`：`nsh run` 读 `nsh.json` 的 `"theme"`（默认那套
    // 配色），而 `cargo run`（以及发布版 exe）走的是这一条，所以要自己按配置
    // 挑一套 `themes/*.json` 解析进来，否则界面是 nsh 的默认黑白主题。
    // 换配色只换 token，明暗（浅色 / 深色 / 跟随系统）由 `.appearance()` 单独
    // 决定；两者在运行中都能从界面上改，见 `ui::palette`。
    let palette = ui::palette::initial(config.palette);
    let root = platform::Root::of::<tun2proxy_gui::App>()
        .tokens(ui::palette::tokens(palette))
        .appearance(ui::palette::appearance(ui::palette::initial_setting(
            config.theme,
        )));
    let _ = platform::run_app(
        platform::AppSource::Rust(root),
        platform::AppOptions {
            title: WINDOW_TITLE.to_owned(),
            app_id: APP_ID.to_owned(),
            size: (900, 650),
            ..Default::default()
        },
    );

    // 6. 退出前确保子进程结束，避免残留 TUN 网卡和路由。
    engine.shutdown_blocking();
}
