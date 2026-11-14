//! 左侧边栏：品牌 + 导航 + 状态 + 外观 / 语言 + 一键启停。
//!
//! 侧边栏是整个界面唯一「常驻」的区域：切到任何一页都能看到当前状态、
//! 都能直接启动或停止，不必先回到仪表盘。信息从上到下按重要性递减排列：
//! 我是谁 → 我在哪一页 → 我现在什么状态 → 界面长什么样 → 主操作。
//!
//! 两种形态：
//!
//! - **完整**（[`SIDEBAR_WIDTH`]）：图标 + 文字，选中项铺一层 `accent_bg()`
//!   并在左侧画一条 3dp 竖条；
//! - **收起**（[`RAIL_WIDTH`]）：只有一列 40dp 图标，选中项同样是强调色底。
//!
//! 底色用 `surface()`、内容区用 `bg()`，右侧再压一条 `border()` 的 1dp 竖线：
//! 侧边栏与内容区是两块并列的面板，层次靠色差，边界靠那条线。
//!
//! 收起只省地方，不藏功能：导航项全带 tooltip，状态点带状态文字，
//! 外观、语言和启停按钮也还在原位。
//!
//! # 外观 / 语言为什么在侧栏
//!
//! 设置页里有「界面语言」「界面主题」两个下拉框（那是**改设置**的地方），
//! 侧栏底部这一行（[`appearance_row`]）是它们的**快捷入口**：明暗和语言都是
//! 「当场想看一眼」的东西，选一次下拉框要点两下、还得先翻到那一页；而换个
//! 主题或换种语言看看，本来就是随手的事。
//!
//! 两件事并排在一行里、各占一半，格子里只放图标 + 当前值（名称交给 tooltip）：
//! 并排之后每格 78dp，塞不下「名称 + 值」两段文字，而值才是要读的那个。
//!
//! 也没有并进状态行：状态行剩下的宽度只有 100dp 出头，「正在启动…」
//! 「Not running」本来就贴着省略号，再塞按钮会先被吃掉。

use gpui::{AnyElement, Context, Div, Hsla, IntoElement, Svg, div, px, rems, prelude::*};
use gpuikit::theme::{Theme, ThemeVariant, Themeable};
use gpuikit::{DefaultIcons, elements::icon_button::icon_button};

use crate::core::APP_VERSION;
use crate::core::config::ThemeSetting;
use crate::core::i18n::{LangSetting, t, tf};

use super::cards::{status_dot_color, status_label};
use super::state::{App as UiApp, Msg, Page, Snapshot};
use super::theme::{CONTROL_HEIGHT_LARGE, TEXT_BODY, TEXT_GROUP, TEXT_SMALL, WEIGHT_SEMIBOLD};
use super::widgets::{SPACE_S, SPACE_XL, SPACE_XS, dot, hairline, icon, v_hairline};

/// 侧边栏宽度（逻辑像素）。
///
/// 宽度由两件事撑出来，取更宽的那个：
///
/// - 品牌行：`Tun2ProxyGUI`（14dp 半粗）+ `v0.5.0`（11dp 灰）+ 间距 + 收起按钮
///   32dp，约 170dp；
/// - 版本胶囊：`tun2proxy v0.8.6` 在 14dp 下约 130dp，加左右留白约 146dp。
///
/// 200 - 左右各 8 = 184dp 可用宽，两件都装得下。176dp 时 `v0.5.0` 会被收起按钮
/// 压掉最后一个字符，所以品牌名加上 `GUI` 之后必须跟着放宽。
pub const SIDEBAR_WIDTH: f32 = 200.0;
/// 收起后的图标栏宽度：40dp 的导航项 + 左右各 6dp 留白。
pub const RAIL_WIDTH: f32 = 52.0;
/// 侧边栏内部左右留白。
const SIDEBAR_PADDING: f32 = 8.0;
/// 图标栏内部留白。
const RAIL_PADDING: f32 = 6.0;
/// 收起态导航项的宽度（40dp 的方形图标格）。
const RAIL_ITEM_WIDTH: f32 = 40.0;
/// 导航项高度。
const ITEM_HEIGHT: f32 = 38.0;

