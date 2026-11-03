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

use gpui::{
    AnyElement, Context, Div, Entity, Hsla, IntoElement, ScrollWheelEvent, SharedString, div, px,
    prelude::*,
};
use gpuikit::elements::progress::progress;
use gpuikit::elements::text_field::text_field;
use gpuikit::elements::textarea::textarea;
use gpuikit::input::InputState;
use gpuikit::theme::{Theme, Themeable};
use gpuikit::traits::control_sized::ControlSized;
use gpuikit::DefaultIcons;

use crate::core::engine::ProbeState;
use crate::core::i18n::{t, tf};
use crate::core::{Status, Update};

use super::chart::{LEGEND_GAP, legend_swatch, legend_text, traffic_chart};
use super::state::{App, Field, Msg, Page, Snapshot};
use super::theme::{TEXT_BODY, TEXT_SMALL};
use super::widgets::{
    ROW_CONTROL, ROW_CONTROL_NARROW, SPACE_M, SPACE_S, SPACE_XS, compact_row, error_text,
    hero_title, hint, human_bytes, icon, icon_action, labeled_action, log_line, primary_button,
    row_icon, row_value, setting_block, setting_row, settings_group, text_action, toggle_control,
    v_hairline,
};

/// 仪表盘日志框的高度。
///
/// 不再按行数截断：框是能滚的，只留 12 行的话滚一下就到底，等于还是看不见
/// 前面说过什么——而「刚才为什么停了」恰恰在更早的那几行里。
const LOG_PREVIEW_HEIGHT: f32 = 200.0;

/// 首屏卡右侧的流量面板宽度。
///
/// 固定宽度：首屏卡的左列是弹性宽度，图要有稳定的尺度，否则窗口一拉宽，
/// 同样一分钟的流量会被拉成两条平线，看不出峰谷。
const CHART_WIDTH: f32 = 208.0;

/// 状态文字 + 颜色，供侧边栏与首屏卡片共用。
pub fn status_label(status: &Status, theme: &Theme) -> (String, Hsla) {
    match status {
        Status::Stopped => (t("未运行", "Not running").to_string(), theme.fg_muted()),
        Status::Starting => (t("正在启动…", "Starting…").to_string(), theme.warning()),
        Status::Running => (t("运行中", "Running").to_string(), theme.success()),
        Status::Stopping => (t("正在停止…", "Stopping…").to_string(), theme.warning()),
        // 原因写在首屏的说明行里，不塞进胶囊：胶囊只有一行高，长原因会被
        // 截成半句话，反而更像没解释。
        Status::Failed(_) => (t("已停止", "Stopped").to_string(), theme.danger()),
    }
}

/// 控件 id 的拼装：同一个动作在一页里出现多次时，id 必须各不相同，
/// 否则 GPUI 会把它们当成同一个可交互元素（点击只命中第一个）。
fn eid(prefix: &str, index: usize) -> SharedString {
    SharedString::from(format!("{prefix}-{index}"))
}

// —— 输入控件 ——

/// 单行输入框，宽度固定。
///
/// `TextField` 自己不带宽度设置，只能在外面套一层固定宽度的容器：
/// 它内部是 `w_full`，跟着外层走。
fn text_input(state: &Entity<InputState>, width: f32, cx: &mut Context<App>) -> AnyElement {
    div()
        .flex_none()
        .w(px(width))
        .child(text_field(state, cx).full_width(true).large())
        .into_any_element()
}

/// 下拉框，宽度固定。
fn select_input(state: impl IntoElement, width: f32) -> AnyElement {
    div().flex_none().w(px(width)).child(state).into_any_element()
}

// —— 仪表盘 ——

