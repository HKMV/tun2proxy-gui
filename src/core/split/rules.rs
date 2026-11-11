//! 分流规则：一条规则长什么样，以及「一个连接该不该直连」怎么判。
//!
//! # 判定顺序（决定行为，改之前先读这段）
//!
//! 1. 有域名 → 先比**精确**，再比**后缀**（从最长往最短），最后比**关键字**；
//! 2. 没有命中域名规则（或者压根拿不到域名）→ 比 **IP 网段**，最长前缀优先；
//! 3. 全都没命中 → 返回 `None`，由调用方按「走代理」兜底。
//!
//! 为什么域名优先：虚拟 IP 模式下 tun2proxy 交给上游的是**域名**（DNS 映射回来
//! 的），这时候域名比解析出来的地址更能说明「用户想访问谁」；JP 段只是兜底。
//!
//! # 同一目标命中多条时谁赢
//!
//! * 域名：`Exact` > 更长的 `Suffix` > 更短的 `Suffix` > `Keyword`（更长的关键字优先）；
//! * IP：更长的前缀 > 更短的前缀；**同一条前缀长度上同时有直连和代理 → 代理赢**
//!   （保守选择：用户的「强制走代理」是更明确的意图，也避免误判把流量放出去）；
//! * 同一个模式出现两次 → 先出现的赢（规则是有序的，`build` 不会重排）。

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;

use super::ipset::{Cidr, CidrError, IpSet};

/// 命中之后的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// 直连——不经上游代理，用本地网络出去。
    Direct,
    /// 走代理——本期界面上不出这一项，但数据结构先留着：
    /// 「某个国内域名的子域必须走代理」这种例外迟早要表达。
    Proxy,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Direct => "direct",
            Action::Proxy => "proxy",
        }
    }
}

/// 一条规则的匹配方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matcher {
    /// 精确域名：`www.example.com` 只匹配它自己。
    Exact(String),
    /// 域名后缀，按标签边界匹配：`qq.com` 匹配 `www.qq.com`，不匹配 `notqq.com`。
    Suffix(String),
    /// 关键字：域名里含这个串就算命中，留给「拿不准后缀该怎么写」的场合。
    Keyword(String),
    /// IP 网段。
    Cidr(Cidr),
}

/// 匹配方式的分类（日志、统计、界面分组用；不带具体模式文本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MatcherKind {
    Exact,
    Suffix,
    Keyword,
    Cidr,
}

impl MatcherKind {
    pub fn label(self) -> &'static str {
        match self {
            MatcherKind::Exact => "exact",
            MatcherKind::Suffix => "suffix",
            MatcherKind::Keyword => "keyword",
            MatcherKind::Cidr => "cidr",
        }
    }
}

impl Matcher {
    pub fn kind(&self) -> MatcherKind {
        match self {
            Matcher::Exact(_) => MatcherKind::Exact,
            Matcher::Suffix(_) => MatcherKind::Suffix,
            Matcher::Keyword(_) => MatcherKind::Keyword,
            Matcher::Cidr(_) => MatcherKind::Cidr,
        }
    }

    /// 匹配用的文本（`qq.com`、`1.2.3.0/24`）。
    pub fn text(&self) -> String {
        match self {
            Matcher::Exact(text) => text.clone(),
            Matcher::Suffix(text) => text.clone(),
            Matcher::Keyword(text) => text.clone(),
            Matcher::Cidr(cidr) => cidr.to_string(),
        }
    }

    /// 稳定标识：`suffix:qq.com`。界面上的「这一行要不要生效」用它当键，
    /// 存进配置后即使清单顺序变了也对得上。
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind().label(), self.text())
    }
}

impl Rule {
    pub fn key(&self) -> String {
        self.matcher.key()
    }
}

