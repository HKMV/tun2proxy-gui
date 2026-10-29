//! 系统托盘图标。
//!
//! 托盘解决的是一个具体问题：tun2proxy 是常驻后台的服务，窗口关掉之后
//! 「现在到底跑没跑」只能靠重新打开窗口确认。有了托盘图标，状态一眼可见，
//! 启停一步可达。
//!
//! # 实现方式
//!
//! ZSUI 自己管着主窗口的窗口过程，我们拿不到它的 `HWND` 回调，所以托盘
//! **不挂在主窗口上**：这里单开一个后台线程，注册一个从不显示的窗口类，
//! 用它接托盘消息（`Shell_NotifyIconW` 只要求一个能收消息的窗口）。
//!
//! 主窗口句柄用 `FindWindowW` 按标题找——标题是 `main.rs` 传给
//! `native_window()` 的那一个，进程内唯一。
//!
//! # 托盘上的动作
//!
//! | 动作 | 行为 |
//! |------|------|
//! | 双击图标 | 显示并前置主窗口 |
//! | 右键菜单 · 启动 | 用**磁盘上保存的配置**拉起 tun2proxy |
//! | 右键菜单 · 停止 | 与界面上的「停止」同一条路径（先中断，6 秒后强杀） |
//! | 右键菜单 · 退出 | 停服务 → 关窗口 → 进程退出 |
//!
//! 「启动」读磁盘上的配置而不是界面里正在编辑的草稿：托盘线程碰不到界面
//! 状态，而磁盘上的那份才是「上次保存的、可信的」配置。界面上没保存的改动
//! 本来就不该被托盘偷偷拿去用。

use crate::core::engine::Engine;

/// 安装托盘图标。返回 `false` 表示当前平台不支持（或创建失败）。
///
/// 失败不该阻止程序启动：托盘是增强功能，不是运行前提。
pub fn install(engine: Engine) -> bool {
    #[cfg(windows)]
    {
        imp::install(engine)
    }
    #[cfg(not(windows))]
    {
        let _ = engine;
        false
    }
}

/// 托盘点了「退出」时为真。主窗口的关闭处理用它区分
/// 「用户点了标题栏的叉」和「用户在托盘菜单里选了退出」。
#[cfg(windows)]
pub use imp::quit_armed;