/// 首屏：状态胶囊 + 结论标题 + 一句说明 + 权限提示 + 流量面板。
///
/// 仪表盘要回答的第一个问题是「隧道通不通」，所以状态不是一行小字，
/// 而是这一屏的标题。
pub fn hero_card(snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let (status_text, color) = status_label(&snap.status, theme);
    let (title, desc) = match &snap.status {
        Status::Running => (
            t("Tun2Proxy 已就绪", "Tun2Proxy is ready"),
            t(
                "系统流量正在通过 TUN 虚拟网卡转发到上游代理。",
                "System traffic is being forwarded to the upstream proxy through the TUN adapter.",
            )
            .to_string(),
        ),
        Status::Starting => (
            t("正在启动…", "Starting…"),
            t(
                "正在创建 TUN 网卡并配置系统路由，请稍候。",
                "Creating the TUN adapter and configuring system routes…",
            )
            .to_string(),
        ),
        Status::Stopping => (
            t("正在停止…", "Stopping…"),
            t(
                "正在回收 TUN 网卡与路由，请稍候。",
                "Releasing the TUN adapter and routes…",
            )
            .to_string(),
        ),
        // 退出原因直接进说明行：这一屏是用户「刚发现隧道没了」第一眼看的地方，
        // 让他还要去翻日志才知道为什么，等于没解释。
        Status::Failed(reason) => (
            t("已停止", "Stopped"),
            tf(
                t("隧道已经退出：{reason}", "The tunnel exited: {reason}"),
                &[("reason", reason)],
            ),
        ),
        Status::Stopped => (
            t("Tun2Proxy 未启动", "Tun2Proxy is not running"),
            t(
                "选一套配置并点击启动，系统流量将通过 TUN 虚拟网卡转发。",
                "Pick a profile and start it: system traffic is then routed through the TUN adapter.",
            )
            .to_string(),
        ),
    };

    // 首屏卡不放启动/停止：启停在侧边栏常驻，首屏卡重复一个按钮反而
    // 让人以为只有这里能点。只有一种情况需要按钮——权限不够，而「以管理员
    // 身份重启」只有这里能一键完成（否则用户得关掉程序、右键、再打开）。
    let elevated = crate::core::elevate::is_elevated();

    let mut text_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(SPACE_M))
        .child(div().flex().child(super::widgets::pill(status_text, color, theme)))
        .child(hero_title(title, theme))
        .child(hint(desc, theme));

    if !elevated {
        text_column = text_column.child(
            div().flex().child(primary_button(
                "relaunch-elevated",
                t("使用管理员重启", "Restart as administrator"),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::RelaunchElevated, cx)),
            )),
        );
    }

    // 说明行（启动被拦下的原因 / 权限提示）与上面那一组之间拉开距离，
    // 否则它们会读成「标题说明的第四行」。
    //
    // 保存结果和动作报错不在这里：它们由内容区顶部的提示条统一显示
    // （见 [`super::pages::content`]），否则「在代理配置页点保存」就看不到
    // 任何反馈——提示只画在仪表盘上，而按钮在别的页面。
    let mut extras = div().flex().flex_col().gap(px(SPACE_S));
    if let Some(blocked) = &snap.blocked {
        extras = extras.child(error_text(blocked.clone(), theme));
    }
    if let Some(tip) = crate::core::elevate::privilege_hint() {
        extras = extras.child(
            div()
                .flex()
                .items_start()
                .gap(px(SPACE_S))
                .child(icon(DefaultIcons::exclamation_triangle(), 16.0, theme.warning()))
                .child(div().flex_1().min_w_0().child(hint(tip, theme))),
        );
    }

    let card = div()
        .flex()
        .flex_col()
        .gap(px(SPACE_M))
        .p(px(20.0))
        .child(
            // 左文右图，中间一条发丝线。不加这条线时，两块内容只是「并排
            // 放着」，中间那 24dp 的空隙读起来像排版没排完；有了线，右侧
            // 就明确是「附属于这一屏的读数」，而不是另一块正文。
            //
            // `items_stretch` 让分隔线拿到整行的高度；右侧面板自己
            // `justify_center`，在拉高之后垂直居中，和左边那摞文字的重心对齐。
            div()
                .flex()
                .items_stretch()
                .gap(px(SPACE_M))
                .child(text_column)
                .child(v_hairline(theme))
                .child(traffic_panel(snap, theme)),
        )
        .child(extras);

    // surface 底 + 1dp 描边 + 与分组卡同一种圆角：首屏卡靠**内边距**和
    // 字号高一级，不靠另造一套盒子（gpuikit 的 `Card` 也是单层描边）。
    div()
        .flex_none()
        .rounded(px(super::widgets::CARD_RADIUS))
        .border_1()
        .border_color(theme.border())
        .bg(theme.surface())
        .overflow_hidden()
        .child(card)
        .into_any_element()
}

/// 首屏卡右侧的流量面板：标题 + 折线图 + 图例。
fn traffic_panel(snap: &Snapshot, theme: &Theme) -> AnyElement {
    // 还没采到点（未运行 / 刚启动）时不要显示 0，那会被读成「没流量」。
    let live = !snap.traffic.is_empty();
    let rate = |value: u64| {
        if live {
            format!("{}/s", human_bytes(value))
        } else {
            "—".to_string()
        }
    };

    div()
        .flex()
        .flex_col()
        .flex_none()
        .justify_center()
        .w(px(CHART_WIDTH))
        .gap(px(SPACE_XS))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(TEXT_BODY))
                        .text_color(theme.fg())
                        .child(t("实时流量", "Live traffic")),
                )
                .child(
                    div()
                        .text_size(px(TEXT_SMALL))
                        .text_color(theme.fg_muted())
                        .whitespace_nowrap()
                        .child(t("最近 1 分钟", "Last minute")),
                ),
        )
        .child(traffic_chart(&snap.traffic, CHART_WIDTH, theme))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(LEGEND_GAP))
                .child(legend_swatch(theme.accent(), theme))
                .child(legend_text(
                    tf(t("下行 {}", "Down {}"), &[("", &rate(snap.rate.down))]),
                    theme,
                ))
                // 上行色块必须和 chart.rs 里那条线取同一个颜色，否则图例在说谎。
                .child(legend_swatch(theme.success(), theme))
                .child(legend_text(
                    tf(t("上行 {}", "Up {}"), &[("", &rate(snap.rate.up))]),
                    theme,
                )),
        )
        .into_any_element()
}

