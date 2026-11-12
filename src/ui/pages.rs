//! 右侧内容区：细顶栏 + 可滚动的分组列表。
//!
//! 视觉分三层（从下往上）：
//!
//! 1. **页面底色** `bg`——内容区整块铺底，才看得出「内容区」在哪；
//! 2. **细顶栏**：单行标题 + 右侧一句说明 + 一条 1dp 发丝线，高约 45dp，
//!    只用来回答「我在哪一页」，不占高度；
//! 3. **卡片**：`surface` + 圆角，比底色亮一档，自然浮起来。
//!
//! 这里没有一处写死整窗尺寸：内容区 `flex_1`，卡片列表 `w_full`，
//! 卡片宽度跟着窗口走，文字按新宽度重新换行。
//!
//! 滚动用 gpui 原生的 `overflow_y_scroll` + 每页一个 [`ScrollHandle`]：
//! 句柄存在 [`App`] 里，切页时各自的偏移量互不影响（共用一个句柄的话，
//! 「程序与日志」一打开就是半截，第一行被上一页的偏移量顶掉了）。

use gpui::{AnyElement, Context, IntoElement, div, px, prelude::*};
use gpuikit::DefaultIcons;
use gpuikit::theme::{Theme, Themeable};

use crate::core::i18n::{t, tf};

use super::cards::{
    about_group, appearance_group, binary_group, bypass_group, editing_profile,
    log_group, log_preview_group, network_group, profile_edit_group, profile_list_group,
    profile_picker_group, proxy_group, runtime_group, split_custom_group, split_decisions_group,
    split_detail_group, split_hero_group, split_lists_group, split_outlet_group,
    split_update_group, stat_group, status_tips, trend_group, udpgw_group, window_group,
};
use super::state::{App, Msg, Snapshot};
use super::theme::TEXT_BODY;
use super::widgets::{
    CONTENT_PADDING, SPACE_M, SPACE_S, SPACE_XL, ScrollBarKey, caption_nowrap, hairline, icon,
    icon_action, page_title, primary_button, scroll_bar,
};

/// 右侧内容区：细顶栏 + 提示条 + 可滚动的卡片列表。
pub fn content(app: &App, snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme.bg())
        .child(page_header(app, snap, theme, cx))
        .children(notice_bar(app, snap, theme, cx))
        .child(page_scroll_area(app, snap, theme, cx))
        .into_any_element()
}

/// 顶部提示条：保存结果、动作出错、启动被拦下、会话打满、已复制…都在这里说。
///
/// 原来这些提示只画在仪表盘的首屏卡里，可「保存到磁盘」的按钮在代理配置页
/// 的顶栏上——点完什么也不会出现，看起来就像没生效。提示条挂在内容区，
/// 任何一页做任何动作都能立刻看到结果。
///
/// 只显示一条：错误优先（出问题时不该被上一句「已保存」盖住），其次是动作
/// 回执，然后是「启动被拦下的原因」——它由后台的检测线程写进快照，在别的
/// 页面点「启动」时原来同样毫无反馈。最后是「会话打满」：它不是错误（隧道
/// 还在跑），但优先级得低于上面几条，否则用户刚做的动作会看不到回执。
fn notice_bar(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let (text, color, glyph) = match (&app.error, &app.notice, &snap.blocked) {
        (Some(error), _, _) => (
            error.clone(),
            theme.danger(),
            DefaultIcons::info_circled(),
        ),
        (None, Some(notice), _) => (
            notice.clone(),
            theme.success(),
            DefaultIcons::info_circled(),
        ),
        (None, None, Some(blocked)) => (
            blocked.clone(),
            theme.warning(),
            DefaultIcons::exclamation_triangle(),
        ),
        (None, None, None) => {
            if !snap.sessions_full {
                return None;
            }
            // 数字读的是「当前配置」，不是打满那一刻的值：提示的用途是让人去
            // 调大，显示他现在填的那个数比显示历史更贴题。
            let limit = app.profile().max_sessions.to_string();
            (
                tf(
                    t(
                        "并发会话数已达到上限（{limit}），新连接被丢弃，已有连接不受影响。到「网络与绕过 → 最大会话数」调大即可。",
                        "The concurrent session limit ({limit}) has been reached: new connections are dropped while existing ones keep running. Raise “Max sessions” under Network.",
                    ),
                    &[("limit", &limit)],
                ),
                theme.warning(),
                DefaultIcons::exclamation_triangle(),
            )
        }
    };

    Some(
        div()
            .flex()
            .flex_none()
            .w_full()
            .px(px(CONTENT_PADDING))
            .pb(px(SPACE_M))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(SPACE_S))
                    .w_full()
                    .p(px(SPACE_M))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(color.opacity(0.35))
                    .bg(color.opacity(0.10))
                    .child(
                        div()
                            .flex_none()
                            .child(icon(glyph, TEXT_BODY, color)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(TEXT_BODY))
                            .text_color(theme.fg())
                            .child(text),
                    )
                    .child(icon_action(
                        "dismiss-notice",
                        t("关闭提示", "Dismiss"),
                        DefaultIcons::cross_1(),
                        cx.listener(|this, _, _, cx| this.handle(Msg::DismissNotice, cx)),
                    )),
            )
            .into_any_element(),
    )
}

