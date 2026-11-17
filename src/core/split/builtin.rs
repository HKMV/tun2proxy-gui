//! 编译期内置的分流清单快照。
//!
//! 三个文本文件在 `assets/rules/`，由 `tools/regen-rules` 生成（见那边的
//! `README.md`）。这里用 `include_str!` 把它们嵌进二进制，好处是**离线可用、
//! 启动不用等下载**；代价是更新清单要重新发版——所以将来运行时会用下载到的
//! 新数据覆盖这份快照，内置的只是兜底。
//!
//! # 别手改 `assets/rules/` 里的文件
//!
//! 它们是生成物，头三行写着来源和生成日期。手改会在下次 `generate` 时被整份
//! 覆盖，而且改出来的条数会和 `lists` 的统计对不上。

use super::ipset::Cidr;
use super::lists::parse_domain_list;
use super::rules::{parse_rules_text, Action, Matcher, Rule, RuleSet, Stats};

/// 国内 IPv4 段（APNIC 委派数据聚合后的快照）。
pub const CHINA_IPV4: &str = include_str!("../../../assets/rules/china-ipv4.txt");
/// 国内 IPv6 段。
pub const CHINA_IPV6: &str = include_str!("../../../assets/rules/china-ipv6.txt");
/// 国内域名后缀 / 精确域名（v2fly 清单展开后的快照）。
pub const CHINA_DOMAIN: &str = include_str!("../../../assets/rules/china-domain.txt");

/// 本机、内网、保留地址：任何情况下都直连，不走代理。
///
/// 这一段**不**来自数据文件，而是写死在代码里——它属于「代理程序的基本正确性」，
/// 不该依赖某次清单更新的成败。`0.0.0.0/8`、组播和保留段放进来是因为把它们
/// 送到代理只会得到一个神秘的超时。
const RESERVED_RANGES: [&str; 12] = [
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "224.0.0.0/4",
    "240.0.0.0/4",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

/// 企业认证门户：按环境写死的直连段。
///
/// 深信服 aTrust / SSL VPN 的登录门户是一个**公网 IP**，客户端拿 IP 直连
/// （`https://<ip>/` 302 到 `/login.html?name_jpgraph_antispam=…`），而且登录
/// 流量走的是物理网卡、不是它自己那张 VPN 网卡——所以这段地址既不在私有保留段里，
/// 也不在 [`super::hostnet`] 探测到的 VPN 网卡网段里。握手一旦被送进上游代理就
/// 再也完不成，表现为「VPN 一直转圈、登不上」。
///
/// 它属于「某家公司机房的地址」，没有通用值可算，只能当常量维护：换网关就改这里。
/// 注释里不要写出具体地址——那串数字对每个部署都不一样，写了也只会误导下一个人。
const AUTH_RANGES: [&str; 1] = [
    // 深信服认证门户（aTrust / SSL VPN 登录页）：登录握手必须直连
    "1.1.1.0/24",
];

/// 本地网络规则（直连）。
///
/// 两组分开解析是为了让来源分得清：界面上「局域网与保留地址」那一组里，哪几条是
/// 常量保留段、哪几条是某家公司机房的认证门户，看来源列就知道。
pub fn local_rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    for (ranges, source) in [
        (&RESERVED_RANGES[..], "本地网络"),
        (&AUTH_RANGES[..], "深信服认证"),
    ] {
        let (mut parsed, errors) =
            parse_rules_text(&ranges.join("\n"), Action::Direct, source);
        debug_assert!(
            errors.is_empty(),
            "内置直连段必须全部合法（{source}）：{errors:?}"
        );
        rules.append(&mut parsed);
    }
    rules
}

