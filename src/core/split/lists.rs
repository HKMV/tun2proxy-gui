//! 清单：把上游的「分配记录 / 域名清单」变成我们的规则。
//!
//! 两个来源，两种格式，一个去处：
//!
//! | 来源 | 格式 | 产物 |
//! |------|------|------|
//! | APNIC `delegated-apnic-latest` | 每行一条「起始地址 + 数量/前缀长度」 | 国内 IPv4 / IPv6 前缀集 |
//! | v2fly `domain-list-community` | 每行一个后缀，支持 `full:` / `keyword:` / `include:` | 域名规则 |
//!
//! # 为什么这里要自己聚合，而不是直接抄一份 chnroutes
//!
//! 聚合规则是这段代码的全部价值：**不能多覆盖，也不能少覆盖**。多覆盖会把境外
//! 地址判成国内（真出线上故障），少覆盖只是那条走了代理（功能退化）。所以
//! `merge_ranges_v4` / `ranges_to_cidrs_v4` 有专门的测试盯着「覆盖范围相等」。
//!
//! # 为什么 `regexp:` 直接丢掉
//!
//! v2fly 的清单里混着正则行。引一个正则引擎进来，等于让几千条用户数据参与
//! 「每条连接都跑一次」的热路径，还得考虑回溯爆炸。宁可漏几条，不要一个卡住
//! 整台机器的可能。

use std::collections::HashSet;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};

use super::ipset::{aggregate_v4, aggregate_v6, Cidr};
use super::rules::{normalize_domain, Action, Matcher, Rule};

/// 默认从 APNIC 记录里挑哪个国家。
pub const COUNTRY_CN: &str = "CN";

/// `parse_*` 过程中攒下来的规模统计，界面上「N 条」与日志自检都用它。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApnicStats {
    /// 读到的有效数据行（不含空行与注释）。
    pub lines: usize,
    /// 国家与类型都匹配、状态可用的记录数。
    pub records: usize,
    pub ipv4_records: usize,
    pub ipv6_records: usize,
    /// 状态是 reserved / available 之类、被跳过的条数。
    pub skipped_status: usize,
    /// 同国家但类型不认识（asn）被跳过的条数。
    pub skipped_type: usize,
}

/// APNIC 解析 + 聚合的结果。
pub struct ChinaIp {
    pub v4: Vec<Cidr>,
    pub v6: Vec<Cidr>,
    pub stats: ApnicStats,
}

/// 从 APNIC 的 `delegated-apnic-latest` 里取某个国家的 IPv4 / IPv6 网段。
///
/// 行格式：`registry|country|type|start|value|date|status`，
/// IPv4 的 `value` 是**数量**（不一定是 2 的幂），IPv6 的 `value` 是**前缀长度**。
pub fn china_ip_from_apnic(text: &str) -> ChinaIp {
    let mut stats = ApnicStats::default();
    let mut v4_ranges: Vec<(u32, u32)> = Vec::new();
    let mut v6_prefixes: Vec<(u128, u8)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        stats.lines += 1;

        let fields: Vec<&str> = line.split('|').collect();
        // 第 0 行是版本行（`2|apnic|20260101|...`），字段数也对不上，一起被这里挡掉。
        if fields.len() < 7 || !fields[0].eq_ignore_ascii_case("apnic") {
            continue;
        }
        if !fields[1].eq_ignore_ascii_case(COUNTRY_CN) {
            continue;
        }

        let status = fields[6].trim();
        if !(status.eq_ignore_ascii_case("allocated") || status.eq_ignore_ascii_case("assigned")) {
            stats.skipped_status += 1;
            continue;
        }

        match fields[2].trim() {
            "ipv4" => {
                let (Ok(start), Ok(count)) =
                    (fields[3].trim().parse::<Ipv4Addr>(), fields[4].trim().parse::<u32>())
                else {
                    continue;
                };
                if count == 0 {
                    continue;
                }
                let start = u32::from(start);
                // `起始地址 + 数量 - 1` 用 u64 算，避免在 255.255.255.255 上溢出。
                let end = u64::from(start) + u64::from(count) - 1;
                if end > u64::from(u32::MAX) {
                    continue;
                }
                v4_ranges.push((start, end as u32));
                stats.records += 1;
                stats.ipv4_records += 1;
            }
            "ipv6" => {
                let (Ok(start), Ok(len)) = (
                    fields[3].trim().parse::<Ipv6Addr>(),
                    fields[4].trim().parse::<u8>(),
                ) else {
                    continue;
                };
                if len > 128 {
                    continue;
                }
                v6_prefixes.push((u128::from(start), len));
                stats.records += 1;
                stats.ipv6_records += 1;
            }
            _ => stats.skipped_type += 1,
        }
    }

    ChinaIp {
        v4: aggregate_v4(&v4_ranges),
        v6: aggregate_v6(&v6_prefixes),
        stats,
    }
}

