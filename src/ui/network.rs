//! 网络与绕过页：当前配置的网卡参数、UDP 转发与绕过列表。
//!
//! 和「代理配置 → 编辑」有一处重叠（绕过列表两边都能改）。旧界面就是这样：
//! 绕过地址本来就是每套配置自己的字段，在详情页里改是顺手的，在「网络与绕过」
//! 这一页集中改也是顺手的，两个入口共用同一份数据，不存在谁覆盖谁。
//!
//! 这一页改的都是**当前那套配置**，改完要重启隧道才生效——每块都写着这句话。

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::core::config::{Profile, DEFAULT_BYPASS};
use crate::core::i18n::t;

use super::state;
use super::widgets::{
    action_row, card_head, card_shell, field, ghost_button, note_result, primary_button,
    text_area, text_field, toggle, Note,
};

/// 这一页的界面状态。
///
/// 文本框都是草稿：UDP 转发三项与绕过列表在点保存之前不进配置，敲到一半的
/// `10.0.0.0/3` 不该被当成一条规则去校验。
#[derive(Clone, Copy)]
pub struct NetworkUi {
    /// 当前配置被改过几次（开关改完加一，页面据此重绘）。
    rev: Signal<u64>,
    udpgw_server: Signal<String>,
    udpgw_connections: Signal<String>,
    udpgw_keepalive: Signal<String>,
    udp_note: Signal<Option<Note>>,
    bypass: Signal<String>,
    bypass_note: Signal<Option<Note>>,
}

impl NetworkUi {
    pub fn new() -> Self {
        let profile = current_profile();
        Self {
            rev: signal(0u64),
            udpgw_server: signal(profile.udpgw_server.clone()),
            udpgw_connections: signal(profile.udpgw_connections.to_string()),
            udpgw_keepalive: signal(profile.udpgw_keepalive.to_string()),
            udp_note: signal(None),
            bypass: signal(profile.bypass.clone()),
            bypass_note: signal(None),
        }
    }

    fn bump(self) {
        self.rev.set(self.rev.get() + 1);
    }
}

/// 当前那套配置（订阅 `rev`，开关一改就重读）。
fn current_profile() -> Profile {
    state::config()
        .map(|config| config.active_profile().clone())
        .unwrap_or_default()
}

fn profile(ui: NetworkUi) -> Profile {
    let _ = ui.rev.get();
    current_profile()
}

/// 页面入口。
pub fn page(ui: NetworkUi) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .flex_col()
            .gap_4()
            .child(network_card(ui))
            .child(udpgw_card(ui))
            .child(bypass_card(ui)),
    )
}

// —— 网络参数 ——

fn network_card(ui: NetworkUi) -> View {
    let settings = profile(ui);
    card_shell()
        .child(card_head(
            t("网络参数", "Network parameters"),
            Some(
                t(
                    "改完要重启隧道才生效。MTU、超时与最大会话数在「代理配置 → 编辑」里。",
                    "A restart applies changes. MTU, timeouts and max sessions live under Profiles → Edit.",
                )
                .to_string(),
            ),
            view(),
        ))
        .child(field(
            t("启用 IPv6", "Enable IPv6"),
            Some(
                t(
                    "关掉后只转发 IPv4：IPv6 站点会先等超时再回落，访问会明显变慢。",
                    "When off only IPv4 is forwarded: IPv6 sites stall until the timeout, then fall back.",
                )
                .to_string(),
            ),
            toggle(settings.ipv6, move || {
                let next = !profile(ui).ipv6;
                if state::set_profile_flag(state::ProfileFlag::Ipv6, next) {
                    ui.bump();
                }
            }),
        ))
        .child(field(
            t("自动配置系统路由", "Configure system routes"),
            Some(
                t(
                    "关掉后需要自己写路由规则，否则流量不会进隧道。",
                    "When off you have to write the routing rules yourself, otherwise traffic never enters the tunnel.",
                )
                .to_string(),
            ),
            toggle(settings.setup, move || {
                let next = !profile(ui).setup;
                if state::set_profile_flag(state::ProfileFlag::Setup, next) {
                    ui.bump();
                }
            }),
        ))
        .child(field(
            t("卸载模式", "Unshare"),
            Some(
                t(
                    "Linux 上把隧道放进独立的网络命名空间；Windows 上没有作用，保持关闭即可。",
                    "On Linux the tunnel runs in its own network namespace; on Windows it has no effect, keep it off.",
                )
                .to_string(),
            ),
            toggle(settings.unshare, move || {
                let next = !profile(ui).unshare;
                if state::set_profile_flag(state::ProfileFlag::Unshare, next) {
                    ui.bump();
                }
            }),
        ))
}

// —— UDP 转发 ——

