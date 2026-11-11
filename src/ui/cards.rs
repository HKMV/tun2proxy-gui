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
    AnyElement, Context, Div, Entity, Hsla, IntoElement, ScrollWheelEvent, SharedString, Svg, div,
    px, prelude::*,
};
use gpuikit::elements::progress::progress;
use gpuikit::elements::text_field::text_field;
use gpuikit::elements::textarea::textarea;
use gpuikit::elements::tooltip::tooltip;
use gpuikit::input::InputState;
use gpuikit::theme::{ControlSize, Theme, Themeable};
use gpuikit::traits::control_sized::ControlSized;
use gpuikit::DefaultIcons;

use crate::core::engine::ProbeState;
use crate::core::i18n::{t, tf};
use crate::core::split::SplitGroup;
use crate::core::{DnsMode, Status, Update};

use super::chart::{LEGEND_GAP, legend_swatch, legend_text, split_trend_chart, traffic_chart};
use super::state::{App, Field, Msg, Page, Snapshot};
use super::theme::{TEXT_BODY, TEXT_SMALL};
use super::widgets::{
    ROW_CONTROL, ROW_CONTROL_NARROW, ROW_PADDING, SPACE_M, SPACE_S, SPACE_XS, compact_row,
    hint, human_bytes, icon, icon_action, icon_action_off, labeled_action, log_line, primary_button,
    row_icon, row_value, setting_block, setting_row, settings_group, text_action, toggle_control,
};

/// 仪表盘日志框的高度。
///
/// 不再按行数截断：框是能滚的，只留 12 行的话滚一下就到底，等于还是看不见
/// 前面说过什么——而「刚才为什么停了」恰恰在更早的那几行里。
const LOG_PREVIEW_HEIGHT: f32 = 200.0;

/// 状态灯（侧栏那颗 8dp 小圆点）的颜色。
///
/// 和托盘的图标同一套口径：未运行是灰的（熄灭）、运行中是绿的、过渡是黄的、
/// 失败是红的——一眼看出服务在不在跑。
///
/// 只给圆点用，文字仍旧走 [`status_label`]：圆点是图形元素，非文本对比度够
/// 3:1 就能看清；而整行文字也压成 muted，才是上面说的「几乎读不出来」。
/// 正文色的圆点则相反——它和文字同色，看着像句首的圆点符号，不像状态灯。
pub fn status_dot_color(status: &Status, theme: &Theme) -> Hsla {
    match status {
        Status::Stopped => theme.fg_muted(),
        Status::Starting | Status::Stopping => theme.warning(),
        Status::Running => theme.success(),
        Status::Failed(_) => theme.danger(),
    }
}