/// 「当前配置」：选哪套、它长什么样。
///
/// 下拉框放在右侧控件位（任务管理器的做法），不在左边铺满整行：
/// 配置名一般很短，铺满会留下一条空白的输入框。
pub fn profile_picker_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let config = &app.config;
    let profile = app.profile();

    let mut rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::file(), theme).into_any_element()),
            t("配置文件", "Profile"),
            Some(
                t(
                    "隧道用这套配置启动，切换后下次启动生效",
                    "The tunnel starts with this profile; switching applies on the next start",
                )
                .to_string(),
            ),
            None,
            Some(select_input(
                app.selects.profile.clone(),
                ROW_CONTROL,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::gear(), theme).into_any_element()),
            t("上游代理", "Upstream proxy"),
            Some(
                t(
                    "流量经它转发；点右侧铅笔修改地址与 DNS",
                    "Traffic is forwarded through it. Use the pencil to edit the address and DNS.",
                )
                .to_string(),
            ),
            None,
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_S))
                    .child(div().max_w(px(220.0)).child(row_value(profile.summary(), theme)))
                    .child(icon_action(
                        eid("picker-edit", config.active),
                        t("编辑这套配置", "Edit this profile"),
                        DefaultIcons::pencil_1(),
                        cx.listener(move |this, _, _, cx| {
                            this.handle(Msg::ProfileEditStart(config_active(this)), cx)
                        }),
                    ))
                    .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    ];

    // 空地址是启动失败的头号原因，在选配置的地方直接点名，别等点了启动才报错。
    if profile.proxy_url.trim().is_empty() {
        rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::exclamation_triangle(), theme).into_any_element()),
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
                None,
                Some(icon_action(
                    "picker-fill",
                    t("去填写", "Fill it in"),
                    DefaultIcons::pencil_1(),
                    cx.listener(|this, _, _, cx| {
                        let index = this.config.active;
                        this.handle(Msg::ProfileEditStart(index), cx)
                    }),
                )
                .into_any_element()),
                theme,
            )
            .into_any_element(),
        );
    }

    settings_group(t("当前配置", "Current profile"), rows, theme).into_any_element()
}

/// 「运行参数」：三个关键数字。
///
/// 做成三行而不是三块并排小卡：一行一个参数、值靠右对齐，扫读时视线只上下移动。
pub fn stat_group(app: &App, theme: &Theme) -> AnyElement {
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

    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::layers(), theme).into_any_element()),
            t("网络适配器", "Network adapter"),
            Some(
                t(
                    "TUN 网卡名称，留空由系统自动分配",
                    "TUN adapter name; leave empty to let the system pick one",
                )
                .to_string(),
            ),
            None,
            Some(row_value(adapter, theme).into_any_element()),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::sewing_pin(), theme).into_any_element()),
            t("分流与绕过", "Bypass"),
            Some(t("这些地址不走代理", "These addresses skip the proxy").to_string()),
            None,
            Some(
                row_value(
                    tf(t("{bypass} 条", "{bypass}"), &[("bypass", &bypass.to_string())]),
                    theme,
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::mixer_horizontal(), theme).into_any_element()),
            t("会话上限", "Session limit"),
            Some(
                t(
                    "同时保持的最大连接数",
                    "Maximum number of concurrent connections",
                )
                .to_string(),
            ),
            None,
            Some(row_value(sessions, theme).into_any_element()),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("运行参数", "Runtime"), rows, theme).into_any_element()
}

/// 仪表盘上的日志：最近若干行 + 去「程序与日志」看全部。
pub fn log_preview_group(app: &App, snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let rows = vec![
        setting_row(
            None,
            t("实时输出", "Live output"),
            Some(
                t(
                    "从最新一行往回翻",
                    "Scroll back from the newest line",
                )
                .to_string(),
            ),
            None,
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_S))
                    .child(icon_action(
                        "copy-log-preview",
                        t("复制全部日志", "Copy the whole log"),
                        DefaultIcons::copy(),
                        cx.listener(|this, _, _, cx| this.handle(Msg::CopyLogs, cx)),
                    ))
                    .child(icon_action(
                        "open-full-log",
                        t("查看全部日志", "Open the full log"),
                        DefaultIcons::chevron_right(),
                        cx.listener(|this, _, _, cx| {
                            this.handle(Msg::PageSelected(Page::Program), cx)
                        }),
                    ))
                    .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        log_pane(app, snap, snap.logs.len(), LOG_PREVIEW_HEIGHT, theme, cx),
    ];

    settings_group(t("运行日志", "Log"), rows, theme).into_any_element()
}

// —— 代理配置 ——