/// 导航项 / 侧栏按钮悬停时的底色。
///
/// 全侧栏共用一档（`surface_secondary()`），不另起一档更淡的灰：悬停和选中
/// 是两件事——悬停是「手在哪儿」，选中是「你在哪一页」，前者靠一层灰、后者靠
/// 强调色底 + 左侧竖条，本来就分得开。改成更淡的一层之后悬停几乎看不见，
/// 反而让人怀疑「这个能不能点」。
fn hover_bg(theme: &Theme) -> Hsla {
    theme.surface_secondary()
}

/// 构建整条侧边栏。
pub fn sidebar(
    app: &UiApp,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<UiApp>,
) -> AnyElement {
    let pane = if app.sidebar_collapsed {
        rail(app, snap, theme, cx)
    } else {
        expanded(app, snap, theme, cx)
    };
    div()
        .flex()
        .flex_none()
        .items_stretch()
        .child(pane)
        .child(v_hairline(theme))
        .into_any_element()
}

/// 完整形态：图标 + 文字。
fn expanded(app: &UiApp, snap: &Snapshot, theme: &Theme, cx: &mut Context<UiApp>) -> Div {
    let (status_text, color) = status_label(&snap.status, theme);
    // 圆点取状态灯色（未运行是熄灭的灰），文字仍用上面这档：两者同色的话，
    // 那颗点看着只是句首的圆点符号，不像一盏灯。
    let dot_color = status_dot_color(&snap.status, theme);

    let brand = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(SPACE_S))
                .child(
                    // 名称后面紧跟本程序自己的版本：排查时第一句话往往是
                    // 「你用的是哪一版」，它得在一眼看得见的地方，而不是
                    // 「程序与日志」页面里。字重和字号都降一档，别跟名称抢。
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        // 按基线对齐：15dp 的名称和 11dp 的版本光靠垂直居中
                        // 会差出一两个像素，看起来像版本号掉在了名称下面。
                        .items_baseline()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_size(px(TEXT_GROUP))
                                .font_weight(WEIGHT_SEMIBOLD)
                                .text_color(theme.fg())
                                .whitespace_nowrap()
                                .child("Tun2ProxyGUI"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme.fg_muted())
                                .whitespace_nowrap()
                                .child(format!("v{APP_VERSION}")),
                        ),
                )
                .child(toggle(app, theme, cx)),
        )
        // 版本号是小字而不是彩色胶囊：胶囊是「状态」的形状（未运行 / 运行中
        // 都是胶囊），版本号是静态信息，给它同一个形状会让侧栏顶部出现两个
        // 视觉焦点，也让人以为版本会变。
        //
        // 这一行是 **tun2proxy 二进制的版本**，和名称后面那个 GUI 版本是两件
        // 事：二进制可以随时「检查更新」换掉，前端版本只在装新版程序时变。
        .child(
            div()
                .px(px(4.0))
                .text_size(px(11.0))
                .text_color(theme.fg_muted())
                .whitespace_nowrap()
                .child(version_label(snap)),
        );

    let nav = div().flex().flex_col().gap(px(2.0)).children(
        Page::ALL
            .into_iter()
            .map(|page| nav_item(app, page, theme, cx)),
    );

    // 主操作：和状态同一行，一个 28dp 的方形图标按钮。
    //
    // 不撑满侧栏宽度、不占独立一行：整条侧栏最重的色块如果是底部那个按钮，
    // 视线会先落到它上面，再倒回去找「我在哪一页」——侧栏的主线是导航，
    // 启停是常驻的快捷入口，不该抢这条主线。图标按钮仍然常驻、仍然一步可达，
    // 悬停有 tooltip 说明。
    let (action_id, action_icon, action_label, action_msg) = if snap.status.is_active() {
        (
            "sidebar-stop",
            DefaultIcons::stop(),
            t("停止", "Stop"),
            Msg::Stop,
        )
    } else {
        (
            "sidebar-start",
            DefaultIcons::play(),
            t("启动", "Start"),
            Msg::Start,
        )
    };
    // 忙碌（蒙版挂着）的时候这颗按钮也点不动：遮罩挡住了内容区，但侧栏这颗
    // 按钮就压在遮罩下面，用户会先点它——连点两次会起两条检测线程。置灰比
    // 「点了弹一句不行」好：不必先点一次才知道现在不能点；tooltip 换成正发生
    // 的那件事（「正在启动隧道…」），说明为什么点不动。
    let busy = app.busy.action().map(|action| action.label());
    // 用 gpuikit 的 `icon_button` 而不是自绘方框：它的盒模型、悬停底色、
    // 禁用态和 tooltip 都由组件自己维护，侧栏顶部那个收起按钮也是它，
    // 两处口径一致。32dp 的盒 = `Large` 档，和内容区的按钮、输入框同高；
    // 16dp 的图标仍取 `ink`。
    let action = icon_button(action_id, action_icon)
        .box_size(rems(2.0))
        .icon_size(rems(1.0))
        .disabled(busy.is_some())
        .tooltip(gpuikit::elements::tooltip::tooltip(
            busy.unwrap_or(action_label),
        ))
        .on_click(cx.listener(move |this, _, _, cx| this.handle(action_msg.clone(), cx)));

    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .p(px(SIDEBAR_PADDING))
        .gap(px(SPACE_S))
        .bg(theme.surface())
        .child(brand)
        .child(div().flex_none().h(px(SPACE_XL)))
        .child(nav)
        .child(div().flex_1())
        // 底部：一条发丝线把「导航」和「状态 / 外观 / 启停」分开，否则状态
        // 那一行悬在空白里，读起来像是没画完。
        .child(hairline(theme))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(SPACE_S))
                .px(px(4.0))
                .child(dot(dot_color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(TEXT_BODY))
                        .text_color(color)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(status_text),
                )
                .child(action),
        )
        // 底部并排两格：外观在左、语言在右。压在状态行下面，就不会抢
        // 「现在什么状态」那条视线，同时仍然常驻、仍然一步可达。
        .child(appearance_row(app, theme, cx))
}