/// 一份域名清单解析出来的东西。
#[derive(Clone, Debug, Default)]
pub struct DomainList {
    pub rules: Vec<Rule>,
    /// 需要调用方再去取的 `include:` 名字（本函数不碰网络 / 文件系统）。
    pub includes: Vec<String>,
    pub stats: DomainStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DomainStats {
    /// 有效数据行数（含被跳过的正则行）。
    pub lines: usize,
    pub suffix: usize,
    pub exact: usize,
    pub keyword: usize,
    /// `regexp:` 行数——这些被**故意**丢掉。
    pub regexp_skipped: usize,
    /// 解析不出域名的行数。
    pub invalid: usize,
    /// 同一个文件里重复出现的条数。
    pub duplicates: usize,
}

/// 解析一份 v2fly 风格的域名清单。
///
/// 支持的写法：裸后缀、`suffix:` / `domain:`、`full:` / `exact:`、`keyword:`、
/// `include:`、行尾 `#` 注释、行尾 `@attribute`（属性一律忽略——本程序不做
/// 「给广告域名单独分组」这种事）。
pub fn parse_domain_list(text: &str, source: &str) -> DomainList {
    let mut list = DomainList::default();
    let mut seen: HashSet<(u8, String)> = HashSet::new();
    let mut seen_includes: HashSet<String> = HashSet::new();

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // `include:eastmoney @-!cn` → 取第一段，属性丢掉。
        let token = line.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            continue;
        }
        list.stats.lines += 1;

        if let Some(name) = token.strip_prefix("include:") {
            if !name.is_empty() && seen_includes.insert(name.to_string()) {
                list.includes.push(name.to_string());
            }
            continue;
        }
        if token.starts_with("regexp:") {
            list.stats.regexp_skipped += 1;
            continue;
        }

        let (kind, value) = match token.split_once(':') {
            Some((kind, value))
                if matches!(
                    kind.to_ascii_lowercase().as_str(),
                    "suffix" | "domain" | "full" | "exact" | "keyword"
                ) =>
            {
                (kind.to_ascii_lowercase(), value)
            }
            _ => ("suffix".to_string(), token),
        };

        let matcher = match kind.as_str() {
            "full" | "exact" => match normalize_domain(value) {
                Some(domain) => Matcher::Exact(domain),
                None => {
                    list.stats.invalid += 1;
                    continue;
                }
            },
            "keyword" => {
                let keyword = value.trim().to_ascii_lowercase();
                if keyword.is_empty() {
                    list.stats.invalid += 1;
                    continue;
                }
                Matcher::Keyword(keyword)
            }
            _ => match normalize_domain(value) {
                Some(domain) => Matcher::Suffix(domain),
                None => {
                    list.stats.invalid += 1;
                    continue;
                }
            },
        };

