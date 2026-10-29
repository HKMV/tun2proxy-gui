//! 主窗口的查找与显隐。
//!
//! ZSUI 自己管着窗口过程，没有对外暴露 `HWND`。托盘线程、关闭确认这些
//! 「在别处操作主窗口」的代码因此拿不到句柄，只能反查——这里把反查收成
//! 一处，并且按**进程归属**找，而不是按标题找：
//!
//! 按标题 `FindWindowW` 找过一版，托盘「显示窗口」点了没反应——标题一旦
//! 带上了状态（`Tun2Proxy GUI · 运行中`）或者被系统改过，就再也匹配不上。
//! 按 `GetWindowThreadProcessId == 本进程` 找，名字怎么变都不影响。

#[cfg(windows)]
pub use windows_window::{find_main, hide_main_deferred, show_main};

#[cfg(not(windows))]
pub use other_window::{find_main, hide_main_deferred, show_main};

#[cfg(windows)]
mod windows_window {
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
        IsWindow, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
    };

    /// 上次找到的句柄。窗口被销毁（比如换了标题栏）后要重新找，所以每次
    /// 用之前都先 `IsWindow` 验一下。
    static CACHED: Mutex<Option<isize>> = Mutex::new(None);

    /// 找本进程的主窗口：有标题、可见的顶层窗口。
    pub fn find_main() -> Option<HWND> {
        if let Ok(cached) = CACHED.lock() {
            if let Some(raw) = *cached {
                let hwnd = raw as HWND;
                // SAFETY: 句柄值来自本进程枚举结果；`IsWindow` 对失效句柄返回 0。
                if unsafe { IsWindow(hwnd) } != 0 {
                    return Some(hwnd);
                }
            }
        }

        let mut found: Option<HWND> = None;
        // SAFETY: 回调只在本调用期间被 user32 调用，`context` 指向本栈上的
        // `found`，回调里只做只读查询与写入该指针。
        unsafe {
            EnumWindows(Some(collect), &mut found as *mut Option<HWND> as LPARAM);
        }

        if let Some(hwnd) = found {
            if let Ok(mut cached) = CACHED.lock() {
                *cached = Some(hwnd as isize);
            }
        }
        found
    }

    /// `EnumWindows` 回调：挑本进程里「有标题」的顶层窗口，可见的优先。
    unsafe extern "system" fn collect(
        hwnd: HWND,
        context: LPARAM,
    ) -> windows_sys::Win32::Foundation::BOOL {
        let found = &mut *(context as *mut Option<HWND>);
        let mut pid = 0u32;
        // SAFETY: `hwnd` 由 user32 传入，`pid` 是本栈上的有效地址。
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid != unsafe { GetCurrentProcessId() } {
            return TRUE;
        }
        // SAFETY: 同上，都是对系统句柄的只读查询。
        if unsafe { GetWindowTextLengthW(hwnd) } == 0 {
            return TRUE;
        }
        // 已经拿到一个可见窗口就不再改（枚举顺序不保证，但可见性比顺序重要）。
        if let Some(existing) = *found {
            if unsafe { IsWindowVisible(existing) } != 0 {
                return TRUE;
            }
        }
        *found = Some(hwnd);
        TRUE
    }

    /// 显示并前置主窗口（从托盘唤回时用）。
    pub fn show_main() -> bool {
        let Some(hwnd) = find_main() else {
            return false;
        };
        // SAFETY: `hwnd` 已通过 `IsWindow` 校验；`IsIconic` / `ShowWindow` /
        // `SetForegroundWindow` 都是对它的常规窗口操作。
        unsafe {
            // 最小化过的窗口要先 RESTORE，否则 SHOW 只是把它留在任务栏上。
            let command = if IsIconic(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            ShowWindow(hwnd, command);
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
        }
        true
    }

    /// 延后藏窗口：给 UI 线程留出「把这次事件处理完」的机会。
    ///
    /// `ShowWindow(SW_HIDE)` 会**同步**把 `WM_SHOWWINDOW` / `WM_SIZE` 派回本进程的
    /// 窗口过程，而 ZSUI 的窗口过程要拿视图状态锁重建界面；此时我们还在
    /// `update()` 里、锁还在手上——于是自己等自己，UI 线程直接挂死（实测：
    /// 点「最小化到托盘」后窗口不消失，`IsHungAppWindow` 为真）。
    ///
    /// 换成另一个线程稍等再藏：消息到达时 `update()` 早已返回，锁已释放。
    pub fn hide_main_deferred() -> bool {
        let Some(hwnd) = find_main() else {
            return false;
        };
        // `HWND` 是裸指针，不能直接跨线程搬；转成整数带过去再转回来。
        let raw = hwnd as isize;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            // SAFETY: 句柄在 `find_main` 里校验过；即便中途窗口被销毁，
            // `ShowWindow` 对失效句柄只是返回 0，不会崩。
            unsafe {
                ShowWindow(raw as HWND, SW_HIDE);
            }
        });
        true
    }
}

#[cfg(not(windows))]
mod other_window {
    /// 非 Windows 平台暂不支持「最小化到托盘」：没有托盘，藏起来就找不回来。
    pub fn find_main() -> Option<()> {
        None
    }

    pub fn show_main() -> bool {
        false
    }

    pub fn hide_main_deferred() -> bool {
        false
    }
}

/// 供 `core` 之外判断「能不能藏窗口」：不能藏就不该给出这个选项。
pub fn can_hide() -> bool {
    cfg!(windows)
}
