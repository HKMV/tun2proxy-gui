//! 分流功能的用户设置（存在 `config.json` 里）。
//!
//! 和 [`crate::core::config::Profile`] 平级而不是放进某一套代理配置：分流规则
//! 描述的是「哪些站点不该走代理」，与用哪个代理无关；换个上游代理不该让用户
//! 重新勾一遍清单。
//!
//! # 「行单独开关」怎么存
//!
//! 内置清单有八千多条，把每条都存成 `{key, enabled}` 会把配置文件撑成几百 KB，
//! 而且清单更新后对不上。所以只存**被关掉的那些行**（[`SplitSettings::disabled`]），
//! 键是 [`crate::core::split::rules::Matcher::key`] 的文本（`suffix:qq.com`）：
//!
//! * 默认全开 → 配置文件里一个字节都不多；
//! * 清单更新后，被关掉的行依然对得上；
//! * 想恢复默认只要清空这个列表。

use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use crate::core::i18n::t;
use crate::tr_format;

use super::rules::{normalize_domain, parse_rule_line, Action, Matcher, Rule, RuleError};

/// 内置清单的三个分组。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitGroup {
    /// 本机 / 内网 / 保留地址。
    LocalNetworks,
    /// 国内 IP 段（APNIC 委派数据聚合）。
    ChinaIp,
    /// 国内域名后缀。
    ChinaDomain,
}

impl SplitGroup {
    pub const ALL: [SplitGroup; 3] = [
        SplitGroup::LocalNetworks,
        SplitGroup::ChinaIp,
        SplitGroup::ChinaDomain,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SplitGroup::LocalNetworks => t("局域网与保留地址", "LAN & reserved"),
            SplitGroup::ChinaIp => t("国内 IP 段", "China IP ranges"),
            SplitGroup::ChinaDomain => t("国内域名", "China domains"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SplitGroup::LocalNetworks => t(
                "本机、私有网段、组播与链路本地地址，任何情况下都直连",
                "Loopback, private ranges, multicast and link-local addresses always go direct",
            ),
            SplitGroup::ChinaIp => t(
                "APNIC 委派给中国大陆的地址段",
                "Address blocks delegated to mainland China by APNIC",
            ),
            SplitGroup::ChinaDomain => t(
                "中国大陆常用站点的域名后缀与精确域名",
                "Domain suffixes and exact domains of sites in mainland China",
            ),
        }
    }
}

/// 直连 DNS 的默认值。
///
/// 选 223.5.5.5（阿里公共 DNS）而不是 114.114.114.114 或 8.8.8.8：它是国内
/// 递归解析，返回的 CDN 节点在国内；用境外 DNS 解析国内站点会拿到更远的节点，
/// 「直连」反而比走代理还慢。
pub const DEFAULT_DIRECT_DNS: &str = "223.5.5.5";

/// 分流功能的一整份设置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitSettings {
    /// 总开关。关掉时**完全不起分流器**，`cli_args()` 与现在一字不差。
    pub enabled: bool,
    pub local_networks: bool,
    pub china_ip: bool,
    pub china_domain: bool,
    /// 自定义规则，一行一条（写法见 [`SplitSettings::parse_custom`]）。
    pub custom: String,
    /// 被单独关掉的行，键形如 `suffix:qq.com`。
    pub disabled: Vec<String>,
    /// 直连时用的 DNS 服务器（只填 IP，固定 53 端口）。
    pub direct_dns: String,
    /// 直连出口网卡：留空 = 自动判断；也可以填网卡名或接口索引。
    pub direct_interface: String,
}

impl Default for SplitSettings {
    fn default() -> Self {
        Self {
            // 默认关：老用户升级后行为必须与升级前完全一致。
            enabled: false,
            local_networks: true,
            china_ip: true,
            china_domain: true,
            custom: String::new(),
            disabled: Vec::new(),
            direct_dns: DEFAULT_DIRECT_DNS.to_string(),
            direct_interface: String::new(),
        }
    }
}

impl SplitSettings {
    pub fn group_enabled(&self, group: SplitGroup) -> bool {
        match group {
            SplitGroup::LocalNetworks => self.local_networks,
            SplitGroup::ChinaIp => self.china_ip,
            SplitGroup::ChinaDomain => self.china_domain,
        }
    }

    pub fn set_group(&mut self, group: SplitGroup, enabled: bool) {
        match group {
            SplitGroup::LocalNetworks => self.local_networks = enabled,
            SplitGroup::ChinaIp => self.china_ip = enabled,
            SplitGroup::ChinaDomain => self.china_domain = enabled,
        }
    }

    /// 这一行是否生效（没被单独关掉）。
    pub fn row_enabled(&self, key: &str) -> bool {
        !self.disabled.iter().any(|item| item == key)
    }

    /// 翻转某一行的开关，返回翻转后的状态。
    pub fn toggle_row(&mut self, key: &str) -> bool {
        if let Some(index) = self.disabled.iter().position(|item| item == key) {
            self.disabled.remove(index);
            true
        } else {
            self.disabled.push(key.to_string());
            false
        }
    }

    /// 直连 DNS 的地址（固定 53 端口）。
    pub fn direct_dns_addr(&self) -> Option<SocketAddr> {
        let text = self.direct_dns.trim();
        let ip: IpAddr = text.parse().ok()?;
        Some(SocketAddr::new(ip, 53))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.direct_dns_addr().is_none() {
            return Err(t(
                "直连 DNS 需要填一个 IP 地址，例如 223.5.5.5",
                "The direct DNS must be an IP address, e.g. 223.5.5.5",
            )
            .to_string());
        }
        if !(self.local_networks || self.china_ip || self.china_domain || self.has_custom()) {
            return Err(t(
                "至少打开一组清单，否则所有流量都会照常走代理",
                "Enable at least one list, otherwise every connection still uses the proxy",
            )
            .to_string());
        }
        self.parse_custom().map(|_| ())
    }

