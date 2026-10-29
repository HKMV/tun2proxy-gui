//! 右侧内容区：细顶栏 + 浮动卡片列表。
//!
//! 视觉分三层（从下往上）：
//!
//! 1. **页面底色** `Surface`（浅灰）——内容区整块铺灰，才看得出「内容区」在哪；
//! 2. **细顶栏**：单行标题 + 右侧一句说明 + 一条 1dp 发丝线，高约 45dp，
//!    只用来回答「我在哪一页」，不占高度；
//! 3. **卡片**：`SurfaceRaised`（白）+ 圆角，落在灰底上自然浮起来。
//!
//! 这里没有一处写死像素宽度或高度：
//!
//! - 内容区 `flex(1.0)`：窗口变宽，内容区跟着变宽（侧边栏是固定宽度）；
//! - 卡片列表 `flex(1.0)`：占满顶栏剩下的全部高度；
//! - 卡片 `align(Stretch)`：卡片宽度跟着内容区走，文字按新宽度重新换行。
//!
//! 滚动的实现用 `auto_scroll_y()` 而不是 `scroll()`：
//! 前者把滚动位置存在节点的「自适应」字段里，重建视图时按 id 恢复；
//! 后者把 offset 写在 `Scroll` 节点的数据里，每次重建都从 0 开始——
//! 日志一刷新就会跳回顶部。

use crate::core::i18n::t;
use zsui::{
    column, primary_button, row, spacer, Dp, ThemeColorToken, ViewAlign, ViewNode, WidgetId, ZsIcon,
};

use super::cards::{
    about_group, binary_group, bypass_group, hero_card, language_group, log_group,
    log_preview_group, network_group, profile_edit_group, profile_list_group, profile_picker_group,
    proxy_group, runtime_group, stat_group, udpgw_group, window_group,
};
use super::state::{App, Msg, Page, Snapshot};
use super::widgets::{
    caption_nowrap, hairline, icon_action, page_title, push, CONTENT_PADDING, CONTENT_SCROLL_ID,
    SPACE_M, SPACE_S, SPACE_XL,
};

/// 右侧内容区：细顶栏 + 可滚动的卡片列表。
pub fn content(app: &App, snap: &Snapshot) -> ViewNode<Msg> {
    column([
        page_header(app),
        // 卡片列表撑满剩余高度；内容超出时才出现滚动条。
        //
        // 留白（[CONTENT_PADDING]）放在**内层**而不是滚动容器上：
        // 滚动条是悬浮式的，画在滚动视口的右边缘内侧；如果留白算在容器上，
        // 滚动条就会落在留白中间，右边多出一条 20dp 的空带子。
        // 把留白给内层，滚动条就贴着窗口右边，卡片离它 20dp——和左边对称。
        column([column(page_body(app, snap))
            .gap(Dp::new(SPACE_XL))
            .padding(Dp::new(CONTENT_PADDING))
            .align(ViewAlign::Stretch)])
        .id(content_scroll_id(app.page))
        .auto_scroll_y()
        .align(ViewAlign::Stretch)
        .flex(1.0),
    ])
    .gap(Dp::new(0.0))
    .align(ViewAlign::Stretch)
    .flex(1.0)
    // 整块内容区铺灰：卡片是白的，白卡片落在灰底上才有「浮起来」的层次。
    .bg(ThemeColorToken::Surface)
}