/// 上面那两组内置直连段，解析成 CIDR。
///
/// 给 [`super::hostnet`] 去重用：本机网卡落在 `10/8` 里的地址没必要再单独作为一条
/// 规则出现，否则界面上「本地网络」组里会冒出一串被大段盖住的重复行。
pub fn builtin_ranges() -> Vec<Cidr> {
    let mut ranges = Vec::new();
    for text in RESERVED_RANGES.iter().chain(AUTH_RANGES.iter()) {
        let (parsed, errors) = parse_rules_text(text, Action::Direct, "本地网络");
        debug_assert!(errors.is_empty(), "内置直连段必须全部合法：{errors:?}");
        ranges.extend(parsed.into_iter().filter_map(|rule| match rule.matcher {
            Matcher::Cidr(cidr) => Some(cidr),
            _ => None,
        }));
    }
    ranges
}

/// 国内 IP 规则（直连）。
pub fn china_ip_rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    for (text, source) in [(CHINA_IPV4, "内置IPv4"), (CHINA_IPV6, "内置IPv6")] {
        let (mut parsed, errors) = parse_rules_text(text, Action::Direct, source);
        debug_assert!(errors.is_empty(), "{source} 快照有坏行：{errors:?}");
        rules.append(&mut parsed);
    }
    rules
}

/// 国内域名规则（直连）。
pub fn china_domain_rules() -> Vec<Rule> {
    let list = parse_domain_list(CHINA_DOMAIN, "内置域名");
    // 快照是「展开后」的：生成时就该把 `include:` 全部展开，程序里不再联网。
    debug_assert!(
        list.includes.is_empty(),
        "快照里还剩 include: {:?}，说明 generate 时没给全子清单",
        list.includes
    );
    debug_assert_eq!(list.stats.invalid, 0, "快照里有解析不出的行");
    list.rules
}

/// 全部内置规则，顺序即编译顺序：本地网络 → 国内 IP → 国内域名。
pub fn rules() -> Vec<Rule> {
    let mut all = local_rules();
    all.extend(china_ip_rules());
    all.extend(china_domain_rules());
    all
}

/// 编译成可查询的规则集。
///
/// 编译一次约 3 ms、常驻约 1.5 MB（16 k 条规则）。将来接进引擎时要建一次就
/// 存起来（`OnceLock`），别在每连接上重建。
pub fn rule_set() -> (RuleSet, Stats) {
    RuleSet::build(&rules())
}

/// 快照规模 `(本地, IPv4, IPv6, 域名)`，给界面和日志用。
///
/// 「本地」是保留段与认证门户两组之和——界面上的「局域网与保留地址」显示的就是这一个数。
pub fn snapshot_size() -> (usize, usize, usize, usize) {
    (
        RESERVED_RANGES.len() + AUTH_RANGES.len(),
        count_data_lines(CHINA_IPV4),
        count_data_lines(CHINA_IPV6),
        count_data_lines(CHINA_DOMAIN),
    )
}

fn count_data_lines(text: &str) -> usize {
    text.lines().filter(|line| is_data(line)).count()
}

