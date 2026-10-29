//! 流量采样：读 tun2proxy 子进程的累计 IO 字节数，差分出「每秒速率」。
//!
//! 为什么采样子进程而不是统计网卡：TUN 网卡的收发计数要按网卡名去找，
//! 而网卡名是 tun2proxy 自己起的（还可能是自动分配的），拿不到稳定的句柄。
//! 子进程的 IO 计数器是操作系统直接给的，不需要知道它在跟谁说话——对一个
//! 「把流量从 TUN 转到上游代理」的进程来说，这两个数就是它实际搬了多少字节。
//!
//! 采样只用于界面上的折线图，不做计量：读失败（进程刚起、平台不支持、
//! 权限不足）一律返回 `None`，界面显示「—」而不是报错。

use std::collections::VecDeque;
use std::process::Child;
use std::time::Instant;

/// 折线图保留的采样点数。每秒一个点，60 点 = 最近一分钟。
pub const HISTORY_LEN: usize = 60;

/// 一次采样的瞬时速率（字节/秒）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficSample {
    /// 下行（进程读到的字节）。
    pub down: u64,
    /// 上行（进程写出的字节）。
    pub up: u64,
}

impl TrafficSample {
    /// 这条采样里较大的一个方向，用来定折线图的纵轴。
    pub fn peak(self) -> u64 {
        self.down.max(self.up)
    }
}

/// 采样器：记住上一次的累计值，差分出速率。
///
/// 第一次采样没有参照物，速率按 0 算（否则会把「开机以来的总量」当成
/// 一秒内的流量，折线图开局就是一个巨大的尖峰）。
#[derive(Default)]
pub struct TrafficMeter {
    last: Option<(u64, u64)>,
    last_at: Option<Instant>,
}

impl TrafficMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 采样一次。进程还没起来 / 平台不支持时返回 `None`。
    pub fn sample(&mut self, child: &Child) -> Option<TrafficSample> {
        let (read, written) = process_bytes(child)?;
        let now = Instant::now();
        let sample = match (self.last, self.last_at) {
            (Some((last_read, last_written)), Some(at)) => {
                // 至少按 1ms 算：两次采样撞在同一毫秒会除出天文数字。
                let seconds = now.duration_since(at).as_secs_f64().max(0.001);
                TrafficSample {
                    down: (read.saturating_sub(last_read) as f64 / seconds) as u64,
                    up: (written.saturating_sub(last_written) as f64 / seconds) as u64,
                }
            }
            _ => TrafficSample::default(),
        };
        self.last = Some((read, written));
        self.last_at = Some(now);
        Some(sample)
    }

    /// 忘记上一次的读数：停止 / 重启进程后累计值会归零，不清掉就会算成
    /// 「负数」（被 `saturating_sub` 兜成 0）或者一段虚假的流量。
    pub fn reset(&mut self) {
        self.last = None;
        self.last_at = None;
    }
}

/// 把一条采样推进历史，丢掉最旧的。历史为空时用 0 补齐到 [`HISTORY_LEN`]。
pub fn push(history: &mut VecDeque<TrafficSample>, sample: TrafficSample) {
    history.push_back(sample);
    while history.len() > HISTORY_LEN {
        history.pop_front();
    }
}

#[cfg(windows)]
fn process_bytes(child: &Child) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::{GetProcessIoCounters, IO_COUNTERS};

    let mut counters: IO_COUNTERS = unsafe { std::mem::zeroed() };
    // SAFETY: 句柄来自 `std::process::Child`（spawn 时以 PROCESS_QUERY_INFORMATION
    // 打开），只要 child 还活着就有效；`counters` 是本栈上的可写结构体。
    let ok = unsafe { GetProcessIoCounters(child.as_raw_handle() as _, &mut counters) };
    if ok == 0 {
        return None;
    }
    Some((counters.ReadTransferCount, counters.WriteTransferCount))
}

#[cfg(target_os = "linux")]
fn process_bytes(child: &Child) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(format!("/proc/{}/io", child.id())).ok()?;
    let mut read = None;
    let mut written = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("read_bytes:") {
            read = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("write_bytes:") {
            written = rest.trim().parse().ok();
        }
    }
    Some((read?, written?))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_bytes(_child: &Child) -> Option<(u64, u64)> {
    // macOS 需要 `proc_pid_rusage`（libproc），暂不支持：界面显示「—」。
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_capped_at_the_configured_length() {
        let mut history = VecDeque::new();
        for _ in 0..(HISTORY_LEN + 5) {
            push(&mut history, TrafficSample { down: 1, up: 2 });
        }
        assert_eq!(history.len(), HISTORY_LEN);
    }

    #[test]
    fn peak_picks_the_busier_direction() {
        let sample = TrafficSample { down: 10, up: 99 };
        assert_eq!(sample.peak(), 99);
    }
}
