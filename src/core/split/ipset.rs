//! IP 段集合：CIDR 解析、最长前缀查询、区间聚合。
//!
//! 分流判定里「IP 这一半」全靠这里：国内 IP 表是几千条 CIDR，每条连接都要查一次。
//!
//! # 为什么不用 `tproxy_config::IpCidr` 线性扫
//!
//! 那个类型只提供「解析」和「是否包含」，集合侧没有索引。几千条逐个比对在整机
//! 流量下（并发两千条会话、每条至少一次判定）不算卡死，但完全没必要——按前缀
//! 长度分桶建哈希表之后，一次查询最多 32 次（IPv6 128 次）常数级哈希。
//!
//! # 为什么这一层不用任何 crate
//!
//! `core/split` 下的三个模块（本文件、[`super::rules`]、[`super::lists`]）只依赖
//! 标准库，好处有两个：
//!
//! * 脱开 GUI 与平台层单独跑测试，`cargo test` 不必先编译出一整套 GPUI；
//! * `tools/regen-rules`（清单生成工具）可以用 `#[path]` 直接复用同一份源码，
//!   不存在「工具里一份、程序里一份」的漂移。
//!
//! 解析出来的 [`Cidr`] 与 `tproxy_config::IpCidr` 语义一致（网络号 + 前缀长度），
//! 需要转成命令行参数时再在 config 层转一次即可。

use std::collections::HashSet;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 地址位数。
pub const V4_BITS: u8 = 32;
/// IPv6 地址位数。
pub const V6_BITS: u8 = 128;

/// 一条 IP 网段：网络号 + 前缀长度。
///
/// 构造时一律把主机位清零（`10.1.2.3/8` → `10.0.0.0/8`）。用户手填时几乎总是
/// 想写「这个地址所在的网段」，而不是被拒绝——`tproxy_config::IpCidr` 走的是
/// 严格路线（带主机位直接报错），那是命令行参数该有的严格，不是界面上该有的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cidr {
    V4 { net: u32, len: u8 },
    V6 { net: u128, len: u8 },
}

impl Cidr {
    /// 单个 IPv4 地址（等价于 `/32`）。
    pub fn host_v4(net: u32) -> Self {
        Self::V4 {
            net,
            len: V4_BITS,
        }
    }

    /// IPv4 网段，`len` 会被夹到 `0..=32`。
    pub fn v4(net: u32, len: u8) -> Self {
        let len = len.min(V4_BITS);
        Self::V4 {
            net: mask_v4(net, len),
            len,
        }
    }

    /// IPv6 网段，`len` 会被夹到 `0..=128`。
    pub fn v6(net: u128, len: u8) -> Self {
        let len = len.min(V6_BITS);
        Self::V6 {
            net: mask_v6(net, len),
            len,
        }
    }

    /// 前缀长度。
    pub fn len(self) -> u8 {
        match self {
            Cidr::V4 { len, .. } | Cidr::V6 { len, .. } => len,
        }
    }

    pub fn is_v4(self) -> bool {
        matches!(self, Cidr::V4 { .. })
    }

    /// 这个地址是否落在网段内。
    pub fn contains(self, ip: IpAddr) -> bool {
        match (self, ip) {
            (Cidr::V4 { net, len }, IpAddr::V4(v4)) => mask_v4(u32::from(v4), len) == net,
            (Cidr::V6 { net, len }, IpAddr::V6(v6)) => mask_v6(u128::from(v6), len) == net,
            _ => false,
        }
    }

    /// 解析 `1.2.3.0/24`、`1.2.3.4`、`public-ip-...`/`2001:db8::/32` 这类写法。
    ///
    /// 前缀省略时按主机地址处理（`/32` 或 `/128`）。
    pub fn parse(input: &str) -> Result<Self, CidrError> {
        let text = input.trim();
        if text.is_empty() {
            return Err(CidrError::new(input, "empty"));
        }
        let (addr, len) = match text.split_once('/') {
            Some((addr, len)) => {
                let len: u8 = len
                    .trim()
                    .parse()
                    .map_err(|_| CidrError::new(input, "bad prefix length"))?;
                (addr.trim(), Some(len))
            }
            None => (text, None),
        };
        let ip: IpAddr = addr
            .parse()
            .map_err(|_| CidrError::new(input, "bad address"))?;
        match ip {
            IpAddr::V4(v4) => {
                let len = len.unwrap_or(V4_BITS);
                if len > V4_BITS {
                    return Err(CidrError::new(input, "prefix length out of range"));
                }
                Ok(Self::v4(u32::from(v4), len))
            }
            IpAddr::V6(v6) => {
                let len = len.unwrap_or(V6_BITS);
                if len > V6_BITS {
                    return Err(CidrError::new(input, "prefix length out of range"));
                }
                Ok(Self::v6(u128::from(v6), len))
            }
        }
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Cidr::V4 { net, len } => write!(f, "{}/{}", Ipv4Addr::from(net), len),
            Cidr::V6 { net, len } => write!(f, "{}/{}", Ipv6Addr::from(net), len),
        }
    }
}