/// 配置列表（「代理配置」页的默认态）。
///
/// 每套配置一行：单选圈决定用哪套，标题是名字，说明是摘要，右侧是改 / 删。
pub fn profile_list_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let config = &app.config;
    let mut rows: Vec<AnyElement> = Vec::new();

    for (index, profile) in config.profiles.iter().enumerate() {
        let active = config.active == index;
        rows.push(
            setting_row(
                Some(
                    div()
                        .id(eid("profile-pick", index))
                        .flex_none()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.handle(Msg::ProfileSelected(index), cx)
                        }))
                        .child(radio_dot(active, theme))
                        .into_any_element(),
                ),
                profile.name.trim(),
                Some(profile.summary()),
                None,
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SPACE_S))
                        .child(icon_action(
                            eid("profile-edit", index),
                            t("编辑这套配置", "Edit this profile"),
                            DefaultIcons::pencil_1(),
                            cx.listener(move |this, _, _, cx| {
                                this.handle(Msg::ProfileEditStart(index), cx)
                            }),
                        ))
                        .child(icon_action(
                            eid("profile-remove", index),
                            t("删除这套配置", "Delete this profile"),
                            DefaultIcons::trash(),
                            cx.listener(move |this, _, _, cx| {
                                this.handle(Msg::ProfileRemovedAt(index), cx)
                            }),
                        ))
                        .into_any_element(),
                ),
                theme,
            )
            .into_any_element(),
        );
    }

    rows.push(
        div()
            .flex()
            .items_center()
            .gap(px(SPACE_S))
            .px(px(super::widgets::ROW_PADDING))
            .py(px(super::widgets::ROW_PADDING))
            .child(labeled_action(
                "profile-add",
                t("新建", "New"),
                DefaultIcons::plus(),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::ProfileAdded, cx)),
            ))
            .child(labeled_action(
                "profile-duplicate",
                t("复制当前", "Duplicate"),
                DefaultIcons::copy(),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::ProfileDuplicated, cx)),
            ))
            .child(labeled_action(
                "profile-save",
                t("保存到磁盘", "Save to disk"),
                DefaultIcons::archive(),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::SaveConfig, cx)),
            ))
            .into_any_element(),
    );

    settings_group(
        tf(
            t("配置 · {} 套", "Profiles · {}"),
            &[("", &config.profiles.len().to_string())],
        ),
        rows,
        theme,
    )
    .into_any_element()
}

/// 单选圈：选中的画实心圆点，未选中的只有外圈。
fn radio_dot(selected: bool, theme: &Theme) -> Div {
    let accent = theme.accent();
    div()
        .flex_none()
        .w(px(16.0))
        .h(px(16.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if selected { accent } else { theme.border_secondary() })
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(8.0))
                .h(px(8.0))
                .rounded(px(4.0))
                .bg(if selected { accent } else { theme.surface() }),
        )
}

/// 单套配置的名字。
///
/// 这里只放「这一套叫什么」，不放保存按钮：保存是本页的主操作，全页只该有
/// 一个，位置在顶栏右上角（见 [`super::pages::page_header`]）。
pub fn profile_edit_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let profile = app.profile();
    let state = app.inputs.get(Field::Name);
    settings_group(
        tf(
            t("编辑配置 · {}", "Edit profile · {}"),
            &[("", profile.name.trim())],
        ),
        vec![
            setting_row(
                Some(row_icon(DefaultIcons::pencil_1(), theme).into_any_element()),
                t("配置名称", "Profile name"),
                Some(
                    t(
                        "侧边栏与下拉框里显示的名字",
                        "Shown in the sidebar and the picker",
                    )
                    .to_string(),
                ),
                None,
                Some(text_input(&state, ROW_CONTROL, cx)),
                theme,
            )
            .into_any_element(),
        ],
        theme,
    )
    .into_any_element()
}

/// 代理与 DNS。
pub fn proxy_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let profile = app.profile();

    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::gear(), theme).into_any_element()),
            t("上游代理地址", "Upstream proxy address"),
            Some(
                t(
                    "socks5:// 或 http:// 开头，例如 socks5://127.0.0.1:7890",
                    "Starts with socks5:// or http://, e.g. socks5://127.0.0.1:7890",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::ProxyUrl),
                ROW_CONTROL,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        probe_row(snap, theme, cx),
        setting_row(
            Some(row_icon(DefaultIcons::magnifying_glass(), theme).into_any_element()),
            t("DNS 处理模式", "DNS mode"),
            Some(profile.dns.hint().to_string()),
            None,
            Some(select_input(app.selects.dns.clone(), ROW_CONTROL)),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::globe(), theme).into_any_element()),
            t("远程 DNS 服务器", "Remote DNS server"),
            Some(
                t(
                    "DNS 模式为 Override 时使用",
                    "Used when the DNS mode is Override",
                )
                .to_string(),
            ),
            None,
            Some(text_input(&app.inputs.get(Field::DnsAddr), ROW_CONTROL, cx)),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::code(), theme).into_any_element()),
            t("虚拟地址池", "Virtual address pool"),
            Some(
                t(
                    "接管 DNS 时分配给域名的假地址段",
                    "Fake range handed out to domains when DNS is intercepted",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::VirtualPool),
                ROW_CONTROL,
                cx,
            )),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("代理连接", "Proxy"), rows, theme).into_any_element()
}