fn is_data(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && !line.starts_with('#')
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::super::rules::MatcherKind;
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("测试里的地址必须合法")
    }

    #[test]
    fn ipv4_snapshot_is_complete() {
        let (rules, errors) = parse_rules_text(CHINA_IPV4, Action::Direct, "test");
        assert!(errors.is_empty(), "IPv4 快照有坏行：{errors:?}");
        assert_eq!(rules.len(), 5499);
        assert!(rules.iter().all(|rule| rule.matcher.kind() == MatcherKind::Cidr));
    }

    #[test]
    fn ipv6_snapshot_is_complete() {
        let (rules, errors) = parse_rules_text(CHINA_IPV6, Action::Direct, "test");
        assert!(errors.is_empty(), "IPv6 快照有坏行：{errors:?}");
        assert_eq!(rules.len(), 2019);
    }

    #[test]
    fn domain_snapshot_is_complete() {
        let list = parse_domain_list(CHINA_DOMAIN, "test");
        assert!(list.includes.is_empty(), "还剩 include：{:?}", list.includes);
        assert_eq!(list.stats.invalid, 0);
        assert_eq!(list.stats.regexp_skipped, 0, "快照里不该有 regexp 行");
        assert_eq!(list.rules.len(), 8592);
    }

    #[test]
    fn snapshot_size_matches_rules() {
        let (local, v4, v6, domain) = snapshot_size();
        assert_eq!(local, RESERVED_RANGES.len() + AUTH_RANGES.len());
        assert_eq!(v4, 5499);
        assert_eq!(v6, 2019);
        assert_eq!(domain, 8592);
        assert_eq!(rules().len(), local + v4 + v6 + domain);
    }

    #[test]
    fn first_and_last_snapshot_domains_hit() {
        let (set, _) = rule_set();
        let is_data = |line: &&str| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        };
        let first = CHINA_DOMAIN.lines().find(is_data).expect("快照不能是空的");
        let last = CHINA_DOMAIN.lines().filter(is_data).next_back().expect("快照不能是空的");
        for pattern in [first.trim(), last.trim()] {
            let domain = format!("www.{pattern}");
            let hit = set
                .match_domain(&domain)
                .unwrap_or_else(|| panic!("{domain} 应该命中内置清单"));
            assert_eq!(hit.action(), Action::Direct, "{domain} 应该直连");
        }
    }

    #[test]
    fn local_networks_go_direct() {
        let (set, _) = rule_set();
        for text in ["10.0.0.1", "192.168.1.1", "172.31.0.1", "127.0.0.1", "169.254.1.1"] {
            let hit = set
                .match_ip(ip(text))
                .unwrap_or_else(|| panic!("{text} 是内网地址，必须命中"));
            assert_eq!(hit.action(), Action::Direct);
        }
        let loopback = set.match_ip(ip("::1")).expect("::1 必须命中");
        assert_eq!(loopback.action(), Action::Direct);
    }

    #[test]
    fn enterprise_auth_portals_go_direct() {
        let (set, _) = rule_set();
        for text in AUTH_RANGES {
            // 网段的网络地址本身也在段内，拿它当代表地址。
            let (addr, _len) = text.split_once('/').expect("内置段都带前缀长度");
            let hit = set
                .match_ip(ip(addr))
                .unwrap_or_else(|| panic!("{addr} 是认证门户段，必须命中直连"));
            assert_eq!(hit.action(), Action::Direct, "{addr} 必须直连");
        }
        // 来源单独标出来：界面上要能一眼看出这条不是本机网段、也不是私有段。
        let marked = local_rules()
            .iter()
            .filter(|rule| rule.source == "深信服认证")
            .count();
        assert_eq!(marked, AUTH_RANGES.len());
    }

    #[test]
    fn foreign_domains_fall_through() {
        let (set, _) = rule_set();
        for domain in [
            "github.com",
            "www.google.com",
            "api.openai.com",
            "example.org",
            "www.example.com",
        ] {
            assert!(
                set.match_domain(domain).is_none(),
                "{domain} 不该命中内置清单，否则它会被错误地直连"
            );
        }
    }

    #[test]
    fn every_builtin_rule_is_direct_and_enabled() {
        assert!(
            rules()
                .iter()
                .all(|rule| rule.action == Action::Direct && rule.enabled),
            "内置清单只允许「直连 + 已启用」：代理是兜底行为，不需要规则"
        );
    }

    #[test]
    fn major_chinese_sites_are_direct() {
        let (set, _) = rule_set();
        for domain in [
            "www.baidu.com",
            "www.qq.com",
            "www.taobao.com",
            "www.bilibili.com",
            "www.163.com",
            "www.tsinghua.edu.cn",
        ] {
            let hit = set
                .match_domain(domain)
                .unwrap_or_else(|| panic!("{domain} 是主要国内站点，内置清单必须覆盖"));
            assert_eq!(hit.action(), Action::Direct);
        }
    }
}
