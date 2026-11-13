//! 单实例：第二次启动不再开一个新窗口，而是把已经在跑的那个叫到前台。
//!
//! # 为什么不是「第二个进程直接 ShowWindow」
//!
//! 最早那版就是那样写的：枚举窗口 → `ShowWindow` + `SetForegroundWindow`。
//! 同权限下它确实让窗口「可见」了（`IsWindowVisible` 变成真），但用户看到的
//! 是**一帧白窗口**——GPUI 的渲染由它自己的循环驱动，被别的进程从外面
//! `ShowWindow` 之后没有人让它重绘。而对方提权时（正常启动就是这样）连这
//! 一点都做不到，UIPI 把窗口消息挡在门外，表现就是「双击了没反应」。
//!
//! 所以改成「打个招呼，让它自己显示」：
//!
//! * [`create_show_event`] —— 第一个实例开一个命名事件；
//! * [`request_show`] —— 第二个实例 `SetEvent` 一下就走，完全不碰对方的窗口；
//! * [`take_show_request`] —— 第一个实例的托盘线程每 200ms 看一眼这个事件，
//!   有请求就走它自己的「显示窗口」那条路（和双击托盘图标是同一段代码），
//!   GPUI 照常重绘、照常抢焦点。
//!
//! 为什么宁可绕这一圈也不用 `PostMessage`：命名内核对象只受 DACL 限制，同一
//! 个用户的中完整性进程能拿到事件的 `EVENT_MODIFY_STATE`；而窗口消息那边，
//! `WM_APP` 以上的自定义消息从低完整性发往高完整性会被 UIPI **静默丢弃**，
//! 消息发出去就等于石沉大海。
//!
//! # 唯一实例的认领
//!
//! [`claim`] / [`release`] 用一个命名互斥量守住「本会话里只有一个实例」。
//! 提权重启出来的子进程正是接手的那个，而父进程从头到尾没有窗口、也不占
//! 互斥量，所以两者不会互相挡；[`TAKEOVER_ARG`] 同时让接手的那个**不去**
//! [`request_show`]——那个「已经有实例」正是还没退干净的旧进程，请它反而会
//! 把新进程自己关掉。
//!
//! 非 Windows 平台暂时没有这套东西（托盘、窗口显隐也一样）：[`request_show`]
//! 与 [`take_show_request`] 恒为 `false`、[`claim`] 恒为 `true`，也就是
//! 「允许开多个」。

/// 「我是接手的那一个」标记。
///
/// 提权重启（启动时的 UAC，和界面上「以管理员身份重启」）会把旧进程换成新
/// 进程。旧进程的窗口可能还活着几十毫秒，新进程要是按「有窗口就是已经有实例」
/// 判断，就会把自己关掉，用户看到的是「点了重启，程序没了」。
#[cfg(windows)]
pub const TAKEOVER_ARG: &str = "--takeover";

#[cfg(windows)]
pub use imp::{
    claim, create_show_event, focus_window, release, request_show, take_show_request,
};