/// 「连通性检测」这一行：按钮 + 上一次的结论。
///
/// 结论写在说明里而不是弹窗：用户点完「检测」眼睛就在这一行上，
/// 结果出现在原地才不用再找一遍；颜色区分成功/失败。
fn probe_row(snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let (desc, tone, action_label, icon) = match &snap.probe {
        None => (
            t(
                "启动前先确认地址真的连得通（TCP + 协议握手）",
                "Checks that the address really answers (TCP + protocol handshake)",
            )
            .to_string(),
            None,
            t("检测", "Check"),
            DefaultIcons::magnifying_glass(),
        ),
        Some(ProbeState::Running) => (
            t("正在连接…", "Connecting…").to_string(),
            Some(theme.fg_muted()),
            t("检测中…", "Checking…"),
            DefaultIcons::magnifying_glass(),
        ),
        Some(ProbeState::Done(report)) if report.ok => (
            report.detail.clone(),
            Some(theme.success()),
            t("重新检测", "Check again"),
            DefaultIcons::reload(),
        ),
        Some(ProbeState::Done(report)) => (
            report.detail.clone(),
            Some(theme.danger()),
            t("重新检测", "Check again"),
            DefaultIcons::reload(),
        ),
    };
    // 前导图标跟着结论变：失败时再挂一个对勾，读起来像「检测通过」。
    let leading = match &snap.probe {
        Some(ProbeState::Done(report)) if !report.ok => DefaultIcons::exclamation_circled(),
        _ => DefaultIcons::check_circled(),
    };

    setting_row(
        Some(row_icon(leading, theme).into_any_element()),
        t("连通性检测", "Connectivity check"),
        Some(desc),
        tone,
        Some(
            labeled_action(
                "probe-check",
                action_label,
                icon,
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::CheckProxy, cx)),
            )
            .into_any_element(),
        ),
        theme,
    )
    .into_any_element()
}

// —— 网络与绕过 ——

/// 网络参数。
pub fn network_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let profile = app.profile();

    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::ruler_horizontal(), theme).into_any_element()),
            "MTU",
            Some(
                t(
                    "虚拟网卡的最大传输单元，默认 1500",
                    "MTU of the virtual adapter, 1500 by default",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::Mtu),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::timer(), theme).into_any_element()),
            t("TCP 超时（秒）", "TCP timeout (s)"),
            None,
            None,
            Some(text_input(
                &app.inputs.get(Field::TcpTimeout),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::timer(), theme).into_any_element()),
            t("UDP 超时（秒）", "UDP timeout (s)"),
            None,
            None,
            Some(text_input(
                &app.inputs.get(Field::UdpTimeout),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::people(), theme).into_any_element()),
            t("最大会话数", "Max sessions"),
            Some(
                t(
                    "同时保持的连接上限，留空用默认值",
                    "Connection cap; leave empty for the default",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::MaxSessions),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::code(), theme).into_any_element()),
            t("TUN 名称", "TUN name"),
            Some(t("留空自动生成", "Leave empty to generate one").to_string()),
            None,
            Some(text_input(&app.inputs.get(Field::TunName), ROW_CONTROL, cx)),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::check_circled(), theme).into_any_element()),
            t("启用 IPv6", "Enable IPv6"),
            Some(
                t(
                    "关闭时只转发 IPv4 流量",
                    "When off, only IPv4 traffic is forwarded",
                )
                .to_string(),
            ),
            None,
            Some(
                toggle_control(
                    "toggle-ipv6",
                    profile.ipv6,
                    cx.listener(|this, _, _, cx| {
                        let value = !this.profile().ipv6;
                        this.handle(Msg::SetIpv6(value), cx)
                    }),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::reload(), theme).into_any_element()),
            t("自动配置系统路由", "Configure system routes"),
            Some(
                t(
                    "关闭后需要自己写路由规则",
                    "When off you have to write the routing rules yourself",
                )
                .to_string(),
            ),
            None,
            Some(
                toggle_control(
                    "toggle-routes",
                    profile.setup,
                    cx.listener(|this, _, _, cx| {
                        let value = !this.profile().setup;
                        this.handle(Msg::SetSetupRoutes(value), cx)
                    }),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("网络参数", "Network parameters"), rows, theme).into_any_element()
}

