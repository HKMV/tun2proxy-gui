//! 整窗骨架：顶部状态栏 + 左侧导航 + 内容区。
//!
//! 视觉取向是「隧道控制台」而不是设置向导：深空蓝底、青色主色（直连）配
//! 紫色（代理）、状态光点、等宽数字。所有颜色都从 `theme.json` 的语义 token
//! 取，浅色 / 深色各一套，切换外观时不用改一行代码。
//!
//! 组件只跑一次：读信号的地方一律写成闭包（`.bg(move || …)`）或包进
//! `dynamic(move || …)`，否则切页、状态变化时界面不会跟着变。

use nsh_base::theme;
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::current_window;
use nsh_core::prelude::*;

use crate::core::config::{Palette, ThemeSetting};
use crate::core::APP_VERSION;
use crate::core::i18n::t;
use crate::core::paths;

use super::live::{Live, rate_text};
use super::page::Page;
use super::{palette, state};

/// 顶栏高度。
const TOP_BAR_HEIGHT: f32 = 52.0;
/// 左侧导航宽度。
const RAIL_WIDTH: f32 = 196.0;
/// 导航行高。
const NAV_ROW_HEIGHT: f32 = 36.0;

/// 窗口右上角留给系统最小化 / 最大化 / 关闭三个按钮的宽度。
///
/// 自绘顶栏之后这三个按钮仍由系统画在窗口右上角：每个 46dp，一共三个
/// （`nsh-platform-windows` 的 `BUTTON_WIDTH` × `caption_buttons().len()`），
/// 顶栏右侧要空出这段，否则配色选择器会被压在按钮底下。
const CAPTION_BUTTONS_WIDTH: f32 = 138.0;

/// 整窗骨架。
pub fn shell() -> impl IntoElement {
    let page = signal(Page::Dashboard);
    let live = signal(Live::now());
    // 配色是整窗共用的一个值：顶栏上的选择器改它，`palette::apply` 顺手把
    // 新的 token 换上去，整窗按 token 重绘。
    let palette = signal(palette::initial(
        state::config().map(|config| config.palette).unwrap_or_default(),
    ));
    // 明暗（浅色 / 深色 / 跟随系统）与配色各管一层，互不影响。
    let theme_setting = signal(palette::initial_setting(
        state::config().map(|config| config.theme).unwrap_or_default(),
    ));
    // 根组件只跑一次，窗口属性也在这里设一次。
    hide_system_title_bar();
    view()
        .size_full()
        .flex_col()
        .bg(move || theme().background)
        .child(top_bar(live, palette, theme_setting))
        .child(
            view()
                .flex_1()
                .flex_row()
                .child(rail(page, live))
                .child(content(page, live)),
        )
}

/// 去掉系统标题栏，让顶栏从窗口最上沿开始画。
///
/// 系统标题栏是浅灰蓝的，压在深色控制台上非常突兀；隐藏之后窗口的客户区
/// 一直顶到边上，最小化 / 最大化 / 关闭三个按钮仍由系统画在右上角，拖动窗口
/// 靠顶栏上的 `window_drag`。
fn hide_system_title_bar() {
    let mut title_bar = TitleBarOptions::default();
    title_bar.style = TitleBarStyle::Hidden;
    // 高度和顶栏对齐：系统按这个高度画右上角那三个按钮。
    title_bar.height = Some(px(TOP_BAR_HEIGHT));
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(title_bar);
    current_window().set(attributes);
}