/// 一条规则。
///
/// `source` 是「这条规则从哪来的」（`国内域名` / `局域网` / `自定义`），只用于
/// 日志和界面展示：用户看到 `[直连] suffix:baidu.com ← 国内域名` 才知道该去关
/// 哪一组，而不是对着一个孤立域名发愣。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub enabled: bool,
    pub action: Action,
    pub matcher: Matcher,
    pub source: String,
}

impl Rule {
    pub fn new(action: Action, matcher: Matcher, source: impl Into<String>) -> Self {
        Self {
            enabled: true,
            action,
            matcher,
            source: source.into(),
        }
    }
}

/// 一次命中。借用 [`RuleSet`] 里的数据，不分配内存。
#[derive(Clone, Debug)]
pub enum Match<'a> {
    Domain {
        action: Action,
        kind: MatcherKind,
        pattern: &'a str,
        source: &'a str,
    },
    Ip {
        action: Action,
        cidr: Cidr,
        source: &'a str,
    },
}

impl Match<'_> {
    pub fn action(&self) -> Action {
        match self {
            Match::Domain { action, .. } | Match::Ip { action, .. } => *action,
        }
    }

    /// 给决策日志用的一行说明。
    pub fn describe(&self) -> String {
        match self {
            Match::Domain {
                action,
                kind,
                pattern,
                source,
            } => format!("{} {}:{} [{}]", action.label(), kind.label(), pattern, source),
            Match::Ip {
                action,
                cidr,
                source,
            } => format!("{} cidr:{} [{}]", action.label(), cidr, source),
        }
    }
}

impl fmt::Display for Match<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

#[derive(Clone, Debug)]
struct Entry {
    action: Action,
    source: String,
}

/// 编译好的规则集：查询全部走哈希 / 前缀索引，不再遍历原始规则列表。
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    exact: HashMap<String, Entry>,
    suffix: HashMap<String, Entry>,
    /// 按长度降序：更长的关键字更具体，先试它。
    keywords: Vec<(String, Entry)>,
    direct_ip: IpSet,
    proxy_ip: IpSet,
}

/// 编译后的规模统计（界面上的「N 条」和日志里的自检都用它）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub exact: usize,
    pub suffix: usize,
    pub keyword: usize,
    pub direct_cidrs: usize,
    pub proxy_cidrs: usize,
    /// 被关掉而没进集合的规则数。
    pub disabled: usize,
}

impl Stats {
    pub fn total(&self) -> usize {
        self.exact + self.suffix + self.keyword + self.direct_cidrs + self.proxy_cidrs
    }
}

impl RuleSet {
    /// 编译一批规则。
    ///
    /// 关掉的规则直接跳过（用户开关就是走这条路）；同一个模式重复出现时先到的赢。
    pub fn build(rules: &[Rule]) -> (Self, Stats) {
        let mut set = RuleSet::default();
        let mut stats = Stats::default();

        for rule in rules {
            if !rule.enabled {
                stats.disabled += 1;
                continue;
            }
            match &rule.matcher {
                Matcher::Exact(domain) => {
                    let Some(domain) = normalize_domain(domain) else {
                        continue;
                    };
                    if set.exact.contains_key(&domain) {
                        continue;
                    }
                    stats.exact += 1;
                    set.exact.insert(
                        domain,
                        Entry {
                            action: rule.action,
                            source: rule.source.clone(),
                        },
                    );
                }
                Matcher::Suffix(domain) => {
                    let Some(domain) = normalize_domain(domain) else {
                        continue;
                    };
                    if set.suffix.contains_key(&domain) {
                        continue;
                    }
                    stats.suffix += 1;
                    set.suffix.insert(
                        domain,
                        Entry {
                            action: rule.action,
                            source: rule.source.clone(),
                        },
                    );
                }
                Matcher::Keyword(keyword) => {
                    let keyword = keyword.trim().to_ascii_lowercase();
                    if keyword.is_empty() || set.keywords.iter().any(|(k, _)| *k == keyword) {
                        continue;
                    }
                    stats.keyword += 1;
                    set.keywords.push((
                        keyword,
                        Entry {
                            action: rule.action,
                            source: rule.source.clone(),
                        },
                    ));
                }
                Matcher::Cidr(cidr) => {
                    let inserted = match rule.action {
                        Action::Direct => set.direct_ip.insert(*cidr),
                        Action::Proxy => set.proxy_ip.insert(*cidr),
                    };
                    if inserted {
                        match rule.action {
                            Action::Direct => stats.direct_cidrs += 1,
                            Action::Proxy => stats.proxy_cidrs += 1,
                        }
                    }
                }
            }
        }

        set.keywords.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        (set, stats)
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty()
            && self.suffix.is_empty()
            && self.keywords.is_empty()
            && self.direct_ip.is_empty()
            && self.proxy_ip.is_empty()
    }

