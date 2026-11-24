//! 导航页：侧边栏里的顺序、名字与页面标题下的一行说明。
//!
//! 和 GPUI 时代的 [`crate::ui::state::Page`] 是同一个枚举、同一套文案，只是
//! 从「界面状态文件」里拆了出来：它是纯数据，任何界面框架下都长这样。

use crate::core::i18n::t;

/// 右侧内容区的五页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// 服务状态 + 当前配置速览。
    Dashboard,
    /// 多套代理配置的列表与代理 / DNS 参数。
    Profiles,
    /// 分流规则：国内 IP / 域名直连。
    Split,
    /// 网络参数、UDP 转发与路由绕过规则。
    Network,
    /// tun2proxy 二进制的下载 / 更新、运行方式与运行日志。
    Program,
}

impl Page {
    /// 侧边栏里的顺序。
    pub const ALL: [Page; 5] = [
        Page::Dashboard,
        Page::Profiles,
        Page::Split,
        Page::Network,
        Page::Program,
    ];

    /// 导航按钮与页面标题用的名字。
    pub fn label(self) -> &'static str {
        match self {
            Page::Dashboard => t("仪表盘", "Dashboard"),
            Page::Profiles => t("代理配置", "Profiles"),
            Page::Split => t("分流规则", "Split routing"),
            Page::Network => t("网络与绕过", "Network"),
            Page::Program => t("程序与日志", "Program"),
        }
    }

    /// 页面标题下面的一行说明。
    pub fn subtitle(self) -> &'static str {
        match self {
            Page::Dashboard => t(
                "服务状态、当前使用的配置与最近输出。",
                "Service status, the active profile and recent output.",
            ),
            Page::Profiles => t(
                "维护多套代理配置，随时切换要用的那套。",
                "Keep several profiles and switch between them.",
            ),
            Page::Split => t(
                "国内 IP 与域名直连，其余流量照常走代理。",
                "China IPs and domains go direct; everything else uses the proxy.",
            ),
            Page::Network => t(
                "TUN 网卡、UDP 转发与绕过规则。",
                "TUN adapter, UDP relay and bypass rules.",
            ),
            Page::Program => t(
                "tun2proxy 二进制的下载更新与实时日志。",
                "Binary updates and the live log.",
            ),
        }
    }
}
