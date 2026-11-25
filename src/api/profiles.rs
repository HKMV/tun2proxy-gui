//! 「代理配置」页要的数据与动作。
//!
//! 一页两态：**列表**（点一行切过去、删、检测）和**编辑**（一套配置的全部参数）。
//! 业务侧只关心「有哪些配置、当前是哪套、改了要落盘」，界面怎么摆不管。
//!
//! # 表单为什么传字符串
//!
//! 数字字段（MTU、超时、会话数）在表单里先是**用户敲的字符串**：敲到一半的
//! 「15」不该被当成 15 存进配置。所以草稿里它们都是 `String`，保存时才解析，
//! 解析失败当场说「哪个字段、要什么」——这段判断留在后端，界面只管打字。

use serde::{Deserialize, Serialize};

use crate::core::config::{DnsMode, Profile, Verbosity};
use crate::core::paths;

use super::state;

/// 列表里的一套配置。
///
/// [`Profile`] 的字段原样摊平（`name`、`proxy_url`…，就是配置文件里的键名），
/// 加上两个界面要显示、但配置里不存在的派生字段。前端改一个字段再传回来，
/// 不用做「驼峰 ↔ 下划线」的翻译，也就不会漏字段。
#[derive(Serialize)]
pub struct ProfileItem {
    #[serde(flatten)]
    pub profile: Profile,
    /// 列表里的主标题（名字为空时给个占位）。
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// 列表里的副标题：地址 · DNS · 网卡。
    #[serde(rename = "summary")]
    pub summary: String,
}

/// 这一页的全部数据。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesView {
    /// 配置文件路径（页面底下那行「存在 …」）。
    pub path: String,
    /// 当前生效的那套的下标。
    pub active: usize,
    pub items: Vec<ProfileItem>,
}

impl ProfilesView {
    /// 读一次配置。
    pub fn now() -> Self {
        let config = state::config().unwrap_or_default();
        Self {
            path: paths::config_path().display().to_string(),
            active: config.active,
            items: config
                .profiles
                .iter()
                .map(|profile| ProfileItem {
                    profile: profile.clone(),
                    display_name: profile.display_name().to_string(),
                    summary: profile.summary(),
                })
                .collect(),
        }
    }
}

/// 表单里的一套值。数字是字符串，见模块注释。
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub name: String,
    pub proxy_url: String,
    pub dns: DnsMode,
    pub dns_addr: String,
    pub virtual_dns_pool: String,
    pub tun_name: String,
    pub mtu: String,
    pub tcp_timeout: String,
    pub udp_timeout: String,
    pub max_sessions: String,
    pub bypass: String,
    pub verbosity: Verbosity,
}

impl Draft {
    /// 一套配置 → 草稿（进编辑态时用）。
    pub fn of(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            proxy_url: profile.proxy_url.clone(),
            dns: profile.dns,
            dns_addr: profile.dns_addr.clone(),
            virtual_dns_pool: profile.virtual_dns_pool.clone(),
            tun_name: profile.tun_name.clone(),
            mtu: profile.mtu.to_string(),
            tcp_timeout: profile.tcp_timeout.to_string(),
            udp_timeout: profile.udp_timeout.to_string(),
            max_sessions: profile.max_sessions.to_string(),
            bypass: profile.bypass.clone(),
            verbosity: profile.verbosity,
        }
    }

    /// 合回一套完整配置：数字字段在这里解析，失败给出「哪个字段、要什么」。
    ///
    /// `base` 提供表单里没有的那些字段（`udpgw_*`、`ipv6`、`setup` 等），它们
    /// 在别的页面改，保存这一页不该把它们冲掉。
    pub fn into_profile(self, base: &Profile) -> Result<Profile, String> {
        let mut profile = base.clone();
        profile.name = self.name.trim().to_string();
        profile.proxy_url = self.proxy_url.trim().to_string();
        profile.dns = self.dns;
        profile.dns_addr = self.dns_addr.trim().to_string();
        profile.virtual_dns_pool = self.virtual_dns_pool.trim().to_string();
        profile.tun_name = self.tun_name.trim().to_string();
        profile.bypass = self.bypass;
        profile.verbosity = self.verbosity;
        profile.mtu = number(&self.mtu, "MTU", 576, 9000)? as u16;
        profile.tcp_timeout = number(&self.tcp_timeout, "TCP 超时", 1, 86_400)?;
        profile.udp_timeout = number(&self.udp_timeout, "UDP 超时", 1, 86_400)?;
        profile.max_sessions = number(&self.max_sessions, "最大会话数", 1, 1_000_000)? as usize;
        profile.validate()?;
        Ok(profile)
    }
}

/// 解析一个整数并检查范围，错误里带上字段名。
fn number(raw: &str, label: &'static str, low: u64, high: u64) -> Result<u64, String> {
    state::parse_number(raw, label, low, high)
}

/// 某套配置的草稿（进编辑态）。
pub fn draft(index: usize) -> Option<Draft> {
    let config = state::config()?;
    config.profiles.get(index).map(Draft::of)
}

/// 保存某套配置。表单内容坏了 / 数字越界 / 落盘失败，都返回能直接显示的原因。
pub fn save(index: usize, raw: &str) -> Result<(), String> {
    let config = state::config().unwrap_or_default();
    let base = config
        .profiles
        .get(index)
        .ok_or_else(|| "这套配置已经不在了".to_string())?;
    let draft: Draft = serde_json::from_str(raw)
        .map_err(|error| format!("表单内容解析失败：{error}"))?;
    let profile = draft.into_profile(base)?;
    if state::save_profile(index, profile) {
        Ok(())
    } else {
        Err("保存失败，看看日志里的原因".to_string())
    }
}