/// 状态文字 + 颜色，供侧边栏与首屏卡片共用。
///
/// 「未运行」用正文色而不是 `fg_muted()`：它是**当前状态**，不是说明文字。
/// muted 在 Catppuccin 的两套色板上都只有约 3:1，压在白卡或 `surface` 上
/// 那颗状态点和那行字几乎读不出来——而这恰恰是用户第一眼要找的东西。
pub fn status_label(status: &Status, theme: &Theme) -> (String, Hsla) {
    match status {
        Status::Stopped => (t("未运行", "Not running").to_string(), theme.fg()),
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
///
/// 占位文字必须经 `TextField::placeholder()` 传进去：`InputState` 里那份
/// 只有实体自己知道，`Input` 元素不读它——少传这一句，空输入框在界面上
/// 就是一个纯色方块，用户看不出这里该填什么、留空会怎样。
fn text_input(state: &Entity<InputState>, width: f32, cx: &mut Context<App>) -> AnyElement {
    field_box(state, width, false, cx)
}

/// 当前模式用不上的输入框：只读 + 灰一档。
///
/// 用 `read_only` 而不是 disabled（`TextField` 没有 disabled 构造器，只有
/// `read_only`）：值仍然可以选中复制（用户可能要拿它去别处试），但改不了。
/// 灰一档是为了让「这一项现在不生效」一眼可见，不必去读说明文字。
fn text_input_off(state: &Entity<InputState>, width: f32, cx: &mut Context<App>) -> AnyElement {
    field_box(state, width, true, cx)
}

fn field_box(
    state: &Entity<InputState>,
    width: f32,
    off: bool,
    cx: &mut Context<App>,
) -> AnyElement {
    let placeholder = state.read(cx).placeholder().clone();
    let mut field = text_field(state, cx).full_width(true).large();
    if !placeholder.is_empty() {
        field = field.placeholder(placeholder);
    }
    if off {
        field = field.read_only(true);
    }
    div()
        .flex_none()
        .w(px(width))
        .when(off, |style| style.opacity(0.55))
        .child(field)
        .into_any_element()
}

/// 下拉框，宽度固定。
fn select_input(state: impl IntoElement, width: f32) -> AnyElement {
    div().flex_none().w(px(width)).child(state).into_any_element()
}

// —— 仪表盘 ——

// —— 仪表盘 ——

/// 顶栏右侧的状态条：「运行中」胶囊 + 一句短语。
///
/// 隧道通不通是仪表盘第一个要回答的问题，但它不值得占一整张大卡片：状态在
/// 侧栏底部和这里各出现一次就够了，中间那块留给两张折线图。
pub fn status_strip(snap: &Snapshot, theme: &Theme) -> AnyElement {
    let (status_text, color) = status_label(&snap.status, theme);
    let short = match &snap.status {
        Status::Running => t("系统流量已接管", "System traffic is routed"),
        Status::Starting => t("正在创建虚拟网卡…", "Creating the TUN adapter…"),
        Status::Stopping => t("正在回收虚拟网卡…", "Releasing the TUN adapter…"),
        Status::Failed(_) => t("隧道已退出，原因见下方", "The tunnel exited; see below"),
        Status::Stopped => t("尚未接管系统流量", "Not routing system traffic yet"),
    };

    div()
        .flex()
        .items_center()
        .gap(px(SPACE_M))
        .child(super::widgets::pill(status_text, color, theme))
        .child(
            div()
                .text_size(px(TEXT_SMALL))
                .text_color(theme.fg_muted())
                .whitespace_nowrap()
                .child(short),
        )
        .into_any_element()
}

/// 仪表盘顶部的提示，只在真出事时出现：隧道异常退出、缺管理员权限。
///
/// 以前这两句挂在首屏大卡里；大卡去掉之后说明不能跟着消失——「为什么停了」
/// 「为什么起不来」恰恰是最需要读到的两句，所以留下，但只在需要时占一行。
pub fn status_tips(snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> Vec<AnyElement> {
    let mut tips = Vec::new();

    if let Status::Failed(reason) = &snap.status {
        tips.push(tip_bar(
            DefaultIcons::info_circled(),
            theme.danger(),
            tf(
                t("隧道已经退出：{reason}", "The tunnel exited: {reason}"),
                &[("reason", reason)],
            ),
            None,
            theme,
        ));
    }

    if let Some(note) = crate::core::elevate::privilege_hint() {
        tips.push(tip_bar(
            DefaultIcons::exclamation_triangle(),
            theme.warning(),
            note,
            Some(
                labeled_action(
                    "relaunch-elevated",
                    t("使用管理员重启", "Restart as administrator"),
                    DefaultIcons::reload(),
                    theme,
                    cx.listener(|this, _, _, cx| this.handle(Msg::RelaunchElevated, cx)),
                )
                .into_any_element(),
            ),
            theme,
        ));
    }

    tips
}

/// 一条浅色提示条：图标 + 文字 + 可选的补救按钮。
///
/// 配色跟内容区顶部那条提示条（`pages::notice_bar`）一致：同一类「系统在跟你
/// 说话」的信息不该有两种长相。
fn tip_bar(
    glyph: Svg,
    color: Hsla,
    text: String,
    action: Option<AnyElement>,
    theme: &Theme,
) -> AnyElement {
    let mut bar = div()
        .flex()
        .items_center()
        .gap(px(SPACE_M))
        .w_full()
        .p(px(SPACE_M))
        .rounded(px(6.0))
        .border_1()
        .border_color(color.opacity(0.35))
        .bg(color.opacity(0.10))
        .child(icon(glyph, TEXT_BODY, color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(TEXT_BODY))
                .text_color(theme.fg())
                .child(text),
        );
    if let Some(action) = action {
        bar = bar.child(action);
    }
    bar.into_any_element()
}

/// 折线图上方的小标题行：名字在左，时间窗在右。
///
/// 两张图共用这一个形状：都是「最近 2 分钟」的时序，标题右边写清楚窗口长度，
/// 读者不用去别处找。
fn chart_header(title: &'static str, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .child(
            div()
                .text_size(px(TEXT_BODY))
                .text_color(theme.fg())
                .child(title),
        )
        .child(
            div()
                .text_size(px(TEXT_SMALL))
                .text_color(theme.fg_muted())
                .whitespace_nowrap()
                .child(t("最近 2 分钟", "Last 2 minutes")),
        )
        .into_any_element()
}

/// 卡片里的「实时流量」一行：标题 + 折线图 + 当前速率的图例。
fn traffic_row(snap: &Snapshot, theme: &Theme) -> AnyElement {
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
        .w_full()
        .gap(px(SPACE_S))
        .px(px(ROW_PADDING))
        // 上下留白不对称：两张图在同一张卡里上下相邻，各自的 `ROW_PADDING`
        // 会在中间叠成 24dp、外加一条分隔线，看着像两块互不相干的卡片。
        // 靠分隔线这侧各收一半，卡片最外侧仍是 ROW_PADDING。
        .pt(px(ROW_PADDING))
        .pb(px(SPACE_S))
        .child(chart_header(t("实时流量", "Live traffic"), theme))
        .child(traffic_chart(&snap.traffic, theme))
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
            Some(row_icon(DefaultIcons::server(), theme).into_any_element()),
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
                    .child(
                        // 摘要是「地址 · DNS 模式」，长地址必然被省略号截断
                        // （`socks5://192.168.10.121:1010 · DNS 虚拟 IP` 有
                        // 近 40 个字符），而地址正是这一行唯一要读的东西。
                        // 放宽一档，并把全文挂到 tooltip 上。
                        div()
                            .id("profile-summary")
                            .max_w(px(300.0))
                            .tooltip(tooltip(profile.summary()))
                            .child(row_value(profile.summary(), theme)),
                    )
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
    let adapter = match profile.tun_name_configured() {
        Some(name) => name.to_string(),
        None => t("每次启动随机", "Random each start").to_string(),
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
                    "TUN 网卡名称，留空则每次启动随机生成",
                    "TUN adapter name; a random one is used each start when empty",
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
                profile.display_name(),
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
                        .child(if config.profiles.len() > 1 {
                            icon_action(
                                eid("profile-remove", index),
                                t("删除这套配置", "Delete this profile"),
                                DefaultIcons::trash(),
                                // 删除不直接执行：先弹一句确认（见
                                // `App::ask_remove_profile`）。
                                cx.listener(move |this, _, window, cx| {
                                    this.ask_remove_profile(index, window, cx)
                                }),
                            )
                            .into_any_element()
                        } else {
                            // 只剩一套时禁用而不是「点了才说不行」：这条规则
                            // 是固定的，没必要让用户先撞一次。
                            icon_action_off(
                                eid("profile-remove", index),
                                t("至少要保留一套配置", "Keep at least one profile"),
                                DefaultIcons::trash(),
                            )
                            .into_any_element()
                        })
                        .into_any_element(),
                ),
                theme,
            )
            .into_any_element(),
        );
    }

    rows.push(
        div()
            .id("profile-actions")
            .flex()
            .flex_none()
            .w_full()
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
    let state = app.inputs.get(Field::Name);
    settings_group(
        // 顶栏已经写着「编辑配置」，分组标题再重复一遍只是噪音；改的是哪
        // 一套，下面那个输入框自己说得清。
        t("这套配置", "This profile"),
        vec![
            setting_row(
                Some(row_icon(DefaultIcons::pencil_1(), theme).into_any_element()),
                t("配置名称", "Profile name"),
                Some(
                    // 名字出现在两处：配置列表的每一行，以及仪表盘上的「配置文件」
                    // 下拉框。别写成「侧边栏」——侧边栏显示的是程序名和运行状态。
                    t(
                        "配置列表与仪表盘下拉框里显示的名字",
                        "Shown in the profile list and the dashboard picker",
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
    // 这两个字段是否参与，只看 DNS 模式（见下面两行的说明）。
    let virtual_dns = profile.dns == DnsMode::Virtual;

    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::server(), theme).into_any_element()),
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
        // 「远程 DNS 服务器」和「虚拟地址池」各自只服务一半的 DNS 模式：
        // 前者在「虚拟 IP」下完全不参与（本地应答，查询不出网），后者只
        // 有「虚拟 IP」用得到。两个框以前一直可编辑，填了不生效也没人
        // 说——现在跟着模式灰掉，并在说明里写清为什么不生效。
        setting_row(
            Some(row_icon(DefaultIcons::globe(), theme).into_any_element()),
            t("远程 DNS 服务器", "Remote DNS server"),
            Some(if virtual_dns {
                t(
                    "「虚拟 IP」在本地应答，这一项不参与",
                    "Virtual IP answers locally, so this is not used",
                )
                .to_string()
            } else {
                t(
                    "查询经隧道发往这台服务器",
                    "Queries are sent to this server through the tunnel",
                )
                .to_string()
            }),
            None,
            Some(if virtual_dns {
                text_input_off(&app.inputs.get(Field::DnsAddr), ROW_CONTROL, cx)
            } else {
                text_input(&app.inputs.get(Field::DnsAddr), ROW_CONTROL, cx)
            }),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::database(), theme).into_any_element()),
            t("虚拟地址池", "Virtual address pool"),
            Some(if virtual_dns {
                t(
                    "接管 DNS 时分配给域名的假地址段",
                    "Fake range handed out to domains when DNS is intercepted",
                )
                .to_string()
            } else {
                t(
                    "只有「虚拟 IP」模式会用到",
                    "Only the Virtual IP mode uses this",
                )
                .to_string()
            }),
            None,
            Some(if virtual_dns {
                text_input(&app.inputs.get(Field::VirtualPool), ROW_CONTROL, cx)
            } else {
                text_input_off(&app.inputs.get(Field::VirtualPool), ROW_CONTROL, cx)
            }),
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
    // 前导图标跟着结论变。没检测过 / 正在检测时**不能**用对勾：那看起来
    // 像「已经通过了」，而这一行真正要读的是右侧那个按钮和下面那句结论。
    let leading = match &snap.probe {
        Some(ProbeState::Done(report)) if report.ok => DefaultIcons::check_circled(),
        Some(ProbeState::Done(_)) => DefaultIcons::exclamation_circled(),
        _ => DefaultIcons::magnifying_glass(),
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
            Some(row_icon(DefaultIcons::stopwatch(), theme).into_any_element()),
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
            Some(row_icon(DefaultIcons::container(), theme).into_any_element()),
            t("TUN 名称", "TUN name"),
            Some(t("留空每次启动随机", "Random each start when empty").to_string()),
            None,
            Some(text_input(&app.inputs.get(Field::TunName), ROW_CONTROL, cx)),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::globe_2(), theme).into_any_element()),
            t("启用 IPv6", "Enable IPv6"),
            Some(
                t(
                    "关闭后只转发 IPv4：IPv6 站点会先等超时再回落，访问会明显变慢",
                    "When off, only IPv4 is forwarded: IPv6 sites stall until the timeout and then fall back",
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
    // 滚动条叠在输入框的内边距里：上下各让出「内边距 + 1dp 描边」，条就正好
    // 贴住文字的视口，不会跑进圆角。`.large()` 的内边距是 0.75rem。
    let text_inset = theme.control(ControlSize::Large).padding_x.0 * 16.0 + 1.0;
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
                .relative()
                .w_full()
                .child(textarea(&bypass, cx).rows(6).max_height(px(150.0)).large())
                // 输入框自己只滚不画条（gpui 的 `text_area` 就这样），
                // 条由应用补上，样式与页面 / 日志那两条一致。
                .child(super::widgets::input_scroll_bar(
                    "bypass-scroll-bar",
                    super::widgets::ScrollBarKey::Input("bypass"),
                    &bypass,
                    text_inset,
                    theme,
                    &app.scroll_drag,
                )),
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

// —— 分流规则 ——

/// 规则明细里最多画多少行。
///
/// 一万六千条全画出来，滚动容器每一步都要重新布局一万六千行，页面会明显拖
/// 顿；明细的作用是「让我看一眼这类规则长什么样、顺手关掉某一条」，100 行 +
/// 筛选框足够，下面那句「只显示前 N 条」也会说清不是全部。
const SPLIT_ROWS_SHOWN: usize = 100;

/// 规则明细那个定高列表的高度：约六行，再多就抢走页面的注意力。
const SPLIT_LIST_HEIGHT: f32 = 264.0;

/// 「最近判定」列表的高度：比规则明细矮一档——它是一块「现在发生了什么」的
/// 简报，不是拿来找东西的清单。
const SPLIT_DECISIONS_HEIGHT: f32 = 288.0;

/// 仪表盘上的「实时流量与连接去向」：两张 2 分钟的折线图上下排在一张卡里。
///
/// 为什么合到一张卡：两张图看的是**同一段时间**。以前流量图挂在首屏卡右侧、
/// 去向图单独占一张卡，读者要在两个位置之间来回对时间；现在同宽、同窗口、
/// 同一组时间竖线，上下扫一眼就能对上「流量高峰」和「分流高峰」。
///
/// 「连接去向」原来是分流页顶上的一排大数字（直连 / 走代理 / 失败 / 总数）。
/// 数字只回答「一共多少」，回答不了「什么时候在直连、什么时候全在走代理」——
/// 而后者才是用户判断「分流真的生效了没有」的依据，它必须挂在时间轴上才看得出来。
///
/// 折线是最近 2 分钟，数字是本次运行累计：两个窗口不一样，说明行里写明白，
/// 否则用户会拿曲线下的面积去对总数。
pub fn trend_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    settings_group(
        t("实时流量与连接去向", "Traffic and routing"),
        vec![
            traffic_row(snap, theme),
            split_trend_row(app, snap, theme, cx),
        ],
        theme,
    )
    .into_any_element()
}

/// 卡片里的「连接去向」一行：标题 + 折线图 + 图例（带百分比）+ 说明。
fn split_trend_row(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let runtime = &snap.split;
    let routed = runtime.direct + runtime.proxy;
    let enabled = app.config.split.enabled;

    // 百分比只算「真的走了哪条路」的两条，失败不进分母：分母里混进一批既没
    // 直连也没代理成功的连接，两段加起来不到 100%，看着像算错了。
    let pct = |value: u64| -> String {
        if routed == 0 {
            String::new()
        } else {
            format!("{}%", (value as f64 / routed as f64 * 100.0).round() as u64)
        }
    };

    let legend_row = div()
        .flex()
        .items_center()
        .w_full()
        .min_w_0()
        .gap(px(SPACE_M))
        .child(legend_item(
            t("直连", "Direct"),
            runtime.direct,
            pct(runtime.direct),
            theme.success(),
            theme,
        ))
        .child(legend_item(
            t("走代理", "Proxied"),
            runtime.proxy,
            pct(runtime.proxy),
            theme.accent(),
            theme,
        ))
        .child(legend_item(
            t("失败", "Failed"),
            runtime.failed,
            String::new(),
            theme.danger(),
            theme,
        ))
        .child(div().flex_1())
        .child(icon_action(
            "split-trend-open",
            t("打开分流规则", "Open the split rules"),
            DefaultIcons::chevron_right(),
            cx.listener(|this, _, _, cx| this.handle(Msg::PageSelected(Page::Split), cx)),
        ));

    let caption = if !enabled {
        t(
            "「国内流量直连」已关闭：所有连接照常走上游代理，这里不会有直连曲线。",
            "“Route China traffic directly” is off: everything still goes through the proxy, so there is no direct line here.",
        )
        .to_string()
    } else if !runtime.running && runtime.sessions == 0 {
        t(
            "隧道启动后这里会画出每条连接的去向：命中国内清单的直连，其余走代理。",
            "Once the tunnel starts, this chart shows where each connection went: China traffic direct, everything else proxied.",
        )
        .to_string()
    } else if runtime.sessions == 0 {
        t(
            "还没有连接经过分流器。没有流量时它什么也不做，这属于正常。",
            "No connection has reached the splitter yet. That is normal while idle.",
        )
        .to_string()
    } else {
        let sessions = runtime.sessions.to_string();
        tf(
            t(
                "折线是最近 2 分钟、每 2 秒一格；数字是本次运行累计（共 {sessions} 条连接）。",
                "The line covers the last 2 minutes in 2-second steps; the numbers are this run's totals ({sessions} connections).",
            ),
            &[("sessions", &sessions)],
        )
    };

    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(SPACE_S))
        .px(px(ROW_PADDING))
        // 和上面「实时流量」一行对称：靠分隔线这侧 8dp，卡片底部 12dp。
        .pt(px(SPACE_S))
        .pb(px(ROW_PADDING))
        .child(chart_header(t("连接去向", "Where traffic goes"), theme))
        .child(split_trend_chart(&runtime.trend, theme))
        // 图例和说明是同一件事（「图例说的是这 2 分钟」），凑成一簇；分开
        // 8dp 会让说明读起来像图的第二种东西。
        .child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(SPACE_XS))
                .child(legend_row)
                .child(hint(caption, theme)),
        )
        .into_any_element()
}