    /// 域名判定：精确 → 后缀（长到短）→ 关键字。
    ///
    /// 后缀逐级比对用的是原字符串的切片（`www.qq.com` → `qq.com` → `com`），
    /// 所以一次查询不产生任何堆分配。
    pub fn match_domain(&self, domain: &str) -> Option<Match<'_>> {
        let domain = normalize_domain(domain)?;

        if let Some((pattern, entry)) = self.exact.get_key_value(domain.as_str()) {
            return Some(Match::Domain {
                action: entry.action,
                kind: MatcherKind::Exact,
                pattern: pattern.as_str(),
                source: entry.source.as_str(),
            });
        }

        let mut start = 0usize;
        loop {
            let candidate = &domain[start..];
            if let Some((pattern, entry)) = self.suffix.get_key_value(candidate) {
                return Some(Match::Domain {
                    action: entry.action,
                    kind: MatcherKind::Suffix,
                    pattern: pattern.as_str(),
                    source: entry.source.as_str(),
                });
            }
            match domain[start..].find('.') {
                Some(offset) => start += offset + 1,
                None => break,
            }
        }

        for (keyword, entry) in &self.keywords {
            if domain.contains(keyword.as_str()) {
                return Some(Match::Domain {
                    action: entry.action,
                    kind: MatcherKind::Keyword,
                    pattern: keyword.as_str(),
                    source: entry.source.as_str(),
                });
            }
        }

        None
    }

    /// IP 判定：最长前缀优先，同长度上代理赢。
    pub fn match_ip(&self, ip: IpAddr) -> Option<Match<'_>> {
        let direct = self.direct_ip.lookup(ip);
        let proxy = self.proxy_ip.lookup(ip);
        match (direct, proxy) {
            (Some(direct), Some(proxy)) => {
                if proxy.len() >= direct.len() {
                    Some(Match::Ip {
                        action: Action::Proxy,
                        cidr: proxy,
                        source: IP_SOURCE,
                    })
                } else {
                    Some(Match::Ip {
                        action: Action::Direct,
                        cidr: direct,
                        source: IP_SOURCE,
                    })
                }
            }
            (Some(direct), None) => Some(Match::Ip {
                action: Action::Direct,
                cidr: direct,
                source: IP_SOURCE,
            }),
            (None, Some(proxy)) => Some(Match::Ip {
                action: Action::Proxy,
                cidr: proxy,
                source: IP_SOURCE,
            }),
            (None, None) => None,
        }
    }

    /// 一次完整的判定：域名优先，IP 兜底，都没命中返回 `None`（调用方按代理处理）。
    pub fn decide(&self, domain: Option<&str>, ip: Option<IpAddr>) -> Option<Match<'_>> {
        if let Some(domain) = domain {
            if let Some(hit) = self.match_domain(domain) {
                return Some(hit);
            }
        }
        ip.and_then(|ip| self.match_ip(ip))
    }
}

