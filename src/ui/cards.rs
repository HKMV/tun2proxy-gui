//! 各页的分组与行。
//!
//! 版式照 Windows 11 任务管理器的设置页：
//!
//! ```text
//! 当前配置                        ← 分组标题（卡片外，加粗小字）
//! ┌────────────────────────────┐
//! │ [图标] 标题          [控件] │  ← 一行
//! │        说明文字             │
//! ├────────────────────────────┤  ← 左右缩进的发丝线
//! │ [图标] 标题          [值]   │
//! └────────────────────────────┘
//! ```
//!
//! 每个函数只负责「把状态画出来 + 把操作变成 [`Msg`]」，不修改任何状态
//! （状态变更统一在 [`super::update`] 里完成）。

use crate::core::engine::ProbeState;
use crate::core::i18n::{t, tf, LangSetting};
use crate::core::{DnsMode, Status, Update, Verbosity};
use zsui::{
    checkbox, column, combo_box, primary_button, progress_bar, radio_button, row, text_editor,
    textbox, ColorRole, Dp, ThemeColorToken, ViewAlign, ViewNode, WidgetId, ZsIcon,
};

use super::chart::{legend_swatch, legend_text, traffic_chart};
use super::state::{App, Msg, Page, Snapshot};
use super::widgets::{
    action_row, caption_nowrap, compact_row, error_text, hero, hint, human_bytes, icon_action,
    labeled_action, log_line, notice, pill, push, row_actions, row_icon, row_value, setting_block,
    setting_row, setting_row_tone, settings_group, text_action, LOG_PANE_HEIGHT, LOG_SCROLL_ID,
    ROW_CONTROL, SPACE_M, SPACE_XS,
};

/// 「连通性检测」这一行：按钮 + 上一次的结论。
///
/// 结论写在说明里而不是弹窗：用户点完「检测」眼睛就在这一行上，
/// 结果出现在原地才不用再找一遍；颜色区分成功/失败。
fn probe_row(snap: &Snapshot) -> ViewNode<Msg> {
    let (desc, tone, action) = match &snap.probe {
        None => (
            t(
                "启动前先确认地址真的连得通（TCP + 协议握手）",
                "Checks that the address really answers (TCP + protocol handshake)",
            )
            .to_string(),
            None,
            labeled_action(t("检测", "Check"), ZsIcon::Search, Msg::CheckProxy),
        ),
        Some(ProbeState::Running) => (
            t("正在连接…", "Connecting…").to_string(),
            Some(ColorRole::SecondaryText),
            labeled_action(t("检测中…", "Checking…"), ZsIcon::Search, Msg::CheckProxy),
        ),
        Some(ProbeState::Done(report)) if report.ok => (
            report.detail.clone(),
            Some(ColorRole::Success),
            labeled_action(
                t("重新检测", "Check again"),
                ZsIcon::Refresh,
                Msg::CheckProxy,
            ),
        ),
        Some(ProbeState::Done(report)) => (
            report.detail.clone(),
            Some(ColorRole::Danger),
            labeled_action(
                t("重新检测", "Check again"),
                ZsIcon::Refresh,
                Msg::CheckProxy,
            ),
        ),
    };
    // 前导图标跟着结论变：失败时再挂一个对勾，读起来像「检测通过」。
    let icon = match &snap.probe {
        Some(ProbeState::Done(report)) if !report.ok => ZsIcon::Info,
        _ => ZsIcon::Check,
    };
    setting_row_tone(
        Some(row_icon(icon)),
        t("连通性检测", "Connectivity check"),
        Some(desc),
        tone,
        Some(action),
    )
}

/// 仪表盘上最多显示多少行日志。
const LOG_PREVIEW_LINES: usize = 12;

/// 状态文字 + 颜色，供侧边栏与首屏卡片共用。
pub fn status_label(status: &Status) -> (String, ColorRole) {
    match status {
        Status::Stopped => (
            t("未运行", "Not running").to_string(),
            ColorRole::SecondaryText,
        ),
        Status::Starting => (t("正在启动…", "Starting…").to_string(), ColorRole::Warning),
        Status::Running => (t("运行中", "Running").to_string(), ColorRole::Success),
        Status::Stopping => (t("正在停止…", "Stopping…").to_string(), ColorRole::Warning),
        Status::Failed(reason) => (
            tf(
                t("已停止：{reason}", "Stopped: {reason}"),
                &[("reason", reason)],
            ),
            ColorRole::Danger,
        ),
    }
}