/// 图例一项：色块 + 「标签 数字（百分比）」。
///
/// 百分比留空就不画括号——失败线没有分母，硬凑一个百分比只会误导。
fn legend_item(
    label: &'static str,
    value: u64,
    pct: String,
    color: Hsla,
    theme: &Theme,
) -> AnyElement {
    let text = if pct.is_empty() {
        format!("{label} {value}")
    } else {
        format!("{label} {value}（{pct}）")
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(LEGEND_GAP))
        .child(legend_swatch(color, theme))
        .child(legend_text(text, theme))
        .into_any_element()
}

/// 最近判定：每条连接一行，结论（直连 / 代理）+ 目标 + 对端地址或原因。
///
/// 数据来自 [`crate::core::split::Decision`]（分流器自己记的结构化记录），不是
/// 从日志里抠字符串：日志有 20 条的限流，这里要的是「最近这几条一条不少」。
pub fn split_decisions_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let runtime = &snap.split;
    let mut rows: Vec<AnyElement> = Vec::new();

    if runtime.decisions.is_empty() {
        rows.push(
            setting_row(
                Some(row_icon(DefaultIcons::globe_2(), theme).into_any_element()),
                if runtime.running {
                    t("还没有连接经过分流器", "No connection yet")
                } else {
                    t(
                        "隧道启动后这里会实时出现每条连接的去向",
                        "Start the tunnel and every connection shows up here live",
                    )
                },
                Some(
                    t(
                        "国内域名与 IP 显示「直连」，其余显示「代理」",
                        "China domains and IPs show “Direct”, everything else “Proxy”",
                    )
                    .to_string(),
                ),
                None,
                None,
                theme,
            )
            .into_any_element(),
        );
    } else {
        let items: Vec<AnyElement> = runtime
            .decisions
            .iter()
            .map(|decision| decision_row(decision, theme))
            .collect();

        // 结构照抄规则明细（见 `split_detail_group` 里的长注释）：相对定位的
        // 定高盒 → `size_full()` 的滚动本体 → 绝对定位的滚动条。别改成
        // 「外层 `overflow_hidden` + 内层滚动」，那样里面的元素收不到点击。
        let handle = app.split_decisions_scroll.clone();
        let viewport = div()
            .relative()
            .w_full()
            .h(px(SPLIT_DECISIONS_HEIGHT))
            .child(
                div()
                    .id("split-decisions-scroll")
                    .size_full()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(theme.border())
                    .bg(theme.surface_secondary())
                    .overflow_y_scroll()
                    .track_scroll(&handle)
                    // 里层还能滚的时候把滚轮截住，否则页面会跟着一起滚。
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                        let delta = event.delta.pixel_delta(window.line_height()).y;
                        let offset = this.split_decisions_scroll.offset().y;
                        let max = this.split_decisions_scroll.max_offset().y;
                        let can_scroll = if delta > px(0.0) {
                            offset < px(0.0)
                        } else if delta < px(0.0) {
                            offset > -max
                        } else {
                            true
                        };
                        if can_scroll {
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .py(px(SPACE_XS))
                            .children(items),
                    ),
            )
            .child(super::widgets::scroll_bar(
                "split-decisions-scroll-bar",
                super::widgets::ScrollBarKey::SplitDecisions,
                &handle,
                theme,
                &app.scroll_drag,
            ));

        rows.push(
            div()
                .w_full()
                // 上下也要各留一个 `ROW_PADDING`：其它行是 `setting_row`，它的
                // py 让内容离卡片边框 12dp；列表只有左右缩进时，方框会直接贴着
                // 卡片上边框和分组标题，跟旁边每一行都不齐。
                .px(px(ROW_PADDING))
                .py(px(ROW_PADDING))
                .child(viewport)
                .into_any_element(),
        );
    }

    rows.push(
        setting_row(
            Some(row_icon(DefaultIcons::info_circled(), theme).into_any_element()),
            t("这些记录从哪来", "Where these come from"),
            Some(t(
                "分流器每处理一条连接就记一行，新的在最上面，最多留最近 20 条；完整日志在「程序与日志」页",
                "One row per connection, newest first, last 20 kept; the full log is on the Program page",
            )
            .to_string()),
            None,
            Some(
                icon_action(
                    "open-split-log",
                    t("查看全部日志", "Open the full log"),
                    DefaultIcons::chevron_right(),
                    cx.listener(|this, _, _, cx| this.handle(Msg::PageSelected(Page::Program), cx)),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    );

    settings_group(t("最近判定", "Recent decisions"), rows, theme).into_any_element()
}

/// 一条判定：连通性记号 + 结论标签 + 目标，下一行是对端地址 / 原因。
///
/// 说明放第二行而不是挤在同一行右边：失败原因是自由文本（`拨号失败：出口拨号
/// 失败：…`），挤在右边只会被截成半句——恰恰是出问题时最需要读完整的那半句。
fn decision_row(decision: &crate::core::split::Decision, theme: &Theme) -> AnyElement {
    use crate::core::split::DecisionRoute;

    let (glyph, glyph_color) = if decision.ok {
        (DefaultIcons::check_circled(), theme.success())
    } else {
        (DefaultIcons::cross_1(), theme.danger())
    };
    let route_color = match decision.route {
        DecisionRoute::Direct => theme.success(),
        DecisionRoute::Proxy => theme.accent(),
    };

    div()
        .flex()
        .items_start()
        .gap(px(SPACE_S))
        .px(px(ROW_PADDING))
        .py(px(SPACE_XS))
        // 图标要和第一行文字的基线看齐，所以自己加一点上边距。
        .child(
            div()
                .flex_none()
                .mt(px(1.0))
                .child(icon(glyph, 14.0, glyph_color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(SPACE_XS))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SPACE_S))
                        .w_full()
                        .min_w_0()
                        // 结论标签定宽：一堆目标能对齐成一列，扫一眼就能分出
                        // 「哪些直连、哪些代理」。
                        .child(
                            div()
                                .flex_none()
                                .w(px(52.0))
                                .text_center()
                                .rounded(px(4.0))
                                .px(px(SPACE_XS))
                                .bg(route_color.opacity(0.14))
                                .text_size(px(TEXT_SMALL))
                                .text_color(route_color)
                                .child(decision.route.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(TEXT_BODY))
                                .font_family(super::theme::MONO_FAMILY)
                                .text_color(theme.fg())
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(decision.target.clone()),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .pl(px(52.0 + SPACE_S))
                        .text_size(px(TEXT_SMALL))
                        .text_color(if decision.ok {
                            theme.fg_muted()
                        } else {
                            theme.danger()
                        })
                        .child(decision.detail.clone()),
                ),
        )
        .into_any_element()
}

/// 分流总开关 + 现在的分流情况。
pub fn split_hero_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let settings = &app.config.split;
    let runtime = &snap.split;

    let (state_text, state_color) = if !settings.enabled {
        (t("已关闭", "Disabled").to_string(), theme.fg_muted())
    } else if runtime.running {
        (t("运行中", "Running").to_string(), theme.success())
    } else {
        (
            t("已开启，隧道启动后生效", "Enabled; applies when the tunnel starts").to_string(),
            theme.warning(),
        )
    };

    // 状态说明按「有没有真的在分流」分开写：关闭时说明它为什么不生效，
    // 运行中就把监听地址和生效条数摆出来——用户核对「到底有没有用上」
    // 靠的就是这两个数。
    let state_desc = if runtime.running {
        let addr = runtime
            .addr
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let rules = runtime.rules.to_string();
        let base = tf(
            t(
                "监听 {addr}，生效 {rules} 条规则",
                "Listening on {addr} with {rules} rules",
            ),
            &[("addr", &addr), ("rules", &rules)],
        );
        // 一行紧凑数字留在这一页：趋势图在仪表盘上，而「刚才分流了几条」
        // 是调规则时最常回头确认的一件事，翻页去看反而绕。
        if runtime.sessions == 0 {
            base
        } else {
            let direct = runtime.direct.to_string();
            let proxy = runtime.proxy.to_string();
            let failed = runtime.failed.to_string();
            tf(
                t(
                    "{base} · 本次直连 {direct} / 走代理 {proxy} / 失败 {failed}",
                    "{base} · this run: {direct} direct / {proxy} proxied / {failed} failed",
                ),
                &[
                    ("base", &base),
                    ("direct", &direct),
                    ("proxy", &proxy),
                    ("failed", &failed),
                ],
            )
        }
    } else if settings.enabled {
        t(
            "重启隧道后接管国内流量",
            "Restart the tunnel to take over China traffic",
        )
        .to_string()
    } else {
        t(
            "所有流量照常走上游代理，与没有这个功能时完全一致",
            "Everything still goes through the upstream proxy, exactly as before",
        )
        .to_string()
    };

    let mut rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::globe_2(), theme).into_any_element()),
            t("国内流量直连", "Route China traffic directly"),
            Some(
                t(
                    "命中国内清单的域名与 IP 用本地网络直连，其余照常走代理。改动总开关需要重启隧道。",
                    "Domains and IPs on the China lists are dialed directly; the rest keep using the proxy. Changing the switch needs a tunnel restart.",
                )
                .to_string(),
            ),
            None,
            Some(
                toggle_control(
                    "split-enabled",
                    settings.enabled,
                    cx.listener(|this, _, _, cx| {
                        let next = !this.config.split.enabled;
                        this.handle(Msg::SetSplitEnabled(next), cx)
                    }),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::ruler_horizontal(), theme).into_any_element()),
            t("分流状态", "Split status"),
            Some(state_desc),
            None,
            Some(
                div()
                    .text_size(px(TEXT_BODY))
                    .text_color(state_color)
                    .child(state_text)
                    .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    ];

    // 域名规则只在 fake-IP（虚拟 IP）策略下才看得见：其它策略里 tun2proxy
    // 交给分流器的只有 IP 字面量，域名清单无从匹配。不提示的话，用户会以为
    // 「国内域名都配上了」，实际只有 IP 段在起作用。
    if settings.enabled && app.config.active_profile().dns != DnsMode::Virtual {
        rows.push(
            div()
                .flex()
                .items_start()
                .gap(px(SPACE_S))
                .px(px(ROW_PADDING))
                .py(px(ROW_PADDING))
                .child(
                    div()
                        .flex_none()
                        .child(icon(DefaultIcons::exclamation_triangle(), 16.0, theme.warning())),
                )
                .child(hint(
                    tf(
                        t(
                            "当前 DNS 策略是「{mode}」：tun2proxy 只能看到 IP，域名清单不会生效，只有 IP 段在分流。把 DNS 策略改成「虚拟 IP」才能用上域名规则。",
                            "DNS mode is “{mode}”: tun2proxy only sees IPs, so the domain lists cannot match and only IP ranges are split. Switch DNS to “Virtual IP” to use domain rules.",
                        ),
                        &[("mode", app.config.active_profile().dns.label())],
                    ),
                    theme,
                ))
                .into_any_element(),
        );
    }

    settings_group(t("开关与状态", "Switch & status"), rows, theme).into_any_element()
}

