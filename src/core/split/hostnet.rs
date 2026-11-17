//! 本机网卡直连网段：把「这台机器自己所在的网络」也算进本地网络。
//!
//! 内置的 [`super::builtin`] 那 12 条保留段只够覆盖常规内网。现实里还有一类地址
//! 是写不进常量表的：
//!
//! * 企业 VPN（深信服 aTrust / SDP、EasyConnect 之类）给客户端网卡分的是**公网
//!   样式的地址**，段还不固定，换个网关就变；
//! * 认证 / 准入服务往往就挂在这张 VPN 网卡的同一网段里——登录握手一旦被送进
//!   上游代理就再也连不上（那是加密隧道，代理转发不了），而用户没有任何地方
//!   「提前」知道该把哪个段写成规则。
//!
//! 所以构建规则集时探一次本机网卡，把每张 Up 网卡的网段加成直连规则，来源写
//! [`SOURCE`]。已经被内置保留段盖住的地址不再重复加（`10/8` 之类），本程序自己
//! 建的隧道网卡也跳过（那是 TUN，不是「本地网络」）。
//!
//! 探测失败一律当成「没有额外网段」：这是兜底增强，不能影响隧道启动。
//!
//! # 平台
//!
//! 目前只有 Windows 有实现（`GetAdaptersAddresses`）。其它平台返回空=只剩内置
//! 保留段；那边的内网认证本来就走私有段，要补也应该是另接 `getifaddrs`，不能
//! 靠在这台机器上盲写。

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::builtin;
use super::ipset::Cidr;
use super::rules::{Action, Matcher, Rule};

/// 这些规则在界面上显示的来源。
pub const SOURCE: &str = "本机网卡";

/// 一张网卡上的一个地址：地址本身 + 前缀长度（`10.1.2.3/8` 里的 8）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostNet {
    pub addr: IpAddr,
    pub prefix: u8,
}

/// 探测本机网卡，转成「直连」规则。
pub fn rules() -> Vec<Rule> {
    collect(&probe(), &builtin::reserved_ranges())
}

/// 把探测结果转成规则。
///
/// 三道过滤：内置保留段已经盖住的不加（否则界面上「本地网络」里会冒出一串被
/// `10/8` 盖住的重复行）、`/0` 不加（那等于把整机流量拽成直连）、同一个段只加一次。
fn collect(hosts: &[HostNet], reserved: &[Cidr]) -> Vec<Rule> {
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for host in hosts {
        if reserved.iter().any(|range| range.contains(host.addr)) {
            continue;
        }
        let cidr = match host.addr {
            IpAddr::V4(addr) => Cidr::v4(u32::from(addr), host.prefix),
            IpAddr::V6(addr) => Cidr::v6(u128::from(addr), host.prefix),
        };
        if cidr.len() == 0 {
            continue;
        }
        if !seen.insert(cidr) {
            continue;
        }
        rules.push(Rule::new(Action::Direct, Matcher::Cidr(cidr), SOURCE));
    }
    rules
}

/// 这张网卡的名字是不是本程序自己建的隧道（`t2p_xxxxxxxx`）。
///
/// `t2p_` 这个字面量来自 [`crate::core::config::TUN_NAME_PREFIX`] 与
/// [`crate::core::config::TUN_NAME_SEPARATOR`]，下面的 `debug_assert` 把它钉住：
/// 常量一改，测试就会在这里报出来，而不是悄悄漏掉一张隧道网卡。
#[cfg(windows)]
fn is_our_tunnel(name: *const u16) -> bool {
    debug_assert_eq!(
        format!(
            "{}{}",
            crate::core::config::TUN_NAME_PREFIX,
            crate::core::config::TUN_NAME_SEPARATOR
        ),
        "t2p_",
        "内置隧道的名字前缀变了，这里的逐字符比较要跟着改"
    );
    if name.is_null() {
        return false;
    }
    let expected = ['t' as u16, '2' as u16, 'p' as u16, '_' as u16];
    // SAFETY: `name` 是系统给的、以 0 结尾的宽字符串；前缀这 4 个字符要么在里面，
    // 要么在 0 处停下（短名字），读到的都是这张缓冲区里的字节。
    unsafe { expected.iter().enumerate().all(|(i, want)| *name.add(i) == *want) }
}

/// 从 `SOCKET_ADDRESS` 里取出地址与前缀长度。
///
/// 直接按 `sockaddr` 的字节布局读，不去碰 `windows-sys` 里那些 union：两种
/// `sockaddr` 的布局是所有平台一致的——`sockaddr_in` 是
/// `family(2) + port(2) + addr(4)`，`sockaddr_in6` 是
/// `family(2) + port(2) + flowinfo(4) + addr(16)`。
#[cfg(windows)]
unsafe fn unicast_host(
    address: &windows_sys::Win32::Networking::WinSock::SOCKET_ADDRESS,
    prefix: u8,
) -> Option<HostNet> {
    let sockaddr = address.lpSockaddr;
    if sockaddr.is_null() || address.iSockaddrLength < 8 {
        return None;
    }
    let bytes =
        std::slice::from_raw_parts(sockaddr as *const u8, address.iSockaddrLength as usize);
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
    let addr = match family {
        // AF_INET
        2 if bytes.len() >= 8 => IpAddr::V4(Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7])),
        // AF_INET6
        23 if bytes.len() >= 24 => {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&bytes[8..24]);
            IpAddr::V6(Ipv6Addr::from(raw))
        }
        _ => return None,
    };
    Some(HostNet { addr, prefix })
}