fn udpgw_card(ui: NetworkUi) -> View {
    card_shell()
        .child(card_head(
            t("UDP 转发", "UDP relay"),
            Some(
                t(
                    "让 UDP（DNS、游戏、视频通话）也能走上游代理；服务器留空表示不用这个功能。",
                    "Relays UDP (DNS, games, calls) through the upstream proxy; leave the server empty to disable it.",
                )
                .to_string(),
            ),
            view(),
        ))
        .child(field(
            t("转发服务器", "Relay server"),
            Some(
                t(
                    "host:port，例如 127.0.0.1:7300。",
                    "host:port, e.g. 127.0.0.1:7300.",
                )
                .to_string(),
            ),
            text_field(
                move || ui.udpgw_server.get(),
                "127.0.0.1:7300",
                true,
                move |text| {
                    ui.udpgw_server.set(text);
                    ui.udp_note.set(None);
                },
            ),
        ))
        .child(field(
            t("连接数", "Connections"),
            Some(
                t(
                    "同时保持几条到转发服务器的连接，默认 5。",
                    "How many connections to the relay are kept open; 5 by default.",
                )
                .to_string(),
            ),
            text_field(
                move || ui.udpgw_connections.get(),
                "5",
                true,
                move |text| {
                    ui.udpgw_connections.set(text);
                    ui.udp_note.set(None);
                },
            ),
        ))
        .child(field(
            t("保活间隔（秒）", "Keepalive (s)"),
            Some(
                t(
                    "空闲连接多久发一次保活包，默认 30。",
                    "How often an idle connection sends a keepalive; 30 by default.",
                )
                .to_string(),
            ),
            text_field(
                move || ui.udpgw_keepalive.get(),
                "30",
                true,
                move |text| {
                    ui.udpgw_keepalive.set(text);
                    ui.udp_note.set(None);
                },
            ),
        ))
        // 按钮放在字段下方而不是卡片头：滚动之后卡片头那一行的点击会落空
        // （命中区还停在没滚动时的位置），字段下面的按钮不受影响。
        .child(action_row(vec![primary_button(t("保存", "Save"), move || {
            let verdict = save_udpgw(ui);
            ui.udp_note.set(Some(verdict));
        })]))
        .child(dynamic(move || note_result(ui.udp_note.get()).into_element()))
}

fn save_udpgw(ui: NetworkUi) -> Note {
    let connections = match state::parse_number(
        &ui.udpgw_connections.get(),
        t("连接数", "Connections"),
        1,
        64,
    ) {
        Ok(value) => value as usize,
        Err(error) => return Note::bad(error),
    };
    let keepalive =
        match state::parse_number(&ui.udpgw_keepalive.get(), t("保活间隔", "Keepalive"), 1, 3600) {
            Ok(value) => value,
            Err(error) => return Note::bad(error),
        };
    match state::save_profile_udpgw(ui.udpgw_server.get(), connections, keepalive) {
        Ok(()) => Note::ok(t(
            "已保存，重启隧道后生效",
            "Saved; applies after a restart",
        )),
        Err(error) => Note::bad(error),
    }
}

// —— 路由绕过 ——

fn bypass_card(ui: NetworkUi) -> View {
    card_shell()
        .child(card_head(
            t("路由绕过", "Bypass list"),
            Some(
                t(
                    "每行一个 IP 或 CIDR；这些地址直连，不经过上游代理。tun2proxy 会把整行当地址解析，所以写不进注释。",
                    "One IP or CIDR per line; these are dialed directly, bypassing the proxy. tun2proxy parses the whole line as an address, so comments are not allowed.",
                )
                .to_string(),
            ),
            view(),
        ))
        .child(text_area(
            move || ui.bypass.get(),
            "10.0.0.0/8",
            148.0,
            move |text| {
                ui.bypass.set(text);
                ui.bypass_note.set(None);
            },
        ))
        .child(action_row(vec![
            ghost_button(t("填入默认值", "Fill in defaults"), move || {
                ui.bypass.set(DEFAULT_BYPASS.to_string());
                ui.bypass_note.set(None);
            }),
            primary_button(t("保存", "Save"), move || {
                let verdict = match state::save_profile_bypass(ui.bypass.get()) {
                    Ok(()) => Note::ok(t(
                        "已保存，重启隧道后生效",
                        "Saved; applies after a restart",
                    )),
                    Err(error) => Note::bad(error),
                };
                ui.bypass_note.set(Some(verdict));
            }),
        ]))
        .child(dynamic(move || note_result(ui.bypass_note.get()).into_element()))
        .child(
            view()
                .flex_col()
                .gap_1()
                .pt_1()
                .child(
                    text(t("写法示例", "Examples"))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().secondary_foreground),
                )
                .child(
                    text(t(
                        "照抄左边一列即可，右边是它代表什么。",
                        "Copy the left column as-is; the right side explains it.",
                    ))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(examples())
}

/// 写法示例：左列能直接粘进列表，右列说明它代表什么。
///
/// 单独摆一张表而不是写进输入框：tun2proxy 把整行当地址解析，示例里带 `#`
/// 注释的话，粘进去就会变成一条解析不出来的地址。
fn examples() -> View {
    const EXAMPLES: [(&str, &str, &str); 5] = [
        (
            "192.168.0.0/16",
            "整个局域网段，家用路由最常见",
            "A whole LAN range, the most common home router case",
        ),
        (
            "10.0.0.0/8",
            "公司内网、容器网络",
            "Corporate intranet, container networks",
        ),
        (
            "172.16.0.0/12",
            "Docker 默认网段的一部分",
            "Part of Docker's default range",
        ),
        (
            "10.1.2.3",
            "单个地址，不用写 /32",
            "A single address, no need for /32",
        ),
        (
            "224.0.0.0/4",
            "组播：投屏、mDNS 发现",
            "Multicast: screen casting, mDNS discovery",
        ),
    ];

    let mut table = view().flex_col().gap_1().pt_1();
    for (pattern, zh, en) in EXAMPLES {
        table = table.child(
            view()
                .flex_row()
                .items_center()
                .gap_4()
                .child(
                    view().w(px(140.0)).child(
                        text(pattern)
                            .text_xs()
                            .text_color(move || theme().card_foreground),
                    ),
                )
                .child(
                    view().flex_1().child(
                        text(t(zh, en))
                            .text_xs()
                            .truncate()
                            .text_color(move || theme().muted_foreground),
                    ),
                ),
        );
    }
    table
}