/// 三组内置清单，各带一个开关。
pub fn split_lists_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let settings = &app.config.split;
    let library = &app.split_library;

    let rows = SplitGroup::ALL
        .iter()
        .map(|group| {
            let group = *group;
            let count = library.group_len(group).to_string();
            setting_row(
                Some(row_icon(DefaultIcons::database(), theme).into_any_element()),
                group.label(),
                Some(tf(
                    t("{hint}（{count} 条）", "{hint} ({count} entries)"),
                    &[("hint", group.hint()), ("count", &count)],
                )),
                None,
                Some(
                    toggle_control(
                        eid("split-group", group as usize),
                        settings.group_enabled(group),
                        cx.listener(move |this, _, _, cx| {
                            let next = !this.config.split.group_enabled(group);
                            this.handle(Msg::SetSplitGroup(group, next), cx)
                        }),
                    )
                    .into_any_element(),
                ),
                theme,
            )
            .into_any_element()
        })
        .collect();

    settings_group(t("内置清单", "Built-in lists"), rows, theme).into_any_element()
}

/// 清单的在线更新与「更新时间」。
///
/// 日期来自清单文件头（数据是哪天的），不是「我刚才点了按钮」——用户想知道的
/// 是这份表有多旧，那个时间点本身没有意义。
pub fn split_update_group(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let library = &app.split_library;
    let updated = if library.date.is_empty() {
        library.origin.label().to_string()
    } else {
        tf(
            t("{origin} · 数据日期 {date}", "{origin} · data dated {date}"),
            &[
                ("origin", library.origin.label()),
                ("date", library.date.as_str()),
            ],
        )
    };
    let total = library.total_len().to_string();

    let mut rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::reload(), theme).into_any_element()),
            t("更新时间", "Last update"),
            Some(tf(
                t("共 {total} 条规则", "{total} rules in total"),
                &[("total", &total)],
            )),
            None,
            Some(
                div()
                    .text_size(px(TEXT_BODY))
                    .text_color(theme.fg())
                    .child(updated)
                    .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::paper_plane(), theme).into_any_element()),
            t("在线更新", "Update online"),
            Some(
                t(
                    "从 APNIC 与 v2fly 重新聚合一遍国内 IP 与域名，大约几秒到十几秒。更新完立刻生效，不用重启隧道。",
                    "Re-aggregate the China IP and domain lists from APNIC and v2fly; usually a few seconds. Takes effect immediately, no tunnel restart needed.",
                )
                .to_string(),
            ),
            None,
            Some(
                labeled_action(
                    "split-update",
                    t("立即更新", "Update now"),
                    DefaultIcons::reload(),
                    theme,
                    cx.listener(|this, _, _, cx| this.handle(Msg::SplitUpdate, cx)),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    ];

    if let Some(text) = snap.split_update.text() {
        let color = match snap.split_update {
            crate::core::engine::SplitUpdate::Failed(_) => theme.danger(),
            crate::core::engine::SplitUpdate::Done(_) => theme.success(),
            _ => theme.fg_muted(),
        };
        rows.push(
            div()
                .flex()
                .items_start()
                .gap(px(SPACE_S))
                .px(px(ROW_PADDING))
                .py(px(ROW_PADDING))
                .child(
                    div()
                        .flex_none()
                        .child(icon(DefaultIcons::info_circled(), 16.0, color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(TEXT_SMALL))
                        .text_color(color)
                        .child(text.to_string()),
                )
                .into_any_element(),
        );
    }

    rows.push(
        setting_row(
            Some(row_icon(DefaultIcons::reset(), theme).into_any_element()),
            t("恢复内置清单", "Restore the built-in lists"),
            Some(
                t(
                    "删掉下载下来的清单，退回编译进程序的那份快照。",
                    "Delete the downloaded lists and fall back to the snapshot compiled into the binary.",
                )
                .to_string(),
            ),
            None,
            Some(
                text_action(
                    "split-reset",
                    t("恢复", "Restore"),
                    theme,
                    cx.listener(|this, _, _, cx| this.handle(Msg::SplitReset, cx)),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    );

    settings_group(t("清单更新", "List updates"), rows, theme).into_any_element()
}

/// 自定义规则：多行输入 + 应用。
pub fn split_custom_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let custom = app.inputs.get(Field::SplitCustom);
    let text_inset = theme.control(ControlSize::Large).padding_x.0 * 16.0 + 1.0;

    let rows = vec![
        setting_block(
            t("规则列表", "Rule lines"),
            Some(
                t(
                    "一行一条；写在清单前面，能盖住内置清单里的同名规则。",
                    "One rule per line; written ahead of the built-in lists so it can override them.",
                )
                .to_string(),
            ),
            div()
                .relative()
                .w_full()
                .child(textarea(&custom, cx).rows(7).max_height(px(170.0)).large())
                .child(super::widgets::input_scroll_bar(
                    "split-custom-scroll-bar",
                    super::widgets::ScrollBarKey::Input("split-custom"),
                    &custom,
                    text_inset,
                    theme,
                    &app.scroll_drag,
                )),
            theme,
        )
        .into_any_element(),
        setting_block(t("写法示例", "Examples"), None, split_custom_examples(theme), theme)
            .into_any_element(),
        compact_row(
            t("改完要应用才会生效", "Apply to take effect"),
            Some(
                labeled_action(
                    "split-apply",
                    t("应用规则", "Apply rules"),
                    DefaultIcons::check_circled(),
                    theme,
                    cx.listener(|this, _, _, cx| this.handle(Msg::SplitApply, cx)),
                )
                .into_any_element(),
            ),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("自定义规则", "Custom rules"), rows, theme).into_any_element()
}

/// 自定义规则写法示例：左边是能直接粘进输入框的一行，右边是它代表什么。
fn split_custom_examples(theme: &Theme) -> Div {
    const EXAMPLES: [(&str, &str, &str); 5] = [
        (
            "suffix:example.cn",
            "这个后缀及其子域名直连",
            "This suffix and its subdomains go direct",
        ),
        (
            "full:only.example.cn",
            "只有这一个域名直连",
            "Only this exact domain goes direct",
        ),
        (
            "public-cidr/24",
            "整个网段直连",
            "The whole range goes direct",
        ),
        (
            "proxy:cdn.example.cn",
            "强制走代理，用来抵消清单里的某一条",
            "Force the proxy, to cancel one built-in entry",
        ),
        (
            "# 注释",
            "以 # 开头的行会被忽略",
            "Lines starting with # are ignored",
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

/// 规则明细：筛选框 + 每一行的单独开关。
pub fn split_detail_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let settings = &app.config.split;
    let library = &app.split_library;
    let filter = app.split_filter.trim().to_lowercase();

    let find_row = setting_row(
        Some(row_icon(DefaultIcons::magnifying_glass(), theme).into_any_element()),
        t("查找规则", "Find a rule"),
        Some(
            t(
                "输入域名或地址的一部分，只影响这里的显示",
                "Type part of a domain or address; affects this list only",
            )
            .to_string(),
        ),
        None,
        Some(text_input(&app.inputs.get(Field::SplitFilter), ROW_CONTROL, cx)),
        theme,
    )
    .into_any_element();

    let mut rule_rows: Vec<AnyElement> = Vec::new();

    let mut shown = 0usize;
    let mut truncated = false;
    for group in SplitGroup::ALL {
        if !settings.group_enabled(group) {
            continue;
        }
        for rule in library.group_rules(group) {
            if !filter.is_empty() && !matcher_contains(&rule.matcher, &filter) {
                continue;
            }
            if shown >= SPLIT_ROWS_SHOWN {
                truncated = true;
                break;
            }
            let key = rule.key();
            let enabled = settings.row_enabled(&key);
            let toggle = toggle_control(
                eid("split-row", shown),
                enabled,
                cx.listener({
                    let key = key.clone();
                    move |this, _, _, cx| this.handle(Msg::SplitRowToggled(key.clone()), cx)
                }),
            );
            rule_rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_M))
                    .px(px(ROW_PADDING))
                    .py(px(SPACE_S))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(TEXT_BODY))
                            .font_family(super::theme::MONO_FAMILY)
                            .text_color(if enabled {
                                theme.fg()
                            } else {
                                theme.fg_muted()
                            })
                            .child(key.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(TEXT_SMALL))
                            .text_color(theme.fg_muted())
                            .child(rule.source.clone()),
                    )
                    .child(toggle.into_any_element())
                    .into_any_element(),
            );
            shown += 1;
        }
        if truncated {
            break;
        }
    }

    if shown == 0 {
        rule_rows.push(
            compact_row(
                if filter.is_empty() {
                    t("没有可显示的规则", "No rules to show")
                } else {
                    t("没有匹配的规则", "No rule matches")
                },
                None,
                theme,
            )
            .into_any_element(),
        );
    }

    let note = if truncated {
        Some(tf(
            t(
                "只显示前 {n} 条，用上面的查找框定位更靠后的规则",
                "Showing the first {n}; use the search field to reach later rules",
            ),
            &[("n", &SPLIT_ROWS_SHOWN.to_string())],
        ))
    } else {
        Some(
            t(
                "取消勾选的那一行不参与判定，其它行不受影响",
                "An unchecked row is left out of matching; the others are unaffected",
            )
            .to_string(),
        )
    };

    // 列表装在定高框里自己滚，否则 100 行会把「直连出口」「清单更新」顶到
    // 很远的地方。结构照抄页面滚动区（`pages::page_scroll_area`）：
    //
    //   相对定位的定高盒 → `size_full()` 的滚动本体 → 绝对定位的滚动条
    //
    // 别改成「外层 `overflow_hidden` + 内层 `size_full` 滚动」：那样虽然看着
    // 一样，实测行尾的勾选框会收不到点击（配置里的 `disabled` 也确实没写进
    // 去），而页面里同一套按钮是正常的——踩过这个坑，别再来一次。
    let handle = app.split_rules_scroll.clone();
    let viewport = div()
        .relative()
        .w_full()
        .h(px(SPLIT_LIST_HEIGHT))
        .child(
            div()
                .id("split-rules-scroll")
                .size_full()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme.border())
                .bg(theme.surface_secondary())
                .overflow_y_scroll()
                .track_scroll(&handle)
                // 里层还能滚的时候把滚轮截住，否则页面会跟着一起滚。
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                    let delta = event.delta.pixel_delta(window.line_height()).y;
                    let offset = this.split_rules_scroll.offset().y;
                    let max = this.split_rules_scroll.max_offset().y;
                    let can_scroll = if delta > px(0.0) {
                        offset < px(0.0)
                    } else if delta < px(0.0) {
                        offset > -max
                    } else {
                        true
                    };
                    if can_scroll {
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .py(px(SPACE_XS))
                        .children(rule_rows),
                ),
        )
        .child(super::widgets::scroll_bar(
            "split-rules-scroll-bar",
            super::widgets::ScrollBarKey::SplitRules,
            &handle,
            theme,
            &app.scroll_drag,
        ));

    let rows = vec![
        find_row,
        div()
            .w_full()
            // 和「最近判定」一样：上下各留 12dp，别让方框贴着卡片边框。
            .px(px(ROW_PADDING))
            .py(px(ROW_PADDING))
            .child(viewport)
            .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::info_circled(), theme).into_any_element()),
            t("单行开关", "Per-rule switches"),
            note,
            None,
            None,
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("规则明细", "Rule detail"), rows, theme).into_any_element()
}