#[cfg(not(windows))]
pub fn quit_armed() -> bool {
    false
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::OnceLock;

    use crate::core::i18n::t;
    use crate::tr_format;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, PostMessageW, RegisterClassW,
        SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HICON, MENU_ITEM_FLAGS,
        MSG, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_COMMAND,
        WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_POPUP,
    };

    use crate::core::config::Config;
    use crate::core::engine::Engine;

    /// 托盘回调消息（`WM_APP + 1` 是托盘图标的惯例位置）。
    const WM_TRAY: u32 = WM_APP + 1;
    const TRAY_ID: u32 = 1;
    const TIMER_ID: usize = 1;
    const MENU_SHOW: usize = 1;
    const MENU_START: usize = 2;
    const MENU_STOP: usize = 3;
    const MENU_QUIT: usize = 4;
    const CLASS_NAME: &str = "Tun2ProxyTrayWindow";

    /// 窗口过程里要用的两样东西。
    ///
    /// 用 `OnceLock` 而不是 `GWLP_USERDATA` 里的裸指针：托盘进程内只有一个，
    /// 静态变量省掉一轮生命周期与释放的推理。
    static ENGINE: OnceLock<Engine> = OnceLock::new();

    /// 托盘点了「退出」时为真：主窗口的关闭处理看到它就直接退出，
    /// 不再问「要不要最小化到托盘」。
    static QUIT_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// 托盘请求的是「彻底退出」而不是「关到托盘」。
    pub fn quit_armed() -> bool {
        QUIT_ARMED.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn install(engine: Engine) -> bool {
        if ENGINE.set(engine).is_err() {
            return false;
        }
        std::thread::Builder::new()
            .name("tun2proxy-tray".to_string())
            .spawn(run)
            .is_ok()
    }

    fn run() {
        // SAFETY: 这里的每个 Win32 调用都是「创建 + 传自己的指针」，没有跨线程
        // 共享的可变状态；窗口过程只在下面这个线程的消息循环里被调用。
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            if instance.is_null() {
                return;
            }
            let class_name = wide(CLASS_NAME);
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            if RegisterClassW(&class) == 0 && GetLastError() != 1410 {
                // 1410 = ERROR_CLASS_ALREADY_EXISTS（重复安装时不算失败）。
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return;
            }

            let mut data = icon_data(hwnd);
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                // 提示区被策略禁用 / 资源管理器没起来：记一行，别静默失败。
                if let Some(engine) = ENGINE.get() {
                    engine.log_line(
                        t(
                            "托盘：图标注册失败（系统托盘不可用）",
                            "Tray: icon registration failed (no system tray)",
                        )
                        .to_string(),
                    );
                }
                return;
            }
            if let Some(engine) = ENGINE.get() {
                engine.log_line(t("托盘：图标已注册", "Tray: icon registered").to_string());
            }
            // 每 2 秒刷一次提示文字：状态是后台线程改的，托盘得自己跟上。
            SetTimer(hwnd, TIMER_ID, 2000, None);

            let mut message: MSG = std::mem::zeroed();
            loop {
                let result = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            data.uFlags = 0;
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_TRAY => {
                match lparam as u32 {
                    WM_LBUTTONDBLCLK => {
                        crate::core::window::show_main();
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xffff {
                    MENU_SHOW => {
                        crate::core::window::show_main();
                    }
                    MENU_START => start_service(),
                    MENU_STOP => stop_service(),
                    MENU_QUIT => quit(),
                    _ => {}
                }
                0
            }
            WM_TIMER => {
                if wparam == TIMER_ID {
                    refresh_tooltip(hwnd);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn show_menu(hwnd: HWND) {
        let Some(engine) = ENGINE.get() else {
            return;
        };
        let active = engine.is_active();
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let string: MENU_ITEM_FLAGS = 0x0000_0000;
            let grayed: MENU_ITEM_FLAGS = 0x0000_0001;
            let separator: MENU_ITEM_FLAGS = 0x0000_0800;
            AppendMenuW(
                menu,
                string,
                MENU_SHOW,
                wide(t("显示窗口", "Show window")).as_ptr(),
            );
            AppendMenuW(
                menu,
                string | if active { grayed } else { 0 },
                MENU_START,
                wide(t("启动", "Start")).as_ptr(),
            );
            AppendMenuW(
                menu,
                string | if active { 0 } else { grayed },
                MENU_STOP,
                wide(t("停止", "Stop")).as_ptr(),
            );
            AppendMenuW(menu, separator, 0, std::ptr::null());
            AppendMenuW(menu, string, MENU_QUIT, wide(t("退出", "Quit")).as_ptr());

            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            // 弹出菜单前把焦点抢过来，否则点菜单外面菜单不会消失。
            SetForegroundWindow(hwnd);
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            DestroyMenu(menu);
            // 官方文档里的经典 workaround：给窗口发一条空消息，菜单才会真正消失。
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
    }

    fn start_service() {
        let Some(engine) = ENGINE.get() else {
            return;
        };
        if engine.is_active() {
            return;
        }
        // 用磁盘上保存的配置：托盘线程读不到界面里的草稿。
        let config = Config::load();
        let Some(profile) = config.profiles.get(config.active) else {
            engine.log_line(t("托盘：没有可用的配置", "Tray: no profile available").to_string());
            return;
        };
        match engine.start(profile) {
            Ok(()) => engine.log_line(
                t(
                    "托盘：已请求启动 tun2proxy",
                    "Tray: asked to start tun2proxy",
                )
                .to_string(),
            ),
            Err(err) => engine.log_line(tr_format!(
                "托盘：启动失败 - {err}",
                "Tray: start failed - {err}"
            )),
        }
    }

    fn stop_service() {
        if let Some(engine) = ENGINE.get() {
            engine.stop();
        }
    }

    fn quit() {
        // 先举起「真退出」的旗子，再走正常的关窗路径：窗口的关闭处理会
        // 看到旗子，跳过「是否最小化到托盘」直接放行。
        QUIT_ARMED.store(true, std::sync::atomic::Ordering::SeqCst);
        let Some(engine) = ENGINE.get() else {
            std::process::exit(0);
        };
        engine.shutdown_blocking();
        match crate::core::window::find_main() {
            // 关窗口 → zsui 的事件循环退出 → main 收尾。
            Some(hwnd) => unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            },
            None => std::process::exit(0),
        }
    }

    fn refresh_tooltip(hwnd: HWND) {
        let data = icon_data(hwnd);
        // SAFETY: `data` 是本栈上的完整 `NOTIFYICONDATAW`，句柄属于本线程创建的窗口。
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
        if ok == 0 {
            // 图标被资源管理器重排掉了（重启资源管理器、切换 DPI）：重新加一次。
            unsafe {
                Shell_NotifyIconW(NIM_ADD, &data);
            }
        }
    }

    /// 托盘图标的描述结构：图标、回调消息、提示文字。
    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = TRAY_ID;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = app_icon();
        let tip = wide(&tooltip_text());
        let len = tip.len().min(data.szTip.len() - 1);
        data.szTip[..len].copy_from_slice(&tip[..len]);
        data
    }

    fn tooltip_text() -> String {
        let state = ENGINE
            .get()
            .map(|engine| match engine.status() {
                crate::core::Status::Running => t("运行中", "Running"),
                crate::core::Status::Starting => t("正在启动", "Starting"),
                crate::core::Status::Stopping => t("正在停止", "Stopping"),
                crate::core::Status::Failed(_) => t("运行失败", "Failed"),
                crate::core::Status::Stopped => t("未运行", "Not running"),
            })
            .unwrap_or(t("未运行", "Not running"));
        format!("Tun2Proxy · {state}")
    }

    /// 图标：直接用内嵌的 ico 字节造句柄，和标题栏 / 任务栏同一份图。
    ///
    /// 之前用 `ExtractIconExW` 按 exe 路径反查，拿到的是资源管理器缓存里
    /// 的那一份——换了图标之后托盘还是老图，和任务栏对不上。
    fn app_icon() -> HICON {
        // 32px 起步：托盘在 100% 缩放下是 16px，但 150%/200% 下是 24/32px，
        // 给小图会让 Windows 去放大，边缘发虚。
        crate::core::icon::hicon(32).unwrap_or(std::ptr::null_mut())
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 让编译器知道 `c_void` / `WINDOW_EX_STYLE` 这些类型确实来自 windows-sys。
    #[allow(dead_code)]
    fn _types(_: *const c_void, _: WINDOW_EX_STYLE) {}
}
