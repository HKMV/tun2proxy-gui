//! 清单来源：优先用下载到用户目录的那一份，没有就用编译进二进制的快照。
//!
//! 两者格式完全一样（都是 [`super::lists`] 渲染出来的），所以解析代码只有一份。
//! 区别只在「从哪读」和「哪天的数据」：
//!
//! ```text
//! %APPDATA%\tun2proxy-gui\rules\china-ipv4.txt      ← 点了「立即更新」之后有
//! %APPDATA%\tun2proxy-gui\rules\china-ipv6.txt
//! %APPDATA%\tun2proxy-gui\rules\china-domain.txt
//! ```
//!
//! 三个文件必须**同时存在**才算一次完整的更新：只更新了一半（比如域名下载成功、
//! IP 下载失败）时宁可整体退回内置快照，也不要拿一份混了两个日期的表去分流。

use std::path::PathBuf;

use crate::core::paths;

use super::builtin;
use super::lists::parse_domain_list;
use super::rules::{parse_rules_text, Action, Rule, RuleSet};
use super::settings::{SplitGroup, SplitSettings};

/// 这份清单是哪来的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// 编译进二进制的那份。
    Builtin,
    /// 用户点「立即更新」下载下来的那份。
    Downloaded,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Builtin => crate::core::i18n::t("内置快照", "Built-in snapshot"),
            Origin::Downloaded => crate::core::i18n::t("已更新", "Updated"),
        }
    }
}

/// 更新后的清单放在这里。
pub fn snapshot_dir() -> PathBuf {
    paths::app_dir().join("rules")
}

/// 三份清单，已经解析成规则。
#[derive(Clone, Debug)]
pub struct Library {
    pub local: Vec<Rule>,
    pub ip: Vec<Rule>,
    pub domain: Vec<Rule>,
    pub origin: Origin,
    /// 数据日期（`2026-11-11`），读不出来时是空串。
    pub date: String,
}

impl Library {
    /// 从磁盘读；只要有一个文件缺失或解析不出东西，就整体退回内置快照。
    pub fn load() -> Self {
        let dir = snapshot_dir();
        let ipv4 = std::fs::read_to_string(dir.join("china-ipv4.txt")).ok();
        let ipv6 = std::fs::read_to_string(dir.join("china-ipv6.txt")).ok();
        let domain = std::fs::read_to_string(dir.join("china-domain.txt")).ok();
        match (ipv4, ipv6, domain) {
            (Some(ipv4), Some(ipv6), Some(domain)) => {
                let ip = parse_ip_rules(&ipv4, &ipv6);
                let domains = parse_domain_rules(&domain);
                if !ip.is_empty() && !domains.is_empty() {
                    return Self {
                        local: builtin::local_rules(),
                        ip,
                        domain: domains,
                        origin: Origin::Downloaded,
                        date: header_date(&domain),
                    };
                }
                // 文件在但内容是空的：多半是下载到了半截或被杀软清空，
                // 这种情况继续用内置的，界面上还能看到日期没变。
                Self::builtin()
            }
            _ => Self::builtin(),
        }
    }

    /// 内置快照（不碰磁盘）。
    pub fn builtin() -> Self {
        Self {
            local: builtin::local_rules(),
            ip: builtin::china_ip_rules(),
            domain: builtin::china_domain_rules(),
            origin: Origin::Builtin,
            date: header_date(builtin::CHINA_DOMAIN),
        }
    }

    pub fn group_rules(&self, group: SplitGroup) -> &[Rule] {
        match group {
            SplitGroup::LocalNetworks => &self.local,
            SplitGroup::ChinaIp => &self.ip,
            SplitGroup::ChinaDomain => &self.domain,
        }
    }

    pub fn group_len(&self, group: SplitGroup) -> usize {
        self.group_rules(group).len()
    }

    pub fn total_len(&self) -> usize {
        self.local.len() + self.ip.len() + self.domain.len()
    }