/// 细顶栏：单行标题 + 右侧说明 + 底部发丝线。
///
/// 仪表盘右侧显示的是「隧道开着没有」（状态胶囊 + 一句短语），其余页面显示
/// 这一页的说明：以前仪表盘用一整张首屏卡讲这件事，占掉半屏还和侧栏底部的
/// 状态重复；现在压成顶栏上的一行。
fn page_header(
    app: &App,
    snap: &Snapshot,
    theme: &Theme,
    cx: &mut Context<App>,
) -> AnyElement {
    let editing = editing_profile(app);

    let mut row = div()
        .flex()
        .items_center()
        .gap(px(SPACE_M))
        // 左右留白和内容区取同一个数（[`CONTENT_PADDING`]）：用 `SPACE_M`
        // 的话标题比下面的卡片左缩 4dp，整页看起来像没对齐。
        .px(px(CONTENT_PADDING))
        .py(px(SPACE_M));

    // 顶栏左侧：返回 → 标题。
    //
    // 返回按钮只属于「配置详情」这一层，所以紧贴标题左边。收起侧边栏的按钮
    // 不在这里——它长在侧边栏自己的顶行（见 `sidebar::toggle`）。
    if editing {
        row = row.child(icon_action(
            "profile-back",
            t("返回配置列表", "Back to the list"),
            DefaultIcons::chevron_left(),
            cx.listener(|this, _, _, cx| this.handle(Msg::ProfileEditDone, cx)),
        ));
    }
    row = row.child(page_title(
        if editing {
            t("编辑配置", "Edit profile")
        } else {
            app.page.label()
        },
        theme,
    ));

    // 右侧：编辑态给「保存到磁盘」（这一页的主操作，固定在右上角就不会随着
    // 内容滚出屏幕）；仪表盘给运行状态；其余页面给一句说明。
    row = row.child(div().flex_1());
    row = row.child(if editing {
        primary_button(
            "profile-save-header",
            t("保存到磁盘", "Save to disk"),
            theme,
            cx.listener(|this, _, _, cx| this.handle(Msg::SaveConfig, cx)),
        )
        .into_any_element()
    } else if matches!(app.page, super::state::Page::Dashboard) {
        super::cards::status_strip(snap, theme)
    } else {
        caption_nowrap(app.page.subtitle(), theme).into_any_element()
    });

    div()
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .child(row)
        .child(hairline(theme))
        .into_any_element()
}

