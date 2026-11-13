//! 权限处理。
//!
//! 创建 TUN 设备、修改系统路由表需要管理员权限：Windows 上是 UAC 提权，
//! macOS 上必须是 root；Linux 可以用用户命名空间（`--unshare`）免 root。
//!
//! Windows 上的做法：启动时检测进程令牌的 `TokenElevation`，没有权限就弹一个
//! 系统对话框，用户同意后用 `ShellExecuteW(..., "runas", ...)` 重新拉起自己。

use crate::core::i18n;
/// 当前进程是否已具备 tun2proxy 默认模式所需的权限。
pub fn is_elevated() -> bool {
    imp::is_elevated()
}

/// 确保权限足够；平台不强制要求提权时什么也不做。
///
/// 设置环境变量 `TUN2PROXY_GUI_NO_ELEVATE=1` 可以跳过提权提示（调试用，
/// 比如已经在管理员终端里运行）。
pub fn ensure_elevated() {
    if std::env::var_os("TUN2PROXY_GUI_NO_ELEVATE").is_some() {
        return;
    }
    imp::ensure_elevated()
}

/// 以管理员身份重新拉起自己。
///
/// 返回 `true` 表示新进程已经起来了，调用方应当立刻退出；`false` 表示用户
/// 在 UAC 对话框上点了「否」或者拉起失败。启动时那次提示（[`ensure_elevated`]）
/// 会直接退出进程，这里是给界面上「使用管理员重启」按钮用的、不退出进程的版本。
pub fn relaunch_elevated() -> bool {
    imp::relaunch_elevated()
}

/// 权限不足时给界面用的一句提示，权限足够时返回 `None`。
///
/// 提示必须写**后果**，不能只写「做不了什么」：用户看到「无法创建虚拟网卡」
/// 并不知道这意味着什么，真正要告诉他的是一句话——隧道起不来，流量照旧直连，
/// 一个包都不会经过上游代理。
pub fn privilege_hint() -> Option<String> {
    if is_elevated() {
        return None;
    }
    #[cfg(windows)]
    {
        Some(
            i18n::t(
                "当前没有管理员权限，无法创建虚拟网卡：隧道起不来，系统流量照常直连，不会经过代理。",
                "Administrator rights are missing, so the TUN adapter cannot be created: the tunnel cannot start and system traffic keeps going out directly, never through the proxy.",
            )
            .to_string(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Some(
            i18n::t(
                "当前不是 root：macOS 上需要 root 才能创建 utun 并修改路由，隧道起不来，系统流量不会经过代理。",
                "Not running as root: macOS needs root to create a utun device and change routes, so the tunnel cannot start and traffic never reaches the proxy.",
            )
            .to_string(),
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Some(
            "当前不是 root：需要 root，或勾选高级设置里的「用户命名空间（--unshare）」——否则隧道起不来，系统流量不会经过代理。"
                .to_string(),
        )
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_SETFOREGROUND, MB_YESNO,
        SW_SHOWNORMAL,
    };

    const APP_TITLE: &str = "Tun2Proxy";

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// 读取进程令牌里的提权标志位（`TokenElevation`）。
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut returned = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    fn message_box(text: &str, flags: u32) -> i32 {
        let text = wide(OsStr::new(text));
        let title = wide(OsStr::new(APP_TITLE));
        unsafe { MessageBoxW(ptr::null_mut(), text.as_ptr(), title.as_ptr(), flags) }
    }

    pub fn ensure_elevated() {
        if is_elevated() {
            return;
        }
        let choice = message_box(
            "tun2proxy 需要管理员权限才能创建虚拟网卡并修改路由表。\n\n是否以管理员身份重新启动？",
            MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND,
        );
        if choice != IDYES {
            std::process::exit(0);
        }
        if !relaunch_elevated() {
            message_box(
                "以管理员身份启动失败，请手动右键选择「以管理员身份运行」。",
                MB_ICONERROR,
            );
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    pub fn relaunch_elevated() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let mut args: Vec<String> = std::env::args().skip(1).collect();
        // 新进程是**接手**的那一个：旧进程的窗口可能还活着几十毫秒，不带上
        // 这个标记的话，它会把自己当成「已经有一个在跑」而直接退出——用户
        // 看到的是「点了重启，程序没了」。见 `core::single_instance`。
        if !args
            .iter()
            .any(|arg| arg == crate::core::single_instance::TAKEOVER_ARG)
        {
            args.push(crate::core::single_instance::TAKEOVER_ARG.to_string());
        }
        let args = args
            .iter()
            .map(|arg| {
                if arg.contains(' ') {
                    format!("\"{arg}\"")
                } else {
                    arg.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let verb = wide(OsStr::new("runas"));
        let file = wide(exe.as_os_str());
        let params = wide(OsStr::new(&args));
        unsafe {
            let result = ShellExecuteW(
                ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                ptr::null(),
                SW_SHOWNORMAL,
            );
            result as isize > 32
        }
    }
}

#[cfg(unix)]
mod imp {
    pub fn is_elevated() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    pub fn ensure_elevated() {}

    /// 非 Windows 平台没有「右键以管理员身份运行」这条路。
    pub fn relaunch_elevated() -> bool {
        false
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn is_elevated() -> bool {
        true
    }

    pub fn ensure_elevated() {}

    pub fn relaunch_elevated() -> bool {
        false
    }
}