fn mask_v4(bits: u32, len: u8) -> u32 {
    if len == 0 {
        0
    } else {
        bits & (u32::MAX << (V4_BITS - len))
    }
}

fn mask_v6(bits: u128, len: u8) -> u128 {
    if len == 0 {
        0
    } else {
        bits & (u128::MAX << (V6_BITS - len))
    }
}

/// CIDR 解析失败。文案只给日志和测试用，界面上的中文由 UI 层自己拼。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CidrError {
    pub input: String,
    pub reason: &'static str,
}

impl CidrError {
    fn new(input: &str, reason: &'static str) -> Self {
        Self {
            input: input.to_string(),
            reason,
        }
    }
}

impl fmt::Display for CidrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "「{}」不是合法的 IP 或 CIDR（{}）", self.input, self.reason)
    }
}

impl std::error::Error for CidrError {}

/// 前缀集合：按前缀长度分桶，查询走最长前缀匹配。
///
/// 内存换时间：3900 条约 1 MB，换来单次查询几十纳秒。
#[derive(Clone, Debug)]
pub struct IpSet {
    /// `v4[i]` 放着所有长度是 `i` 的网络号（已右移到最低位）。
    v4: Vec<HashSet<u32>>,
    /// `v6[i]` 放着所有长度是 `i` 的网络号。
    v6: Vec<HashSet<u128>>,
    count: usize,
}

impl Default for IpSet {
    fn default() -> Self {
        Self::new()
    }
}

impl IpSet {
    pub fn new() -> Self {
        Self {
            v4: vec![HashSet::new(); V4_BITS as usize + 1],
            v6: vec![HashSet::new(); V6_BITS as usize + 1],
            count: 0,
        }
    }

    /// 插入一条网段，返回是否是新条目（重复插入返回 `false`）。
    pub fn insert(&mut self, cidr: Cidr) -> bool {
        let added = match cidr {
            // `len == 0`（整个 v4 空间）移 32 位会 panic，和 IPv6 一样单独处理。
            Cidr::V4 { net, len } => {
                let key = if len == 0 {
                    0
                } else {
                    net >> (V4_BITS - len)
                };
                self.v4[len as usize].insert(key)
            }
            // 同理，len == 0 时移 128 位会 panic。
            Cidr::V6 { net, len } => {
                let key = if len == 0 {
                    0
                } else {
                    net >> (V6_BITS - len)
                };
                self.v6[len as usize].insert(key)
            }
        };
        if added {
            self.count += 1;
        }
        added
    }

    /// 解析并插入，失败时返回原错误（用于导入用户清单）。
    pub fn insert_str(&mut self, text: &str) -> Result<bool, CidrError> {
        let cidr = Cidr::parse(text)?;
        Ok(self.insert(cidr))
    }

    pub fn extend<I: IntoIterator<Item = Cidr>>(&mut self, cidrs: I) {
        for cidr in cidrs {
            self.insert(cidr);
        }
    }

    /// 条目数（去重后）。
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn clear(&mut self) {
        for bucket in &mut self.v4 {
            bucket.clear();
        }
        for bucket in &mut self.v6 {
            bucket.clear();
        }
        self.count = 0;
    }

    /// 命中即返回**最长前缀**那条网段；没命中返回 `None`。
    ///
    /// 返回具体的 CIDR 而不是布尔值，是因为分流器要把「命中了哪条」写进决策
    /// 日志——用户排查「为什么这个站走了代理」时，看到 `public-ip-.../24` 才知道
    /// 是哪条规则干的。
    pub fn lookup(&self, ip: IpAddr) -> Option<Cidr> {
        match ip {
            IpAddr::V4(v4) => self
                .lookup_v4(u32::from(v4))
                .map(|(net, len)| Cidr::V4 { net, len }),
            IpAddr::V6(v6) => self
                .lookup_v6(u128::from(v6))
                .map(|(net, len)| Cidr::V6 { net, len }),
        }
    }