/// 收起形态：只剩一列图标。
fn rail(app: &UiApp, snap: &Snapshot, theme: &Theme, cx: &mut Context<UiApp>) -> Div {
    let (status_text, _) = status_label(&snap.status, theme);
    let dot_color = status_dot_color(&snap.status, theme);

    let nav = div().flex().flex_col().items_center().gap(px(2.0)).children(
        Page::ALL
            .into_iter()
            .map(|page| rail_item(app, page, theme, cx)),
    );

    // 启停按钮：和导航项同一套渲染（同样的方框、同样的悬停态）。
    // 图标集里「启动」用播放三角，「停止」用方块。
    let (action_label, action_icon, action_msg) = if snap.status.is_active() {
        (t("停止", "Stop"), DefaultIcons::stop(), Msg::Stop)
    } else {
        (t("启动", "Start"), DefaultIcons::play(), Msg::Start)
    };
    // 忙碌时和上面完整形态那颗图标按钮一样：置灰、点不动、tooltip 说明原因。
    // 收起态没有 `icon_button` 可用（这一格要和导航项同形状），所以手写一份
    // 禁用态：去掉光标与点击、图标降到 `fg_disabled()`。
    let busy = app.busy.action().map(|action| action.label());
    let action = div()
        .id("rail-action")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(RAIL_ITEM_WIDTH))
        .h(px(ITEM_HEIGHT))
        .rounded(px(6.0))
        .when(busy.is_none(), |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(hover_bg(theme)))
                .on_click(cx.listener(move |this, _, _, cx| this.handle(action_msg.clone(), cx)))
        })
        .when(busy.is_some(), |this| this.cursor_not_allowed())
        .tooltip(gpuikit::elements::tooltip::tooltip(
            busy.unwrap_or(action_label),
        ))
        .child(icon(
            action_icon,
            16.0,
            if busy.is_some() {
                theme.fg_disabled()
            } else {
                theme.fg_muted()
            },
        ));

    div()
        .flex()
        .flex_col()
        .items_center()
        .flex_none()
        .w(px(RAIL_WIDTH))
        .h_full()
        .p(px(RAIL_PADDING))
        .gap(px(SPACE_S))
        .bg(theme.surface())
        .child(toggle(app, theme, cx))
        .child(nav)
        .child(div().flex_1())
        // 状态只剩一个色点：文字挤不进 52dp，颜色 + tooltip 足够。这一处
        // 没有文字陪衬，圆点的颜色就是全部信息，更要和展开态、托盘一致。
        .child(
            div()
                .id("rail-status")
                .flex_none()
                .tooltip(gpuikit::elements::tooltip::tooltip(status_text))
                .child(dot(dot_color)),
        )
        .child(hairline(theme))
        .child(action)
        // 外观、语言跟着启停排在最后两格：三样都是常驻的快捷动作，形状一致。
        .child(rail_theme(app, theme, cx))
        .child(rail_lang(app, theme, cx))
}

