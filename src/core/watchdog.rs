//! 界面线程卡顿探针。
//!
//! 症状：窗口还在、任务管理器里 `Responding=True`、CPU 也不高，但点哪儿都没
//! 反应。这种「假死」很难从外面判断——`IsHungAppWindow` 只统计消息队列是否还在
//! 回答 `WM_NULL`，而界面线程卡在某个处理器里时，外部的探测往往看不出异常。
//!
//! 这里提供一个极轻量的心跳：界面线程进入一段可能有风险的代码前后各打一个
//! 标记，后台线程每 500ms 看一次，同一段代码停留超过 [`STALL_LIMIT`] 就把
//! **卡在哪一段**写进 `<数据目录>/stall.log`。下次再假死，直接看名字就知道是
//! 哪个处理器/哪条路径卡住，不用再猜。
//!
//! 成本：进入/离开各一次原子写，没有锁，也不分配内存。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// 超过这个时长就算「卡住了」。
///
/// 3 秒：正常操作里最慢的是落盘与子进程收尾，都在百毫秒级；超过 3 秒用户已经
/// 会开始重复点击了。
const STALL_LIMIT: Duration = Duration::from_secs(3);

/// 段落编号 → 名字。下标即编号，0 表示空闲。
const SECTIONS: &[&str] = &[
    "idle",
    "Msg::Start",
    "Msg::Stop",
    "Msg::SaveConfig",
    "Msg::CheckProxy",
    "Msg::CheckUpdate",
    "Msg::InstallUpdate",
    "Msg::RelaunchElevated",
    "Msg::CloseRequested",
    "Msg::CloseDialog",
    "Msg::MinimizeToTray",
    "view",
    "snapshot",
    "update",
    "other",
];

/// 一个可被观察的段落。
#[derive(Clone, Copy)]
pub struct Section(u64);

impl Section {
    const fn at(index: usize) -> Self {
        Section(index as u64)
    }

    /// 名字表里的可读名。
    pub fn name(self) -> &'static str {
        SECTIONS.get(self.0 as usize).copied().unwrap_or("other")
    }
}

pub const MSG_START: Section = Section::at(1);
pub const MSG_STOP: Section = Section::at(2);
pub const MSG_SAVE_CONFIG: Section = Section::at(3);
pub const MSG_CHECK_PROXY: Section = Section::at(4);
pub const MSG_CHECK_UPDATE: Section = Section::at(5);
pub const MSG_INSTALL_UPDATE: Section = Section::at(6);
pub const MSG_RELAUNCH_ELEVATED: Section = Section::at(7);
pub const MSG_CLOSE_REQUESTED: Section = Section::at(8);
pub const MSG_CLOSE_DIALOG: Section = Section::at(9);
pub const MSG_MINIMIZE_TO_TRAY: Section = Section::at(10);
pub const VIEW: Section = Section::at(11);
pub const OTHER: Section = Section::at(14);

/// 当前正在执行的段落编号。0 表示空闲。
static CURRENT: AtomicU64 = AtomicU64::new(0);
/// 进入当前段落的时刻（进程启动以来的毫秒数）。
static SINCE: AtomicU64 = AtomicU64::new(0);

/// 标记「开始执行某段代码」，返回的守卫离开作用域时自动清除。
///
/// 用 RAII 而不是手工配对 `enter`/`leave`：中途提前返回、panic 展开都能正确
/// 清除，不会留下一个永远「正在执行」的假状态。
pub fn enter(section: Section) -> Guard {
    SINCE.store(now_ms(), Ordering::Relaxed);
    CURRENT.store(section.0, Ordering::Relaxed);
    Guard
}

/// 离开段落。
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.store(0, Ordering::Relaxed);
    }
}

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// 启动看门狗线程。重复调用无副作用。
pub fn install() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("ui-watchdog".to_string())
            .spawn(|| {
                // 已经报过的段落：同一次卡顿只写一行。
                let mut reported = 0u64;
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    let id = CURRENT.load(Ordering::Relaxed);
                    if id == 0 {
                        reported = 0;
                        continue;
                    }
                    if reported == id {
                        continue;
                    }
                    let since = SINCE.load(Ordering::Relaxed);
                    if now_ms().saturating_sub(since) < STALL_LIMIT.as_millis() as u64 {
                        continue;
                    }
                    reported = id;
                    let line = format!(
                        "{} 界面线程在 `{}` 里停留超过 {}s\n",
                        timestamp(),
                        Section(id).name(),
                        STALL_LIMIT.as_secs()
                    );
                    append(&line);
                }
            });
    });
}

/// 追加到 `<数据目录>/stall.log`。
///
/// 写文件失败就算了：探针本身不能成为新的故障源。
fn append(line: &str) {
    use std::io::Write;
    let path = crate::core::paths::app_dir().join("stall.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_readable_name() {
        assert_eq!(MSG_START.name(), "Msg::Start");
        assert_eq!(MSG_CLOSE_DIALOG.name(), "Msg::CloseDialog");
        assert_eq!(VIEW.name(), "view");
        assert_eq!(OTHER.name(), "other");
    }

    #[test]
    fn guard_clears_the_current_section_on_drop() {
        {
            let _guard = enter(MSG_START);
            assert_eq!(CURRENT.load(Ordering::Relaxed), MSG_START.0);
        }
        assert_eq!(CURRENT.load(Ordering::Relaxed), 0);
    }
}
