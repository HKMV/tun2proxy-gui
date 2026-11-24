//! 界面读到的「这一刻的真实状态」。
//!
//! 一张快照，全部来自内核：服务状态、tun2proxy 版本、两路当前速率、两分钟的
//! 流量历史、最近的日志行。界面不自己编数字，也不自己算速率——内核的后台采样
//! 线程每秒写一次 `Shared`，界面每秒读一次。
//!
//! 为什么是「快照」而不是界面直接读 `Shared`：nsh 的绑定每帧都可能重跑，
//! 拿着内核的锁去渲染会把锁的持有时间拉长到整个布局过程；而且很多元素
//! （比如图表里的每一列）只需要同一帧的一份数据，重复加锁只会让数字对不上。

use crate::core::engine::Status;
use crate::core::i18n::t;
use crate::core::logs::LogLine;
use crate::core::traffic::RouteTraffic;

use super::state;

/// 日志面板保留多少行。
///
/// 内核留 2000 行（点「清空」只清内存，文件保留），界面只拿最后 400 行：
/// 日志面板要看的是「刚刚发生了什么」，400 行已经远超一屏；整份 2000 行
/// 每秒克隆一次是白花的分配。
pub const LOG_TAIL: usize = 400;

/// 界面上要显示的一份实时数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// 服务状态。
    pub status: Status,
    /// 已安装的 tun2proxy 版本；还没下载时是 `None`。
    pub binary: Option<String>,
    /// 直连方向当前的上下行字节/秒（`(up, down)`）。
    pub direct: (u64, u64),
    /// 代理方向当前的上下行字节/秒。
    pub proxy: (u64, u64),
    /// 最近两分钟每秒一个点的流量历史（内核裁剪好的定长窗口）。
    pub history: Vec<RouteTraffic>,
    /// 最近的日志行（最后 [`LOG_TAIL`] 行）。
    pub logs: Vec<LogLine>,
    /// 「启动被拦下」的原因：上游探测没通过时由内核写入。
    pub blocked: Option<String>,
    /// 会话名额被打满过（tun2proxy 在日志里报了 `dropping new session`）。
    pub sessions_full: bool,
}

impl Live {
    /// 读一次内核（拿不到引擎时给一份「未启动」的空快照）。
    pub fn now() -> Self {
        let Some(engine) = state::engine() else {
            return Self::empty();
        };
        let status = engine.status();
        let shared = engine.shared();
        let Ok(guard) = shared.lock() else {
            return Self {
                status,
                ..Self::empty()
            };
        };
        let history = guard.traffic.iter().copied().collect::<Vec<_>>();
        let logs = guard
            .logs
            .iter()
            .rev()
            .take(LOG_TAIL)
            .rev()
            .cloned()
            .collect::<Vec<_>>();
        Self {
            status,
            binary: guard.version.clone(),
            direct: (guard.rate.direct.up, guard.rate.direct.down),
            proxy: (guard.rate.proxy.up, guard.rate.proxy.down),
            history,
            logs,
            blocked: guard.start_blocked.clone(),
            sessions_full: guard.sessions_full,
        }
    }

    /// 空快照。
    pub fn empty() -> Self {
        Self {
            status: Status::Stopped,
            binary: None,
            direct: (0, 0),
            proxy: (0, 0),
            history: Vec::new(),
            logs: Vec::new(),
            blocked: None,
            sessions_full: false,
        }
    }

    /// 服务状态的一句话。
    pub fn status_label(&self) -> &'static str {
        match &self.status {
            Status::Stopped => t("未启动", "Stopped"),
            Status::Starting => t("正在启动", "Starting"),
            Status::Running => t("运行中", "Running"),
            Status::Stopping => t("正在停止", "Stopping"),
            Status::Failed(_) => t("启动失败", "Failed"),
        }
    }

    /// 状态行的补充说明：失败原因就摊开写，不藏在日志里。
    pub fn status_detail(&self) -> Option<String> {
        match &self.status {
            Status::Failed(reason) => Some(reason.clone()),
            Status::Running => self
                .binary
                .as_ref()
                .map(|version| format!("tun2proxy {version}")),
            _ => None,
        }
    }

    /// 是否处于运行（或切换中）状态。
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }

    /// 是否正在启动 / 停止：这段时间要立蒙版，按钮点不动。
    ///
    /// 和 [`Self::is_active`] 的区别：运行中也是 active，但那时不该挡着用户。
    pub fn is_busy(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Stopping)
    }

    /// 正在等哪一步——蒙版上那句话。
    pub fn busy_label(&self) -> &'static str {
        match self.status {
            Status::Starting => t("正在启动隧道…", "Starting the tunnel…"),
            Status::Stopping => t("正在停止隧道…", "Stopping the tunnel…"),
            _ => "",
        }
    }

    /// tun2proxy 版本，没下载时显示一句占位。
    pub fn binary_label(&self) -> String {
        self.binary
            .clone()
            .unwrap_or_else(|| t("未安装", "not installed").to_string())
    }

    /// 两路下行速率之和：图表按它定纵轴，两路一起画在同一个刻度上。
    pub fn peak(&self) -> u64 {
        self.history
            .iter()
            .map(|sample| sample.direct.down + sample.proxy.down)
            .max()
            .unwrap_or(0)
    }
}

/// 字节/秒 → `12.3 MB/s` 这种人读的写法。
///
/// 和旧界面的图表标注统一：不足 1 KB 直接给字节，其余按 1024 进制走
/// KB / MB / GB，保留一位小数。
pub fn rate_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B/s")
    } else if value < MB {
        format!("{:.1} KB/s", value / KB)
    } else if value < GB {
        format!("{:.1} MB/s", value / MB)
    } else {
        format!("{:.2} GB/s", value / GB)
    }
}