/// 侧边栏的收起/展开按钮。
///
/// 长在侧边栏里，而不是内容区顶栏：它管的是侧边栏，就该贴着侧边栏——
/// 跑到内容区标题旁边，看起来像是在管「当前这一页」。
fn toggle(app: &UiApp, _theme: &Theme, cx: &mut Context<UiApp>) -> impl IntoElement {
    let label = if app.sidebar_collapsed {
        t("展开侧边栏", "Expand sidebar")
    } else {
        t("收起侧边栏", "Collapse sidebar")
    };
    icon_button("sidebar-toggle", DefaultIcons::panel_left())
        .box_size(rems(2.0))
        .icon_size(rems(1.0))
        .tooltip(gpuikit::elements::tooltip::tooltip(label))
        .on_click(cx.listener(|this, _, _, cx| this.handle(Msg::ToggleSidebar, cx)))
}

/// 点一下「外观」之后该变成哪一档。
///
/// 「跟随系统」往哪一档跳，取决于**现在实际画出来的是明还是暗**：系统本来
/// 就是浅色时跳到「浅色」，画面上一点变化都没有，按钮看起来就像坏了。所以
/// 跟随系统一律跳到当前外观的反面；剩下两档按 浅色 → 深色 → 跟随系统 循环
/// （从「深色」回到「跟随系统」这一步，在深色系统上同样看不出变化，但它的
/// 语义是「交回给系统」——行上的字随即变成「外观 跟随系统」，不会让人以为
/// 没生效）。
fn next_theme(current: ThemeSetting, theme: &Theme) -> ThemeSetting {
    match current {
        ThemeSetting::Light => ThemeSetting::Dark,
        ThemeSetting::Dark => ThemeSetting::Auto,
        ThemeSetting::Auto if theme.variant == ThemeVariant::Dark => ThemeSetting::Light,
        ThemeSetting::Auto => ThemeSetting::Dark,
    }
}

/// 点一下「语言」之后该变成哪一档：跟随系统 → 中文 → English → 跟随系统。
///
/// 这里按 `LangSetting::ALL` 的顺序走，不学主题那套「跳到当前值的反面」：
/// 中英两档的名字各自用本语言写（「中文」/「English」），切过去一眼就认得出，
/// 不存在「切了看不出变化」的问题；而「跟随系统」会解析成哪种语言随系统变，
/// 猜反面反而不好预期。
fn next_lang(current: LangSetting) -> LangSetting {
    match current {
        LangSetting::Auto => LangSetting::Zh,
        LangSetting::Zh => LangSetting::En,
        LangSetting::En => LangSetting::Auto,
    }
}

/// 三档外观各自的图标。跟随系统用显示器：语义就是「交给这台机器决定」。
fn theme_icon(setting: ThemeSetting) -> Svg {
    match setting {
        ThemeSetting::Auto => DefaultIcons::desktop(),
        ThemeSetting::Light => DefaultIcons::sun(),
        ThemeSetting::Dark => DefaultIcons::moon(),
    }
}

/// 外观行的 tooltip：只说「点一下是干什么的」，不写「会变成哪一档」。
///
/// 写过一版「当前 X（点击切换为 Y）」，看起来更有用，实际会骗人：gpui 的
/// tooltip 是悬停那一刻建出来的视图，之后不会跟着状态刷新——用户在原地连着
/// 点两下，看到的还是第一下之前的那句「会变成 Y」。三档循环恰恰是最容易被
/// 连点的控件，所以这里只描述循环本身（当前是哪一档，行上右边那几个字已经
/// 写着）。
///
/// 一旦环境变量钉死了主题，点了不会有任何变化，就直说被钉住，别让人反复点。
fn theme_tooltip() -> String {
    if let Some(name) = super::theme::env_override() {
        return tf(
            t(
                "界面主题被环境变量 TUN2PROXY_GUI_THEME={name} 固定",
                "Theme pinned by TUN2PROXY_GUI_THEME={name}",
            ),
            &[("name", &name)],
        );
    }

    t(
        "点击切换界面外观（浅色 / 深色 / 跟随系统）",
        "Click to cycle the theme (light / dark / system)",
    )
    .to_string()
}

/// 语言行的 tooltip，道理同 [`theme_tooltip`]：只描述循环，不写「会变成哪个」。
fn lang_tooltip() -> String {
    t(
        "点击切换界面语言（跟随系统 / 中文 / English）",
        "Click to cycle the UI language (system / 中文 / English)",
    )
    .to_string()
}

