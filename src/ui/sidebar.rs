//! 左侧边栏：品牌 + 导航 + 状态 + 一键启停。
//!
//! 侧边栏是整个界面唯一「常驻」的区域：切到任何一页都能看到当前状态、
//! 都能直接启动或停止，不必先回到仪表盘。信息从上到下按重要性递减排列：
//! 我是谁 → 我在哪一页 → 我现在什么状态 → 主操作。
//!
//! 两种形态，都照 WinUI 3 的 NavigationView 做：
//!
//! - **完整**（[`SIDEBAR_WIDTH`]）：图标 + 文字，选中项由框架画高亮块；
//! - **收起**（[`RAIL_WIDTH`]）：48dp 图标栏，选中项 = 40×36 圆角底
//!   + 左侧 3×16 的强调色竖条（NavigationView 的 selection indicator）。
//!
//! 右侧统一压一条 1dp 发丝线：侧边栏白、内容区灰，再加一条线，
//! 两块区域的边界才不靠「色差刚刚好」这种巧合。
//!
//! 收起只省地方，不藏功能：导航项全带 tooltip，状态点带状态文字，
//! 启停按钮也还在原位。

use crate::core::i18n::t;
use zsui::{
    button, column, navigation_item, primary_button, row, spacer, styled_text, ColorRole, Dp,
    SemanticTextStyle, TextRole, TextWeight, ThemeColorToken, ViewAlign, ViewNode, ZsIcon,
};

use super::cards::{status_color, status_label};
use super::state::{App, Msg, Page, Snapshot};
use super::widgets::{dot, hairline, pill, push, v_hairline, SPACE_S, SPACE_XL};

/// 侧边栏宽度（逻辑像素）。
///
/// 宽度由两件事撑出来，取更宽的那个：
///
/// - 品牌行：`Tun2Proxy`（BodyLarge 18 半粗，实测 110dp）+ 间距 12 + 收起按钮 32
///   = 154dp；
/// - 版本胶囊：文字改成 `Body`（14）后实测 130dp，加左右各 4dp 间隔与 4dp 内边距
///   = 146dp（12px 时是 118dp，160 的侧边栏刚好装得下，现在装不下了）。
///
/// 168 - 左右各 8dp 留白 = 152dp 可用宽，两件都装得下还各余 6dp。
///
/// ZSUI 不压缩控件（装不下就裁），这里的宽度必须大于内容的内在
/// 最小宽度，由 `ui::layout_probe` 的 `sidebar_keeps_its_width` 与
/// `no_node_exceeds_the_window_width` 守住。
pub const SIDEBAR_WIDTH: f32 = 168.0;
/// 收起后的图标栏宽度：40dp 的导航项 + 左右各 4dp 留白。
///
/// 40 是 WinUI NavigationView 收起态的项宽，48 是它的栏宽，两者别混。
pub const RAIL_WIDTH: f32 = 48.0;
/// 侧边栏内部左右留白。
const SIDEBAR_PADDING: f32 = 8.0;
/// 图标栏内部留白：48 - 40 = 8，左右各 4。
const RAIL_PADDING: f32 = 4.0;
/// 导航项 / 底部按钮的宽度：撑满可用宽度，看起来才像一条侧边栏。
const ITEM_WIDTH: f32 = SIDEBAR_WIDTH - SIDEBAR_PADDING * 2.0;
/// 收起态导航项的宽度（WinUI 的 compact 项宽 40，栏宽 48）。
const RAIL_ITEM_WIDTH: f32 = 40.0;

/// 构建整条侧边栏。
pub fn sidebar(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let pane = if app.sidebar_collapsed {
        rail(app, snap)
    } else {
        expanded(app, snap)
    };
    row([pane, v_hairline()])
        .gap(Dp::new(0.0))
        .align(ViewAlign::Stretch)
        .flex(0.0)
}

