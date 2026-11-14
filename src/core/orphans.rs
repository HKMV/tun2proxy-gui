//! 结束本程序上一次留下的孤儿 tun2proxy 进程。
//!
//! # 为什么必须做
//!
//! 隧道子进程（`tun2proxy-bin.exe`）不是只有一条退出路径：任务管理器结束 GUI、
//! 安装版升级时替换 exe、上次直接崩溃……GUI 一死，子进程就成了孤儿，还攥着那
//! 张 Wintun 网卡和它改过的路由。
//!
//! 而清网卡那段有一条正确、但在这里会咬人的保护：**正在用的网卡不许删**
//! （[`super::adapters`]，判据是 `OperStatus == Up`）。孤儿进程活着，它那张网卡
//! 就是 Up 的，于是：
//!
//! 1. 网卡删不掉（明明没人要了）；
//! 2. 下一次启动要么和旧进程抢同一张网卡、要么和它抢路由，报出来的只有一句
//!    `WintunCreateAdapter failed: 拒绝访问`，用户完全看不出是这个原因。
//!
//! 所以顺序是死的：**先结束进程，等它们真的退出，再清网卡**（[`sweep`] 把这件
//! 事固定在一个地方，别在调用点重排）。反过来做等于什么也没清。
//!
//! # 只认自己的那一个
//!
//! 判据是「映像的完整路径 == 本程序下载的那个 exe」（[`is_orphan`]）：机器上可能
//! 另装了 tun2proxy、也可能有别的 VPN 用同名的 Wintun 服务，按进程名一刀切会
//! 把别人的东西杀掉。查不到路径的进程（跨权限、正在退出）一律不动。

use std::path::Path;

/// 一次清理的结果，调用方拿去写日志。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cleanup {
    /// 结束掉的进程号。
    pub killed: Vec<u32>,
    /// 认出来了但结束不掉的（通常是权限不够：对方是高完整性进程）。
    pub failed: Vec<u32>,
}

/// 启动前的清理编排：先结束孤儿进程，再删残留网卡。
///
/// 顺序写死在这里是有意的：两步分开调用时，谁先谁后全看调用点的记性，而顺序
/// 反过来（先删网卡）时**不会有任何报错**——只是网卡一张也删不掉，等到下次
/// 启动失败才有人回头查。所以编排做成一个函数，单测盯着它的调用顺序。
pub fn sweep(
    terminate: impl FnOnce() -> Cleanup,
    remove_adapters: impl FnOnce() -> super::adapters::Cleanup,
) -> (Cleanup, super::adapters::Cleanup) {
    let orphans = terminate();
    let adapters = remove_adapters();
    (orphans, adapters)
}

/// 结束孤儿进程；非 Windows 上直接返回空结果。
///
/// Linux 的 tun 设备随进程消失、也没有 Wintun 那种「设备节点留在系统里」的
/// 问题，所以那边不需要这一趟。
#[cfg(windows)]
pub fn terminate(exe: &Path, skip: &[u32]) -> Cleanup {
    imp::terminate(exe, skip)
}

#[cfg(not(windows))]
pub fn terminate(_exe: &Path, _skip: &[u32]) -> Cleanup {
    Cleanup::default()
}

/// 这个候选进程算不算「本程序留下的孤儿」。
///
/// `image` 是查到的完整映像路径（查不到时 `None`）；`skip` 是明确不该动的进程
/// （本程序自己拉起来的子进程）。
fn is_orphan(image: Option<&Path>, expected: &Path, pid: u32, self_pid: u32, skip: &[u32]) -> bool {
    if pid == self_pid || skip.contains(&pid) {
        return false;
    }
    // 查不到路径就不动：可能是别的用户的进程，也可能是正在退出的僵尸。
    let Some(image) = image else {
        return false;
    };
    same_file(image, expected)
}