/// 状态对应的主题色（画实心圆点时用）。
pub fn status_color(status: &Status) -> ThemeColorToken {
    match status {
        Status::Stopped => ThemeColorToken::TextSecondary,
        Status::Starting | Status::Stopping => ThemeColorToken::Warning,
        Status::Running => ThemeColorToken::Success,
        Status::Failed(_) => ThemeColorToken::Danger,
    }
}

// —— 仪表盘 ——

/// 首屏：状态胶囊 + 结论标题 + 一句说明 + 主操作。
///
/// 仪表盘要回答的第一个问题是「隧道通不通」，所以状态不是一行小字，
/// 而是这一屏的标题；启停按钮就贴在结论旁边，不用先去侧边栏找。
pub fn hero_card(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let (status_text, color) = status_label(&snap.status);
    let (title, desc) = match &snap.status {
        Status::Running => (
            t("Tun2Proxy 已就绪", "Tun2Proxy is ready"),
            t("系统流量正在通过 TUN 虚拟网卡转发到上游代理。", "System traffic is being forwarded to the upstream proxy through the TUN adapter."),
        ),
        Status::Starting => (t("正在启动…", "Starting…"), t("正在创建 TUN 网卡并配置系统路由，请稍候。", "Creating the TUN adapter and configuring system routes…")),
        Status::Stopping => (t("正在停止…", "Stopping…"), t("正在回收 TUN 网卡与路由，请稍候。", "Releasing the TUN adapter and routes…")),
        Status::Failed(_) => (t("启动失败", "Start failed"), t("上次运行异常退出，具体原因见下方运行日志。", "The last run exited abnormally. See the log below for details.")),
        Status::Stopped => (
            t("Tun2Proxy 未启动", "Tun2Proxy is not running"),
            t("选一套配置并点击启动，系统流量将通过 TUN 虚拟网卡转发。", "Pick a profile and start it: system traffic is then routed through the TUN adapter."),
        ),
    };

    // 首屏卡不再放启动/停止：启停在侧边栏常驻，首屏卡重复一个按钮反而
    // 让人以为只有这里能点。只有一种情况需要按钮——权限不够，而「以管理员
    // 身份重启」只有这里能一键完成（否则用户得关掉程序、右键、再打开）。
    let elevated = crate::core::elevate::is_elevated();
    let action = (!elevated).then(|| {
        primary_button(t("使用管理员重启", "Restart as administrator"))
            .on_click(Msg::RelaunchElevated)
    });

    let mut extras: Vec<ViewNode<Msg>> = Vec::new();
    if let Some(blocked) = &snap.blocked {
        extras.push(error_text(blocked.clone()));
    }
    if let Some(error) = &app.error {
        extras.push(error_text(error.clone()));
    }
    if let Some(notice) = &app.notice {
        extras.push(hint(notice.clone()));
    }
    // 权限提示带图标：它是一句「你现在做不了这件事」的告警，不是第四行说明。
    if let Some(tip) = crate::core::elevate::privilege_hint() {
        extras.push(notice(ZsIcon::Info, ColorRole::Warning, tip));
    }

    hero(
        pill(status_text, color),
        title,
        desc,
        action,
        extras,
        Some(traffic_panel(snap)),
    )
}

/// 首屏卡右侧的流量面板：标题 + 折线图 + 图例。
///
/// 宽度固定 208：首屏卡的左列是弹性宽度，图要有稳定的尺度，否则窗口一拉宽，
/// 同样一分钟的流量会被拉成两条平线，看不出峰谷。
const CHART_WIDTH: f32 = 208.0;

fn traffic_panel(snap: &Snapshot) -> ViewNode<Msg> {
    // 还没采到点（未运行 / 刚启动）时不要显示 0，那会被读成「没流量」。
    let live = !snap.traffic.is_empty();
    let rate = |value: u64| {
        if live {
            format!("{}/s", human_bytes(value))
        } else {
            "—".to_string()
        }
    };

    column([
        row([
            caption_nowrap(t("实时流量", "Live traffic")),
            push(),
            legend_text(t("最近 1 分钟", "Last minute")),
        ])
        .align(ViewAlign::Center),
        traffic_chart(&snap.traffic, CHART_WIDTH),
        row([
            legend_swatch(ColorRole::Accent),
            legend_text(tf(t("下行 {}", "Down {}"), &[("", &rate(snap.rate.down))])),
            legend_swatch(ColorRole::Success),
            legend_text(tf(t("上行 {}", "Up {}"), &[("", &rate(snap.rate.up))])),
        ])
        .gap(Dp::new(SPACE_XS))
        .align(ViewAlign::Center),
    ])
    .gap(Dp::new(SPACE_XS))
    .width(Dp::new(CHART_WIDTH))
    .align(ViewAlign::Stretch)
    .flex(0.0)
}