#[cfg(windows)]
fn probe() -> Vec<HostNet> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };

    /// `GetAdaptersAddresses` 的「缓冲区不够」返回码。
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    /// `AF_UNSPEC`：IPv4 与 IPv6 一起要。
    const AF_UNSPEC: u32 = 0;

    // 缓冲区用 `u64` 分配：`IP_ADAPTER_ADDRESSES_LH` 里有指针和 `u64` 字段，
    // `Vec<u8>` 的对齐（1）不保证够，拿它当结构体缓冲区是未定义行为。
    fn buffer(size: u32) -> Vec<u64> {
        vec![0u64; (size as usize).div_ceil(8)]
    }

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut words = buffer(size);
    // SAFETY: 缓冲区按 `size` 字节准备好，长度通过 `&mut size` 交给系统写回；
    // 失败时系统不写指针内容，下面按返回码判断。
    let mut status = unsafe {
        GetAdaptersAddresses(
            AF_UNSPEC,
            flags,
            std::ptr::null(),
            words.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
            &mut size,
        )
    };
    if status == ERROR_BUFFER_OVERFLOW {
        words = buffer(size);
        // SAFETY: 同上，这次按系统要求的长度重新分配过。
        status = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                flags,
                std::ptr::null(),
                words.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut size,
            )
        };
    }
    if status != 0 {
        return Vec::new();
    }

    // IF_OPER_STATUS 里 1 是 Up，与 core::traffic 读流量时的判断同一个口径。
    const IF_OPER_STATUS_UP: i32 = 1;

    let mut hosts = Vec::new();
    let mut adapter = words.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    // SAFETY: 整张链表都由系统在我们提供的这部分缓冲区里构造，`Next` 指向缓冲区
    // 内部或为空，不会指向别处。
    unsafe {
        while !adapter.is_null() {
            let current = &*adapter;
            if current.OperStatus == IF_OPER_STATUS_UP && !is_our_tunnel(current.FriendlyName) {
                let mut unicast = current.FirstUnicastAddress;
                while !unicast.is_null() {
                    let entry = &*unicast;
                    if let Some(host) = unicast_host(&entry.Address, entry.OnLinkPrefixLength) {
                        hosts.push(host);
                    }
                    unicast = entry.Next;
                }
            }
            adapter = current.Next;
        }
    }
    hosts
}

/// 非 Windows：不探测，只剩内置保留段（见文件头「平台」一节）。
#[cfg(not(windows))]
fn probe() -> Vec<HostNet> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(addr: &str, prefix: u8) -> HostNet {
        HostNet {
            addr: addr.parse().expect("测试里的地址必须合法"),
            prefix,
        }
    }

    #[test]
    fn a_public_style_vpn_subnet_becomes_a_direct_rule() {
        // 企业 VPN 给网卡分公网样式地址时，这条就是他存在的理由：公网段本来
        // 没有理由直连，但它是「本机所在的网络」。
        let rules = collect(&[host("203.0.113.9", 24)], &builtin::reserved_ranges());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, Action::Direct);
        assert_eq!(rules[0].source, SOURCE);
        assert_eq!(rules[0].key(), "cidr:203.0.113.0/24", "主机位应当被清零");
    }

    #[test]
    fn addresses_already_covered_by_reserved_ranges_are_not_repeated() {
        let reserved = builtin::reserved_ranges();
        let rules = collect(
            &[
                host("10.0.0.33", 24),
                host("192.168.10.10", 24),
                host("127.0.0.1", 8),
                host("169.254.227.201", 16),
            ],
            &reserved,
        );
        assert!(rules.is_empty(), "这些都在内置保留段里：{rules:?}");
    }

    #[test]
    fn the_same_subnet_from_two_adapters_is_added_once() {
        let rules = collect(
            &[host("203.0.113.9", 24), host("203.0.113.200", 24)],
            &builtin::reserved_ranges(),
        );
        assert_eq!(rules.len(), 1, "{rules:?}");
    }

    #[test]
    fn a_zero_prefix_is_skipped() {
        // `/0` 等于「整机流量都直连」，绝不能因为某张网卡报了个怪前缀就中招。
        let rules = collect(&[host("203.0.113.9", 0)], &builtin::reserved_ranges());
        assert!(rules.is_empty(), "{rules:?}");
    }

    #[test]
    fn ipv6_prefix_is_kept() {
        let rules = collect(&[host("2001:db8:aa::1", 64)], &builtin::reserved_ranges());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].key(), "cidr:2001:db8:aa::/64");
    }

    #[test]
    fn rules_never_panic_and_stay_direct() {
        // 真实探测：拿到几张网卡取决于这台机器，这里只验「不炸 + 全是直连」。
        for rule in rules() {
            assert_eq!(rule.action, Action::Direct);
            assert!(rule.enabled);
            assert_eq!(rule.source, SOURCE);
        }
    }

    #[cfg(windows)]
    #[test]
    fn probing_this_machine_reads_at_least_the_loopback() {
        assert!(
            !probe().is_empty(),
            "回环网卡永远是 Up 的，探测返回空说明 GetAdaptersAddresses 没用对"
        );
    }
}