/// 顶栏：品牌、状态灯、明暗切换、配色选择器、版本号。
fn top_bar(
    live: Signal<Live>,
    palette: Signal<Palette>,
    theme_setting: Signal<ThemeSetting>,
) -> impl IntoElement {
    view()
        .h(px(TOP_BAR_HEIGHT))
        .flex_row()
        .items_center()
        .gap_3()
        .pl_4()
        .pr(px(CAPTION_BUTTONS_WIDTH))
        .shadow_sm()
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_color(move || theme().sidebar_border)
        .child(brand_mark())
        .child(
            view()
                .flex_col()
                .gap_1()
                .child(
                    text("Tun2Proxy")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(t("隧道控制台", "Tunnel console"))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        // 顶栏就是标题栏：系统标题栏已经藏起来，中间这段空白负责拖窗口。
        //
        // `window_drag` 只加在空白占位上，不加整条顶栏：整条加会把状态胶囊、
        // 明暗开关、色块一起变成拖动区，点上去只剩拖窗，什么都点不动。
        .child(view().flex_1().window_drag(true))
        .child(status_pill(live))
        .child(appearance_segments(theme_setting))
        .child(palette_picker(palette))
        .child(
            view()
                .h(px(24.0))
                .px_2()
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(move || theme().muted)
                .border_1()
                .border_color(move || theme().border)
                .child(
                    text(move || format!("v{APP_VERSION}"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

/// 品牌方块：青色到紫色的渐变，控制台里唯一的「装饰」。
fn brand_mark() -> impl IntoElement {
    view()
        .w(px(26.0))
        .h(px(26.0))
        .rounded_md()
        .items_center()
        .justify_center()
        .bg_linear_to_br()
        .gradient_from(theme().chart_1)
        .gradient_to(theme().chart_2)
        // 品牌方块用自己的颜色发光：整个界面里最"抬头"的一块。
        .shadow_md()
        .shadow_color(theme().chart_1)
        .child(
            text("T")
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// 状态胶囊：光点 + 一句话。
fn status_pill(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let active = snapshot.is_active();
        // 未启动时让光点呼吸：nsh 的 `animate-pulse`，不用自己起定时器。
        let light = view()
            .w(px(6.0))
            .h(px(6.0))
            .rounded_full()
            .bg(if active { ok_color() } else { warn_color() });
        let light = if active { light } else { light.animate_pulse() };
        view()
            .h(px(26.0))
            .flex_row()
            .items_center()
            .gap_2()
            .pl_2()
            .pr_3()
            .rounded_full()
            .shadow_xs()
            .bg(move || theme().muted)
            .border_1()
            .border_color(move || theme().border)
            .child(light)
            .child(
                text(snapshot.status_label())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().secondary_foreground),
            )
    })
}

/// 明暗切换：跟随系统 / 浅色 / 深色，三段平铺，点哪个是哪个。
///
/// 做成三段而不是「点一下循环」：切换是可逆的状态，按钮上看得见当前在哪一档，
/// 也不用猜下一次会跳到哪儿。
fn appearance_segments(theme_setting: Signal<ThemeSetting>) -> impl IntoElement {
    view()
        .h(px(26.0))
        .flex_row()
        .items_center()
        .gap_1()
        .p_1()
        .rounded_full()
        .shadow_xs()
        .bg(move || theme().muted)
        .border_1()
        .border_color(move || theme().border)
        .child(segment(ThemeSetting::Auto, theme_setting))
        .child(segment(ThemeSetting::Light, theme_setting))
        .child(segment(ThemeSetting::Dark, theme_setting))
}

/// 明暗切换里的一格。
///
/// 选中态用主色实心块：深色底色下「比旁边稍亮一点」几乎看不出来，实心色块
/// 一眼就能认出当前在哪一档。
fn segment(setting: ThemeSetting, theme_setting: Signal<ThemeSetting>) -> impl IntoElement {
    dynamic(move || {
        let chosen = theme_setting.get() == setting;
        view()
            .h(px(18.0))
            .px_2()
            .rounded_full()
            .items_center()
            .justify_center()
            .cursor_pointer()
            // 选中的那一格自带主色光晕，像从胶囊里浮起来。
            .when(chosen, |this| {
                this.shadow_xs().shadow_color(theme().primary)
            })
            // 没选中的格子悬停时先浮出一层底色，按下去再缩一点点。
            .when(!chosen, |this| this.hover(|state| state.bg(theme().accent)))
            .transition_colors()
            .transition_transform()
            .duration(140.0)
            .ease_out()
            .active(|this| this.scale(0.95))
            .bg(move || {
                if theme_setting.get() == setting {
                    theme().primary
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .on_press(move || {
                if theme_setting.get() == setting {
                    return;
                }
                theme_setting.set(setting);
                palette::apply_appearance(setting);
            })
            .child(
                text(setting.label())
                    .text_xs()
                    .font_weight(if chosen {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(move || {
                        if theme_setting.get() == setting {
                            theme().primary_foreground
                        } else {
                            theme().muted_foreground
                        }
                    }),
            )
    })
}

/// 配色选择器：当前配色名 + 四个色块，点一下就整套换掉。
///
/// 没用下拉框：nsh 的 `select` 是无样式的基础件，自己拼一个下拉还要处理
/// 弹出层和键盘；四套配色本来就是常量，一排色块更直接——看见颜色就点，
/// 也不用先读名字。
fn palette_picker(palette: Signal<Palette>) -> impl IntoElement {
    view()
        .h(px(26.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded_full()
        .shadow_xs()
        .bg(move || theme().muted)
        .border_1()
        .border_color(move || theme().border)
        .child(
            text(move || palette.get().label())
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(swatch(Palette::ALL[0], palette))
        .child(swatch(Palette::ALL[1], palette))
        .child(swatch(Palette::ALL[2], palette))
        .child(swatch(Palette::ALL[3], palette))
}

/// 一个配色色块：选中的加一圈前景色描边，未选中的描边透明。
///
/// 描边用前景色而不是主色：描边和色块本身经常是同一个颜色（极光紫的主色就是
/// 紫色，正好压在紫色块上），换成前景色才有对比。
///
/// 颜色画在**这个**节点上，不在里面再套一个带底色的小方块：nsh 的点击命中
/// 落在最上层那个有底色的节点上，套一层子方块会把点击吃掉，只有点到描边
/// 才响应。
fn swatch(item: Palette, palette: Signal<Palette>) -> impl IntoElement {
    dynamic(move || {
        view()
            .w(px(16.0))
            .h(px(16.0))
            .rounded_sm()
            .bg(palette::swatch(item))
            .shadow_2xs()
            // 选中的那块用自己的颜色发光，一眼看出当前配色。
            .when(palette.get() == item, |this| {
                this.shadow_sm().shadow_color(palette::swatch(item))
            })
            .border_2()
            .border_color(move || {
                if palette.get() == item {
                    theme().foreground
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .cursor_pointer()
            // 悬停放大 + 自己的颜色发光，按下缩回去：手指能摸到的反馈。
            // 140ms + ease-out：够快，跟得上手；有收尾，不像硬切。
            .transition_transform()
            .transition_shadow()
            .duration(140.0)
            .ease_out()
            .hover(|this| {
                this.scale(1.18)
                    .shadow_sm()
                    .shadow_color(palette::swatch(item))
            })
            .active(|this| this.scale(0.9))
            .on_press(move || {
                if palette.get() == item {
                    return;
                }
                palette.set(item);
                palette::apply(item);
            })
    })
}

/// 左侧导航 + 底部的内核版本。
fn rail(page: Signal<Page>, live: Signal<Live>) -> impl IntoElement {
    view()
        .w(px(RAIL_WIDTH))
        .h_full()
        .flex_col()
        .gap_1()
        .py_4()
        .px_3()
        .shadow_sm()
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_color(move || theme().sidebar_border)
        .child(
            view().px_3().pb_2().child(
                text(t("导航", "NAVIGATION"))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().muted_foreground),
            ),
        )
        .child(
            Page::ALL
                .iter()
                .map(move |page_item| nav_item(*page_item, page)),
        )
        .child(view().flex_1())
        .child(rail_footer(live))
}

/// 一个导航项：选中时左侧亮起一条竖线。
fn nav_item(page_item: Page, page: Signal<Page>) -> impl IntoElement {
    dynamic(move || {
        let selected = page.get() == page_item;
        let bar = view().w(px(3.0)).h(px(16.0)).rounded_full();
        let bar = if selected {
            bar.bg(theme().primary)
        } else {
            bar.bg(theme().sidebar)
        };
        let row = view()
            .h(px(NAV_ROW_HEIGHT))
            .px_3()
            .rounded_md()
            .flex_row()
            .items_center()
            .gap_3()
            .cursor_pointer()
            // 悬停时整行往右挪一格并浮起底色，按下再沉一下。
            .transition_colors()
            .transition_transform()
            .duration(140.0)
            .ease_out()
            .hover(|state| state.bg(theme().sidebar_accent).translate_x(px(2.0)))
            .active(|state| state.translate_y(px(1.0)))
            .on_press(move || page.set(page_item));
        let row = if selected {
            row.bg(theme().sidebar_accent).shadow_xs()
        } else {
            row
        };
        row.child(bar).child(
            text(page_item.label())
                .text_sm()
                .font_weight(if selected {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(move || {
                    if page.get() == page_item {
                        theme().sidebar_accent_foreground
                    } else {
                        theme().sidebar_foreground
                    }
                }),
        )
    })
}

/// 导航底部：tun2proxy 内核版本，一直看得见。
fn rail_footer(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        view()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_lg()
            .shadow_md()
            .transition_shadow()
            .duration(200.0)
            .ease_out()
            .hover(|this| this.shadow_lg())
            .bg(move || theme().muted)
            .border_1()
            .border_color(move || theme().border)
            .child(
                text(t("隧道内核", "Tunnel core"))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                text(snapshot.binary_label())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .tabular_nums()
                    .text_color(move || theme().secondary_foreground),
            )
    })
}

/// 内容区：页面标题 + 状态卡 + 两张信息卡。
fn content(page: Signal<Page>, live: Signal<Live>) -> impl IntoElement {
    view()
        .flex_1()
        .h_full()
        .flex_col()
        .p_6()
        .gap_5()
        .child(header(page, live))
        .child(hero(live))
        .child(
            view()
                .flex_row()
                .gap_4()
                .child(split_card())
                .child(env_card()),
        )
}

/// 页面标题 + 右侧一句实时读数。
fn header(page: Signal<Page>, live: Signal<Live>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(
                    text(move || page.get().label())
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(move || page.get().subtitle())
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            dynamic(move || {
                let snapshot = live.get();
                text(move || format!("{} · {}", snapshot.status_label(), live.get().binary_label()))
                    .text_xs()
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground)
            }),
        )
}

/// 状态卡：顶上一条霓虹色带 + 大号状态 + 两路当前速率。
fn hero(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let active = snapshot.is_active();
        view()
            .w_full()
            .flex_col()
            .rounded_xl()
            .overflow_hidden()
            // 整窗最重的一块：主色光晕 + 大阴影，托起"当前状态"。
            .shadow_lg()
            .shadow_color(theme().chart_1)
            .transition_shadow()
            .duration(240.0)
            .ease_out()
            .hover(|this| this.shadow_xl().shadow_color(theme().chart_1))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().card)
            .child(
                // 状态色带：直连（青）→ 代理（紫），整个界面的「信号灯」。
                view()
                    .w_full()
                    .h(px(3.0))
                    .bg_linear_to_r()
                    .gradient_from(theme().chart_1)
                    .gradient_to(theme().chart_2),
            )
            .child(
                view()
                    .w_full()
                    .flex_row()
                    .items_center()
                    .gap_6()
                    .p_6()
                    .child(
                        view()
                            .flex_col()
                            .gap_2()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(dot(
                                        if active { ok_color() } else { warn_color() },
                                        10.0,
                                        !active,
                                    ))
                                    .child(
                                        text(t("隧道状态", "TUNNEL STATUS"))
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(
                                text(snapshot.status_label())
                                    .text_3xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text(match snapshot.status_detail() {
                                    Some(detail) => detail,
                                    None => t(
                                        "启动后这里显示直连与代理的实时速率。",
                                        "Rates for direct and proxied traffic appear here once the tunnel runs.",
                                    )
                                    .to_string(),
                                })
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(view().flex_1())
                    .child(metric(
                        t("直连 ↓", "Direct ↓"),
                        rate_text(snapshot.direct.1),
                        direct_color(),
                    ))
                    .child(metric(
                        t("直连 ↑", "Direct ↑"),
                        rate_text(snapshot.direct.0),
                        direct_color(),
                    ))
                    .child(metric(
                        t("代理 ↓", "Proxy ↓"),
                        rate_text(snapshot.proxy.1),
                        proxy_color(),
                    ))
                    .child(metric(
                        t("代理 ↑", "Proxy ↑"),
                        rate_text(snapshot.proxy.0),
                        proxy_color(),
                    )),
            )
    })
}

/// 一列指标：小标题 + 大数字。
fn metric(label: &'static str, value: String, accent: AdaptiveColor) -> impl IntoElement {
    view()
        .flex_col()
        .gap_1()
        .items_end()
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(value)
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .tabular_nums()
                .text_color(accent),
        )
}

/// 分流策略卡：两条路各自的颜色与含义。
fn split_card() -> impl IntoElement {
    card(
        t("分流策略", "Split routing"),
        t(
            "每个连接按规则判一次，命中就直连，其余走上游代理。",
            "Each connection is classified once: matches go direct, the rest use the upstream proxy.",
        ),
        view()
            .flex_col()
            .gap_2()
            .child(rule_row(
                t("国内 IP 与域名", "China IPs and domains"),
                t("直连", "Direct"),
                direct_color(),
            ))
            .child(rule_row(
                t("其余流量", "Everything else"),
                t("上游代理", "Upstream proxy"),
                proxy_color(),
            )),
    )
}

/// 运行环境卡：三份真实路径，启动排障时要看。
fn env_card() -> impl IntoElement {
    card(
        t("运行环境", "Environment"),
        t(
            "配置文件与日志都在这里，出问题时先看日志目录。",
            "The config and the logs live here; check the log directory first.",
        ),
        view()
            .flex_col()
            .gap_2()
            .child(path_row(t("配置", "Config"), paths::config_path()))
            .child(path_row(t("二进制", "Binaries"), paths::bin_dir()))
            .child(path_row(t("日志", "Logs"), paths::logs_dir())),
    )
}

/// 一张信息卡：标题、说明，加一块调用方拼好的内容。
fn card(title: &'static str, hint: &'static str, rows: impl IntoElement) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded_xl()
        .shadow_lg()
        // 鼠标经过时抬高一层：卡片本身不可点，但界面因此"活"着。
        .transition_shadow()
        .duration(220.0)
        .ease_out()
        .hover(|this| this.shadow_xl())
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(
            text(title)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().card_foreground),
        )
        .child(
            text(hint)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(rows)
}

/// 规则行：颜色点 + 谁 + 走向。
fn rule_row(who: &'static str, target: &'static str, accent: AdaptiveColor) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(dot(accent, 6.0, false))
        .child(
            text(who)
                .text_xs()
                .text_color(move || theme().secondary_foreground),
        )
        .child(view().flex_1())
        .child(
            text(target)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent),
        )
}

/// 路径行：左边名字，右边路径（过长截断）。
fn path_row(label: &'static str, path: std::path::PathBuf) -> impl IntoElement {
    view()
        .flex_col()
        .gap_1()
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(short_path(&path))
                .text_xs()
                .truncate()
                .text_color(move || theme().secondary_foreground),
        )
}

/// 把应用目录这段公共前缀换成 `…`。
///
/// 三条路径都在同一个目录下，原样截断会得到三行一模一样的
/// `C:\Users\…\tun2proxy-gu…`——最关键的尾巴（`config.json` / `bin` /
/// `logs`）反而被切掉了。
fn short_path(path: &std::path::Path) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    match path.strip_prefix(paths::app_dir()) {
        Ok(relative) if !relative.as_os_str().is_empty() => {
            format!("…{sep}{}", relative.display())
        }
        _ => path.display().to_string(),
    }
}

/// 一个圆点（状态灯）。
///
/// `pulse` 打开 nsh 的 `animate-pulse`：未启动这类"等你动手"的状态才呼吸，
/// 运行中常亮——一直闪会让人以为出事了。
fn dot(color: AdaptiveColor, size: f32, pulse: bool) -> impl IntoElement {
    let dot = view().w(px(size)).h(px(size)).rounded_full().bg(color);
    if pulse { dot.animate_pulse() } else { dot }
}

/// 「直连」的颜色：主题里的 `direct` token，没有就退到 `chart-1`。
fn direct_color() -> AdaptiveColor {
    theme().color("direct").unwrap_or(theme().chart_1)
}

/// 「代理」的颜色：主题里的 `proxy` token，没有就退到 `chart-2`。
fn proxy_color() -> AdaptiveColor {
    theme().color("proxy").unwrap_or(theme().chart_2)
}

/// 「正常」的颜色：`success` token，没有就退到 `chart-3`。
fn ok_color() -> AdaptiveColor {
    theme().color("success").unwrap_or(theme().chart_3)
}

/// 「需注意」的颜色：`warning` token，没有就退到 `chart-4`。
fn warn_color() -> AdaptiveColor {
    theme().color("warning").unwrap_or(theme().chart_4)
}