    /// 按设置筛出真正要生效的直连规则。
    ///
    /// 顺序有意义：**自定义规则排在最前**，这样用户写 `proxy:example.cn` 能盖住
    /// 后面同名的内置行（规则集里「同模式先到先得」）。
    pub fn direct_rules(&self, settings: &SplitSettings) -> Result<Vec<Rule>, String> {
        let mut rules = settings.parse_custom()?;
        let mut taken = std::collections::HashSet::new();
        for rule in &rules {
            taken.insert(rule.key());
        }
        for group in SplitGroup::ALL {
            if !settings.group_enabled(group) {
                continue;
            }
            for rule in self.group_rules(group) {
                let key = rule.key();
                if !settings.row_enabled(&key) {
                    continue;
                }
                if !taken.insert(key) {
                    // 自定义里已经有同一个模式了，内置的这条不重复加：重复加
                    // 只会让「谁先命中」变得依赖遍历顺序。
                    continue;
                }
                rules.push(rule.clone());
            }
        }
        Ok(rules)
    }

    /// 编译成规则集（供分流器热更新用）。
    pub fn rule_set(&self, settings: &SplitSettings) -> Result<RuleSet, String> {
        let rules = self.direct_rules(settings)?;
        Ok(RuleSet::build(&rules).0)
    }
}

fn parse_ip_rules(ipv4: &str, ipv6: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    for (text, source) in [(ipv4, "国内IPv4"), (ipv6, "国内IPv6")] {
        let (mut parsed, _errors) = parse_rules_text(text, Action::Direct, source);
        rules.append(&mut parsed);
    }
    rules
}

fn parse_domain_rules(text: &str) -> Vec<Rule> {
    parse_domain_list(text, "国内域名").rules
}

/// 从文件头注释里取日期：`# 来源：… 生成于 2026-11-11；…`
fn header_date(text: &str) -> String {
    for line in text.lines().take(4) {
        let Some(rest) = line.split("生成于").nth(1).or_else(|| line.split("dated ").nth(1)) else {
            continue;
        };
        let token: String = rest
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if token.len() >= 8 {
            return token;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_library_has_all_three_groups() {
        let library = Library::builtin();
        assert_eq!(library.origin, Origin::Builtin);
        assert!(library.group_len(SplitGroup::LocalNetworks) >= 9);
        assert!(library.group_len(SplitGroup::ChinaIp) > 7000);
        assert!(library.group_len(SplitGroup::ChinaDomain) > 8000);
        assert_eq!(
            library.total_len(),
            library.local.len() + library.ip.len() + library.domain.len()
        );
    }

    #[test]
    fn the_snapshot_carries_its_date() {
        let library = Library::builtin();
        assert!(!library.date.is_empty(), "内置快照头里应当写了生成日期");
        assert!(library.date.starts_with("20"), "{}", library.date);
    }

    #[test]
    fn custom_rules_come_first_and_shadow_the_builtin_ones() {
        let library = Library::builtin();
        let settings = SplitSettings {
            custom: "proxy:qq.com".to_string(),
            ..SplitSettings::default()
        };
        let rules = library.direct_rules(&settings).expect("应当能筛出来");
        // 自定义的 proxy 规则在最前，内置的 suffix:qq.com 被它顶掉。
        assert_eq!(rules[0].key(), "suffix:qq.com");
        assert_eq!(rules[0].action, Action::Proxy);
        let qq = rules.iter().filter(|rule| rule.key() == "suffix:qq.com").count();
        assert_eq!(qq, 1, "同一个模式不该出现两次");

        let set = RuleSet::build(&rules).0;
        assert_eq!(
            set.match_domain("www.qq.com").map(|hit| hit.action()),
            Some(Action::Proxy),
            "用户写了 proxy:qq.com，就必须真的走代理"
        );
    }

    #[test]
    fn disabled_rows_are_dropped() {
        let library = Library::builtin();
        let mut settings = SplitSettings::default();
        assert!(library
            .direct_rules(&settings)
            .unwrap()
            .iter()
            .any(|rule| rule.key() == "suffix:qq.com"));
        settings.disabled.push("suffix:qq.com".to_string());
        assert!(!library
            .direct_rules(&settings)
            .unwrap()
            .iter()
            .any(|rule| rule.key() == "suffix:qq.com"));
    }

    #[test]
    fn turning_a_group_off_removes_it() {
        let library = Library::builtin();
        let settings = SplitSettings {
            china_domain: false,
            ..SplitSettings::default()
        };
        let rules = library.direct_rules(&settings).unwrap();
        assert!(!rules.iter().any(|rule| rule.key() == "suffix:qq.com"));
        assert!(rules.iter().any(|rule| rule.key().starts_with("cidr:")));
    }
}
