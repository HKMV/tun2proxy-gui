//! 进程级平台初始化。
//!
//! 目前只有一件事：**Windows 上的 DPI 感知**。
//!
//! 如果不声明 DPI 感知，Windows 会把这个进程当成「96 DPI 老程序」，先把窗口
//! 画在 96 DPI 的位图上，再整体拉伸到显示器当前缩放比例（125% / 150%），
//! 结果是界面发虚、字号偏大、`GetWindowRect` 返回的尺寸也和请求值对不上。
//!
//! GPUI 的 Windows 后端会**读** DPI（`GetDpiForWindow` / `GetDpiForMonitor`，
//! 并且处理 `WM_DPICHANGED`），但不会替应用**声明**感知级别——那是进程级的
//! 决定，库不该替宿主做。所以这一步仍然留在我们自己的启动流程里，
//! 并且必须在 `open_window` 之前完成。
//!
//! macOS / Linux 不需要：AppKit 与 GTK 自己处理缩放。

/// 声明 DPI 感知。必须在创建任何窗口之前调用。
pub fn init() {
    imp::init()
}

/// 让 Windows 的原生标题栏跟随界面深浅。
///
/// GPUI 只在**系统配色变化**时同步 DWM 的深色标志，而本应用的主题是按
/// `TUN2PROXY_GUI_THEME` 选的——系统是浅色、界面是 Gruvbox 深色时，
/// 顶上就会顶一条白带。窗口创建后自己设一次，两边才一致。
///
/// 返回是否设置成功；失败只是标题栏保持原样，不影响功能。
pub fn set_dark_titlebar(dark: bool) -> bool {
    imp::set_dark_titlebar(dark)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    pub fn init() {
        // SAFETY: 只设置本进程的 DPI 感知级别，不涉及任何指针或所有权。
        // 返回值只表示「是否已经设置过」（比如被清单文件抢先声明过），
        // 重复调用无害，因此忽略。
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    pub fn set_dark_titlebar(dark: bool) -> bool {
        // 窗口句柄只能靠标题反查：gpui 的 `Window` 不暴露 HWND，
        // 而本进程只有一个带标题的顶层窗口（见 `core::window::find_main`）。
        let Some(hwnd) = crate::core::window::find_main() else {
            return false;
        };
        let value: i32 = if dark { 1 } else { 0 };
        // SAFETY: 传的是一个本地 `i32` 的地址，长度也如实给了 4 字节；
        // DWM 只读这个值，不会留下指针。
        let hr = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &value as *const _ as *const _,
                std::mem::size_of::<i32>() as u32,
            )
        };
        hr == 0
    }
}

#[cfg(not(windows))]
mod imp {
    /// macOS 与 Linux 的窗口系统自己处理缩放。
    pub fn init() {}

    /// 非 Windows 平台由窗口系统自己跟随深浅，无需额外设置。
    pub fn set_dark_titlebar(_dark: bool) -> bool {
        false
    }
}
