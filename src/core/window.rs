//! 主窗口的查找与显隐。
//!
//! GPUI / gpuikit 自己管着窗口过程，没有对外暴露 `HWND`。托盘线程、关闭确认
//! 这些「在别处操作主窗口」的代码因此拿不到句柄，只能反查——这里把反查收成
//! 一处，并且按**进程归属**找，而不是按标题找：
//!
//! 按标题 `FindWindowW` 找过一版，托盘「显示窗口」点了没反应——标题一旦
//! 带上了状态（`Tun2Proxy GUI · 运行中`）或者被系统改过，就再也匹配不上。
//! 按 `GetWindowThreadProcessId == 本进程` 找，名字怎么变都不影响。
//!
//! 但「本进程 + 有标题」还不够：同进程里还有托盘那个从不显示的窗口和两个
//! 输入法辅助窗口，它们都有标题，而 `EnumWindows` 的次序（Z 序）会随开窗顺序
//! 变。抓错对象的后果很具体——`ShowWindow` 一个 0×0 的空窗口，用户看到的就是
//! 「双击了没反应」，前台还被让给了它。实测这几个诱饵都是 0×0（输入法的还带
//! owner），主窗口是有尺寸、无 owner 的那个，判据就照这个来（见 [`collect`]）。

#[cfg(windows)]
pub use windows_window::{find_main, hide_main_deferred, show_main};

#[cfg(not(windows))]
pub use other_window::{find_main, hide_main_deferred, show_main};

#[cfg(windows)]
mod windows_window {
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindow, GetWindowRect, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow,
        ShowWindow, SwitchToThisWindow, GW_OWNER, SW_HIDE, SW_RESTORE, SW_SHOW,
    };

    /// 上次找到的句柄。窗口被销毁（比如换了标题栏）后要重新找，所以每次
    /// 用之前都先 `IsWindow` 验一下。
    static CACHED: Mutex<Option<isize>> = Mutex::new(None);

    /// 找本进程的主窗口：有标题、有尺寸、无 owner 的顶层窗口，可见的优先。
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

        // `(可见, 面积, 句柄)`：挑分最高的那个。
        let mut found: Option<((i64, i64), HWND)> = None;
        // SAFETY: 回调只在本调用期间被 user32 调用，`context` 指向本栈上的
        // `found`，回调里只做只读查询与写入该指针。
        unsafe {
            EnumWindows(Some(collect), &mut found as *mut Option<((i64, i64), HWND)> as LPARAM);
        }

        let hwnd = found.map(|(_, hwnd)| hwnd);
        if let Some(hwnd) = hwnd {
            if let Ok(mut cached) = CACHED.lock() {
                *cached = Some(hwnd as isize);
            }
        }
        hwnd
    }

    /// `EnumWindows` 回调：在本进程里挑**主**窗口。
    ///
    /// 三道筛子，都是照着实测的差别定的：
    ///
    /// 1. 有标题——没有标题的窗口不是主窗口；
    /// 2. 没有 owner——有 owner 的是挂出来的辅助窗口（输入法、弹出层）；
    /// 3. 有面积——托盘窗口和输入法窗口都是 0×0。
    ///
    /// 剩下的按「可见优先、其次面积大」取一个：隐藏状态下主窗口依然是 914×688，
    /// 面积这一层顺带把「将来又多出个有尺寸的小窗口」也压住了。
    unsafe extern "system" fn collect(
        hwnd: HWND,
        context: LPARAM,
    ) -> windows_sys::Win32::Foundation::BOOL {
        let best = &mut *(context as *mut Option<((i64, i64), HWND)>);
        let mut pid = 0u32;
        // SAFETY: `hwnd` 由 user32 传入，`pid` 是本栈上的有效地址。
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid != unsafe { GetCurrentProcessId() } {
            return TRUE;
        }
        // SAFETY: 下面都是对 `hwnd` 的只读查询。
        unsafe {
            if GetWindowTextLengthW(hwnd) == 0 {
                return TRUE;
            }
            if !GetWindow(hwnd, GW_OWNER).is_null() {
                return TRUE;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return TRUE;
            }
            let area = (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64;
            if area <= 0 {
                return TRUE;
            }
            let key = (IsWindowVisible(hwnd) as i64, area);
            match *best {
                Some((current, _)) if current >= key => {}
                _ => *best = Some((key, hwnd)),
            }
        }
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
            // 前台锁：本进程这时候通常不是前台进程，`SetForegroundWindow` 只会
            // 闪一下任务栏（用户看到「窗口回来了但还在别的窗口后面」）。
            // `SwitchToThisWindow` 是系统留给「用户明确要求切过去」的路子。
            if SetForegroundWindow(hwnd) == 0 {
                SwitchToThisWindow(hwnd, TRUE);
            }
        }
        true
    }

    /// 延后藏窗口：给 UI 线程留出「把这次事件处理完」的机会。
    ///
    /// `ShowWindow(SW_HIDE)` 会**同步**把 `WM_SHOWWINDOW` / `WM_SIZE` 派回本进程的
    /// 窗口过程，而我们发起它的位置正是「点击事件的处理函数」——消息重入
    /// 一层套一层，界面线程容易在这里打个难查的死结。
    ///
    /// 换成另一个线程稍等再藏：消息到达时这次点击早已处理完。
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
