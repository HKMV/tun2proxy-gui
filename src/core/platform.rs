//! 进程级平台初始化。
//!
//! 目前只有一件事：**Windows 上的 DPI 感知**。
//!
//! 如果不声明 DPI 感知，Windows 会把这个进程当成「96 DPI 老程序」，先把窗口
//! 画在 96 DPI 的位图上，再整体拉伸到显示器当前缩放比例（125% / 150%），
//! 结果是界面发虚、字号偏大、`GetWindowRect` 返回的尺寸也和请求值对不上。
//! 声明 `PER_MONITOR_AWARE_V2` 之后，ZSUI 才能通过 `GetDpiForWindow` 拿到
//! 真实 DPI，按正确比例排版和绘制。
//!
//! macOS / Linux 不需要这一步：AppKit 与 GTK 自己处理缩放。

/// 初始化平台设置，返回主显示器的缩放比例（1.0 = 100%，1.5 = 150%）。
///
/// 必须在创建任何窗口之前调用；窗口的像素尺寸要按这个比例换算。
pub fn init() -> f32 {
    imp::init()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    pub fn init() -> f32 {
        unsafe {
            // 返回值只表示「是否已经设置过」，重复调用无害。
            let ok = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let dpi = GetDpiForSystem();
            eprintln!("[debug] SetProcessDpiAwarenessContext={ok} GetDpiForSystem={dpi}");
            (dpi.max(96) as f32) / 96.0
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// macOS 与 Linux 的窗口系统自己处理缩放，界面统一按 1.0 计算。
    pub fn init() -> f32 {
        1.0
    }
}