        let key = (kind_tag(&matcher), matcher_text(&matcher).to_string());
        if !seen.insert(key) {
            list.stats.duplicates += 1;
            continue;
        }
        match matcher.kind() {
            super::rules::MatcherKind::Exact => list.stats.exact += 1,
            super::rules::MatcherKind::Suffix => list.stats.suffix += 1,
            super::rules::MatcherKind::Keyword => list.stats.keyword += 1,
            super::rules::MatcherKind::Cidr => {}
        }
        list.rules.push(Rule::new(Action::Direct, matcher, source));
    }

    list.includes.dedup();
    list
}
/// 展开 `include:` 之后的结果。
#[derive(Clone, Debug, Default)]
pub struct ExpandedDomains {
    pub rules: Vec<Rule>,
    /// 成功读到的文件数（含 roots）。
    pub files: usize,
    /// 取不到的文件名。
    pub unresolved: Vec<String>,
    /// `regexp:` 被丢掉的总行数。
    pub regexp_skipped: usize,
    /// 解析不出域名的总行数。
    pub invalid: usize,
    /// 跨文件重复、被合并掉的条数。
    pub duplicates: usize,
}

/// 递归展开一批清单的 `include:`，并按模式去重。
///
/// `fetch` 负责「给个名字，取回内容」——本函数不碰网络也不碰文件系统，
/// 所以测试里可以给一张内存表，正式代码里给的是 HTTP 拉取 + 本地缓存。
///
/// 有深度上限（默认 8 层）和访问集合：清单之间一旦互相 include，不至于把
/// 程序转进死循环。
pub fn expand_domain_lists(
    roots: &[(&str, &str)],
    fetch: &mut dyn FnMut(&str) -> Option<String>,
    source: &str,
) -> ExpandedDomains {
    const MAX_DEPTH: usize = 8;

    let mut out = ExpandedDomains::default();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut seen_rules: HashSet<(u8, String)> = HashSet::new();
    let mut queue: Vec<(String, String, usize)> = roots
        .iter()
        .map(|(name, text)| ((*name).to_string(), (*text).to_string(), 0usize))
        .collect();

    while let Some((name, text, depth)) = queue.pop() {
        if !seen_files.insert(name.clone()) {
            continue;
        }
        out.files += 1;

        let list = parse_domain_list(&text, source);
        out.regexp_skipped += list.stats.regexp_skipped;
        out.invalid += list.stats.invalid;

        for rule in list.rules {
            let key = (kind_tag(&rule.matcher), matcher_text(&rule.matcher).to_string());
            if seen_rules.insert(key) {
                out.rules.push(rule);
            } else {
                out.duplicates += 1;
            }
        }

        if depth >= MAX_DEPTH {
            out.unresolved.extend(list.includes);
            continue;
        }
        for include in list.includes {
            match fetch(&include) {
                Some(text) => queue.push((include, text, depth + 1)),
                None => out.unresolved.push(include),
            }
        }
    }

    out
}

fn kind_tag(matcher: &Matcher) -> u8 {
    match matcher.kind() {
        super::rules::MatcherKind::Exact => 0,
        super::rules::MatcherKind::Suffix => 1,
        super::rules::MatcherKind::Keyword => 2,
        super::rules::MatcherKind::Cidr => 3,
    }
}

fn matcher_text(matcher: &Matcher) -> &str {
    match matcher {
        Matcher::Exact(text) | Matcher::Suffix(text) | Matcher::Keyword(text) => text,
        Matcher::Cidr(_) => "",
    }
}

/// 渲染一份网段快照（`#` 开头的行是注释，解析时会被跳过）。
pub fn render_cidr_snapshot(title: &str, note: &str, cidrs: &[Cidr]) -> String {
    let mut out = String::with_capacity(cidrs.len() * 20 + 256);
    let _ = writeln!(out, "# {title}");
    let _ = writeln!(out, "# {note}");
    let _ = writeln!(out, "# 共 {} 条；重新生成见 tools/regen-rules/README.md", cidrs.len());
    for cidr in cidrs {
        let _ = writeln!(out, "{cidr}");
    }
    out
}