/// 可滚动的内容区。
///
/// 留白（[`CONTENT_PADDING`]）放在**内层**而不是滚动容器上：滚动条画在滚动
/// 视口的右边缘内侧，留白若算在容器上，滚动条就会落在留白中间，右边多出
/// 一条空带子。把留白给内层，滚动条就贴着窗口右边，卡片离它 16dp。
fn page_scroll_area(app: &App, snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> AnyElement {
    let slot = app.page.slot();
    let handle = app.page_scroll[slot].clone();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            div()
                .id(("page-scroll", slot))
                .flex()
                .flex_col()
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&handle)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .gap(px(SPACE_XL))
                        .p(px(CONTENT_PADDING))
                        .children(page_body(app, snap, theme, cx)),
                ),
        )
        .child(scroll_bar(
            ("page-scroll-bar", slot),
            ScrollBarKey::Page(slot),
            &handle,
            theme,
            &app.scroll_drag,
        ))
        .into_any_element()
}

/// 每一页的卡片组合。
///
/// 返回的是「按阅读顺序排列的卡片」，横向自适应由父级负责。
fn page_body(app: &App, snap: &Snapshot, theme: &Theme, cx: &mut Context<App>) -> Vec<AnyElement> {
    match app.page {
        // 仪表盘：状态提示（只在出事时出现）+ 实时流量与连接去向 + 选配置 +
        // 三个关键参数 + 最近日志。参数编辑搬到「代理配置」页：仪表盘只回答
        // 「现在什么状态、用哪套配置」。
        //
        // 两张折线图放在同一张卡里上下排（流量在上、去向在下）：它们看的是同一段
        // 时间，分开摆的时候读者得自己在两个位置之间来回对时间轴。
        super::state::Page::Dashboard => {
            let mut rows = status_tips(snap, theme, cx);
            rows.push(trend_group(app, snap, theme, cx));
            rows.push(profile_picker_group(app, theme, cx));
            rows.push(stat_group(app, theme));
            rows.push(log_preview_group(app, snap, theme, cx));
            rows
        }
        // 代理：默认是配置列表（选中即切换），点铅笔才进单套配置的详情。
        super::state::Page::Profiles if app.profile_editing => vec![
            profile_edit_group(app, theme, cx),
            proxy_group(app, snap, theme, cx),
            bypass_group(app, theme, cx),
        ],
        super::state::Page::Profiles => vec![profile_list_group(app, theme, cx)],
        // 分流：总开关 / 状态 → 清单更新 → 最近判定 → 三组清单 → 自定义规则
        // → 规则明细 → 出口。
        //
        // 「清单更新」紧跟「开关与状态」：开关决定要不要分流，清单决定拿什么
        // 判断，两者是同一件事的两半——都属于「现在的判定依据对不对」，所以
        // 排在「最近实际发生了什么」前面。计数与趋势在仪表盘（连接去向卡），
        // 这一页不再重复摆一遍数字。
        super::state::Page::Split => vec![
            split_hero_group(app, snap, theme, cx),
            split_update_group(app, snap, theme, cx),
            split_decisions_group(app, snap, theme, cx),
            split_lists_group(app, theme, cx),
            split_custom_group(app, theme, cx),
            split_detail_group(app, theme, cx),
            split_outlet_group(app, theme, cx),
        ],
        // 网络：网卡 / 路由参数、UDP 转发与绕过规则。
        super::state::Page::Network => vec![
            network_group(app, theme, cx),
            udpgw_group(app, theme, cx),
            bypass_group(app, theme, cx),
        ],
        // 程序：二进制版本 / 更新 + 运行方式 + 日志（含日志级别）。
        super::state::Page::Program => vec![
            binary_group(snap, theme, cx),
            runtime_group(app, theme, cx),
            window_group(app, theme, cx),
            appearance_group(app, theme),
            log_group(app, snap, theme, cx),
            about_group(theme),
        ],
    }
}