/// 两个路径是不是同一个可执行文件。
///
/// 不比 `canonicalize`：要清理的进程映像可能**已经被删掉了**（升级替换 exe 之后
/// 的孤儿就属于这种），那时规范化会失败，而我们照样得认出它。
fn same_file(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

/// Windows 的路径比较口径：不区分大小写、`\` 和 `/` 等价、可能带 `\\?\` 前缀。
fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('/', "\\").to_lowercase()
}

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };

    use super::{Cleanup, is_orphan};

    /// 「还活着」的退出码（`STILL_ACTIVE`）。
    const STILL_ACTIVE: u32 = 259;

    /// 等进程真的退出最多花多久。
    ///
    /// `TerminateProcess` 是异步的：进程立刻标记成已退出，但内核回收它持有的
    /// TUN 句柄要一会儿，这期间那张网卡还是 Up 的。等不到也继续往下走——宁可
    /// 网卡这次清不掉，也不能把启动卡在这里。
    const WAIT_LIMIT: Duration = Duration::from_secs(2);
    const WAIT_STEP: Duration = Duration::from_millis(50);

    pub fn terminate(exe: &Path, skip: &[u32]) -> Cleanup {
        let self_pid = std::process::id();
        let mut result = Cleanup::default();

        for (pid, image) in candidates() {
            if !is_orphan(image.as_deref(), exe, pid, self_pid, skip) {
                continue;
            }
            if kill(pid) {
                result.killed.push(pid);
            } else {
                result.failed.push(pid);
            }
        }

        if !result.killed.is_empty() {
            wait_gone(&result.killed);
        }
        result
    }

    /// 本进程能看见的、映像名和 `tun2proxy-bin.exe` 相同的进程，带完整路径。
    ///
    /// 先按**文件名**粗筛，再逐个 `OpenProcess` 拿路径：路径才是判据，但给系统里
    /// 每个进程都开一次句柄不值当。
    fn candidates() -> Vec<(u32, Option<PathBuf>)> {
        let mut out = Vec::new();
        // SAFETY: 快照句柄在函数末尾关闭；`PROCESSENTRY32W` 由 API 填充。
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return out;
        }

        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            let pid = entry.th32ProcessID;
            if pid != 0 && wide_name(&entry.szExeFile).eq_ignore_ascii_case(BIN_NAME) {
                out.push((pid, image_path(pid)));
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }

        // SAFETY: `snapshot` 由上面那次调用分配，只在这里归还。
        unsafe { CloseHandle(snapshot) };
        out
    }

    /// 下载下来的隧道二进制的文件名（`core::binary` 里装的就是这个名字）。
    const BIN_NAME: &str = "tun2proxy-bin.exe";

    fn wide_name(raw: &[u16]) -> String {
        let len = raw.iter().position(|&ch| ch == 0).unwrap_or(raw.len());
        String::from_utf16_lossy(&raw[..len])
    }

    /// 进程映像的完整路径；拿不到（权限不够 / 已经在退出）时 `None`。
    fn image_path(pid: u32) -> Option<PathBuf> {
        // SAFETY: 句柄由 `OpenProcess` 返回，用后即关。
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) } != 0;
        // SAFETY: 同上，只关一次。
        unsafe { CloseHandle(handle) };
        if !ok {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..size as usize],
        )))
    }

    fn kill(pid: u32) -> bool {
        // SAFETY: 句柄由 `OpenProcess` 返回，用后即关。
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let ok = unsafe { TerminateProcess(handle, 1) } != 0;
        // SAFETY: 只关一次。
        unsafe { CloseHandle(handle) };
        ok
    }

    fn wait_gone(pids: &[u32]) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while Instant::now() < deadline {
            if pids.iter().all(|pid| !alive(*pid)) {
                return;
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    fn alive(pid: u32) -> bool {
        // SAFETY: 句柄由 `OpenProcess` 返回，用后即关。
        let handle: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // 打不开就是已经不在了（或者成了我们看不见的进程）。
            return false;
        }
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
        // SAFETY: 只关一次。
        unsafe { CloseHandle(handle) };
        !ok || code == STILL_ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Users\me\AppData\Local\tun2proxy-gui\tun2proxy-bin.exe")
    }

    /// 自己的进程永远不动：清理函数是在自己的进程里跑的，认错一次就把程序自杀了。
    #[test]
    fn never_touches_our_own_process() {
        let image = exe();
        assert!(!is_orphan(Some(&image), &exe(), 7, 7, &[]));
    }

    /// 当前引擎拉着的子进程不是孤儿——停止、重启的那一瞬它还在跑。
    #[test]
    fn skips_the_child_we_are_holding() {
        let image = exe();
        assert!(!is_orphan(Some(&image), &exe(), 42, 7, &[42]));
    }

    /// 机器上另装了一份 tun2proxy：名字一样、路径不同，不能杀。
    #[test]
    fn leaves_another_installation_alone() {
        let other = PathBuf::from(r"C:\tools\tun2proxy\tun2proxy-bin.exe");
        assert!(!is_orphan(Some(&other), &exe(), 42, 7, &[]));
    }

    /// 同一条路径的写法可能不一样：大小写、斜杠方向、`\\?\` 前缀。
    #[test]
    fn recognises_the_same_file_written_differently() {
        let expected = PathBuf::from(r"C:\Users\me\AppData\Local\tun2proxy-gui\tun2proxy-bin.exe");
        for written in [
            r"c:\users\me\appdata\local\tun2proxy-gui\tun2proxy-bin.exe",
            r"C:/Users/me/AppData/Local/tun2proxy-gui/tun2proxy-bin.exe",
            r"\\?\C:\Users\me\AppData\Local\tun2proxy-gui\tun2proxy-bin.exe",
        ] {
            assert!(
                is_orphan(Some(&PathBuf::from(written)), &expected, 42, 7, &[]),
                "{written} 应该认成同一个 exe"
            );
        }
    }

    /// 查不到映像路径时不动：可能是别人的进程，也可能正在退出。
    #[test]
    fn unknown_images_are_left_alone() {
        assert!(!is_orphan(None, &exe(), 42, 7, &[]));
    }

    /// 清理顺序是「先结束进程、再删网卡」。
    ///
    /// 反过来做不会报错，只是网卡一张也删不掉（孤儿进程让它的网卡一直是 Up，
    /// 而 Up 的网卡按设计不删），等到下次启动失败才有人回头查。所以顺序本身
    /// 值得一条测试盯着。
    #[test]
    fn processes_are_ended_before_adapters_are_removed() {
        let events = RefCell::new(Vec::new());
        let (orphans, adapters) = sweep(
            || {
                events.borrow_mut().push("terminate");
                Cleanup {
                    killed: vec![4242],
                    failed: vec![],
                }
            },
            || {
                events.borrow_mut().push("adapters");
                crate::core::adapters::Cleanup::default()
            },
        );
        assert_eq!(*events.borrow(), vec!["terminate", "adapters"]);
        assert_eq!(orphans.killed, vec![4242]);
        assert!(adapters.removed.is_empty());
    }

    /// 上一步没找到孤儿也要继续往下走：网卡清理不是它的后续条件。
    #[test]
    fn adapter_cleanup_runs_even_when_there_was_nothing_to_kill() {
        let events = RefCell::new(Vec::new());
        sweep(
            || {
                events.borrow_mut().push("terminate");
                Cleanup::default()
            },
            || {
                events.borrow_mut().push("adapters");
                crate::core::adapters::Cleanup::default()
            },
        );
        assert_eq!(*events.borrow(), vec!["terminate", "adapters"]);
    }
}