/// 底部的外观 / 语言：一格里一件事，两格并排。
///
/// 并排的代价是每格只剩 78dp（160 减掉中间 4dp），所以格子里**不放名称**，
/// 只放图标 + 当前值：名称由 tooltip 交代，图标负责「这是哪件事」，值才是要
/// 读的东西。值用 12dp 而不是正文字号——扣掉图标 16 和间距 4，文字只剩 58dp，
/// 「跟随系统」在 14dp 下是 56dp（余 2dp，字体一回退就是省略号），12dp 只要
/// 48dp，四种取值（跟随系统 / 浅色 / 深色 / 中文 / English）全都完整、还有余量。
fn appearance_row(app: &UiApp, theme: &Theme, cx: &mut Context<UiApp>) -> impl IntoElement {
    let theme_setting = app.config.theme;
    let lang_setting = app.config.language;
    // 环境变量在场时主题点了不会变，那一格就别让它看起来能点。
    let theme_next = super::theme::env_override()
        .is_none()
        .then(|| Msg::SetTheme(next_theme(theme_setting, theme)));

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(SPACE_XS))
        .w_full()
        .child(appearance_cell(
            "sidebar-theme",
            theme_icon(theme_setting),
            theme_setting.label(),
            theme_tooltip(),
            theme_next,
            theme,
            cx,
        ))
        .child(appearance_cell(
            "sidebar-language",
            DefaultIcons::globe(),
            lang_setting.label(),
            lang_tooltip(),
            Some(Msg::SetLanguage(next_lang(lang_setting))),
            theme,
            cx,
        ))
}

/// 并排的其中一格：图标 + 当前值，点一下换下一档。
///
/// 和导航项共用一套盒模型（6dp 圆角、[`hover_bg`] 悬停底），但高度压到 32dp：
/// 它在底部是「顺手改一下」的常驻项，不该和导航抢视线。图标用 `fg_muted()`、
/// 值用 `fg()`：12dp 的小字若也用 muted 色，浅色主题下压在白底上就读不出来了。
///
/// `next` 为 `None` 表示这一格现在点不动（主题被环境变量钉死时），这时不画
/// 悬停底、鼠标也不变成手型，tooltip 里说明原因。
fn appearance_cell(
    id: &'static str,
    glyph: Svg,
    value: &str,
    tooltip: String,
    next: Option<Msg>,
    theme: &Theme,
    cx: &mut Context<UiApp>,
) -> impl IntoElement {
    let clickable = next.is_some();
    let ink = if clickable {
        theme.fg_muted()
    } else {
        theme.fg_disabled()
    };
    let text = if clickable {
        theme.fg()
    } else {
        theme.fg_disabled()
    };

    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .h(px(CONTROL_HEIGHT_LARGE))
        .rounded(px(6.0))
        .tooltip(gpuikit::elements::tooltip::tooltip(tooltip))
        .when(clickable, |style| {
            style
                .cursor_pointer()
                .hover(|style| style.bg(hover_bg(theme)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(msg) = next.clone() {
                        this.handle(msg, cx)
                    }
                }))
        })
        .child(icon(glyph, 16.0, ink))
        .child(
            div()
                .min_w_0()
                .text_size(px(TEXT_SMALL))
                .text_color(text)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(value.to_string()),
        )
}

/// 收起态的图标按钮（外观 / 语言共用）：只剩图标，名字与循环说明由 tooltip 给。
fn rail_button(
    id: &'static str,
    glyph: Svg,
    tooltip: String,
    next: Option<Msg>,
    theme: &Theme,
    cx: &mut Context<UiApp>,
) -> impl IntoElement {
    let clickable = next.is_some();
    let fg = if clickable {
        theme.fg_muted()
    } else {
        theme.fg_disabled()
    };

    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(RAIL_ITEM_WIDTH))
        .h(px(ITEM_HEIGHT))
        .rounded(px(6.0))
        .tooltip(gpuikit::elements::tooltip::tooltip(tooltip))
        .when(clickable, |style| {
            style
                .cursor_pointer()
                .hover(|style| style.bg(hover_bg(theme)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(msg) = next.clone() {
                        this.handle(msg, cx)
                    }
                }))
        })
        .child(icon(glyph, 16.0, fg))
}

