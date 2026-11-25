//! 「分流规则」页要的数据与动作。
//!
//! 这一页管的是「哪些流量不该走代理」：总开关、三组内置清单、自定义规则、
//! 单条规则的开关、直连出口，外加深层清单的更新。运行期的计数与最近判定
//! 不在这里——它们每秒都在变，属于 [`super::Snapshot`]。
//!
//! # 为什么大多数动作是「一改一存」
//!
//! 分流设置是**一个整体**（`config.json` 里的 `split`）：单条规则开关、组开关、
//! 总开关改完立刻落盘，没有「编辑态 / 保存」这一步。唯一的例外是两段长文本
//! （自定义规则、绕过地址）：它们是用户一行行敲出来的，得点保存才进配置——
//! 敲到一半的 `10.0.0.0/3` 不该被当成规则去校验。

use serde::Serialize;

use crate::core::split::settings::{SplitGroup, DEFAULT_DIRECT_DNS};
use crate::core::split::SplitSettings;

use super::state;

/// 明细里最多给前端多少条。
///
/// 域名清单一万六千多条，一次全塞进 JSON 既没必要（没人会一行行翻）也会让
/// 每帧的解析变慢；前 100 条配上查找框足够定位到想关的那一条。
pub const RULE_ROWS: usize = 100;

/// 一个内置清单分组。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    /// 稳定的字符串 id：`local` / `chinaIp` / `chinaDomain`（动作往回传它）。
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// 这一组有多少条规则。
    pub count: usize,
    pub enabled: bool,
}

/// 这一页的静态设置。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitView {
    pub enabled: bool,
    pub groups: Vec<GroupView>,
    /// 自定义规则原文（一行一条）。
    pub custom: String,
    pub direct_dns: String,
    pub direct_interface: String,
    /// 当前配置的 DNS 策略 id 与它的名字。
    ///
    /// 域名清单只在「虚拟 IP」策略下看得见：tun2proxy 在别的策略里交给分流器的
    /// 只有 IP 字面量。把这两个摆出来，页面才能解释「为什么域名规则没生效」。
    pub dns_mode: &'static str,
    pub dns_mode_label: &'static str,
    /// 只填 IP 的直连 DNS 默认值（输入框的占位符）。
    pub default_direct_dns: &'static str,
}

impl SplitView {
    /// 读一次配置。
    pub fn now() -> Self {
        let config = state::config().unwrap_or_default();
        let settings = config.split.clone();
        let profile = config.active_profile();
        let counts = group_counts();
        Self {
            enabled: settings.enabled,
            groups: SplitGroup::ALL
                .iter()
                .enumerate()
                .map(|(index, group)| GroupView {
                    id: id_of(*group),
                    label: group.label(),
                    hint: group.hint(),
                    count: counts[index],
                    enabled: settings.group_enabled(*group),
                })
                .collect(),
            custom: settings.custom.clone(),
            direct_dns: settings.direct_dns.clone(),
            direct_interface: settings.direct_interface.clone(),
            dns_mode: dns_id(profile.dns),
            dns_mode_label: profile.dns.label(),
            default_direct_dns: DEFAULT_DIRECT_DNS,
        }
    }
}

/// 规则明细里的一行。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleView {
    /// 行开关的键（`suffix:qq.com`）。
    pub key: String,
    /// 展开后的匹配内容。
    pub text: String,
    /// 匹配类型（后缀 / 精确 / 关键词 / 网段）。
    pub kind: &'static str,
    /// 这条规则来自哪份清单。
    pub source: String,
    pub enabled: bool,
}

/// 三组内置清单各有多少条。
fn group_counts() -> [usize; 3] {
    let (local, china_ip, china_domain) = state::split_counts();
    [local, china_ip, china_domain]
}

/// 某个分组在配置文件里的名字。
fn id_of(group: SplitGroup) -> &'static str {
    match group {
        SplitGroup::LocalNetworks => "local",
        SplitGroup::ChinaIp => "chinaIp",
        SplitGroup::ChinaDomain => "chinaDomain",
    }
}

/// 反过来：前端传回来的 id → 分组。认不出来就是 `None`。
fn group_of(id: &str) -> Option<SplitGroup> {
    SplitGroup::ALL
        .into_iter()
        .find(|group| id_of(*group) == id)
}

/// DNS 策略的稳定 id。
fn dns_id(mode: crate::core::config::DnsMode) -> &'static str {
    use crate::core::config::DnsMode;
    match mode {
        DnsMode::Virtual => "virtual",
        DnsMode::Direct => "direct",
        DnsMode::OverTcp => "tcp",
    }
}

/// 当前的分流设置（动作内部用）。
fn settings() -> SplitSettings {
    state::config().map(|config| config.split).unwrap_or_default()
}

/// 展开后的规则明细：只列**开着的那几组**，`filter` 非空时按内容过滤。
///
/// 顺序固定为「局域网 → 国内 IP → 国内域名」，截断时也是从前 100 条截，
/// 这样同一份设置每次看到的都是同一段。
pub fn rules(filter: &str, limit: usize) -> Vec<RuleView> {
    let settings = settings();
    let filter = filter.trim().to_lowercase();
    let limit = limit.clamp(1, RULE_ROWS);

    state::with_split_library(|library| {
        let mut rows = Vec::new();
        for group in SplitGroup::ALL {
            if !settings.group_enabled(group) {
                continue;
            }
            for rule in library.group_rules(group) {
                let text = rule.matcher.text();
                if !filter.is_empty() && !text.to_lowercase().contains(&filter) {
                    continue;
                }
                if rows.len() >= limit {
                    return rows;
                }
                let key = rule.key();
                rows.push(RuleView {
                    enabled: settings.row_enabled(&key),
                    key,
                    text,
                    kind: rule.matcher.kind().label(),
                    source: rule.source.clone(),
                });
            }
        }
        rows
    })
}

/// 开关总闸，返回改后的值。
pub fn toggle_enabled() -> bool {
    let next = !settings().enabled;
    if state::set_split_enabled(next) {
        next
    } else {
        !next
    }
}

/// 开关某一组，返回改后的值。认不出来的分组原样返回。
pub fn set_group(id: &str, enabled: bool) -> bool {
    match group_of(id) {
        Some(group) => {
            if state::set_split_group(group, enabled) {
                enabled
            } else {
                !enabled
            }
        }
        None => !enabled,
    }
}

/// 保存自定义规则。写法不对时错误信息能直接显示。
pub fn save_custom(custom: &str) -> Result<(), String> {
    state::save_split_custom(custom.to_string())
}

/// 翻转单条规则的开关。
pub fn toggle_row(key: &str) -> bool {
    state::toggle_split_row(key)
}

/// 保存直连出口。
pub fn save_outlet(direct_dns: &str, direct_interface: &str) -> Result<(), String> {
    state::save_split_outlet(direct_dns.to_string(), direct_interface.to_string())
}

/// 从 APNIC / v2fly 重新拉一份清单（异步，进度从快照的 `splitUpdate` 读）。
pub fn update_lists() {
    state::update_split_lists();
}
