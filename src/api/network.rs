//! 「网络与绕过」页要的数据与动作。
//!
//! 这一页改的是**当前那套配置**里的网络参数：IPv6、系统路由、卸载模式、UDP
//! 转发三项，以及绕过清单。改完都要重启隧道才生效——分流器与 tun2proxy 都是
//! 启动时按当时的参数拉起来的。
//!
//! 和「代理配置 → 编辑」有一处重叠：绕过清单两边都能改。绕过程序本来就是每套
//! 配置自己的字段，两个入口共用同一份数据，不存在谁覆盖谁。
//!
//! # 表单为什么传字符串
//!
//! 和 [`super::profiles`] 同一个理由：连接数、保活间隔在表单里是用户敲的字符串，
//! 敲到一半的「3」不该被当成 3 存进配置。解析与范围检查留在后端，错误信息带上
//! 字段名，前端直接显示。

use serde::Serialize;

use crate::core::config::Profile;

use super::state;

/// 这一页的全部数据。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    /// 当前那套配置的名字（页面标题下那行「正在改 …」）。
    pub profile: String,
    pub ipv6: bool,
    pub setup: bool,
    pub unshare: bool,
    pub udpgw_server: String,
    /// 连接数是字符串：它要原样填回输入框。
    pub udpgw_connections: String,
    pub udpgw_keepalive: String,
    pub bypass: String,
    /// 局域网绕过清单的默认值（「填入默认值」按钮用）。
    pub default_bypass: &'static str,
}

impl NetworkView {
    /// 读一次当前配置。
    pub fn now() -> Self {
        let profile = current();
        Self {
            profile: profile.display_name().to_string(),
            ipv6: profile.ipv6,
            setup: profile.setup,
            unshare: profile.unshare,
            udpgw_server: profile.udpgw_server.clone(),
            udpgw_connections: profile.udpgw_connections.to_string(),
            udpgw_keepalive: profile.udpgw_keepalive.to_string(),
            bypass: profile.bypass.clone(),
            default_bypass: crate::core::config::DEFAULT_BYPASS,
        }
    }
}

/// 当前生效的那套配置（读不到时给一份默认值，字段都是安全的空）。
fn current() -> Profile {
    state::config()
        .map(|config| config.active_profile().clone())
        .unwrap_or_default()
}

/// 开关列表里那三个布尔项，返回改后的值。
pub fn set_flag(flag: &str, value: bool) -> bool {
    let flag = match flag {
        "ipv6" => state::ProfileFlag::Ipv6,
        "setup" => state::ProfileFlag::Setup,
        "unshare" => state::ProfileFlag::Unshare,
        _ => return !value,
    };
    if state::set_profile_flag(flag, value) {
        value
    } else {
        !value
    }
}

/// 保存 UDP 转发的三项。数字在这里解析，失败时错误信息能直接显示。
pub fn save_udpgw(server: &str, connections: &str, keepalive: &str) -> Result<(), String> {
    let connections = state::parse_number(connections, "连接数", 1, 64)? as usize;
    let keepalive = state::parse_number(keepalive, "保活间隔", 1, 3600)?;
    state::save_profile_udpgw(server.to_string(), connections, keepalive)
}

/// 保存绕过清单（每行一个 IP 或 CIDR）。
pub fn save_bypass(bypass: &str) -> Result<(), String> {
    state::save_profile_bypass(bypass.to_string())
}
