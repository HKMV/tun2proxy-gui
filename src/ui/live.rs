//! 界面读到的「这一刻的真实状态」。
//!
//! 一张快照，全部来自内核：服务状态、tun2proxy 版本、各路当前速率。界面不
//! 自己编数字——M1 还没有轮询，所以这是**打开窗口那一刻**的值；M2 接上启停
//! 与实时图表后，同一个结构改由计时器每秒刷新。

use crate::core::engine::Status;
use crate::core::i18n::t;

use super::state;

/// 界面上要显示的一份实时数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// 服务状态。
    pub status: Status,
    /// 已安装的 tun2proxy 版本；还没下载时是 `None`。
    pub binary: Option<String>,
    /// 直连方向当前的上下行字节/秒。
    pub direct: (u64, u64),
    /// 代理方向当前的上下行字节/秒。
    pub proxy: (u64, u64),
}

impl Live {
    /// 读一次内核（拿不到引擎时给一份「未启动」的空快照）。
    pub fn now() -> Self {
        let Some(engine) = state::engine() else {
            return Self::empty();
        };
        let status = engine.status();
        let shared = engine.shared();
        let guard = shared.lock().ok();
        let binary = guard.as_ref().and_then(|shared| shared.version.clone());
        let (direct, proxy) = match guard.as_ref() {
            Some(shared) => (
                (shared.rate.direct.up, shared.rate.direct.down),
                (shared.rate.proxy.up, shared.rate.proxy.down),
            ),
            None => ((0, 0), (0, 0)),
        };
        Self {
            status,
            binary,
            direct,
            proxy,
        }
    }

    /// 空快照。
    pub fn empty() -> Self {
        Self {
            status: Status::Stopped,
            binary: None,
            direct: (0, 0),
            proxy: (0, 0),
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

    /// tun2proxy 版本，没下载时显示一句占位。
    pub fn binary_label(&self) -> String {
        self.binary
            .clone()
            .unwrap_or_else(|| t("未安装", "not installed").to_string())
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