/// 渲染一份域名快照。后缀写成裸行（和上游清单一致），精确与关键字带前缀。
pub fn render_domain_snapshot(title: &str, note: &str, rules: &[Rule]) -> String {
    let mut out = String::with_capacity(rules.len() * 18 + 256);
    let _ = writeln!(out, "# {title}");
    let _ = writeln!(out, "# {note}");
    let _ = writeln!(out, "# 共 {} 条", rules.len());
    for rule in rules {
        match &rule.matcher {
            Matcher::Exact(text) => {
                let _ = writeln!(out, "full:{text}");
            }
            Matcher::Suffix(text) => {
                let _ = writeln!(out, "{text}");
            }
            Matcher::Keyword(text) => {
                let _ = writeln!(out, "keyword:{text}");
            }
            Matcher::Cidr(cidr) => {
                let _ = writeln!(out, "{cidr}");
            }
        }
    }
    out
}

/// 一段前缀集覆盖多少个地址（给测试和「清单规模」报告用）。
pub fn address_count(cidrs: &[Cidr]) -> u128 {
    cidrs
        .iter()
        .map(|cidr| match *cidr {
            Cidr::V4 { len, .. } => 1u128 << (32 - len),
            Cidr::V6 { len, .. } => 1u128 << (128 - len),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这批网段是否覆盖某个地址。
    ///
    /// 直接用 `Cidr::contains` 而不是拿 `IpSet` 来测：这个模块的测试只关心
    /// 「聚合出来的范围对不对」，不想依赖前缀索引的实现。
    fn covers(cidrs: &[Cidr], text: &str) -> bool {
        let ip: std::net::IpAddr = text.parse().expect("测试里的地址必须合法");
        cidrs.iter().any(|cidr| cidr.contains(ip))
    }

    #[test]
    fn apnic_records_are_filtered_by_country_type_and_status() {
        // 版本行、注释、别的国家、asn、reserved 全都不该进来。
        let text = "\
2|apnic|20260101|3|19850701|20260101|+1000
apnic|CN|asn|4608|1|20000101|allocated
apnic|JP|ipv4|10.1.0.0|256|20000101|allocated
apnic|CN|ipv4|10.0.0.0|256|20000101|allocated
apnic|CN|ipv4|10.2.0.0|256|20000101|assigned
apnic|CN|ipv4|10.3.0.0|256|20000101|reserved
apnic|CN|ipv6|2001:db8::|32|20000101|assigned
";
        let parsed = china_ip_from_apnic(text);
        assert_eq!(parsed.stats.ipv4_records, 2);
        assert_eq!(parsed.stats.ipv6_records, 1);
        assert_eq!(parsed.stats.skipped_status, 1);
        assert_eq!(parsed.stats.skipped_type, 1);

        assert!(covers(&parsed.v4, "10.0.0.5"));
        assert!(covers(&parsed.v4, "10.2.0.5"));
        assert!(!covers(&parsed.v4, "10.3.0.5"), "reserved 不该进表");
        assert!(!covers(&parsed.v4, "10.1.0.5"), "别的国家不该进表");
        assert!(covers(&parsed.v6, "2001:db8::1"));
    }
    #[test]
    fn non_power_of_two_allocations_keep_exactly_the_same_coverage() {
        // APNIC 给的是数量（1000 个），不是前缀长度，必须拆准：
        // 覆盖的地址数要刚好等于 1000，不能多也不能少。
        let text = "apnic|CN|ipv4|10.0.0.0|1000|20000101|allocated\n";
        let parsed = china_ip_from_apnic(text);
        assert_eq!(parsed.stats.ipv4_records, 1);
        assert!(parsed.v4.len() > 1, "1000 不是 2 的幂，必然要拆成多条");
        assert_eq!(address_count(&parsed.v4), 1000);

        // 10.0.0.0 起 1000 个 = 10.0.0.0 .. 10.0.3.231
        assert!(covers(&parsed.v4, "10.0.0.0"));
        assert!(covers(&parsed.v4, "10.0.3.231"));
        assert!(!covers(&parsed.v4, "10.0.3.232"), "一个地址都不能多吃");
    }
    #[test]
    fn adjacent_allocations_from_the_same_holder_get_merged() {
        // 分开的两条 /24 拼成一个 /23：这才是「Aggregated to N」的来源。
        let text = "\
apnic|CN|ipv4|10.0.0.0|256|20000101|allocated
apnic|CN|ipv4|10.0.1.0|256|20000101|allocated
";
        let parsed = china_ip_from_apnic(text);
        assert_eq!(parsed.v4.len(), 1);
        assert_eq!(parsed.v4[0].len(), 23);
        assert_eq!(address_count(&parsed.v4), 512);
    }

    #[test]
    fn domain_lists_handle_prefixes_attributes_and_skips() {
        let text = "\
# 注释行
baidu.com
suffix:qq.com
full:www.qq.com
keyword:bilibili
regexp:^ad\\.[a-z]+\\.com$
include:tld-cn
include:geolocation-cn @-!cn
hiocloud.com # 浙ICP备2020040255号-2
baidu.com
";
        let list = parse_domain_list(text, "内陆域名");
        // 三条后缀：baidu.com、qq.com、hiocloud.com（最后一条的重复行不算）。
        assert_eq!(list.stats.suffix, 3, "baidu.com 重复只算一次");
        assert_eq!(list.stats.exact, 1);
        assert_eq!(list.stats.keyword, 1);
        assert_eq!(list.stats.regexp_skipped, 1);
        assert_eq!(list.stats.duplicates, 1);
        assert_eq!(list.includes, vec!["tld-cn", "geolocation-cn"]);
        assert_eq!(list.rules.len(), 5);
        assert!(list.rules.iter().all(|rule| rule.action == Action::Direct));
    }

    #[test]
    fn includes_are_expanded_once_and_cycles_do_not_hang() {
        // a → (b, missing)，b → a：形成环，必须只读一次就停下。
        let files = std::collections::HashMap::from([
            ("a", "include:b\ninclude:missing\naa.com\n"),
            ("b", "include:a\nfull:bb.com\n"),
        ]);
        let mut fetch = |name: &str| files.get(name).map(|text| (*text).to_string());
        // root 用文件 a 的内容：里面有 b 和 missing 两个 include。
        let expanded =
            expand_domain_lists(&[("a", "include:b\ninclude:missing\naa.com\n")], &mut fetch, "测试");

        assert_eq!(expanded.files, 2, "a 与 b 各读一次，环不会重复读");
        assert_eq!(expanded.unresolved, vec!["missing"]);
        let texts: Vec<String> = expanded
            .rules
            .iter()
            .map(|rule| match &rule.matcher {
                Matcher::Exact(text) | Matcher::Suffix(text) => text.clone(),
                Matcher::Keyword(text) => text.clone(),
                Matcher::Cidr(cidr) => cidr.to_string(),
            })
            .collect();
        assert!(texts.contains(&"aa.com".to_string()));
        assert!(texts.contains(&"bb.com".to_string()));
        assert_eq!(
            expanded.duplicates, 0,
            "同一个文件被 include 时按文件名去重，不会重复解析"
        );
    }

    #[test]
    fn snapshots_round_trip_through_our_own_parser() {
        // 渲染出去的文件必须能被自己的解析器读回来：快照是「程序内部格式」，
        // 不是给人看的临时文件。
        let rules = vec![
            Rule::new(Action::Direct, Matcher::Suffix("baidu.com".into()), "x"),
            Rule::new(Action::Direct, Matcher::Exact("www.qq.com".into()), "x"),
            Rule::new(Action::Direct, Matcher::Keyword("bilibili".into()), "x"),
        ];
        let text = render_domain_snapshot("测试", "说明", &rules);
        let parsed = parse_domain_list(&text, "x");
        assert_eq!(parsed.stats.suffix, 1);
        assert_eq!(parsed.stats.exact, 1);
        assert_eq!(parsed.stats.keyword, 1);
        assert_eq!(parsed.stats.invalid, 0);

        let cidrs = vec![Cidr::parse("10.0.0.0/8").unwrap()];
        let text = render_cidr_snapshot("测试", "说明", &cidrs);
        assert_eq!(text.lines().filter(|line| !line.starts_with('#')).count(), 1);
    }
}
