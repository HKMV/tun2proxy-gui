//! 流量采样：读 TUN 网卡的收发计数，差分出「每秒速率」。
//!
//! 为什么读网卡而不是读子进程的 IO 计数：Windows 上 Wintun 的收发走的是共享
//! 内存环形缓冲，不经过 `ReadFile`/`WriteFile`，所以 `GetProcessIoCounters` 的
//! 读方向永远是 0——换上真 tun2proxy 之后「下行」会一直贴在 0 上（之前用假替身
//! 时看不出来，那是替身自己的计数）。网卡的收发计数由驱动层记账，两个方向都准，
//! 而且方向语义明确：`InOctets` 是隧道递给系统的字节（下行），`OutOctets` 是
//! 系统塞进隧道的字节（上行）。实测一次 192 KB 下载：`received +162001`、
//! `sent +2295`。
//!
//! 网卡怎么找：按名字。名字不是猜的——启动时由引擎定好（用户填的，或本次随机
//! 生成的），再用同一个字符串传给 tun2proxy 的 `--tun` 和这里的采样器，不存在
//! 「它自己起了个名、我们去找」的问题。Windows 上 tun2proxy 用这个名字建 Wintun
//! 适配器，Linux 上就是接口名。
//!
//! 找不到就返回 `None`，界面显示「—」——宁可空着，也不要拿别的网卡（机器上可能
//! 还躺着别的 Wintun 适配器）冒充隧道流量。
//!
//! 采样只用于界面上的折线图，不做计量：读失败（进程刚起、网卡还没建出来、
//! 平台不支持、权限不足）一律返回 `None`，界面显示「—」而不是报错。

use std::collections::VecDeque;
use std::process::Child;
use std::time::Instant;

/// 折线图保留的采样点数。每秒一个点，120 点 = 最近两分钟。
///
/// 必须和分流趋势图的窗口一样长（`TREND_BUCKET_SECS * TREND_BUCKETS`）：
/// 两张图并排看的时候，「流量高峰」和「分流高峰」要落在同一段时间里，
/// 窗口长短不一就会读成「流量早就掉下来了，分流还在涨」。下面有单测锁住。
pub const HISTORY_LEN: usize = 120;

/// 一次采样的瞬时速率（字节/秒）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficSample {
    /// 下行（隧道递给系统的字节，即下载方向）。
    pub down: u64,
    /// 上行（系统塞进隧道的字节，即上传方向）。
    pub up: u64,
}

impl TrafficSample {
    /// 这条采样里较大的一个方向，用来定折线图的纵轴。
    pub fn peak(self) -> u64 {
        self.down.max(self.up)
    }
}

/// 网卡表里的一行，抽出来是为了让「挑网卡」这段逻辑能写单测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterRow {
    /// 系统里显示的网卡名（Windows 的 `Alias`，Linux 的接口名）。
    pub name: String,
    /// 驱动描述。Wintun 建出来的适配器是「<名字> Tunnel」。
    pub description: String,
    /// 系统给的接口索引，用来把「持有某个地址的网卡」和这一行对上。
    pub if_index: u32,
    pub up: bool,
    /// 驱动收到的字节数（下行）。
    pub in_octets: u64,
    /// 驱动发出的字节数（上行）。
    pub out_octets: u64,
}

impl AdapterRow {
    /// 名字是否就是这个网卡。
    ///
    /// 先比网卡名，再比「<名字> Tunnel」这种描述：Windows 上 `Alias` 和
    /// `Description` 都由建适配器时的名字决定，但不同版本 Wintun 填哪个字段
    /// 不完全一致，两个都认一下更稳。描述按后缀剥而不是按第一个空格切，网卡名
    /// 里带空格（用户自己填的）也能对上。
    fn matches(&self, name: &str) -> bool {
        if self.name.eq_ignore_ascii_case(name) {
            return true;
        }
        let description = self.description.trim();
        match description.strip_suffix(" Tunnel") {
            Some(head) => head.eq_ignore_ascii_case(name),
            None => description.eq_ignore_ascii_case(name),
        }
    }
}

/// 从网卡表里挑出隧道网卡。同名多张时优先挑在用的那张。
pub fn pick_adapter<'a>(rows: &'a [AdapterRow], name: &str) -> Option<&'a AdapterRow> {
    let mut fallback = None;
    for row in rows.iter().filter(|row| row.matches(name)) {
        if row.up {
            return Some(row);
        }
        fallback.get_or_insert(row);
    }
    fallback
}