    /// IPv4 版：从最长的前缀往短里试，第一个命中就是最长前缀。
    pub fn lookup_v4(&self, bits: u32) -> Option<(u32, u8)> {
        for len in (0..=V4_BITS).rev() {
            let key = (u64::from(bits) >> (V4_BITS - len)) as u32;
            if self.v4[len as usize].contains(&key) {
                let net = if len == 0 {
                    0
                } else {
                    key << (V4_BITS - len)
                };
                return Some((net, len));
            }
        }
        None
    }

    /// IPv6 版。
    pub fn lookup_v6(&self, bits: u128) -> Option<(u128, u8)> {
        for len in (0..=V6_BITS).rev() {
            let key = if len == 0 { 0 } else { bits >> (V6_BITS - len) };
            if self.v6[len as usize].contains(&key) {
                let net = if len == 0 {
                    0
                } else {
                    key << (V6_BITS - len)
                };
                return Some((net, len));
            }
        }
        None
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.lookup(ip).is_some()
    }
}

// —— 聚合：把一条条「分配记录」变成最小等价的前缀集 ——
//
// APNIC 给的是「起始地址 + 数量」，数量多半不是 2 的幂（比如 1000 个），所以
// 先拆成闭区间、合并相邻区间，再按对齐规则拆回 CIDR。这一套跟 chnroutes 的
// 做法一致：原始几万条记录最后能压到几千条前缀。