/// 规则文本里是否包含某段文字（不分配内存的版本，给筛选用）。
fn matcher_contains(matcher: &crate::core::split::Matcher, needle: &str) -> bool {
    use crate::core::split::Matcher;
    match matcher {
        Matcher::Exact(text) | Matcher::Suffix(text) | Matcher::Keyword(text) => {
            text.contains(needle)
        }
        // CIDR 只能连着掩码一起比：用户输 `1.2.3` 也想命中 `1.2.3.0/24`。
        Matcher::Cidr(cidr) => cidr.to_string().contains(needle),
    }
}

/// 直连出口：用哪个 DNS、从哪块网卡出去。
pub fn split_outlet_group(app: &App, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let rows = vec![
        setting_row(
            Some(row_icon(DefaultIcons::globe(), theme).into_any_element()),
            t("直连 DNS", "Direct DNS"),
            Some(
                t(
                    "直连的域名用这个 DNS 解析。填国内递归 DNS，否则国内站点会被解析到境外节点，直连反而更慢。",
                    "Domains dialed directly are resolved with this server. Use a domestic resolver, or China sites resolve to overseas nodes and go slower than the proxy.",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::SplitDns),
                ROW_CONTROL_NARROW,
                cx,
            )),
            theme,
        )
        .into_any_element(),
        setting_row(
            Some(row_icon(DefaultIcons::link_1(), theme).into_any_element()),
            t("直连出口网卡", "Direct outbound interface"),
            Some(
                t(
                    "留空则自动选一块不是 TUN 的网卡。填电脑上网卡名可以强制走它，例如 以太网 / WLAN。",
                    "Leave empty to auto-pick a non-TUN adapter. Fill in an adapter name (e.g. Ethernet / Wi-Fi) to force it.",
                )
                .to_string(),
            ),
            None,
            Some(text_input(
                &app.inputs.get(Field::SplitInterface),
                ROW_CONTROL,
                cx,
            )),
            theme,
        )
        .into_any_element(),
    ];

    settings_group(t("直连出口", "Direct outlet"), rows, theme).into_any_element()
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
            Some(row_icon(DefaultIcons::link_1(), theme).into_any_element()),
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
        // 滚动条自己画：gpui 的滚动容器不画条，没有它就看不出「这里还能滚」。
        .child(super::widgets::scroll_bar(
            "log-scroll-bar",
            super::widgets::ScrollBarKey::Log,
            &scroll_handle,
            theme,
            &app.scroll_drag,
        ));

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