/// 采样器：记住上一次的累计值，差分出速率。
///
/// 第一次采样没有参照物，速率按 0 算（否则会把「开机以来的总量」当成
/// 一秒内的流量，折线图开局就是一个巨大的尖峰）。
pub struct TrafficMeter {
    /// 隧道网卡名，和传给 tun2proxy 的 `--tun` 是同一个值。
    name: String,
    /// 认出来的隧道网卡索引。认到之后一直用它，省得每秒重找一遍。
    if_index: Option<u32>,
    last: Option<(u64, u64)>,
    last_at: Option<Instant>,
}

impl TrafficMeter {
    /// `tun_name` 必须是引擎实际传给 tun2proxy 的那个名字（用户填的，或本次随机
    /// 生成的）。留空就没有名字可找，采样一律返回 `None`。
    pub fn new(tun_name: &str) -> Self {
        Self {
            name: tun_name.trim().to_string(),
            if_index: None,
            last: None,
            last_at: None,
        }
    }

    /// 采样一次。网卡还没建出来 / 平台不支持时返回 `None`。
    ///
    /// `child` 只给 Linux 上的兜底采样用，Windows 上完全不用它。
    pub fn sample(&mut self, child: Option<&Child>) -> Option<TrafficSample> {
        let (down_total, up_total) = self.counters(child)?;
        let now = Instant::now();
        let sample = match (self.last, self.last_at) {
            (Some((last_down, last_up)), Some(at)) => {
                // 至少按 1ms 算：两次采样撞在同一毫秒会除出天文数字。
                let seconds = now.duration_since(at).as_secs_f64().max(0.001);
                TrafficSample {
                    down: (down_total.saturating_sub(last_down) as f64 / seconds) as u64,
                    up: (up_total.saturating_sub(last_up) as f64 / seconds) as u64,
                }
            }
            _ => TrafficSample::default(),
        };
        self.last = Some((down_total, up_total));
        self.last_at = Some(now);
        Some(sample)
    }

    /// 忘记上一次的读数：停止 / 重启后网卡会被重建，累计值从头开始，
    /// 不清掉就会算成「负数」（被 `saturating_sub` 兜成 0）或者一段虚假的流量。
    pub fn reset(&mut self) {
        self.last = None;
        self.last_at = None;
        self.if_index = None;
    }

    /// 读隧道网卡的累计收发字节，返回 `(下行, 上行)`。
    #[cfg(windows)]
    fn counters(&mut self, _child: Option<&Child>) -> Option<(u64, u64)> {
        if self.name.is_empty() {
            return None;
        }
        let rows = windows_adapters()?;

        // 之前认过的网卡还在就接着用，省得每秒把整张表筛一遍。
        if let Some(index) = self.if_index {
            if let Some(row) = rows.iter().find(|row| row.if_index == index) {
                return Some((row.in_octets, row.out_octets));
            }
            // 隧道重启过，网卡换了，重新认。
            self.if_index = None;
        }

        let row = pick_adapter(&rows, &self.name)?;
        self.if_index = Some(row.if_index);
        Some((row.in_octets, row.out_octets))
    }

    #[cfg(target_os = "linux")]
    fn counters(&mut self, child: Option<&Child>) -> Option<(u64, u64)> {
        if !self.name.is_empty() {
            let read = |file: &str| -> Option<u64> {
                std::fs::read_to_string(format!(
                    "/sys/class/net/{}/statistics/{file}",
                    self.name
                ))
                .ok()?
                .trim()
                .parse()
                .ok()
            };
            // rx_bytes 是网卡收到的（下行），tx_bytes 是网卡发出的（上行）。
            if let (Some(rx), Some(tx)) = (read("rx_bytes"), read("tx_bytes")) {
                return Some((rx, tx));
            }
        }
        // 名字找不到（比如用了 `--unshare`，隧道在另一个命名空间里）时退回进程
        // IO 计数：tun 设备走 read/write，这个数至少是有意义的。
        child.and_then(process_bytes)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn counters(&mut self, _child: Option<&Child>) -> Option<(u64, u64)> {
        // macOS 需要 `proc_pid_rusage`（libproc）或 `getifaddrs` 的 64 位计数，
        // 暂不支持：界面显示「—」。
        None
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
fn windows_adapters() -> Option<Vec<AdapterRow>> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };

    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: `GetIfTable2` 分配表并把指针写进 `table`；失败时返回非 0 且
    // 不写指针。成功时这张表必须用 `FreeMibTable` 归还。
    let status = unsafe { GetIfTable2(&mut table) };
    if status != 0 || table.is_null() {
        return None;
    }

    // SAFETY: 上面已经确认指针非空且由系统分配；行数在 `NumEntries` 里。
    let rows = unsafe {
        let table = &*table;
        let entries = std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize);
        entries
            .iter()
            .map(|entry| AdapterRow {
                name: wide_to_string(&entry.Alias),
                description: wide_to_string(&entry.Description),
                if_index: entry.InterfaceIndex,
                // IF_OPER_STATUS 里 1 就是 Up。
                up: entry.OperStatus == 1,
                in_octets: entry.InOctets,
                out_octets: entry.OutOctets,
            })
            .collect::<Vec<_>>()
    };
    // SAFETY: `table` 是上面 `GetIfTable2` 分配的，只归还一次。
    unsafe { FreeMibTable(table as *const std::ffi::c_void) };
    Some(rows)
}