/// 路由绕过规则。
///
/// 两个入口共用这一组：「网络与绕过」页（一级）和「代理配置 → 编辑」页
/// （详情）。绕过地址本来就是每套配置自己的字段，在详情页里也能改。
pub fn bypass_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let bypass = app.inputs.get(Field::Bypass);
    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::sewing_pin(), theme).into_any_element()),
            t("不走代理的地址", "Addresses that skip the proxy"),
            Some(t(
                "每行一个 IP 或 CIDR；这些地址直连，不经过上游代理。",
                "One IP or CIDR per line; these are dialed directly, bypassing the upstream proxy.",
            )
            .to_string()),
            None,
            Some(
                labeled_action(
                    "bypass-defaults",
                    t("填入默认值", "Fill in defaults"),
                    DefaultIcons::reset(),
                    theme,
                    cx.listener(|this, _, _, cx| this.handle(Msg::BypassDefaults, cx)),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        setting_block(
            t("绕过列表", "Bypass list"),
            Some(
                t(
                    "默认已填好局域网与保留地址段，通常不用改",
                    "LAN and reserved ranges are pre-filled; usually no need to change",
                )
                .to_string(),
            ),
            div()
                .w_full()
                .child(textarea(&bypass, cx).rows(6).max_height(px(150.0)).large()),
            theme,
        )
        .into_any_element(),
        // 示例单独摆一张表：列表是纯文本、每行一条地址，tun2proxy 会把整行
        // 当地址解析，所以注释和「#」「;」分隔符都写不进去——只能在旁边列出来。
        setting_block(
            t("写法示例", "Examples"),
            Some(
                t(
                    "照抄左边一列即可，右边是它代表什么",
                    "Copy the left column as-is; the right side explains it",
                )
                .to_string(),
            ),
            bypass_examples(theme),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("路由绕过", "Bypass list"), rows, theme).into_any_element()
}

/// 「不走代理的地址」的写法示例：左列是能直接粘进列表的写法，右列是它代表什么。
fn bypass_examples(theme: &Theme) -> Div {
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

    let mut table = div().flex().flex_col().gap(px(SPACE_XS)).w_full();
    for (pattern, zh, en) in EXAMPLES {
        table = table.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(SPACE_M))
                .child(
                    div()
                        .text_size(px(TEXT_BODY))
                        .font_family(super::theme::MONO_FAMILY)
                        .text_color(theme.fg())
                        .child(pattern),
                )
                .child(
                    div()
                        .text_size(px(TEXT_SMALL))
                        .text_color(theme.fg_muted())
                        .child(t(zh, en)),
                ),
        );
    }
    table
}

// —— UDP 转发 ——

/// UDP 转发（`--udpgw`）。
///
/// 放在「网络与绕过」页：它决定的是「UDP 包往哪里走」，和 MTU、绕过规则同属
/// 网络行为。
pub fn udpgw_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::paper_plane(), theme).into_any_element()),
            t("UDPGW 服务器", "UDPGW server"),
            Some(
                t(
                    "ip:port。SOCKS5 不转发 UDP 时，由这台服务器代转；留空则不转发 UDP。",
                    "ip:port. When SOCKS5 cannot carry UDP, this server relays it; leave empty to drop UDP.",
                )
                .to_string(),
            ),
            None,
            Some(text_input(&app.inputs.get(Field::Udpgw), ROW_CONTROL, cx)),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::people(), theme).into_any_element()),
            t("UDPGW 连接数", "UDPGW connections"),
            Some(
                t(
                    "同时保持的 UDP 转发连接数",
                    "Number of UDP relay connections to keep",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::UdpgwConnections),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("UDP 转发", "UDP forwarding"), rows, theme).into_any_element()
}

// —— 程序与日志 ——

/// 运行方式：进程层面的两个开关。
///
/// 都不是「日常会改」的项，但它们是「这个程序怎么跑」而不是「网络怎么走」，
/// 所以归到程序页，而不是网络页。
pub fn runtime_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let profile = app.profile();

    let mut rows = vec![compact_row(
        t("会话打满时退出", "Exit when sessions run out"),
        Some(
            toggle_control(
                "toggle-exit-on-fatal",
                profile.exit_on_fatal,
                cx.listener(|this, _, _, cx| {
                    let value = !this.profile().exit_on_fatal;
                    this.handle(Msg::SetExitOnFatal(value), cx)
                }),
            )
            .into_any_element(),
        ),
        theme,
    )
    .into_any_element()];

    // 用户命名空间是 Linux 专有选项，其它平台不显示。
    if cfg!(target_os = "linux") {
        rows.push(
            compact_row(
                t("免 root 运行", "Run without root"),
                Some(
                    toggle_control(
                        "toggle-unshare",
                        profile.unshare,
                        cx.listener(|this, _, _, cx| {
                            let value = !this.profile().unshare;
                            this.handle(Msg::SetUnshare(value), cx)
                        }),
                    )
                    .into_any_element(),
                ),
                theme,
            )
            .into_any_element(),
        );
    }

    settings_group(t("运行方式", "Process"), rows, theme).into_any_element()
}

/// 窗口与托盘：关窗时的行为。
///
/// 单独一组而不是塞进「运行方式」：它管的是「窗口怎么关」，不是「进程怎么跑」，
/// 而误关窗口把隧道停掉是这个程序最容易踩的坑。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn window_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let mut rows = vec![compact_row(
        t("关闭窗口时询问", "Ask when closing"),
        Some(
            toggle_control(
                "toggle-confirm-close",
                app.config.confirm_close_to_tray,
                cx.listener(|this, _, _, cx| {
                    let value = !this.config.confirm_close_to_tray;
                    this.handle(Msg::SetConfirmCloseToTray(value), cx)
                }),
            )
            .into_any_element(),
        ),
        theme,
    )
    .into_any_element()];

    // 没托盘就藏不起来（藏了就找不回来），这个动作只在 Windows 上给。
    if cfg!(windows) {
        rows.push(
            compact_row(
                t("最小化到托盘", "Minimize to tray"),
                Some(
                    text_action(
                        "minimize-to-tray",
                        t("最小化", "Minimize"),
                        theme,
                        cx.listener(|this, _, _, cx| this.handle(Msg::MinimizeToTray, cx)),
                    )
                    .into_any_element(),
                ),
                theme,
            )
            .into_any_element(),
        );
    }

    settings_group(t("窗口与托盘", "Window & tray"), rows, theme).into_any_element()
}

