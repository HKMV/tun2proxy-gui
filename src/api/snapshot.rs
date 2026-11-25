//! 「这一刻的真实状态」，界面只读这一份。
//!
//! 数据全部来自内核：服务状态、tun2proxy 版本、两路当前速率、两分钟的流量
//! 历史、最近的日志行。界面不自己编数字，也不自己算速率——内核的采样线程每秒
//! 写一次 `Shared`，界面每秒读一次。
//!
//! # 为什么不直接把内核的类型暴露出去
//!
//! 内核的类型是「给 Rust 用的」：枚举带负载（`Status::Failed(String)`）、字段
//! 是下划线命名、时间用 `SystemTime`。这些东西跨到 JS 侧全是解析负担，而且以后
//! 内核想改自己的形状会被界面绑死。所以这里定义**面向界面的形状**：枚举摊平成
//! 字符串、字段驼峰、能算的（进度、峰值）后端算好。内核改内部结构时只动这一层
//! 的转换，`.tsx` 一行不用改。

use serde::Serialize;

use crate::core::engine::{Status, Update};
use crate::core::logs::{LogKind, LogLine};

use super::state;

/// 日志面板保留多少行。
///
/// 内核留 2000 行（点「清空」只清内存，文件保留），界面只拿最后 400 行：
/// 日志面板要看的是「刚刚发生了什么」，400 行已经远超一屏；整份 2000 行
/// 每秒克隆一次是白花的分配。
pub const LOG_TAIL: usize = 400;

/// 一个方向的瞬时速率（字节/秒）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    pub up: u64,
    pub down: u64,
}

/// 流量历史里的一个点（每秒一个）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub direct: Rates,
    pub proxy: Rates,
}

impl HistoryPoint {
    /// 两路下行之和：图表按它定纵轴，两条曲线共用一个刻度。
    pub fn down_total(&self) -> u64 {
        self.direct.down + self.proxy.down
    }
}

/// 日志列表里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// `output`（tun2proxy 子进程）/ `app`（本程序）/ `split`（分流判定）/ `other`。
    pub kind: &'static str,
    pub text: String,
}

impl LogEntry {
    fn of(line: &LogLine) -> Self {
        Self {
            kind: match line.kind {
                LogKind::Output => "output",
                LogKind::App => "app",
                LogKind::Split => "split",
                LogKind::Other => "other",
            },
            text: line.text.clone(),
        }
    }
}

/// 分流器的运行情况。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitInfo {
    /// 分流器在不在跑。
    pub running: bool,
    /// 当前生效的规则条数。
    pub rules: usize,
    /// 本次分流到直连的连接数。
    pub direct: u64,
    /// 本次分流到代理的连接数。
    pub proxy: u64,
    /// 判定失败的连接数。
    pub failed: u64,
    /// 经过分流器的会话总数。
    pub sessions: u64,
}

/// 版本检查 / 下载安装的进度。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// 正在检查 / 下载 / 安装。
    pub busy: bool,
    /// 下载进度 0..=1；不在下载阶段或服务端没给总长时是 `None`
    /// （界面这时显示「已下载 3.2 MB」，而不是一条假装在走的进度条）。
    pub progress: Option<f32>,
    /// 已经下载了多少字节（不在下载阶段时为 0）。
    pub downloaded: u64,
    /// 这一档的一句话（「已是最新」「发现 v0.8.8」…）。
    pub text: Option<String>,
}

/// 界面一帧要用的全部实时数据。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// `stopped` / `starting` / `running` / `stopping` / `failed`。
    pub status: &'static str,
    /// 状态的一句话（已按界面语言翻好）。
    pub status_label: &'static str,
    /// 补充说明：失败原因，或者运行中时的 `tun2proxy 0.8.7`。
    pub detail: Option<String>,
    /// 运行中或正在切换（侧栏按钮显示「停止」）。
    pub active: bool,
    /// 正在启动 / 停止（这段要立蒙版，按钮点不动）。
    pub busy: bool,
    /// 正在等哪一步——蒙版上那句话。
    pub busy_label: &'static str,
    /// 本机的 tun2proxy 装没装。
    pub installed: bool,
    /// 已安装的 tun2proxy 版本。
    pub version: Option<String>,
    /// 「启动被拦下」的原因（上游探测没通过时由内核写入）。
    pub blocked: Option<String>,
    /// 会话名额被打满过（tun2proxy 在日志里报了 `dropping new session`）。
    pub sessions_full: bool,
    pub direct: Rates,
    pub proxy: Rates,
    /// 最近两分钟每秒一个点，旧 → 新。
    pub history: Vec<HistoryPoint>,
    /// 最后 [`LOG_TAIL`] 行，旧 → 新。
    pub logs: Vec<LogEntry>,
    /// 累计写入过多少行（不随 `logs` 的裁剪回退）。
    ///
    /// 日志面板要靠它判断「有没有新行」：`logs.len()` 涨到 [`LOG_TAIL`] 就不再
    /// 变，用它的话「跟随最新」会停在半路，看起来就是「日志不滚了」。
    pub log_total: u64,
    pub split: SplitInfo,
    pub update: UpdateInfo,
}