#[cfg(windows)]
fn wide_to_string(raw: &[u16]) -> String {
    let len = raw.iter().position(|&ch| ch == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..len])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, up: bool, in_octets: u64, out_octets: u64) -> AdapterRow {
        AdapterRow {
            name: name.to_string(),
            description: format!("{name} Tunnel"),
            if_index: 7,
            up,
            in_octets,
            out_octets,
        }
    }

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

    /// 机器上可能躺着别的 Wintun 适配器（掉线残留的），只能按名字挑。
    #[test]
    fn picks_the_adapter_by_name() {
        let name = "t2p3f9c1a04";
        let rows = vec![
            row("stale-tunnel", true, 9_000, 900),
            row(name, true, 1_000, 200),
            row("本地连接 2", false, 5, 5),
        ];
        let picked = pick_adapter(&rows, name).expect("应该按名字挑到隧道网卡");
        assert_eq!(picked.in_octets, 1_000);
        assert_eq!(picked.out_octets, 200);
    }

    /// 网卡还没建出来时不能瞎挑一张，否则图上画的是别人的流量。
    #[test]
    fn missing_adapter_is_not_guessed() {
        let rows = vec![row("stale-tunnel", true, 9_000, 900)];
        assert!(pick_adapter(&rows, "t2p3f9c1a04").is_none());
    }

    /// 名字大小写不该影响匹配（配置里可能填成 TUN2PROXY）。
    #[test]
    fn name_match_ignores_case() {
        let rows = vec![row("wintun", true, 7, 3)];
        assert!(pick_adapter(&rows, "WinTun").is_some());
    }

    /// 网卡名里带空格时，描述「<名字> Tunnel」也要能剥干净对上。
    #[test]
    fn name_match_handles_spaces() {
        let rows = vec![row("my tunnel", true, 7, 3)];
        assert!(pick_adapter(&rows, "my tunnel").is_some());
        // 只比描述、不比名字的那种行（Windows 上 Alias 被系统改了名）也要能对上。
        let rows = vec![AdapterRow {
            name: "本地连接 6".to_string(),
            description: "my tunnel Tunnel".to_string(),
            if_index: 9,
            up: true,
            in_octets: 1,
            out_octets: 2,
        }];
        assert!(pick_adapter(&rows, "my tunnel").is_some());
    }

    /// 同名两张时优先在用的那张：掉线残留的旧适配器计数是死的。
    #[test]
    fn prefers_the_adapter_that_is_up() {
        let name = "t2p3f9c1a04";
        let rows = vec![row(name, false, 1, 1), row(name, true, 500, 300)];
        let picked = pick_adapter(&rows, name).expect("挑到在用的那张");
        assert!(picked.up);
        assert_eq!(picked.in_octets, 500);
    }

    /// 随机名要满足两个条件：每次都不一样（否则固定下来又变成撞名），
    /// 长度在 Linux 的 `IFNAMSIZ` 之内。
    #[test]
    fn random_names_are_unique_and_short_enough() {
        let names: Vec<String> = (0..8)
            .map(|_| crate::core::config::random_tun_name())
            .collect();
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "随机名不该重复：{names:?}");
        for name in &names {
            assert!(name.starts_with(crate::core::config::TUN_NAME_PREFIX));
            assert!(name.chars().count() <= crate::core::config::TUN_NAME_MAX);
            assert!(name.chars().all(|ch| ch.is_ascii_alphanumeric()));
        }
    }

    /// 引擎没定名字（理论上不会发生）时，采样器不能拿空名字去匹配整张网卡表。
    #[test]
    fn an_empty_name_never_matches_an_adapter() {
        let rows = vec![row("t2p3f9c1a04", true, 1, 2)];
        assert!(pick_adapter(&rows, "").is_none());
    }

    /// 两张图的时间窗必须一样长：仪表盘上它们是上下并排的，窗口一长一短，
    /// 同一列像素代表的就不是同一段真实时间。
    #[test]
    fn window_matches_the_split_trend() {
        let split_secs = crate::core::split::TREND_BUCKET_SECS
            * u64::try_from(crate::core::split::TREND_BUCKETS).expect("格数是小整数");
        assert_eq!(
            HISTORY_LEN as u64, split_secs,
            "流量图 {HISTORY_LEN} 点 × 1 秒 与分流图 {split_secs} 秒对不上"
        );
    }
}