/// 网段规则的来源文案。
///
/// 现在 IP 规则是一整张集合（内置段和自定义段合并进去），`IpSet` 只存前缀不存
/// 来源，所以只能给一个笼统的标记。等界面接起来、规则按组分别编译之后，这里
/// 换成每组自己的名字（「国内 IP」/「局域网」/「自定义」）。
const IP_SOURCE: &str = "ip";

/// 域名归一化：小写、去尾部点、去端口、去 `*.` / 前导点。
///
/// 返回 `None` 表示这串东西根本不像域名（空、带空格、带斜杠……）。
/// 注意：**不做 IDN 转换**。中文域名要用户自己写成 `xn--` 开头的形式，
/// 等界面接起来之后再在输入框旁边提示（引 `idna` 依赖是另一件事）。
pub fn normalize_domain(input: &str) -> Option<String> {
    let mut text = input.trim();
    if text.is_empty() {
        return None;
    }

    // 去掉端口：`example.com:443`。IPv6 字面量里的冒号不碰（它有多个冒号）。
    if let Some((host, port)) = text.rsplit_once(':') {
        if !host.contains(':') && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
            text = host;
        }
    }

    let text = text.trim_end_matches('.');
    let text = text.strip_prefix("*.").unwrap_or(text);
    let text = text.trim_start_matches('.');
    let text = text.to_ascii_lowercase();

    if text.is_empty() || text.len() > 253 {
        return None;
    }
    let ok = text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_');
    if !ok {
        return None;
    }
    Some(text)
}

/// 规则行解析失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleError {
    Empty,
    BadCidr(CidrError),
    BadDomain(String),
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleError::Empty => f.write_str("空行"),
            RuleError::BadCidr(err) => write!(f, "{err}"),
            RuleError::BadDomain(text) => write!(f, "「{text}」不是合法的域名"),
        }
    }
}

impl std::error::Error for RuleError {}

/// 把一行文本解析成规则。
///
/// 支持这几种写法（清单文件和用户手填共用同一套）：
///
/// | 写法 | 结果 |
/// |------|------|
/// | `10.0.0.0/8`、`192.168.1.1` | `Cidr` |
/// | `baidu.com`、`*.baidu.com`、`.baidu.com` | `Suffix` |
/// | `suffix:baidu.com`、`domain:baidu.com` | `Suffix` |
/// | `full:www.baidu.com`、`exact:www.baidu.com` | `Exact` |
/// | `keyword:bilibili` | `Keyword` |
///
/// 行尾 `#` 之后的内容当注释丢掉（内置清单里带 ICANN 备案号，直接透传会污染解析）。
pub fn parse_rule_line(line: &str, action: Action, source: &str) -> Result<Rule, RuleError> {
    let text = line.split('#').next().unwrap_or("").trim();
    if text.is_empty() {
        return Err(RuleError::Empty);
    }

    let (kind, value) = match text.split_once(':') {
        Some((kind, value))
            if matches!(
                kind.trim().to_ascii_lowercase().as_str(),
                "suffix" | "domain" | "full" | "exact" | "keyword"
            ) =>
        {
            (kind.trim().to_ascii_lowercase(), value.trim())
        }
        _ => (String::new(), text),
    };

    if value.is_empty() {
        return Err(RuleError::BadDomain(text.to_string()));
    }

    let matcher = match kind.as_str() {
        "full" | "exact" => Matcher::Exact(
            normalize_domain(value).ok_or_else(|| RuleError::BadDomain(value.to_string()))?,
        ),
        "keyword" => Matcher::Keyword(value.to_ascii_lowercase()),
        "suffix" | "domain" => Matcher::Suffix(
            normalize_domain(value).ok_or_else(|| RuleError::BadDomain(value.to_string()))?,
        ),
        _ => {
            // 没写前缀：先当网段试，不是网段就当域名后缀。
            // 但「一看就是地址」的输入（只有数字、点、冒号、斜杠）失败时要报
            // 地址错误——把 `10.0.0.0/33` 说成「不是合法域名」会让用户完全找不到
            // 问题出在前缀长度上。
            match Cidr::parse(value) {
                Ok(cidr) => Matcher::Cidr(cidr),
                Err(err) if looks_like_address(value) => return Err(RuleError::BadCidr(err)),
                Err(_) => Matcher::Suffix(
                    normalize_domain(value)
                        .ok_or_else(|| RuleError::BadDomain(value.to_string()))?,
                ),
            }
        }
    };

    Ok(Rule::new(action, matcher, source))
}