    /// 自定义规则里是否有内容（不含空行和注释）。
    pub fn has_custom(&self) -> bool {
        self.custom
            .lines()
            .map(str::trim)
            .any(|line| !line.is_empty() && !line.starts_with('#'))
    }

    /// 解析自定义规则。
    ///
    /// 写法：
    ///
    /// ```text
    /// qq.com              后缀匹配，直连
    /// suffix:qq.com       同上
    /// full:www.qq.com     精确匹配
    /// keyword:bilili      包含即命中
    /// 1.2.3.0/24   网段
    /// proxy:example.org   命中后强制走代理（用来抵消上面清单里的某一条）
    /// # 注释
    /// ```
    ///
    /// `proxy:` 前缀是刻意留下的逃生口：内置清单里难免有「国内注册、服务器在
    /// 境外」的域名，用户不该为了这一个域名去关掉整张清单。
    pub fn parse_custom(&self) -> Result<Vec<Rule>, String> {
        let mut rules = Vec::new();
        for (index, raw) in self.custom.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (action, body) = match line.split_once(':') {
                Some((prefix, rest)) if prefix.eq_ignore_ascii_case("proxy") => {
                    (Action::Proxy, rest.trim())
                }
                Some((prefix, rest)) if prefix.eq_ignore_ascii_case("direct") => {
                    (Action::Direct, rest.trim())
                }
                _ => (Action::Direct, line),
            };
            let rule = parse_rule_line(body, action, "自定义")
                .map_err(|err| line_error(index + 1, body, err))?;
            rules.push(rule);
        }
        Ok(rules)
    }
}

fn line_error(line: usize, text: &str, err: RuleError) -> String {
    let reason = match err {
        RuleError::Empty => t("这一行是空的", "This line is empty").to_string(),
        RuleError::BadDomain(_) => tr_format!(
            "「{}」不是合法域名",
            "“{}” is not a valid domain",
            text
        ),
        RuleError::BadCidr(_) => tr_format!(
            "「{}」不是合法的 IP 或 CIDR（是不是掩码位数写错了？）",
            "“{}” is not a valid IP or CIDR (check the prefix length)",
            text
        ),
    };
    tr_format!(
        "自定义规则第 {} 行：{}",
        "Custom rules, line {}: {}",
        line,
        reason
    )
}

/// 把一行文本做成「后缀」或「精确」规则（给界面上的「快速添加」用）。
pub fn rule_from_text(text: &str) -> Option<Matcher> {
    if text.contains('/') || text.parse::<IpAddr>().is_ok() {
        return CidrMatcher::parse(text);
    }
    normalize_domain(text).map(Matcher::Suffix)
}

/// 只在这里用一下 CIDR 解析，避免把 `ipset` 的类型撒得到处都是。
struct CidrMatcher;

impl CidrMatcher {
    fn parse(text: &str) -> Option<Matcher> {
        super::ipset::Cidr::parse(text).ok().map(Matcher::Cidr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_the_feature_off() {
        let settings = SplitSettings::default();
        assert!(!settings.enabled);
        assert!(settings.local_networks && settings.china_ip && settings.china_domain);
        assert!(settings.disabled.is_empty());
        assert_eq!(settings.direct_dns_addr().unwrap().port(), 53);
        settings.validate().expect("默认设置必须是合法配置");
    }

    #[test]
    fn row_toggles_round_trip() {
        let mut settings = SplitSettings::default();
        assert!(settings.row_enabled("suffix:qq.com"));
        assert!(!settings.toggle_row("suffix:qq.com"));
        assert!(!settings.row_enabled("suffix:qq.com"));
        assert!(settings.toggle_row("suffix:qq.com"));
        assert!(settings.row_enabled("suffix:qq.com"));
        assert!(settings.disabled.is_empty());
    }

    #[test]
    fn custom_rules_understand_the_escape_hatch() {
        let settings = SplitSettings {
            custom: "qq.com\nfull:www.qq.com\nproxy:img.example.org\n# 注释\n\n".to_string(),
            ..SplitSettings::default()
        };
        let rules = settings.parse_custom().expect("应当解析成功");
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].action, Action::Direct);
        assert_eq!(rules[0].matcher, Matcher::Suffix("qq.com".to_string()));
        assert_eq!(rules[1].matcher, Matcher::Exact("www.qq.com".to_string()));
        // `proxy:` 用来抵消内置清单里的某一条。
        assert_eq!(rules[2].action, Action::Proxy);
        assert_eq!(rules[2].matcher, Matcher::Suffix("img.example.org".to_string()));
    }

    #[test]
    fn bad_custom_lines_point_at_the_line_number() {
        let settings = SplitSettings {
            custom: "qq.com\n10.0.0.0/33\n".to_string(),
            ..SplitSettings::default()
        };
        let err = settings.parse_custom().unwrap_err();
        assert!(err.contains("第 2 行"), "{err}");
    }

    #[test]
    fn an_empty_selection_is_rejected() {
        let settings = SplitSettings {
            local_networks: false,
            china_ip: false,
            china_domain: false,
            ..SplitSettings::default()
        };
        assert!(settings.validate().is_err());
    }
}