/// 外观按钮（收起态）。
fn rail_theme(app: &UiApp, theme: &Theme, cx: &mut Context<UiApp>) -> impl IntoElement {
    let setting = app.config.theme;
    let next = super::theme::env_override()
        .is_none()
        .then(|| Msg::SetTheme(next_theme(setting, theme)));

    rail_button(
        "rail-theme",
        theme_icon(setting),
        theme_tooltip(),
        next,
        theme,
        cx,
    )
}

/// 语言按钮（收起态）。
fn rail_lang(app: &UiApp, theme: &Theme, cx: &mut Context<UiApp>) -> impl IntoElement {
    let setting = app.config.language;

    rail_button(
        "rail-language",
        DefaultIcons::globe(),
        lang_tooltip(),
        Some(Msg::SetLanguage(next_lang(setting))),
        theme,
        cx,
    )
}

/// 展开态的一个导航项：图标 + 文字，选中时强调色底 + 左侧竖条。
fn nav_item(
    app: &UiApp,
    page: Page,
    theme: &Theme,
    cx: &mut Context<UiApp>,
) -> impl IntoElement {
    let selected = app.page == page;
    let fg = if selected {
        theme.accent()
    } else {
        theme.fg()
    };
    let accent = theme.accent();

    div()
        .id(("nav", page.slot()))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .h(px(ITEM_HEIGHT))
        .w_full()
        .pl(px(8.0))
        .pr(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .when(selected, |style| style.bg(theme.accent_bg()))
        .when(!selected, |style| style.hover(|style| style.bg(hover_bg(theme))))
        .on_click(cx.listener(move |this, _, _, cx| this.handle(Msg::PageSelected(page), cx)))
        // 选中指示条：NavigationView 的那条 3×16 竖条。未选中时**不画**，
        // 但仍占着这 3dp——否则选中时图标会横跳 1.5dp。
        //
        // 未选中时不能拿 `surface()` 去填这个洞：平铺的侧栏上它确实看不出来，
        // 可那一项一旦悬停（底色变了），这 3dp 就变成一条白杠，像是画脏了。
        .child(
            div()
                .flex_none()
                .w(px(3.0))
                .h(px(16.0))
                .rounded(px(2.0))
                .when(selected, |style| style.bg(accent)),
        )
        .child(icon(page_icon(page), 18.0, fg))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(TEXT_BODY))
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(page.label()),
        )
}

/// 收起态的一个导航项：只有图标，名字由 tooltip 提供。
fn rail_item(
    app: &UiApp,
    page: Page,
    theme: &Theme,
    cx: &mut Context<UiApp>,
) -> impl IntoElement {
    let selected = app.page == page;
    let fg = if selected {
        theme.accent()
    } else {
        theme.fg_muted()
    };

    div()
        .id(("rail", page.slot()))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(RAIL_ITEM_WIDTH))
        .h(px(ITEM_HEIGHT))
        .rounded(px(6.0))
        .cursor_pointer()
        .when(selected, |style| style.bg(theme.accent_bg()))
        .when(!selected, |style| style.hover(|style| style.bg(hover_bg(theme))))
        .tooltip(gpuikit::elements::tooltip::tooltip(page.label()))
        .on_click(cx.listener(move |this, _, _, cx| this.handle(Msg::PageSelected(page), cx)))
        .child(icon(page_icon(page), 18.0, fg))
}

/// 导航项的图标。
///
/// 图标要能一眼看出「这页管什么」：
/// 仪表盘 = 总览（dashboard），代理配置 = 参数（齿轮），
/// 分流规则 = 双向分流（水平混合器），网络与绕过 = 分流地址（图钉），
/// 程序与日志 = 时间线输出（逆时针时钟）。
pub fn page_icon(page: Page) -> Svg {
    match page {
        Page::Dashboard => DefaultIcons::dashboard(),
        Page::Profiles => DefaultIcons::gear(),
        Page::Split => DefaultIcons::mixer_horizontal(),
        Page::Network => DefaultIcons::sewing_pin(),
        Page::Program => DefaultIcons::counter_clockwise_clock(),
    }
}

/// 版本胶囊的文字：装好了显示版本号，没装就直说。
fn version_label(snap: &Snapshot) -> String {
    snap.version
        .clone()
        .map(|v| format!("tun2proxy v{v}"))
        .unwrap_or_else(|| t("tun2proxy 未安装", "tun2proxy not installed").to_string())
}