/// 「当前配置」：选哪套、它长什么样。
///
/// 下拉框放在右侧控件位（任务管理器的做法），不在左边铺满整行：
/// 配置名一般很短，铺满会留下一条空白的输入框。
pub fn profile_picker_group(app: &App) -> ViewNode<Msg> {
    let config = &app.config;
    let names: Vec<String> = config
        .profiles
        .iter()
        .map(|profile| profile.name.trim().to_string())
        .collect();
    let profile = app.profile();

    let mut rows = vec![
        setting_row(
            Some(row_icon(ZsIcon::File)),
            t("配置文件", "Profile"),
            Some(
                t(
                    "隧道用这套配置启动，切换后下次启动生效",
                    "The tunnel starts with this profile; switching applies on the next start",
                )
                .to_string(),
            ),
            Some(
                combo_box(names, Some(config.active))
                    .on_select(Msg::ProfileSelected)
                    .width(Dp::new(ROW_CONTROL)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::Tool)),
            t("上游代理", "Upstream proxy"),
            Some(
                t(
                    "流量经它转发；点右侧铅笔修改地址与 DNS",
                    "Traffic is forwarded through it. Use the pencil to edit the address and DNS.",
                )
                .to_string(),
            ),
            Some(row_actions([
                row_value(profile.summary()),
                icon_action(
                    t("编辑这套配置", "Edit this profile"),
                    ZsIcon::Edit,
                    Msg::ProfileEditStart(config.active),
                ),
            ])),
        ),
    ];

    // 空地址是启动失败的头号原因，在选配置的地方直接点名，别等点了启动才报错。
    if profile.proxy_url.trim().is_empty() {
        rows.push(setting_row(
            Some(row_icon(ZsIcon::Warning)),
            t(
                "这套配置还没填代理地址",
                "This profile has no proxy address yet",
            ),
            Some(
                t(
                    "点右侧的铅笔补上，否则启动会立刻失败",
                    "Fill it in with the pencil on the right, otherwise starting will fail",
                )
                .to_string(),
            ),
            Some(icon_action(
                t("去填写", "Fill it in"),
                ZsIcon::Edit,
                Msg::ProfileEditStart(config.active),
            )),
        ));
    }

    settings_group(t("当前配置", "Current profile"), rows)
}

/// 「运行参数」：三个关键数字。
///
/// 做成三行而不是三块并排小卡：任务管理器的设置页里，一行一个参数、
/// 值靠右对齐，扫读时视线只上下移动；并排小卡要左右跳。
pub fn stat_group(app: &App) -> ViewNode<Msg> {
    let profile = app.profile();
    let bypass = profile
        .bypass
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let adapter = if profile.tun_name.trim().is_empty() {
        t("自动", "Auto").to_string()
    } else {
        profile.tun_name.trim().to_string()
    };
    let sessions = app.draft.max_sessions.trim();
    let sessions = if sessions.is_empty() {
        t("默认", "Default").to_string()
    } else {
        tf(t("{sessions} 个", "{sessions}"), &[("sessions", sessions)])
    };

    settings_group(
        t("运行参数", "Runtime"),
        [
            setting_row(
                Some(row_icon(ZsIcon::Group)),
                t("网络适配器", "Network adapter"),
                Some(
                    t(
                        "TUN 网卡名称，留空由系统自动分配",
                        "TUN adapter name; leave empty to let the system pick one",
                    )
                    .to_string(),
                ),
                Some(row_value(adapter)),
            ),
            setting_row(
                Some(row_icon(ZsIcon::Tool)),
                t("分流与绕过", "Bypass"),
                Some(t("这些地址不走代理", "These addresses skip the proxy").to_string()),
                Some(row_value(tf(
                    t("{bypass} 条", "{bypass}"),
                    &[("bypass", &bypass.to_string())],
                ))),
            ),
            setting_row(
                Some(row_icon(ZsIcon::Inspector)),
                t("会话上限", "Session limit"),
                Some(
                    t(
                        "同时保持的最大连接数",
                        "Maximum number of concurrent connections",
                    )
                    .to_string(),
                ),
                Some(row_value(sessions)),
            ),
        ],
    )
}