#[cfg(not(windows))]
pub use other::{
    claim, create_show_event, focus_window, release, request_show, take_show_request,
};

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, TRUE,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, GetCurrentProcessId, OpenEventW, OpenProcess,
        QueryFullProcessImageNameW, ResetEvent, SetEvent, WaitForSingleObject,
        EVENT_MODIFY_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
        SwitchToThisWindow, SW_RESTORE, SW_SHOW,
    };

    use super::TAKEOVER_ARG;

    /// 互斥量名字：本会话里「谁是唯一实例」。
    ///
    /// 用 `Local\` 前缀而不是 `Global\`：不同登录会话里各跑一个实例是合理的
    /// （两个用户各自在用），同一个会话里才该只有一个。名字带版本后缀，将来
    /// 真要改这套规则时换个后缀就能和旧版并存，不必先把旧进程杀干净。
    const MUTEX_NAME: &str = "Local\\tun2proxy-gui-single-instance-v1";

    /// 事件名字：「有人找我，把窗口叫出来」。
    const SHOW_EVENT_NAME: &str = "Local\\tun2proxy-gui-show-window-v1";

    /// 互斥量句柄。进程活着就一直在，退出时由系统回收。
    static GUARD: AtomicIsize = AtomicIsize::new(0);

    /// 「显示窗口」事件句柄，只有第一个实例持有（也是「有实例在跑」的凭据）。
    static SHOW_EVENT: AtomicIsize = AtomicIsize::new(0);

    /// 认领「本会话唯一实例」。
    ///
    /// 返回 `false` 表示已经有别的实例占着，调用方应当请求它显示窗口然后退出。
    /// 互斥量本身造不出来时（权限异常之类）**返回 `true`**：宁可多开一个，
    /// 也不要让用户点了程序什么都没发生。
    pub fn claim() -> bool {
        if GUARD.load(Ordering::SeqCst) != 0 {
            return true;
        }
        let name = wide(OsStr::new(MUTEX_NAME));
        // SAFETY: 名字是本模块的常量，已按 UTF-16 + NUL 结尾；`null` 属性表示
        // 默认安全描述符（同用户可访问）。
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return true;
        }
        // SAFETY: 紧跟 `CreateMutexW` 读错误码，中间没有别的调用。
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if exists {
            // SAFETY: 句柄是刚拿到的有效句柄，只是我们不需要它。
            unsafe { CloseHandle(handle) };
            return false;
        }
        GUARD.store(handle as isize, Ordering::SeqCst);
        true
    }

    /// 放开互斥量（「以管理员身份重启」前调用）。
    ///
    /// 不放开的话，接手的那个新进程会以为自己不是唯一实例而直接退出——用户
    /// 看到的就是「点了重启，程序没了」。
    ///
    /// 「显示窗口」事件一起交出去：新进程会自己另开一个，旧进程交完就退出，
    /// 中途不该再有「旧进程的托盘线程把别人的请求领走」这种事。
    pub fn release() {
        let event = SHOW_EVENT.swap(0, Ordering::SeqCst);
        if event != 0 {
            // SAFETY: 值只可能来自 `create_show_event` 里的 `CreateEventW`。
            unsafe { CloseHandle(event as HANDLE) };
        }
        let raw = GUARD.swap(0, Ordering::SeqCst);
        if raw != 0 {
            // SAFETY: 值只可能来自 `claim` 里的 `CreateMutexW`。
            unsafe { CloseHandle(raw as HANDLE) };
        }
    }

    /// 第一个实例开「显示窗口」事件（认领成功之后调一次）。
    ///
    /// 手动重置（`bmanualreset = 1`）：请求一直挂着，直到托盘线程自己清掉——
    /// 托盘线程每 200ms 才看一次，用自动重置的事件容易和「还没看完就被下一次
    /// 请求顶掉」纠缠。
    pub fn create_show_event() {
        if SHOW_EVENT.load(Ordering::SeqCst) != 0 {
            return;
        }
        let name = wide(OsStr::new(SHOW_EVENT_NAME));
        // SAFETY: 名字是常量、已 NUL 结尾；`null` 属性表示默认安全描述符。
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };
        if handle.is_null() {
            return;
        }
        SHOW_EVENT.store(handle as isize, Ordering::SeqCst);
    }

    /// 请求「已经在跑的那个实例」把窗口显示出来。
    ///
    /// 返回 `true` 表示请求已经送到（事件存在 = 确实有实例在跑），调用方应当
    /// 直接退出；`false` 表示没有实例、或者事件打不开（那次就得靠调用方继续
    /// 往下走，比如提权之后再问一次）。
    pub fn request_show() -> bool {
        // 接手的那一个（提权重启出来的）绝不能去「请别人显示窗口」：那个「别人」
        // 正是还没退干净的旧进程，请它只会把自己这个新进程关掉——用户看到的就是
        // 「点了重启，程序没了」。
        if std::env::args().any(|arg| arg == TAKEOVER_ARG) {
            return false;
        }
        let name = wide(OsStr::new(SHOW_EVENT_NAME));
        // SAFETY: 名字是常量、已 NUL 结尾。
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if handle.is_null() {
            return false;
        }
        // SAFETY: 句柄只在这里用一次，随后关闭。
        let ok = unsafe { SetEvent(handle) } != 0;
        unsafe { CloseHandle(handle) };
        ok
    }

    /// 托盘线程问一句：「有人请我们显示窗口吗？」有就顺手清掉。
    ///
    /// 用 0 超时：这只是消息循环里的一瞥，绝不能卡住托盘线程。
    pub fn take_show_request() -> bool {
        let raw = SHOW_EVENT.load(Ordering::SeqCst);
        if raw == 0 {
            return false;
        }
        let handle = raw as HANDLE;
        // SAFETY: 句柄来自本模块的 `CreateEventW`；0 超时不会阻塞。
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            return false;
        }
        // SAFETY: 同上。
        unsafe { ResetEvent(handle) };
        true
    }

    /// 最后兜底：直接显示并前置**别的进程**的主窗口。
    ///
    /// 只有在「事件信号打不出去」时才走这里（典型是托盘没装上、没人看事件），
    /// 而这条路的显示是不可靠的：对方的 GPUI 不会因为外部 `ShowWindow` 重绘，
    /// 很可能只露出一帧白窗口。有事件就该用事件。
    pub fn focus_window(title: &str) -> bool {
        // 接手的那一个（提权重启出来的）不要动旧进程的窗口。
        if std::env::args().any(|arg| arg == TAKEOVER_ARG) {
            return false;
        }
        let Some(hwnd) = find_other_main(title) else {
            return false;
        };
        // SAFETY: `hwnd` 来自 `EnumWindows` 的枚举结果，调用期间有效；这些
        // 都是对它的常规窗口操作（最小化的先 RESTORE，藏进托盘的 SHOW 就够）。
        unsafe {
            let command = if IsIconic(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            ShowWindow(hwnd, command);
            BringWindowToTop(hwnd);
            // 前台锁经常让 `SetForegroundWindow` 直接失败（本进程没有前台
            // 权限时它只闪一下任务栏）。`SwitchToThisWindow` 是系统给「用户
            // 明确要求切过去」留的后门，两条一起用才不会出现「窗口显示了但
            // 还在别的窗口后面」。
            if SetForegroundWindow(hwnd) == 0 {
                SwitchToThisWindow(hwnd, TRUE);
            }
        }
        true
    }

    /// `EnumWindows` 回调用的上下文。
    struct Probe {
        /// 本进程 pid：自己的窗口不算「已经有一个实例」。
        me: u32,
        /// 本进程 exe 文件名（小写），用来排除「标题碰巧一样」的别的程序。
        exe: String,
        /// 主窗口标题。
        title: String,
        found: Option<HWND>,
    }

    fn find_other_main(title: &str) -> Option<HWND> {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
            })
            .unwrap_or_default();
        let mut probe = Probe {
            // SAFETY: 无参数，总是成功。
            me: unsafe { GetCurrentProcessId() },
            exe,
            title: title.to_string(),
            found: None,
        };
        // SAFETY: 回调只在本调用期间被 user32 调用，`context` 指向本栈上的
        // `probe`，回调里只做只读查询与写入该指针。
        unsafe {
            EnumWindows(Some(collect), &mut probe as *mut Probe as LPARAM);
        }
        probe.found
    }

    /// 挑别的进程里那个**主窗口**，可见的优先。
    ///
    /// 判据是「标题等于主窗口标题」，不是「同一个 exe」：托盘那个自建窗口
    /// （标题就是它的类名 `Tun2ProxyTrayWindow`）和 GPUI 的输入法辅助窗口
    /// （`MSCTFIME UI` / `Default IME`）都挂在同一个进程里、也都有标题，按 exe
    /// 名字认会认出一堆「主窗口」，最后很可能把托盘那个藏起来的空窗口当成
    /// 目标——`ShowWindow` 它当然什么也不会发生。
    unsafe extern "system" fn collect(hwnd: HWND, context: LPARAM) -> BOOL {
        // SAFETY: `context` 是 `find_other_main` 里 `&mut probe` 的地址。
        let probe = unsafe { &mut *(context as *mut Probe) };
        let mut pid = 0u32;
        // SAFETY: `hwnd` 由 user32 传入，`pid` 是本栈上的有效地址。
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid == probe.me {
            return TRUE;
        }
        // SAFETY: 都是对系统句柄的只读查询。
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len == 0 || !title_is(hwnd, len, &probe.title) {
            return TRUE;
        }
        // exe 名字读得出来就必须一致（同名的另一个程序不该被我们接管）；
        // 读不出来（对方提权了、`OpenProcess` 被拒）就只按标题认。
        if same_exe(pid, &probe.exe) == Some(false) {
            return TRUE;
        }
        // 已经拿到一个可见窗口就不再改（枚举顺序不保证，但可见性比顺序重要）。
        if let Some(existing) = probe.found {
            // SAFETY: `existing` 是上一次枚举拿到的有效句柄。
            if unsafe { IsWindowVisible(existing) } != 0 {
                return TRUE;
            }
        }
        probe.found = Some(hwnd);
        TRUE
    }

    /// 目标进程的 exe 文件名是不是 `want`（都是小写）。
    ///
    /// `None` 表示**读不出来**（对方提权、句柄被拒），这时不能据此排除它。
    ///
    /// `OpenProcess` 用 `PROCESS_QUERY_LIMITED_INFORMATION`：它在中完整性级别
    /// 也能问高完整性级别的进程，正好是「普通启动的第二个实例去认提权实例」
    /// 这个场景。
    fn same_exe(pid: u32, want: &str) -> Option<bool> {
        if want.is_empty() {
            return None;
        }
        // SAFETY: `pid` 来自 `GetWindowThreadProcessId`，句柄用完即关。
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        // SAFETY: 缓冲区是本栈上的数组，`size` 是它的容量。
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
        // SAFETY: 句柄只在这里用一次，随后关闭。
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_lowercase();
        Some(name == want)
    }

    /// 窗口标题是不是 `want`。
    fn title_is(hwnd: HWND, len: i32, want: &str) -> bool {
        let mut buffer = vec![0u16; len as usize + 1];
        // SAFETY: 缓冲区长度由 `GetWindowTextLengthW` 给出（+1 给结尾的 NUL）。
        let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        if copied <= 0 {
            return false;
        }
        String::from_utf16_lossy(&buffer[..copied as usize]) == want
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod other {
    /// 非 Windows：没有跨进程唤醒的通道，也就不拦（各开各的）。
    pub fn request_show() -> bool {
        false
    }

    pub fn create_show_event() {}

    pub fn take_show_request() -> bool {
        false
    }

    pub fn focus_window(_title: &str) -> bool {
        false
    }

    pub fn claim() -> bool {
        true
    }

    pub fn release() {}
}