/// 完整形态：图标 + 文字。
fn expanded(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let (status_text, color) = status_label(&snap.status);

    // 品牌区：一行「程序名 + 收起按钮」，版本胶囊在下面一行。
    //
    // 程序名用 BodyLarge（18px）+ 半粗，实测 110dp；收起按钮固定 40dp（原生
    // 图标框宽度，图标才会在框里居中），合计 150 ≤ 152 可用宽（[`ITEM_WIDTH`]），
    // 松紧刚好。再大就装不下：Subtitle 是 123dp，加上 40dp 按钮就超了，
    // ZSUI 不压缩控件，会把侧边栏撑宽（`sidebar_keeps_its_width` 会失败）。
    //
    // 胶囊单独一行而不是挤在名字右边：版本号是可变长文字（「tun2proxy 未安装」
    // 比「tun2proxy v0.8.6」长），挤在名字那一行会把宽度变成一个会变的量。
    let brand = column([
        row([
            styled_text(
                "Tun2Proxy",
                SemanticTextStyle {
                    weight: TextWeight::Semibold,
                    ..SemanticTextStyle::for_role(TextRole::BodyLarge)
                },
            ),
            push(),
            toggle(app),
        ])
        .align(ViewAlign::Center)
        .width(Dp::new(ITEM_WIDTH)),
        pill(version_label(snap), ColorRole::Accent).align(ViewAlign::Start),
    ])
    .gap(Dp::new(SPACE_S))
    .align(ViewAlign::Start);

    // 导航项：选中的那页由 ZSUI 画出 NavigationView 的高亮块。
    let nav = column(Page::ALL.into_iter().map(|page| {
        navigation_item(page.label(), page.icon(), app.page == page)
            .width(Dp::new(ITEM_WIDTH))
            .on_click(Msg::PageSelected(page))
    }))
    .gap(Dp::new(2.0))
    .align(ViewAlign::Stretch);
    // 状态：一个色点 + 一句话，运行中一眼能看见。
    //
    // 不再单独占一行写「服务状态」这个标题：一个「● 未运行」自解释，
    // 多一行小标题只会把底部的操作区推得更高、看起来更松。
    let status = row([
        // 实心圆点，不用 `badge(Dot)`：那个胶囊有底色（Control=白），
        // 落在同样浅的侧边栏底色上等于只剩一个白点。
        dot(status_color(&snap.status)),
        styled_text(
            status_text,
            SemanticTextStyle {
                color,
                ..SemanticTextStyle::for_role(TextRole::Body)
            },
        ),
    ])
    .gap(Dp::new(SPACE_S))
    .align(ViewAlign::Center)
    .width(Dp::new(ITEM_WIDTH));

    // 主操作：运行中用普通按钮（停止是「收尾」动作），未运行用主按钮强调。
    let action = if snap.status.is_active() {
        button(t("停止", "Stop")).on_click(Msg::Stop)
    } else {
        primary_button(t("启动", "Start")).on_click(Msg::Start)
    };

    column([
        brand,
        spacer().height(Dp::new(SPACE_XL)),
        nav,
        spacer().flex(1.0),
        // 底部操作区：一条发丝线把「导航」和「状态 + 启停」分开，
        // 否则状态那一行悬在空白里，读起来像是没画完。
        hairline(),
        status,
        action.width(Dp::new(ITEM_WIDTH)),
    ])
    .width(Dp::new(SIDEBAR_WIDTH))
    .padding(Dp::new(SIDEBAR_PADDING))
    .gap(Dp::new(SPACE_S))
    .align(ViewAlign::Stretch)
    // 底色用 Surface 而不是白色：zsui 的 Windows 浅色主题把导航项选中底
    // （`ColorRole::Control`）画成纯白，在白底侧边栏上等于看不见，只剩左侧
    // 那条 3dp 竖条。底色降成灰，选中项就自然浮出一块白胶囊（WinUI 的做法）。
    // 侧边栏与内容区靠右侧那道发丝描边分开。
    .bg(ThemeColorToken::Surface)
}