/// 仪表盘上的日志：最近几行 + 去「程序与日志」看全部。
pub fn log_preview_group(snap: &Snapshot) -> ViewNode<Msg> {
    settings_group(
        t("运行日志", "Log"),
        [
            setting_row(
                None,
                t("实时输出", "Live output"),
                Some(tf(
                    t(
                        "显示最近 {LOG_PREVIEW_LINES} 行",
                        "Shows the last {LOG_PREVIEW_LINES} lines",
                    ),
                    &[("LOG_PREVIEW_LINES", &LOG_PREVIEW_LINES.to_string())],
                )),
                Some(row_actions([icon_action(
                    t("查看全部日志", "Open the full log"),
                    ZsIcon::ChevronRight,
                    Msg::PageSelected(Page::Program),
                )])),
            ),
            log_pane(snap, LOG_PREVIEW_LINES, LOG_PANE_HEIGHT.min(200.0), false),
        ],
    )
}

// —— 代理配置 ——

/// 配置列表（「代理配置」页的默认态）。
///
/// 每套配置一行：单选圈决定用哪套，标题是名字，说明是摘要，右侧是改 / 删。
/// 摘要放进说明行而不是标题旁边：代理地址本来就长，挤在一行里会把按钮
/// 推出卡片（ZSUI 不会自动换行，只会裁掉）。
pub fn profile_list_group(app: &App) -> ViewNode<Msg> {
    let config = &app.config;
    let mut rows: Vec<ViewNode<Msg>> = Vec::new();

    for (index, profile) in config.profiles.iter().enumerate() {
        rows.push(setting_row(
            Some(
                radio_button("", config.active == index)
                    .min_width(Dp::new(20.0))
                    .on_choose(Msg::ProfileSelected(index)),
            ),
            profile.name.trim(),
            Some(profile.summary()),
            Some(row_actions([
                icon_action(
                    t("编辑这套配置", "Edit this profile"),
                    ZsIcon::Edit,
                    Msg::ProfileEditStart(index),
                ),
                icon_action(
                    t("删除这套配置", "Delete this profile"),
                    ZsIcon::Delete,
                    Msg::ProfileRemovedAt(index),
                ),
            ])),
        ));
    }

    rows.push(action_row([
        labeled_action(t("新建", "New"), ZsIcon::Add, Msg::ProfileAdded),
        labeled_action(
            t("复制当前", "Duplicate"),
            ZsIcon::Copy,
            Msg::ProfileDuplicated,
        ),
        labeled_action(
            t("保存到磁盘", "Save to disk"),
            ZsIcon::Save,
            Msg::SaveConfig,
        ),
    ]));

    settings_group(
        tf(
            t("配置 · {} 套", "Profiles · {}"),
            &[("", &config.profiles.len().to_string())],
        ),
        rows,
    )
}

/// 单套配置的名字。
///
/// 这里只放「这一套叫什么」，不放保存按钮：保存是本页的主操作，全页只该有
/// 一个，位置在最后（见 [`save_bar`]）。之前名字组和代理组里各有一个「保存」，
/// 同一个动作出现两次，反而让人不确定点哪个才算存完。
pub fn profile_edit_group(app: &App) -> ViewNode<Msg> {
    let profile = app.profile();
    settings_group(
        tf(
            t("编辑配置 · {}", "Edit profile · {}"),
            &[("", profile.name.trim())],
        ),
        [setting_row(
            Some(row_icon(ZsIcon::Edit)),
            t("配置名称", "Profile name"),
            Some(
                t(
                    "侧边栏与下拉框里显示的名字",
                    "Shown in the sidebar and the picker",
                )
                .to_string(),
            ),
            Some(
                textbox(&profile.name)
                    .placeholder(t("配置名称", "Profile name"))
                    .on_change(Msg::ProfileRenamed)
                    .width(Dp::new(ROW_CONTROL)),
            ),
        )],
    )
}