/// 界面语言 + 主题：都属于「界面怎么显示」，放在同一组里。
///
/// 两个都是下拉框而不是三选一的按钮组：gpuikit 的下拉框已经带上了选中态、
/// 键盘操作和弹出层，用一组自制按钮会把这几样重新实现一遍，还会让设置页
/// 出现两套控件语言。
///
/// 改完立即生效并落盘：语言和主题都是「看一眼就想换」的东西，不该要求
/// 再点一次保存。
pub fn appearance_group(app: &App, theme: &Theme) -> AnyElement {
    let rows = vec![
        compact_row(
            t("界面语言", "Language"),
            Some(select_input(app.selects.language.clone(), ROW_CONTROL)),
            theme,
        )
        .into_any_element(),
        compact_row(
            t("界面主题", "Theme"),
            Some(select_input(app.selects.theme.clone(), ROW_CONTROL)),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("界面", "Appearance"), rows, theme).into_any_element()
}

/// tun2proxy 程序本体：版本、检查更新、下载安装。
///
/// 下载按钮只在「真有事可做」的时候出现：
///
/// - 没装 → 「下载并安装」（这是首次使用最该点的东西）；
/// - 装了且有新版本 → 「下载并更新」；
/// - 装了且已是最新 / 正在检查或下载中 → 不显示（按钮点了也没有任何效果）。
pub fn binary_group(
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let version = snap
        .version
        .clone()
        .unwrap_or_else(|| t("未安装", "Not installed").to_string());
    let installed = snap.version.is_some();
    let update_available = matches!(snap.update, Update::Available { .. });

    let mut actions = div().flex().items_center().gap(px(SPACE_S)).child(text_action(
        "check-update",
        t("检查", "Check"),
        theme,
        cx.listener(|this, _, _, cx| this.handle(Msg::CheckUpdate, cx)),
    ));
    if !snap.update.is_busy() {
        if !installed {
            actions = actions.child(primary_button(
                "install-update",
                t("下载并安装", "Download"),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::InstallUpdate, cx)),
            ));
        } else if update_available {
            actions = actions.child(primary_button(
                "install-update",
                t("下载并更新", "Update"),
                theme,
                cx.listener(|this, _, _, cx| this.handle(Msg::InstallUpdate, cx)),
            ));
        }
    }

    let mut rows = vec![
        compact_row(
            t("当前版本", "Version"),
            Some(row_value(version, theme).into_any_element()),
            theme,
        )
        .into_any_element(),
        compact_row(
            t("检查更新", "Check for updates"),
            Some(actions.into_any_element()),
            theme,
        )
        .into_any_element(),
    ];

    match &snap.update {
        Update::Downloading { got, total } => {
            let fraction = match total {
                Some(total) if *total > 0 => *got as f32 / *total as f32,
                _ => 0.0,
            };
            rows.push(
                setting_block(
                    t("正在下载", "Downloading"),
                    Some(match total {
                        Some(total) => format!("{} / {}", human_bytes(*got), human_bytes(*total)),
                        None => human_bytes(*got),
                    }),
                    progress(fraction.clamp(0.0, 1.0)).width(px(320.0)),
                    theme,
                )
                .into_any_element(),
            );
        }
        Update::Checking => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::magnifying_glass(), theme).into_any_element()),
                t("正在查询最新版本…", "Looking up the latest version…"),
                None,
                None,
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Installing => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::archive(), theme).into_any_element()),
                t("正在解压并安装…", "Extracting and installing…"),
                None,
                None,
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Available { latest } => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::info_circled(), theme).into_any_element()),
                tf(
                    t("发现新版本 {latest}", "Version {latest} is available"),
                    &[("latest", latest)],
                ),
                Some(t("点右侧「下载并更新」", "Click “Update” on the right").to_string()),
                None,
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Latest { version } => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::check_circled(), theme).into_any_element()),
                tf(
                    t("已是最新版本（{version}）", "Up to date ({version})"),
                    &[("version", version)],
                ),
                None,
                None,
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Done { version } => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::check_circled(), theme).into_any_element()),
                tf(
                    t("已更新到 {version}", "Updated to {version}"),
                    &[("version", version)],
                ),
                None,
                None,
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Failed(reason) => rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::exclamation_circled(), theme).into_any_element()),
                t("更新失败", "Update failed"),
                Some(reason.clone()),
                Some(theme.danger()),
                None,
                theme,
            )
            .into_any_element(),
        ),
        Update::Idle => {}
    }

    settings_group(t("tun2proxy 程序", "tun2proxy binary"), rows, theme).into_any_element()
}

