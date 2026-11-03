//! 左侧边栏：品牌 + 导航 + 状态 + 一键启停。
//!
//! 侧边栏是整个界面唯一「常驻」的区域：切到任何一页都能看到当前状态、
//! 都能直接启动或停止，不必先回到仪表盘。信息从上到下按重要性递减排列：
//! 我是谁 → 我在哪一页 → 我现在什么状态 → 主操作。
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
//! 启停按钮也还在原位。

use gpui::{AnyElement, Context, Div, IntoElement, Svg, div, px, rems, prelude::*};
use gpuikit::theme::{Theme, Themeable};
use gpuikit::{DefaultIcons, elements::icon_button::icon_button};

use crate::core::i18n::t;

use super::cards::status_label;
use super::state::{App as UiApp, Msg, Page, Snapshot};
use super::theme::{TEXT_BODY, TEXT_GROUP, WEIGHT_SEMIBOLD};
use super::widgets::{SPACE_S, SPACE_XL, dot, hairline, v_hairline};

/// 侧边栏宽度（逻辑像素）。
///
/// 宽度由两件事撑出来，取更宽的那个：
///
/// - 品牌行：`Tun2Proxy`（15dp 半粗）+ 间距 + 收起按钮 28dp；
/// - 版本胶囊：`tun2proxy v0.8.6` 在 14dp 下约 130dp，加左右留白约 146dp。
///
/// 176 - 左右各 8 = 160dp 可用宽，两件都装得下。
pub const SIDEBAR_WIDTH: f32 = 176.0;
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
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(TEXT_GROUP))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(theme.fg())
                        .whitespace_nowrap()
                        .child("Tun2Proxy"),
                )
                .child(toggle(app, theme, cx)),
        )
        // 版本号是小字而不是彩色胶囊：胶囊是「状态」的形状（未运行 / 运行中
        // 都是胶囊），版本号是静态信息，给它同一个形状会让侧栏顶部出现两个
        // 视觉焦点，也让人以为版本会变。
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
    // 用 gpuikit 的 `icon_button` 而不是自绘方框：它的盒模型、悬停底色、
    // 禁用态和 tooltip 都由组件自己维护，侧栏顶部那个收起按钮也是它，
    // 两处口径一致。28dp 的盒 + 16dp 的图标，正好压在状态文字那一行里。
    let action = icon_button(action_id, action_icon)
        .box_size(rems(1.75))
        .icon_size(rems(1.0))
        .tooltip(gpuikit::elements::tooltip::tooltip(action_label))
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
        // 底部：一条发丝线把「导航」和「状态 + 启停」分开，否则状态那一行
        // 悬在空白里，读起来像是没画完。
        .child(hairline(theme))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(SPACE_S))
                .px(px(4.0))
                .child(dot(color))
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
}

/// 收起形态：只剩一列图标。
fn rail(app: &UiApp, snap: &Snapshot, theme: &Theme, cx: &mut Context<UiApp>) -> Div {
    let (status_text, color) = status_label(&snap.status, theme);

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
    let action = div()
        .id("rail-action")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(RAIL_ITEM_WIDTH))
        .h(px(ITEM_HEIGHT))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme.surface_secondary()))
        .tooltip(gpuikit::elements::tooltip::tooltip(action_label))
        .on_click(cx.listener(move |this, _, _, cx| this.handle(action_msg.clone(), cx)))
        .child(
            div()
                .w(px(16.0))
                .h(px(16.0))
                .text_color(theme.fg_muted())
                .child(action_icon),
        );

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
        // 状态只剩一个色点：文字挤不进 52dp，颜色 + tooltip 足够。
        .child(
            div()
                .id("rail-status")
                .flex_none()
                .tooltip(gpuikit::elements::tooltip::tooltip(status_text))
                .child(dot(color)),
        )
        .child(hairline(theme))
        .child(action)
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
        .tooltip(gpuikit::elements::tooltip::tooltip(label))
        .on_click(cx.listener(|this, _, _, cx| this.handle(Msg::ToggleSidebar, cx)))
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
        .when(!selected, |style| {
            style.hover(|style| style.bg(theme.surface_secondary()))
        })
        .on_click(cx.listener(move |this, _, _, cx| this.handle(Msg::PageSelected(page), cx)))
        // 选中指示条：NavigationView 的那条 3×16 竖条。未选中时不画，
        // 但仍占着这 3dp——否则选中时图标会横跳 1.5dp。
        .child(
            div()
                .flex_none()
                .w(px(3.0))
                .h(px(16.0))
                .rounded(px(2.0))
                .bg(if selected { accent } else { theme.surface() }),
        )
        .child(
            div()
                .flex_none()
                .w(px(18.0))
                .h(px(18.0))
                .text_color(fg)
                .child(page_icon(page)),
        )
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
        .when(!selected, |style| {
            style.hover(|style| style.bg(theme.surface_secondary()))
        })
        .tooltip(gpuikit::elements::tooltip::tooltip(page.label()))
        .on_click(cx.listener(move |this, _, _, cx| this.handle(Msg::PageSelected(page), cx)))
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .text_color(fg)
                .child(page_icon(page)),
        )
}

/// 导航项的图标。
///
/// 图标要能一眼看出「这页管什么」：
/// 仪表盘 = 总览（dashboard），代理配置 = 参数（齿轮），
/// 网络与绕过 = 分流地址（图钉），程序与日志 = 时间线输出（逆时针时钟）。
pub fn page_icon(page: Page) -> Svg {
    match page {
        Page::Dashboard => DefaultIcons::dashboard(),
        Page::Profiles => DefaultIcons::gear(),
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