/// 细顶栏：单行标题 + 右侧说明 + 底部发丝线。
///
/// 之前仪表盘把首屏卡片放在这里（卡片自带 24dp 内边距 + 大标题），
/// 顶栏被撑到一百多像素，看起来像「第二条标题栏」。现在首屏卡片回到内容流里，
/// 顶栏只留一行，用发丝线跟内容区分开——分区靠线，不靠厚度。
fn page_header(app: &App) -> ViewNode<Msg> {
    // 编辑单套配置时，顶栏右侧换成保存按钮：保存是这一页的主操作，
    // 放顶栏右上角（和大多数编辑器的「完成 / 保存」同位置）就不会随着内容
    // 滚动而跑出屏幕——之前它在页尾，字段一多就要先滚到底才能存。
    let trailing = if editing_profile(app) {
        primary_button(t("保存到磁盘", "Save to disk"))
            .tooltip(t(
                "改动会写进配置文件，重启后仍然生效",
                "Changes are written to the config file and survive a restart",
            ))
            .on_click(Msg::SaveConfig)
    } else {
        // 一行说明放在右边：和标题同一行，不额外占一行高度。
        caption_nowrap(app.page.subtitle())
    };

    // 顶栏左侧：返回 → 标题。
    //
    // 返回按钮只属于「配置详情」这一层，所以紧贴标题左边。收起侧边栏的
    // hamburger 不在这里——它长在侧边栏自己的顶行（见 `sidebar::toggle`）。
    let mut leading: Vec<ViewNode<Msg>> = Vec::new();
    if editing_profile(app) {
        leading.push(back_button());
    }
    leading.push(page_title(if editing_profile(app) {
        t("编辑配置", "Edit profile")
    } else {
        app.page.label()
    }));
    leading.push(push());
    leading.push(trailing);
    // 再给右边缘留 8dp：滚动条贴在内容区右边（实测 hit 区从 x=906 起，卡片
    // 右边缘在 900），顶栏文字直接顶到 904 会看起来贴着滚动条。补这一下，
    // 标题行和卡片的右边缘读起来才是同一条线。
    leading.push(spacer().width(Dp::new(SPACE_S)));

    column([
        row(leading)
            .gap(Dp::new(SPACE_M))
            .padding(Dp::new(SPACE_M))
            .align(ViewAlign::Center),
        hairline(),
    ])
    .gap(Dp::new(0.0))
    .align(ViewAlign::Stretch)
    .flex(0.0)
}

/// 某一页内容滚动容器的 widget id。
///
/// 每页一个 id（见 [`scroll_slot`]），探针与页面构建共用这一个函数，
/// 免得两边各写一份偏移量、改一处忘另一处。
pub fn content_scroll_id(page: Page) -> WidgetId {
    WidgetId::new(CONTENT_SCROLL_ID + scroll_slot(page))
}

/// 每页一个独立的滚动位置。
///
/// 滚动 offset 按 widget id 存在自适应字段里：所有页共用一个 id，切页时会带着
/// 上一页的偏移量进来（「程序与日志」打开就是半截，第一行被切掉）。
/// 给每页一个 id，等于给每页各留一个 offset，切过去都从头开始。
fn scroll_slot(page: Page) -> u64 {
    Page::ALL
        .iter()
        .position(|candidate| *candidate == page)
        .unwrap_or(0) as u64
}

/// 是否正在编辑某一套配置（只有配置页有「列表 / 详情」两级）。
fn editing_profile(app: &App) -> bool {
    app.page == Page::Profiles && app.profile_editing
}

/// 顶栏左侧的返回按钮：只在「配置详情」这一层出现。
///
/// 它回到的是「上一级」（配置列表），所以放在标题前，和文件管理器 / 设置的
/// 返回键同位置。之前放在编辑卡最后一行右侧，位置反了——返回是退出当前层级，
/// 不是表单的最后一个动作。
fn back_button() -> ViewNode<Msg> {
    icon_action(
        t("返回配置列表", "Back to the list"),
        ZsIcon::ChevronLeft,
        Msg::ProfileEditDone,
    )
}

/// 每一页的卡片组合。
///
/// 返回的是「按阅读顺序排列的卡片」，横向自适应由父级的 `align(Stretch)` 负责。
fn page_body(app: &App, snap: &Snapshot) -> Vec<ViewNode<Msg>> {
    match app.page {
        // 仪表盘：首屏卡片 + 选配置 + 三个关键参数 + 最近日志。
        // 参数编辑搬到「代理配置」页：仪表盘只回答「现在什么状态、用哪套配置」。
        Page::Dashboard => vec![
            hero_card(app, snap),
            profile_picker_group(app),
            stat_group(app),
            log_preview_group(snap),
        ],
        // 代理：默认是配置列表（选中即切换），点铅笔才进单套配置的详情。
        Page::Profiles if app.profile_editing => {
            vec![
                profile_edit_group(app),
                proxy_group(app, snap),
                bypass_group(app),
            ]
        }
        Page::Profiles => vec![profile_list_group(app)],
        // 网络：网卡 / 路由参数、UDP 转发与绕过规则。
        Page::Network => vec![network_group(app), udpgw_group(app), bypass_group(app)],
        // 程序：二进制版本 / 更新 + 运行方式 + 日志（含日志级别）。
        Page::Program => vec![
            binary_group(snap),
            runtime_group(app),
            window_group(app),
            language_group(app),
            log_group(app, snap),
            about_group(),
        ],
    }
}