/// 运行日志（整页）+ 日志级别。
///
/// 日志级别放在这里而不是单独一页：它唯一的用途就是「让下面这块输出多些或少些」，
/// 摆在同一张卡里，调完立刻能看到效果。
pub fn log_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let rows = vec![
        compact_row(
            t("日志级别", "Log level"),
            Some(select_input(app.selects.verbosity.clone(), ROW_CONTROL)),
            theme,
        )
        .into_any_element(),
        compact_row(
            t("实时输出", "Live output"),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_S))
                    .child(icon_action(
                        "copy-log",
                        t("复制全部日志", "Copy the whole log"),
                        DefaultIcons::copy(),
                        cx.listener(|this, _, _, cx| this.handle(Msg::CopyLogs, cx)),
                    ))
                    .child(icon_action(
                        "clear-log",
                        t("清空日志", "Clear log"),
                        DefaultIcons::eraser(),
                        cx.listener(|this, _, _, cx| this.handle(Msg::ClearLogs, cx)),
                    ))
                    .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        log_pane(
            app,
            snap,
            snap.logs.len(),
            super::widgets::LOG_PANE_HEIGHT,
            theme,
            cx,
        ),
    ];

    settings_group(t("运行日志", "Log"), rows, theme).into_any_element()
}

/// 底部说明。
pub fn about_group(theme: &Theme) -> AnyElement {
    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::dashboard(), theme).into_any_element()),
            "Tun2Proxy GUI",
            Some(
                t(
                    "官方 tun2proxy 二进制的图形前端，Rust + GPUI 原生界面",
                    "A native front end for the official tun2proxy binary, built with Rust + GPUI",
                )
                .to_string(),
            ),
            None,
            None,
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::lock_closed(), theme).into_any_element()),
            t("权限", "Permissions"),
            Some(
                t(
                    "Windows 需管理员权限，macOS 需 root，Linux 可免 root",
                    "Administrator on Windows, root on macOS, root-free on Linux",
                )
                .to_string(),
            ),
            None,
            None,
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("关于", "About"), rows, theme).into_any_element()
}

/// 日志窗格：固定高度 + 自己的滚动条。
///
/// 固定高度必须写在外层容器上，不能写在滚动容器自己身上：滚动容器的高度
/// 就是「视口高度」，给它写死高度会让「内容高度 = 视口高度」，滚动条永远
/// 不会出现。内层填满外层，超出部分靠 `overflow_y_scroll` 滚。
fn log_pane(
    app: &App,
    snap: &Snapshot,
    lines: usize,
    height: f32,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
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
            div().h(px(0.0)),
            theme,
        )
        .into_any_element();
    }

    let tail: Vec<AnyElement> = snap
        .logs
        .iter()
        .rev()
        .take(lines)
        .rev()
        .map(|line| log_line(line.clone(), theme).into_any_element())
        .collect();

    let inner = div()
        .id("log-lines")
        .flex()
        .flex_col()
        .gap(px(2.0))
        .w_full()
        .p(px(10.0))
        .children(tail);

    // 日志框自己处理滚轮：里面滚到头了再把事件交给页面。
    //
    // 两个滚动容器套在一起时 gpui 会把滚轮发给**所有**命中的容器，页面会
    // 跟着日志一起滚——看起来就是「日志滚不动，整页在动」。这里在日志还能
    // 滚的时候截住事件。判断放在元素自己的监听器里，gpui 的内建滚动在同一
    // 元素上先注册、先执行，所以读到的是已经应用过本次增量之后的偏移。
    //
    // 偏移的符号：gpui 把 `delta` 直接加上去，再夹到 `[-max, 0]`，所以顶部
    // 是 0、底部是 -max。往下滚 delta 为正、偏移往 0 走。
    let scroll_handle = app.log_scroll.clone();
    // 滚动条自己画：gpui 的滚动容器不画条，没有它就看不出「这里还能滚」。
    // 拿上一次布局的偏移量算，滚一下 gpui 会通知重绘，条跟着走。
    let indicator = super::widgets::scroll_indicator(&scroll_handle, theme);

    let viewport = div()
        .id("log-viewport")
        .w_full()
        .h(px(height))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border())
        .bg(theme.surface_secondary())
        .overflow_hidden()
        .child(
            div()
                .id("log-scroll")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&scroll_handle)
                .on_scroll_wheel(cx.listener(
                    |this, event: &ScrollWheelEvent, window, cx| {
                        let delta = event.delta.pixel_delta(window.line_height()).y;
                        let offset = this.log_scroll.offset().y;
                        let max = this.log_scroll.max_offset().y;
                        let can_scroll = if delta > px(0.0) {
                            offset < px(0.0)
                        } else if delta < px(0.0) {
                            offset > -max
                        } else {
                            true
                        };
                        // 用户往回翻就停止跟随，翻回底部自动恢复。
                        this.sync_log_follow();
                        if can_scroll {
                            cx.stop_propagation();
                        }
                    },
                ))
                .child(inner),
        )
        .children(indicator);

    div()
        .w_full()
        .px(px(super::widgets::ROW_PADDING))
        .pb(px(super::widgets::ROW_PADDING))
        .child(viewport)
        .into_any_element()
}

/// 「当前配置」行的铅笔要按「现在选中的那套」编辑，不能捕获渲染时的下标
/// （渲染和点击之间用户可能已经切换过配置）。
fn config_active(app: &App) -> usize {
    app.config.active
}

/// 供 `pages.rs` 判断要不要显示「保存」按钮。
pub fn editing_profile(app: &App) -> bool {
    app.page == Page::Profiles && app.profile_editing
}