/// 合并闭区间（重叠与首尾相接都合），返回按起点升序排列的新区间。
pub fn merge_ranges_v4(ranges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    // 用 u64 做中间值：`last.1 + 1` 在 `255.255.255.255` 上会溢出 u32。
    let mut sorted: Vec<(u64, u64)> = ranges
        .iter()
        .filter(|(start, end)| start <= end)
        .map(|&(start, end)| (u64::from(start), u64::from(end)))
        .collect();
    sorted.sort_unstable();

    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(sorted.len());
    for (start, end) in sorted {
        match merged.last_mut() {
            // 相接（start == last_end + 1）也算连续，这样 /24 + /24 能合成 /23。
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
        .into_iter()
        .map(|(start, end)| (start as u32, end as u32))
        .collect()
}

/// 把一个 IPv4 闭区间拆成**最少条数**的 CIDR。
///
/// 每一步取「当前地址对齐的最大块」和「剩余量允许的最大块」中较小的那个。
pub fn ranges_to_cidrs_v4(start: u32, end: u32) -> Vec<Cidr> {
    let mut out = Vec::new();
    let last = u64::from(end);
    let mut cursor = u64::from(start);
    while cursor <= last {
        // 地址本身的对齐块：末尾 0 的个数（0 可以对齐到整个空间）。
        let by_align = if cursor == 0 {
            1u64 << 32
        } else {
            cursor & cursor.wrapping_neg()
        };
        let remaining = last - cursor + 1;
        let by_size = 1u64 << (63 - remaining.leading_zeros());
        let block = by_align.min(by_size);
        let len = V4_BITS - block.trailing_zeros() as u8;
        out.push(Cidr::v4(cursor as u32, len));
        cursor += block;
    }
    out
}

/// 把「起始地址 + 数量」形式的一批分配记录直接聚合成前缀集。
pub fn aggregate_v4(ranges: &[(u32, u32)]) -> Vec<Cidr> {
    merge_ranges_v4(ranges)
        .into_iter()
        .flat_map(|(start, end)| ranges_to_cidrs_v4(start, end))
        .collect()
}

/// IPv6 的前缀聚合：去重 → 去掉被包含的 → 反复合并同父的两个兄弟。
///
/// IPv6 分配记录给的就是前缀长度（不像 IPv4 给数量），所以不需要区间拆分，
/// 只需要收拾成最简集合。
///
/// 内部统一用「(前缀长度, 右移后的网络号)」做键：右移之后判断「兄弟」就是看
/// 最低位、判断「父级」就是再右移一位，省掉一堆掩码运算，也不容易在移位边界
/// 上踩坑（`/0` 和 `/128` 是两个极端）。
pub fn aggregate_v6(prefixes: &[(u128, u8)]) -> Vec<Cidr> {
    let mut set: HashSet<(u8, u128)> = HashSet::new();
    for &(net, len) in prefixes {
        let len = len.min(V6_BITS);
        let net = mask_v6(net, len);
        set.insert((len, key_of(net, len)));
    }

    // 被包含的直接删掉：已经有 `/32` 的话，里面那条 `/48` 就是多余的。
    let sorted: Vec<(u8, u128)> = {
        let mut v: Vec<(u8, u128)> = set.iter().copied().collect();
        v.sort_unstable();
        v
    };
    let mut keep: HashSet<(u8, u128)> = HashSet::new();
    for (len, key) in sorted {
        // 短前缀先处理，所以「已保留的、更短的前缀是否盖住我」一次就够判断。
        // `/128` 这种极端长度上 `key >> (len - 0)` 会移满 128 位，单独处理。
        let covered = (0..len).any(|cover_len| {
            let candidate = if cover_len == 0 {
                0
            } else {
                key >> (len - cover_len)
            };
            keep.contains(&(cover_len, candidate))
        });
        if !covered {
            keep.insert((len, key));
        }
    }
    set = keep;

    // 反复合并：两个兄弟前缀合成父级，直到没有可合并的为止。
    loop {
        let mut merged = false;
        let grouped: Vec<(u8, u128)> = {
            let mut v: Vec<(u8, u128)> = set.iter().copied().collect();
            v.sort_unstable();
            v
        };
        for (len, key) in grouped {
            if len == 0 || !set.contains(&(len, key)) {
                continue;
            }
            // 只从「低半边」发起，一对兄弟只合一次。
            if key & 1 != 0 {
                continue;
            }
            if set.contains(&(len, key | 1)) {
                set.remove(&(len, key));
                set.remove(&(len, key | 1));
                set.insert((len - 1, key >> 1));
                merged = true;
            }
        }
        if !merged {
            break;
        }
    }

    let mut out: Vec<Cidr> = set
        .into_iter()
        .map(|(len, key)| Cidr::V6 {
            net: net_of(key, len),
            len,
        })
        .collect();
    out.sort_unstable_by_key(|cidr| match *cidr {
        Cidr::V6 { net, len } => (len, net),
        Cidr::V4 { .. } => (0, 0),
    });
    out
}

/// 网络号 → 右移后的键（`/0` 统一成 0，避免移 128 位）。
fn key_of(net: u128, len: u8) -> u128 {
    if len == 0 { 0 } else { net >> (V6_BITS - len) }
}

/// 右移后的键 → 网络号。
fn net_of(key: u128, len: u8) -> u128 {
    if len == 0 {
        0
    } else {
        key << (V6_BITS - len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(text: &str) -> Cidr {
        Cidr::parse(text).expect("测试用例里的网段必须能解析")
    }

    #[test]
    fn parsing_masks_host_bits_and_defaults_the_length() {
        // 用户手填 `10.1.2.3/8` 的意思是「这个地址所在的 /8」，不是报错。
        assert_eq!(v4("10.1.2.3/8"), v4("10.0.0.0/8"));
        // 不写前缀就是单个地址。
        assert_eq!(v4("192.168.1.1").len(), 32);
        assert_eq!(v4(" 192.168.1.1 / 32 "), v4("192.168.1.1"));
        // 前缀越界要报错，而不是悄悄夹到 32。
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("10.0.0.0/-1").is_err());
        assert!(Cidr::parse("").is_err());
        assert!(Cidr::parse("not-an-ip").is_err());
        assert!(Cidr::parse("10.0.0.0/8/8").is_err());
    }

    #[test]
    fn containment_respects_the_prefix_length() {
        let lan = v4("192.168.0.0/16");
        assert!(lan.contains("192.168.1.1".parse().unwrap()));
        assert!(!lan.contains("172.31.0.1".parse().unwrap()));
        // 整个 v4 空间。
        assert!(v4("0.0.0.0/0").contains("203.0.113.9".parse().unwrap()));
    }

    #[test]
    fn lookup_returns_the_longest_prefix_not_the_first_match() {
        let mut set = IpSet::new();
        set.insert(v4("10.0.0.0/8"));
        set.insert(v4("10.1.2.0/24"));

        let hit = set.lookup("10.1.2.9".parse().unwrap()).expect("必须命中");
        assert_eq!(hit, v4("10.1.2.0/24"), "更长的前缀要盖住更短的");
        let hit = set.lookup("10.9.9.9".parse().unwrap()).expect("必须命中");
        assert_eq!(hit, v4("10.0.0.0/8"));
        assert!(set.lookup("172.31.0.1".parse().unwrap()).is_none());
    }

    #[test]
    fn lookup_handles_the_zero_length_prefix() {
        // `/0` 在移位时最容易踩越界，单列出来。
        let mut set = IpSet::new();
        set.insert(v4("0.0.0.0/0"));
        assert!(set.contains("8.8.4.4".parse().unwrap()));
        assert_eq!(set.len(), 1);

        let v6 = Cidr::v6(0, 0);
        let mut set = IpSet::new();
        set.insert(v6);
        assert!(set.contains("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn the_same_prefix_is_only_counted_once() {
        let mut set = IpSet::new();
        assert!(set.insert(v4("10.0.0.0/8")));
        assert!(!set.insert(v4("10.0.0.0/8")));
        assert!(!set.insert(v4("10.255.255.255/8")), "主机位不同但网段相同");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn merging_joins_overlapping_touching_and_unsorted_ranges() {
        let ranges = [
            (0xC0A8_0000, 0xC0A8_00FF), // 192.168.0.0/24
            (0xC0A8_0100, 0xC0A8_01FF), // 紧挨着 → 合起来是 192.168.0.0/23
            (0x0A00_0000, 0x0A00_00FF), // 10.0.0.0/24
            (0x0A00_0080, 0x0A00_00FF), // 被包含
        ];
        let merged = merge_ranges_v4(&ranges);
        assert_eq!(
            merged,
            vec![(0x0A00_0000, 0x0A00_00FF), (0xC0A8_0000, 0xC0A8_01FF)]
        );
        // 首尾相接的两个 /24 合成一个 /23。
        let cidrs: Vec<String> = aggregate_v4(&ranges)
            .iter()
            .map(|cidr| cidr.to_string())
            .collect();
        assert!(cidrs.contains(&v4("192.168.0.0/23").to_string()), "{cidrs:?}");
    }

    #[test]
    fn a_range_is_broken_into_the_fewest_possible_cidrs() {
        // 8 个连续地址且起点对齐到 8 → 一条 /29。
        let cidrs = ranges_to_cidrs_v4(0x0A00_0000, 0x0A00_0007);
        assert_eq!(cidrs.len(), 1);
        assert_eq!(cidrs[0].len(), 29);

        // 起点不对齐就得拆：10.0.0.1 起 2 个地址 → /32 + /32。
        let cidrs = ranges_to_cidrs_v4(0x0A00_0001, 0x0A00_0002);
        assert_eq!(cidrs.len(), 2);

        // 整个 v4 空间是一条 /0，且不会在 u32::MAX 上溢出。
        let cidrs = ranges_to_cidrs_v4(0, u32::MAX);
        assert_eq!(cidrs.len(), 1);
        assert_eq!(cidrs[0].len(), 0);

        let cidrs = ranges_to_cidrs_v4(u32::MAX, u32::MAX);
        assert_eq!(cidrs.len(), 1);
        assert_eq!(cidrs[0].len(), 32);
    }

    #[test]
    fn aggregation_is_lossless_for_the_addresses_it_covers() {
        // 拆开再拼起来，覆盖范围必须一模一样：不能多覆盖（会把境外 IP 判成国内，
        // 那是真正的线上故障），也不能少覆盖。
        let ranges = [(0x0A00_0A01, 0x0A00_0A0A), (0x0A00_0A0B, 0x0A00_0AFF)];
        let cidrs = aggregate_v4(&ranges);
        let mut set = IpSet::new();
        set.extend(cidrs);

        for host in [0x0A00_0A01u32, 0x0A00_0A08, 0x0A00_0A0B, 0x0A00_0AFF] {
            assert!(set.contains(IpAddr::V4(Ipv4Addr::from(host))), "{host:#x}");
        }
        assert!(!set.contains(IpAddr::V4(Ipv4Addr::from(0x0A00_0A00u32))));
        assert!(!set.contains(IpAddr::V4(Ipv4Addr::from(0x0A00_0B00u32))));
    }

    #[test]
    fn ipv6_aggregation_drops_contained_prefixes() {
        let base: u128 = 0x2001_0db8_0000_0000_0000_0000_0000_0000;
        // 有一条 /32 盖着，里面的 /48 就是多余的。
        let cidrs = aggregate_v6(&[(base, 32u8), (base | 1, 48)]);
        assert_eq!(cidrs.len(), 1);
        assert_eq!(cidrs[0].len(), 32);
    }

    #[test]
    fn ipv6_aggregation_merges_sibling_prefixes() {
        // 两个 /63 是兄弟（最低位 0/1），应该合成 /62；再上一级没有兄弟就停。
        let base: u128 = 0x2001_0db8_0000_0000_0000_0000_0000_0000;
        let sibling = base | (1u128 << 65);
        let cidrs = aggregate_v6(&[(base, 63u8), (sibling, 63)]);
        assert_eq!(cidrs.len(), 1, "{cidrs:?}");
        assert_eq!(cidrs[0].len(), 62);

        // 不成对的不许硬凑：差在第 66 位上，不是同一个父级下的兄弟。
        let far = base | (1u128 << 66);
        let cidrs = aggregate_v6(&[(base, 63u8), (far, 63)]);
        assert_eq!(cidrs.len(), 2);
    }
}