/// 是不是「一看就像地址」的输入：只有数字、点、冒号和斜杠。
fn looks_like_address(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b':' || b == b'/')
}

/// 把一段多行文本解析成规则；坏行按行号记录下来，不影响其余行。
pub fn parse_rules_text(
    text: &str,
    action: Action,
    source: &str,
) -> (Vec<Rule>, Vec<(usize, RuleError)>) {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in text.lines().enumerate() {
        match parse_rule_line(line, action, source) {
            Ok(rule) => rules.push(rule),
            Err(RuleError::Empty) => {}
            Err(err) => errors.push((index + 1, err)),
        }
    }
    (rules, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffix(pattern: &str, action: Action, source: &str) -> Rule {
        Rule::new(action, Matcher::Suffix(pattern.to_string()), source)
    }

    fn direct_ip(cidr: &str, source: &str) -> Rule {
        Rule::new(
            Action::Direct,
            Matcher::Cidr(Cidr::parse(cidr).unwrap()),
            source,
        )
    }

    #[test]
    fn suffix_matches_on_label_boundaries_only() {
        let (set, stats) = RuleSet::build(&[suffix("qq.com", Action::Direct, "国内域名")]);
        assert_eq!(stats.suffix, 1);

        for hit in ["qq.com", "www.qq.com", "a.b.qq.com", "WWW.QQ.COM."] {
            assert!(set.match_domain(hit).is_some(), "{hit} 应该命中");
        }
        // 这就是简单 `ends_with` 会误伤的地方。
        for miss in ["notqq.com", "qq.com.cn", "qq.com.evil.net", "com"] {
            assert!(set.match_domain(miss).is_none(), "{miss} 不该命中");
        }
    }

    #[test]
    fn exact_beats_a_shorter_suffix_and_a_longer_suffix_beats_a_shorter_one() {
        let (set, _) = RuleSet::build(&[
            suffix("qq.com", Action::Proxy, "组A"),
            suffix("www.qq.com", Action::Direct, "组B"),
            suffix("api.www.qq.com", Action::Proxy, "组C"),
        ]);

        assert_eq!(
            set.match_domain("www.qq.com").unwrap().action(),
            Action::Direct
        );
        assert_eq!(
            set.match_domain("x.www.qq.com").unwrap().action(),
            Action::Direct,
            "更长的后缀盖住更短的"
        );
        assert_eq!(
            set.match_domain("a.api.www.qq.com").unwrap().action(),
            Action::Proxy
        );
        assert_eq!(set.match_domain("other.qq.com").unwrap().action(), Action::Proxy);
    }

    #[test]
    fn exact_set_wins_over_suffix_regardless_of_order() {
        let (set, _) = RuleSet::build(&[
            suffix("example.com", Action::Proxy, "suffix"),
            Rule::new(
                Action::Direct,
                Matcher::Exact("www.example.com".to_string()),
                "exact",
            ),
        ]);
        assert_eq!(
            set.match_domain("www.example.com").unwrap().action(),
            Action::Direct
        );
        assert_eq!(
            set.match_domain("api.example.com").unwrap().action(),
            Action::Proxy
        );
    }

    #[test]
    fn keywords_are_tried_longest_first() {
        let (set, _) = RuleSet::build(&[
            Rule::new(Action::Direct, Matcher::Keyword("bilibili".into()), "kw-short"),
            Rule::new(
                Action::Proxy,
                Matcher::Keyword("api.bilibili".into()),
                "kw-long",
            ),
        ]);
        // 两个关键字都能命中时，更长的（更具体的）先用。
        let hit = set.match_domain("api.bilibili.com").unwrap();
        assert_eq!(hit.action(), Action::Proxy);
        assert_eq!(hit.describe(), "proxy keyword:api.bilibili [kw-long]");
        assert_eq!(
            set.match_domain("www.bilibili.com").unwrap().action(),
            Action::Direct
        );
    }

    #[test]
    fn disabled_rules_never_participate() {
        let mut rule = suffix("baidu.com", Action::Direct, "国内域名");
        rule.enabled = false;
        let (set, stats) = RuleSet::build(&[rule]);
        assert_eq!(stats.disabled, 1);
        assert!(set.match_domain("www.baidu.com").is_none());
    }

    #[test]
    fn the_first_rule_for_the_same_pattern_wins() {
        let (set, stats) = RuleSet::build(&[
            suffix("example.com", Action::Direct, "先"),
            suffix("example.com", Action::Proxy, "后"),
        ]);
        assert_eq!(stats.suffix, 1, "同一个模式只进集合一次");
        assert_eq!(
            set.match_domain("example.com").unwrap().action(),
            Action::Direct
        );
    }

    #[test]
    fn ip_matching_prefers_the_longest_prefix() {
        let (set, stats) = RuleSet::build(&[
            direct_ip("10.0.0.0/8", "局域网"),
            direct_ip("10.1.0.0/16", "自定义"),
            Rule::new(
                Action::Proxy,
                Matcher::Cidr(Cidr::parse("10.1.2.0/24").unwrap()),
                "例外",
            ),
        ]);
        assert_eq!(stats.direct_cidrs, 2);
        assert_eq!(stats.proxy_cidrs, 1);

        // 更长的前缀（代理那条 /24）赢。
        let hit = set.match_ip("10.1.2.9".parse().unwrap()).unwrap();
        assert_eq!(hit.action(), Action::Proxy);
        assert!(hit.describe().contains("10.1.2.0/24"));

        // 不在 /24 里、但在 /16 里。
        assert_eq!(
            set.match_ip("10.1.9.9".parse().unwrap()).unwrap().action(),
            Action::Direct
        );
        // 只在 /8 里。
        assert_eq!(
            set.match_ip("10.9.9.9".parse().unwrap()).unwrap().action(),
            Action::Direct
        );
        assert!(set.match_ip("172.31.0.1".parse().unwrap()).is_none());
    }

    #[test]
    fn the_same_prefix_length_resolves_to_proxy() {
        // 同一条网段上「直连」和「强制走代理」并存时选代理：用户的排除意图更明确，
        // 也避免误判把流量放出去。
        let (set, _) = RuleSet::build(&[
            direct_ip("10.0.0.0/8", "局域网"),
            Rule::new(
                Action::Proxy,
                Matcher::Cidr(Cidr::parse("10.0.0.0/8").unwrap()),
                "排除",
            ),
        ]);
        assert_eq!(
            set.match_ip("10.0.0.1".parse().unwrap()).unwrap().action(),
            Action::Proxy
        );
    }

    #[test]
    fn decide_prefers_the_domain_and_falls_back_to_the_address() {
        let (set, _) = RuleSet::build(&[
            suffix("baidu.com", Action::Direct, "国内域名"),
            direct_ip("10.0.0.0/8", "局域网"),
        ]);

        // 域名命中：即便同时给了 IP 也用域名的结论。
        let hit = set
            .decide(Some("www.baidu.com"), Some("10.0.0.1".parse().unwrap()))
            .unwrap();
        assert!(matches!(hit, Match::Domain { .. }));

        // 域名没命中 → 看 IP。
        let hit = set
            .decide(Some("unknown.example"), Some("10.0.0.1".parse().unwrap()))
            .unwrap();
        assert!(matches!(hit, Match::Ip { .. }));

        // 两个都没命中 → None（调用方按代理兜底）。
        assert!(set
            .decide(Some("unknown.example"), Some("172.31.0.1".parse().unwrap()))
            .is_none());
    }

    #[test]
    fn domain_normalisation_is_boring_but_predictable() {
        assert_eq!(normalize_domain("WWW.QQ.COM"), Some("www.qq.com".into()));
        assert_eq!(normalize_domain("www.qq.com."), Some("www.qq.com".into()));
        assert_eq!(normalize_domain("www.qq.com:443"), Some("www.qq.com".into()));
        assert_eq!(normalize_domain("*.qq.com"), Some("qq.com".into()));
        assert_eq!(normalize_domain(".qq.com"), Some("qq.com".into()));
        assert_eq!(normalize_domain("  qq.com  "), Some("qq.com".into()));
        assert_eq!(normalize_domain(""), None);
        assert_eq!(normalize_domain("   "), None);
        assert_eq!(normalize_domain("has space.com"), None);
        assert_eq!(normalize_domain("path/x.com"), None);
        // 端口不是数字时不该被吃掉。
        assert_eq!(normalize_domain("a.com:http"), None);
    }

    #[test]
    fn rule_lines_accept_cidrs_domains_and_prefixes() {
        let cidr = parse_rule_line("10.0.0.0/8", Action::Direct, "test").unwrap();
        assert_eq!(cidr.matcher.kind(), MatcherKind::Cidr);

        let host = parse_rule_line("192.168.1.1", Action::Direct, "test").unwrap();
        assert_eq!(host.matcher, Matcher::Cidr(Cidr::parse("192.168.1.1/32").unwrap()));

        let suffix_rule = parse_rule_line("baidu.com", Action::Direct, "test").unwrap();
        assert_eq!(suffix_rule.matcher, Matcher::Suffix("baidu.com".into()));

        let wildcard = parse_rule_line("*.baidu.com", Action::Direct, "test").unwrap();
        assert_eq!(wildcard.matcher, Matcher::Suffix("baidu.com".into()));

        let full = parse_rule_line("full:www.baidu.com", Action::Direct, "test").unwrap();
        assert_eq!(full.matcher, Matcher::Exact("www.baidu.com".into()));

        let keyword = parse_rule_line("keyword:BiliBili", Action::Direct, "test").unwrap();
        assert_eq!(keyword.matcher, Matcher::Keyword("bilibili".into()));

        // 行尾注释（内置清单里带备案号）。
        let commented =
            parse_rule_line("hiocloud.com # 浙ICP备2020040255号-2", Action::Direct, "test").unwrap();
        assert_eq!(commented.matcher, Matcher::Suffix("hiocloud.com".into()));

        assert_eq!(
            parse_rule_line("   ", Action::Direct, "test"),
            Err(RuleError::Empty)
        );
        assert!(matches!(
            parse_rule_line("bad domain", Action::Direct, "test"),
            Err(RuleError::BadDomain(_))
        ));
        // 「像地址但拼错」要报地址错误，别说是域名不合法。
        assert!(matches!(
            parse_rule_line("10.0.0.0/33", Action::Direct, "test"),
            Err(RuleError::BadCidr(_))
        ));
    }

    #[test]
    fn bad_lines_are_reported_with_their_line_numbers() {
        let text = "baidu.com\n\nnot a domain\n10.0.0.0/8\n";
        let (rules, errors) = parse_rules_text(text, Action::Direct, "test");
        assert_eq!(rules.len(), 2, "空行不算错，坏行要单独报");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, 3);
    }

    #[test]
    fn an_empty_rule_set_matches_nothing() {
        let (set, stats) = RuleSet::build(&[]);
        assert!(set.is_empty());
        assert_eq!(stats.total(), 0);
        assert!(set.decide(Some("baidu.com"), Some("10.0.0.1".parse().unwrap())).is_none());
    }
}