/// 代理与 DNS。
pub fn proxy_group(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let profile = app.profile();
    let dns_labels: Vec<String> = DnsMode::ALL
        .into_iter()
        .map(|mode| mode.label().to_string())
        .collect();
    let dns_index = DnsMode::ALL
        .iter()
        .position(|mode| *mode == profile.dns)
        .unwrap_or(0);

    settings_group(
        t("代理连接", "Proxy"),
        [
            setting_row(
                Some(row_icon(ZsIcon::Tool)),
                t("上游代理地址", "Upstream proxy address"),
                Some(
                    t(
                        "socks5:// 或 http:// 开头，例如 socks5://127.0.0.1:7890",
                        "Starts with socks5:// or http://, e.g. socks5://127.0.0.1:7890",
                    )
                    .to_string(),
                ),
                Some(
                    textbox(&profile.proxy_url)
                        .placeholder("socks5://127.0.0.1:7890")
                        .on_change(Msg::ProxyUrl)
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
            probe_row(snap),
            setting_row(
                Some(row_icon(ZsIcon::Search)),
                t("DNS 处理模式", "DNS mode"),
                Some(profile.dns.hint().to_string()),
                Some(
                    combo_box(dns_labels, Some(dns_index))
                        .on_select(|index| Msg::SetDns(DnsMode::ALL[index]))
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
            setting_row(
                Some(row_icon(ZsIcon::Group)),
                t("远程 DNS 服务器", "Remote DNS server"),
                Some(
                    t(
                        "DNS 模式为 Override 时使用",
                        "Used when the DNS mode is Override",
                    )
                    .to_string(),
                ),
                Some(
                    textbox(&profile.dns_addr)
                        .placeholder("8.8.8.8")
                        .on_change(Msg::DnsAddr)
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
            setting_row(
                Some(row_icon(ZsIcon::Code)),
                t("虚拟地址池", "Virtual address pool"),
                Some(
                    t(
                        "接管 DNS 时分配给域名的假地址段",
                        "Fake range handed out to domains when DNS is intercepted",
                    )
                    .to_string(),
                ),
                Some(
                    textbox(&profile.virtual_dns_pool)
                        .placeholder("198.18.0.0/15")
                        .on_change(Msg::VirtualPool)
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
        ],
    )
}

// —— 网络与绕过 ——

/// 网络参数。
pub fn network_group(app: &App) -> ViewNode<Msg> {
    let profile = app.profile();

    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut rows = vec![
        setting_row(
            Some(row_icon(ZsIcon::Inspector)),
            "MTU",
            Some(
                t(
                    "虚拟网卡的最大传输单元，默认 1500",
                    "MTU of the virtual adapter, 1500 by default",
                )
                .to_string(),
            ),
            Some(
                textbox(&app.draft.mtu)
                    .on_change(Msg::MtuChanged)
                    .width(Dp::new(120.0)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::History)),
            t("TCP 超时（秒）", "TCP timeout (s)"),
            None,
            Some(
                textbox(&app.draft.tcp_timeout)
                    .on_change(Msg::TcpTimeout)
                    .width(Dp::new(120.0)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::History)),
            t("UDP 超时（秒）", "UDP timeout (s)"),
            None,
            Some(
                textbox(&app.draft.udp_timeout)
                    .on_change(Msg::UdpTimeout)
                    .width(Dp::new(120.0)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::Group)),
            t("最大会话数", "Max sessions"),
            Some(
                t(
                    "同时保持的连接上限，留空用默认值",
                    "Connection cap; leave empty for the default",
                )
                .to_string(),
            ),
            Some(
                textbox(&app.draft.max_sessions)
                    .placeholder(t("默认", "Default"))
                    .on_change(Msg::MaxSessions)
                    .width(Dp::new(120.0)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::Code)),
            t("TUN 名称", "TUN name"),
            Some(t("留空自动生成", "Leave empty to generate one").to_string()),
            Some(
                textbox(&profile.tun_name)
                    .placeholder(t("自动生成", "Auto-generated"))
                    .on_change(Msg::TunName)
                    .width(Dp::new(ROW_CONTROL)),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::Success)),
            t("启用 IPv6", "Enable IPv6"),
            Some(
                t(
                    "关闭时只转发 IPv4 流量",
                    "When off, only IPv4 traffic is forwarded",
                )
                .to_string(),
            ),
            Some(
                checkbox("", profile.ipv6)
                    .min_width(Dp::new(20.0))
                    .on_toggle(Msg::SetIpv6),
            ),
        ),
        setting_row(
            Some(row_icon(ZsIcon::Refresh)),
            t("自动配置系统路由", "Configure system routes"),
            Some(
                t(
                    "关闭后需要自己写路由规则",
                    "When off you have to write the routing rules yourself",
                )
                .to_string(),
            ),
            Some(
                checkbox("", profile.setup)
                    .min_width(Dp::new(20.0))
                    .on_toggle(Msg::SetSetupRoutes),
            ),
        ),
    ];

    // 开关的说明在左侧，右侧只剩一个勾选框。
    settings_group(t("网络参数", "Network parameters"), rows)
}

/// 路由绕过规则。
///
/// 两个入口共用这一组： 「网络与绕过」页（一级）和「代理配置 → 编辑」页
/// （详情）。绕过地址本来就是每套配置自己的字段（`Profile::bypass`），
/// 在详情页里也能改，用户就不用先切回一级页面找它。
pub fn bypass_group(app: &App) -> ViewNode<Msg> {
    settings_group(
        t("路由绕过", "Bypass list"),
        [
            setting_row(
                Some(row_icon(ZsIcon::Pin)),
                t("不走代理的地址", "Addresses that skip the proxy"),
                Some(t("每行一个 IP 或 CIDR；这些地址直连，不经过上游代理。", "One IP or CIDR per line; these are dialed directly, bypassing the upstream proxy.").to_string()),
                Some(row_actions([labeled_action(
                    t("填入默认值", "Fill in defaults"),
                    ZsIcon::Undo,
                    Msg::BypassDefaults,
                )])),
            ),
            setting_block(
                t("绕过列表", "Bypass list"),
                Some(t("默认已填好局域网与保留地址段，通常不用改", "LAN and reserved ranges are pre-filled; usually no need to change").to_string()),
                text_editor(&app.profile().bypass)
                    .on_change(Msg::BypassChanged)
                    .height(Dp::new(140.0)),
            ),
            // 示例单独摆一张表：列表是纯文本、每行一条地址，tun2proxy 会把整行
            // 当地址解析，所以注释和「#」「;」分隔符都写不进去——只能在旁边列出来。
            setting_block(
                t("写法示例", "Examples"),
                Some(t("照抄左边一列即可，右边是它代表什么", "Copy the left column as-is; the right side explains it").to_string()),
                bypass_examples(),
            ),
        ],
    )
}

/// 「不走代理的地址」的写法示例：左列是能直接粘进列表的写法，右列是它代表什么。
fn bypass_examples() -> ViewNode<Msg> {
    // 两种语言并排放在表里（而不是在 `const` 里调 `t()`：那需要 const fn）。
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

    let rows = EXAMPLES.map(|(pattern, zh, en)| {
        row([caption_nowrap(pattern), push(), caption_nowrap(t(zh, en))])
            .gap(Dp::new(SPACE_M))
            .align(ViewAlign::Center)
    });

    column(rows)
        .gap(Dp::new(SPACE_XS))
        .align(ViewAlign::Stretch)
}

// —— UDP 转发 ——

/// UDP 转发（`--udpgw`）。
///
/// 放在「网络与绕过」页：它决定的是「UDP 包往哪里走」，和 MTU、绕过规则同属
/// 网络行为。之前它在「高级选项」里，用户得先猜到「UDPGW」是什么才找得到。
pub fn udpgw_group(app: &App) -> ViewNode<Msg> {
    let profile = app.profile();

    settings_group(
        t("UDP 转发", "UDP forwarding"),
        [
            setting_row(
                Some(row_icon(ZsIcon::Send)),
                t("UDPGW 服务器", "UDPGW server"),
                Some(
                    t("ip:port。SOCKS5 不转发 UDP 时，由这台服务器代转；留空则不转发 UDP。", "ip:port. When SOCKS5 cannot carry UDP, this server relays it; leave empty to drop UDP.")
                        .to_string(),
                ),
                Some(
                    textbox(&profile.udpgw_server)
                        .placeholder(t("留空则不转发 UDP", "Leave empty to drop UDP"))
                        .on_change(Msg::UdpgwChanged)
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
            setting_row(
                Some(row_icon(ZsIcon::Group)),
                t("UDPGW 连接数", "UDPGW connections"),
                Some(t("同时保持的 UDP 转发连接数", "Number of UDP relay connections to keep").to_string()),
                Some(
                    textbox(&app.draft.udpgw_connections)
                        .on_change(Msg::UdpgwConnections)
                        .width(Dp::new(120.0)),
                ),
            ),
        ],
    )
}

// —— 程序与日志 ——

/// 运行方式：进程层面的两个开关。
///
/// 都不是「日常会改」的项，但它们是「这个程序怎么跑」而不是「网络怎么走」，
/// 所以归到程序页，而不是网络页。
pub fn runtime_group(app: &App) -> ViewNode<Msg> {
    let profile = app.profile();

    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut rows = vec![compact_row(
        t("致命错误时退出", "Exit on fatal error"),
        Some(
            checkbox("", profile.exit_on_fatal)
                .min_width(Dp::new(20.0))
                .on_toggle(Msg::SetExitOnFatal),
        ),
    )];

    // 用户命名空间是 Linux 专有选项，其它平台不显示。
    if cfg!(target_os = "linux") {
        rows.push(compact_row(
            t("免 root 运行", "Run without root"),
            Some(
                checkbox("", profile.unshare)
                    .min_width(Dp::new(20.0))
                    .on_toggle(Msg::SetUnshare),
            ),
        ));
    }

    settings_group(t("运行方式", "Process"), rows)
}

/// 窗口与托盘：关窗时的行为。
///
/// 单独一组而不是塞进「运行方式」：它管的是「窗口怎么关」，不是「进程怎么跑」，
/// 而误关窗口把隧道停掉是这个程序最容易踩的坑。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn window_group(app: &App) -> ViewNode<Msg> {
    // 两行都是「标题自己就说得清」的开关：不配前导图标，也不配第二行说明。
    // 带说明时卡片会被撑到两倍高，而 × / ⌄ 这类字形在 20dp 的图标位里
    // 又大又重，比文字还抢眼。
    let mut rows = vec![compact_row(
        t("关闭窗口时询问", "Ask when closing"),
        Some(
            checkbox("", app.config.confirm_close_to_tray)
                .min_width(Dp::new(20.0))
                .on_toggle(Msg::SetConfirmCloseToTray),
        ),
    )];

    // 没托盘就藏不起来（藏了就找不回来），这个动作只在 Windows 上给。
    if cfg!(windows) {
        rows.push(compact_row(
            t("最小化到托盘", "Minimize to tray"),
            Some(text_action(t("最小化", "Minimize"), Msg::MinimizeToTray)),
        ));
    }

    settings_group(t("窗口与托盘", "Window & tray"), rows)
}

// —— 程序与日志 ——

/// 界面语言：跟随系统 / 中文 / English。
///
/// 改完立即生效并落盘：语言是「看一眼就想换」的东西，不该要求再点一次保存。
pub fn language_group(app: &App) -> ViewNode<Msg> {
    let labels: Vec<String> = LangSetting::ALL
        .iter()
        .map(|setting| setting.label().to_string())
        .collect();
    let index = LangSetting::ALL
        .iter()
        .position(|setting| *setting == app.config.language)
        .unwrap_or(0);

    settings_group(
        t("界面语言", "Language"),
        [compact_row(
            t("界面语言", "Language"),
            Some(
                combo_box(labels, Some(index))
                    .on_select(|index| Msg::SetLanguage(LangSetting::ALL[index]))
                    .width(Dp::new(ROW_CONTROL)),
            ),
        )],
    )
}

/// tun2proxy 程序本体：版本、检查更新、下载安装。
///
/// 下载按钮只在「真有事可做」的时候出现：
///
/// - 没装 → 「下载并安装」（这是首次使用最该点的东西）；
/// - 装了且有新版本 → 「下载并更新」；
/// - 装了且已是最新 / 正在检查或下载中 → 不显示（按钮点了也没有任何效果，
///   反而让人以为能点）。
///
/// 之前它总是显示「下载并安装」，于是装完内核后界面上一直摆着一个会把
/// 同一版本再下一遍的按钮，看着像没装成功。
pub fn binary_group(snap: &Snapshot) -> ViewNode<Msg> {
    let version = snap
        .version
        .clone()
        .unwrap_or_else(|| t("未安装", "Not installed").to_string());
    let installed = snap.version.is_some();
    let update_available = matches!(snap.update, Update::Available { .. });

    let mut actions = vec![text_action(t("检查", "Check"), Msg::CheckUpdate)];
    if !snap.update.is_busy() {
        if !installed {
            actions.push(primary_button(t("下载并安装", "Download")).on_click(Msg::InstallUpdate));
        } else if update_available {
            actions.push(primary_button(t("下载并更新", "Update")).on_click(Msg::InstallUpdate));
        }
    }

    let mut rows = vec![
        compact_row(t("当前版本", "Version"), Some(row_value(version))),
        compact_row(
            t("检查更新", "Check for updates"),
            Some(row_actions(actions)),
        ),
    ];

    match &snap.update {
        Update::Downloading { got, total } => {
            let fraction = match total {
                Some(total) if *total > 0 => *got as f32 / *total as f32,
                _ => 0.0,
            };
            rows.push(setting_block(
                t("正在下载", "Downloading"),
                Some(match total {
                    Some(total) => format!("{} / {}", human_bytes(*got), human_bytes(*total)),
                    None => human_bytes(*got),
                }),
                progress_bar(fraction, 0.0..=1.0),
            ));
        }
        Update::Checking => rows.push(setting_row(
            Some(row_icon(ZsIcon::Search)),
            t("正在查询最新版本…", "Looking up the latest version…"),
            None,
            None,
        )),
        Update::Installing => rows.push(setting_row(
            Some(row_icon(ZsIcon::Save)),
            t("正在解压并安装…", "Extracting and installing…"),
            None,
            None,
        )),
        Update::Available { latest } => rows.push(setting_row(
            Some(row_icon(ZsIcon::Info)),
            tf(
                t("发现新版本 {latest}", "Version {latest} is available"),
                &[("latest", latest)],
            ),
            Some(t("点右侧「下载并更新」", "Click “Update” on the right").to_string()),
            None,
        )),
        Update::Latest { version } => rows.push(setting_row(
            Some(row_icon(ZsIcon::Success)),
            tf(
                t("已是最新版本（{version}）", "Up to date ({version})"),
                &[("version", version)],
            ),
            None,
            None,
        )),
        Update::Done { version } => rows.push(setting_row(
            Some(row_icon(ZsIcon::Success)),
            tf(
                t("已更新到 {version}", "Updated to {version}"),
                &[("version", version)],
            ),
            None,
            None,
        )),
        Update::Failed(reason) => rows.push(setting_row(
            Some(row_icon(ZsIcon::Error)),
            t("更新失败", "Update failed"),
            Some(reason.clone()),
            None,
        )),
        Update::Idle => {}
    }

    settings_group(t("tun2proxy 程序", "tun2proxy binary"), rows)
}

/// 运行日志（整页）+ 日志级别。
///
/// 日志级别放在这里而不是单独一页：它唯一的用途就是「让下面这块输出多些或少些」，
/// 摆在同一张卡里，调完立刻能看到效果。
pub fn log_group(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let profile = app.profile();
    let level_labels: Vec<String> = Verbosity::ALL
        .into_iter()
        .map(|level| level.label().to_string())
        .collect();
    let level_index = Verbosity::ALL
        .iter()
        .position(|level| *level == profile.verbosity)
        .unwrap_or(0);

    settings_group(
        t("运行日志", "Log"),
        [
            compact_row(
                t("日志级别", "Log level"),
                Some(
                    combo_box(level_labels, Some(level_index))
                        .on_select(|index| Msg::SetVerbosity(Verbosity::ALL[index]))
                        .width(Dp::new(ROW_CONTROL)),
                ),
            ),
            compact_row(
                t("实时输出", "Live output"),
                Some(row_actions([icon_action(
                    t("清空日志", "Clear log"),
                    ZsIcon::Backspace,
                    Msg::ClearLogs,
                )])),
            ),
            log_pane(snap, snap.logs.len(), LOG_PANE_HEIGHT, true),
        ],
    )
}

/// 底部说明。
pub fn about_group() -> ViewNode<Msg> {
    settings_group(
        t("关于", "About"),
        [
            setting_row(
                Some(row_icon(ZsIcon::App)),
                "Tun2Proxy GUI",
                Some(t("官方 tun2proxy 二进制的图形前端，Rust + ZSUI 原生界面", "A native front end for the official tun2proxy binary, built with Rust + ZSUI").to_string()),
                None,
            ),
            setting_row(
                Some(row_icon(ZsIcon::Info)),
                t("权限", "Permissions"),
                Some(t("Windows 需管理员权限，macOS 需 root，Linux 可免 root", "Administrator on Windows, root on macOS, root-free on Linux").to_string()),
                None,
            ),
        ],
    )
}

/// 日志窗格：固定高度 + 自己的滚动条。
///
/// 固定高度必须写在外层容器上，不能写在滚动容器自己身上：滚动容器的
/// 高度就是「视口高度」，给它写死高度会让「内容高度 = 视口高度」，
/// 滚动条永远不会出现。内层 `flex(1.0)` 填满外层，超出部分靠 `auto_scroll_y()` 滚。
fn log_pane(snap: &Snapshot, lines: usize, height: f32, scroll_id: bool) -> ViewNode<Msg> {
    if snap.logs.is_empty() {
        return setting_block(
            t("暂无日志", "No log yet"),
            Some(
                t(
                    "启动服务后会在这里显示 tun2proxy 的输出。",
                    "tun2proxy's output shows up here once the service starts.",
                )
                .to_string(),
            ),
            column([]),
        );
    }

    let tail: Vec<ViewNode<Msg>> = snap
        .logs
        .iter()
        .rev()
        .take(lines)
        .rev()
        .map(|line| log_line(line.clone()))
        .collect();

    let mut pane = column([column(tail)
        .gap(Dp::new(2.0))
        .align(ViewAlign::Stretch)
        .id(WidgetId::new(LOG_SCROLL_ID))
        .align(ViewAlign::Stretch)
        .auto_scroll_y()
        .flex(1.0)])
    .height(Dp::new(height))
    .align(ViewAlign::Stretch);

    if !scroll_id {
        // 预览窗格不跟随滚动：它只是「最近几行」的剪影。
        pane = column([column(
            snap.logs
                .iter()
                .rev()
                .take(lines)
                .rev()
                .map(|line| log_line(line.clone()))
                .collect::<Vec<_>>(),
        )
        .gap(Dp::new(2.0))
        .align(ViewAlign::Stretch)])
        .height(Dp::new(height))
        .align(ViewAlign::Stretch);
    }

    column([pane])
        .padding(Dp::new(14.0))
        .align(ViewAlign::Stretch)
}