/// 收起形态：只剩一列图标。
///
/// 每个导航项照 WinUI 的 compact 版式：40×36 的项，选中时铺一层浅底
/// （`Control`）并在最左画一条 3×16 的强调色竖条。未选中时竖条不画，
/// 但仍占着那 3dp——否则选中时图标会横跳 1.5dp。
fn rail(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    let (status_text, _color) = status_label(&snap.status);

    let nav = column(Page::ALL.into_iter().map(|page| rail_item(app, page)))
        .gap(Dp::new(2.0))
        .align(ViewAlign::Center);

    // 状态只剩一个色点：文字挤不进 48dp，颜色 + tooltip 足够。
    let status = column([dot(status_color(&snap.status))])
        .tooltip(status_text)
        .align(ViewAlign::Center);

    // 启停按钮：和导航项同一套渲染（`navigation_item`）。
    //
    // 试过三种写法，只有这一种在图标栏里站得住：
    // - `primary_button(t("启", "Go"))`：按钮有平台最小宽度（约 120dp），会把 48dp
    //   的图标栏撑成一条胖栏；
    // - `icon_button`：能压到 40dp，但它画的是「Control 底 + 1dp 描边」的
    //   方框，和旁边无边框的导航项摆在一起就是两种控件。
    // 导航项样式 + 上方一条发丝线：形状与其它项一致，位置（底部、分隔线下）
    // 与 tooltip 说明它是动作而不是第六个页面。图标集里没有播放三角，
    // 用纸飞机（Send）表示「把流量发出去」，方块（Stop）表示停。
    let (action_label, action_icon, action_msg) = if snap.status.is_active() {
        (t("停止", "Stop"), ZsIcon::Stop, Msg::Stop)
    } else {
        (t("启动", "Start"), ZsIcon::Send, Msg::Start)
    };
    let action = navigation_item("", action_icon, false)
        .width(Dp::new(RAIL_ITEM_WIDTH))
        .tooltip(action_label)
        .on_click(action_msg);

    column([
        // 收起态也留一个收起按钮：它管的是侧边栏自己，两种形态下都得够得着。
        // 图标栏里它和导航项一样宽（40），对齐后是一条直线。
        row([toggle(app)]).align(ViewAlign::Center),
        nav,
        spacer().flex(1.0),
        status,
        hairline(),
        action,
    ])
    .width(Dp::new(RAIL_WIDTH))
    .padding(Dp::new(RAIL_PADDING))
    .gap(Dp::new(SPACE_S))
    .align(ViewAlign::Center)
    .bg(ThemeColorToken::Surface)
}

/// 侧边栏的收起/展开按钮。
///
/// 长在侧边栏里，而不是内容区顶栏：它管的是侧边栏，就该贴着侧边栏——
/// 跑到内容区标题旁边，看起来像是在管「当前这一页」。
///
/// 展开态它和程序名同一行、靠右（行里用 `push()` 顶开）；收起态它单独占
/// 顶行居中。两种形态用同一个图标（WinUI 的 hamburger 也这样）：图标代表
/// 「侧边栏」，不是「收起」或「展开」中的某一个方向，所以不需要换图标。
pub fn toggle(app: &App) -> ViewNode<Msg> {
    let label = if app.sidebar_collapsed {
        t("展开侧边栏", "Expand sidebar")
    } else {
        t("收起侧边栏", "Collapse sidebar")
    };
    // 用 navigation_item 而不是 icon_button：icon_button 会画出「Control 底 + 1dp
    // 描边」的方框，和旁边无边框的导航项摆在一起像两种控件。导航项样式与它们同源。
    //
    // 宽度固定 40（`RAIL_ITEM_WIDTH`），不要为了塞下更大的程序名去压窄它：
    // Windows 导航项内部是「40dp 图标框 + 16dp 字形」，字形在 40dp 里居中。
    // 把控件压到 32dp 并不会把图标框缩小，而是把 40dp 的框裁掉右边 8dp——
    // 字形就偏到中心右边去了（看起来就是「图标没在按钮里居中」）。
    // 两种形态都用 40：展开态与下面的导航项、收起态与图标栏都是一条直线。
    navigation_item("", ZsIcon::Sidebar, false)
        .width(Dp::new(RAIL_ITEM_WIDTH))
        .tooltip(label)
        .on_click(Msg::ToggleSidebar)
}

/// 收起态的一个导航项。
///
/// 用 `navigation_item` 而不是 `icon_button`：前者就是 NavigationView 的项
/// （选中铺浅底 + 左侧 3×16 强调色竖条 + 40dp 图标框），和展开态是同一套
/// 渲染；后者是「带边框的方按钮」，一排方框叠在侧边栏里像工具栏，不像导航。
///
/// 标签传空字符串：图标栏只有 40dp，文字画上去会溢出到内容区。名字改由
/// tooltip 提供。
fn rail_item(app: &App, page: Page) -> ViewNode<Msg> {
    navigation_item("", page.icon(), app.page == page)
        .width(Dp::new(RAIL_ITEM_WIDTH))
        .tooltip(page.label())
        .on_click(Msg::PageSelected(page))
}

/// 版本胶囊的文字：装好了显示版本号，没装就直说。
fn version_label(snap: &Snapshot) -> String {
    snap.version
        .clone()
        .map(|v| format!("tun2proxy v{v}"))
        .unwrap_or_else(|| t("tun2proxy 未安装", "tun2proxy not installed").to_string())
}