impl Snapshot {
    /// 读一次内核（拿不到引擎时给一份「未启动」的空快照）。
    pub fn now() -> Self {
        let Some(engine) = state::engine() else {
            return Self::default();
        };
        let status = engine.status();
        let shared = engine.shared();
        let Ok(guard) = shared.lock() else {
            return Self {
                ..Self::of_status(&status)
            };
        };
        let history = guard
            .traffic
            .iter()
            .map(|sample| HistoryPoint {
                direct: Rates {
                    up: sample.direct.up,
                    down: sample.direct.down,
                },
                proxy: Rates {
                    up: sample.proxy.up,
                    down: sample.proxy.down,
                },
            })
            .collect::<Vec<_>>();
        let logs = guard
            .logs
            .iter()
            .rev()
            .take(LOG_TAIL)
            .rev()
            .map(LogEntry::of)
            .collect::<Vec<_>>();
        Self {
            version: guard.version.clone(),
            update: UpdateInfo::of(&guard.update),
            direct: Rates {
                up: guard.rate.direct.up,
                down: guard.rate.direct.down,
            },
            proxy: Rates {
                up: guard.rate.proxy.up,
                down: guard.rate.proxy.down,
            },
            history,
            logs,
            log_total: guard.log_total,
            blocked: guard.start_blocked.clone(),
            sessions_full: guard.sessions_full,
            split: SplitInfo {
                running: guard.split.running,
                rules: guard.split.rules,
                direct: guard.split.direct,
                proxy: guard.split.proxy,
                failed: guard.split.failed,
                sessions: guard.split.sessions,
            },
            ..Self::of_status(&status)
        }
    }

    /// 只带状态的那几个字段（读不到内核数据时也至少说清「在干什么」）。
    fn of_status(status: &Status) -> Self {
        Self {
            status: match status {
                Status::Stopped => "stopped",
                Status::Starting => "starting",
                Status::Running => "running",
                Status::Stopping => "stopping",
                Status::Failed(_) => "failed",
            },
            status_label: status_label(status),
            detail: match status {
                Status::Failed(reason) => Some(reason.clone()),
                _ => None,
            },
            active: status.is_active(),
            busy: matches!(status, Status::Starting | Status::Stopping),
            busy_label: match status {
                Status::Starting => crate::core::i18n::t("正在启动隧道…", "Starting the tunnel…"),
                Status::Stopping => crate::core::i18n::t("正在停止隧道…", "Stopping the tunnel…"),
                _ => "",
            },
            ..Self::default()
        }
    }

    /// 运行中时把版本号补进 `detail`（内核数据读完之后才知道装的是哪一版）。
    pub fn with_version_detail(mut self) -> Self {
        if self.status == "running" && self.detail.is_none() {
            self.detail = self
                .version
                .as_ref()
                .map(|version| format!("tun2proxy {version}"));
        }
        self
    }

    /// 两路下行速率里的峰值：图表按它定纵轴。
    pub fn peak(&self) -> u64 {
        self.history
            .iter()
            .map(HistoryPoint::down_total)
            .max()
            .unwrap_or(0)
    }
}

fn status_label(status: &Status) -> &'static str {
    use crate::core::i18n::t;
    match status {
        Status::Stopped => t("未启动", "Stopped"),
        Status::Starting => t("正在启动", "Starting"),
        Status::Running => t("运行中", "Running"),
        Status::Stopping => t("正在停止", "Stopping"),
        Status::Failed(_) => t("启动失败", "Failed"),
    }
}

impl UpdateInfo {
    fn of(update: &Update) -> Self {
        use crate::core::i18n::t;
        use crate::tr_format;
        let (busy, progress, downloaded, text) = match update {
            Update::Idle => (false, None, 0, None),
            Update::Checking => (true, None, 0, Some(t("正在检查更新…", "Checking…").to_string())),
            Update::Available { latest } => (
                false,
                None,
                0,
                Some(tr_format!("发现新版本 v{}", "v{} available", latest)),
            ),
            Update::Latest { version } => (
                false,
                None,
                0,
                Some(tr_format!("已是最新 v{}", "up to date (v{})", version)),
            ),
            Update::Downloading { got, total } => (
                true,
                total
                    .filter(|total| *total > 0)
                    .map(|total| (*got as f32 / total as f32).clamp(0.0, 1.0)),
                *got,
                Some(tr_format!("正在下载 {}", "Downloading {}", bytes_text(*got))),
            ),
            Update::Installing => (true, None, 0, Some(t("正在安装…", "Installing…").to_string())),
            Update::Done { version } => (
                false,
                None,
                0,
                Some(tr_format!("已安装 v{}", "Installed v{}", version)),
            ),
            Update::Failed(reason) => (false, None, 0, Some(reason.clone())),
        };
        Self {
            busy,
            progress,
            downloaded,
            text,
        }
    }
}

/// 字节/秒 → `12.3 MB/s` 这种人读的写法。
///
/// 不足 1 KB 直接给字节，其余按 1024 进制走 KB / MB / GB，保留一位小数。
pub fn rate_text(bytes: u64) -> String {
    format!("{}/s", bytes_text(bytes))
}

/// 字节数 → `12.3 MB`（不带 `/s`）。
///
/// 下载进度用的是它：那是「已经下了多少」，不是速率。
pub fn bytes_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else if value < GB {
        format!("{:.1} MB", value / MB)
    } else {
        format!("{:.2} GB", value / GB)
    }
}
