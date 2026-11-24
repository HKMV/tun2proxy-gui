//! 进程级平台初始化。
//!
//! 目前只有一件事：**Windows 上的 DPI 感知**。
//!
//! 如果不声明 DPI 感知，Windows 会把这个进程当成「96 DPI 老程序」，先把窗口
//! 画在 96 DPI 的位图上，再整体拉伸到显示器当前缩放比例（125% / 150%），
//! 结果是界面发虚、字号偏大、`GetWindowRect` 返回的尺寸也和请求值对不上。
//!
//! 窗口后端会**读** DPI（`GetDpiForWindow` / `GetDpiForMonitor`，并且处理
//! `WM_DPICHANGED`），但不会替应用**声明**感知级别——那是进程级的决定，库
//! 不该替宿主做。所以这一步仍然留在我们自己的启动流程里，并且必须在开窗口
//! 之前完成。
//!
//! macOS / Linux 不需要：AppKit 与 GTK 自己处理缩放。
//!
//! 标题栏的深浅不在这里处理：界面用自绘标题栏（`TitleBarStyle::Hidden`），
//! 系统那一条根本不显示。原先按窗口标题反查 HWND 再设 DWM 深色标志的那段
//! Win32 代码随着系统标题栏一起去掉了。

/// 声明 DPI 感知。必须在创建任何窗口之前调用。
pub fn init() {
    imp::init()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    pub fn init() {
        // SAFETY: 只设置本进程的 DPI 感知级别，不涉及任何指针或所有权。
        // 返回值只表示「是否已经设置过」（比如被清单文件抢先声明过），
        // 重复调用无害，因此忽略。
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }
}

#[cfg(not(windows))]
mod imp {
    /// macOS 与 Linux 的窗口系统自己处理缩放。
    pub fn init() {}
}
