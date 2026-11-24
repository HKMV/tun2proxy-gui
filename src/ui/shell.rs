//! 整窗骨架：顶部状态栏 + 左侧导航 + 内容区。
//!
//! 视觉取向是「隧道控制台」而不是设置向导：深空蓝底、青色主色（直连）配
//! 紫色（代理）、状态光点、等宽数字。所有颜色都从 `themes/*.json` 的语义 token
//! 取，浅色 / 深色各一套，切换外观时不用改一行代码。
//!
//! 组件只跑一次：读信号的地方一律写成闭包（`.bg(move || …)`）或包进
//! `dynamic(move || …)`，否则切页、状态变化时界面不会跟着变。
//!
//! # 这一层和内核的关系
//!
//! 界面不自己算任何东西：状态、速率、流量历史、日志全部来自 [`Live`] 快照，
//! 由一个每秒触发一次的 [`set_interval`] 重读内核；动作（启动 / 停止 / 清空
//! 日志）全部经 [`state`]，界面从不直接碰 tun2proxy 子进程。

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use nsh_base::theme;
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::{
    MenuItem, TrayClickEvent, current_window, on_quit_request, quit, set_tray, tray,
};
use nsh_core::prelude::*;

use crate::core::APP_VERSION;
use crate::core::config::{CloseAction, Palette, ThemeSetting};
use crate::core::engine::Update;
use crate::core::i18n::t;
use crate::core::logs::LogKind;
use crate::core::paths;
use crate::core::single_instance;
use crate::core::traffic::RouteTraffic;
use crate::tr_format;

use super::live::{Live, bytes_text, rate_text};
use super::page::Page;
use super::state::LogFilter;
use super::widgets::{danger_color, direct_color, info_color, ok_color, proxy_color, warn_color};
use super::{palette, profile, state};

/// 顶栏高度。
const TOP_BAR_HEIGHT: f32 = 52.0;
/// 左侧导航宽度。
const RAIL_WIDTH: f32 = 196.0;
/// 导航行高。
const NAV_ROW_HEIGHT: f32 = 36.0;
/// 流量图的高度（不含图例与刻度）。
///
/// 72dp 是「900×650 的默认窗口里，仪表盘一屏放得下」的结果：状态卡 + 这张图
/// + 下面两张信息卡加起来刚好不滚。窗口拉小的时候整页可以滚（见 [`dashboard`]）。
const CHART_HEIGHT: f32 = 72.0;

/// 窗口右上角留给系统最小化 / 最大化 / 关闭三个按钮的宽度。
///
/// 自绘顶栏之后这三个按钮仍由系统画在窗口右上角：每个 46dp，一共三个
/// （`nsh-platform-windows` 的 `BUTTON_WIDTH` × `caption_buttons().len()`），
/// 顶栏右侧要空出这段，否则配色选择器会被压在按钮底下。
const CAPTION_BUTTONS_WIDTH: f32 = 138.0;

/// 日志面板的界面状态：过滤档、是否跟随、滚动句柄。
///
/// 三样都不属于内核快照：`Live` 每秒重建，放进去会被下一帧冲掉；而切页时
/// 这一页的元素会整棵重建，放在 `program()` 里又会一换页就回到默认档。所以
/// 在 [`shell`] 里建一次，跟参数一路传下去。
#[derive(Clone, Copy)]
struct LogView {
    /// 显示哪一档。
    filter: Signal<LogFilter>,
    /// 新日志到了要不要自动滚到底。
    follow: Signal<bool>,
    /// 刚复制了几行（按钮上回一句「已复制 N 行」，换档时复位）。
    copied: Signal<Option<usize>>,
    /// 列表的滚动句柄（「跟随」和「跳到最新」都用它）。
    listing: ElementRef,
}

impl LogView {
    fn new() -> Self {
        Self {
            filter: signal(LogFilter::All),
            follow: signal(true),
            copied: signal(Option::<usize>::None),
            listing: element_ref(),
        }
    }
}

/// 各页的界面状态。
///
/// 都建在 [`shell`] 里（根组件只跑一次），这样切页不会把「正在编辑的草稿」
/// 「日志过滤档」这类东西冲掉。
#[derive(Clone, Copy)]
struct Ui {
    logs: LogView,
    profiles: profile::ProfileUi,
}

/// 整窗骨架。
pub fn shell() -> impl IntoElement {    let page = signal(Page::Dashboard);
    let live = signal(Live::now());
    // 本地校验的错误（「还没下载 tun2proxy」这种）走这里，内核给出的
    // 「上游不通」「会话打满」从快照里读，两条都显示在仪表盘的提示条上。
    let error = signal(Option::<String>::None);
    // 配色是整窗共用的一个值：顶栏上的选择器改它，`palette::apply` 顺手把
    // 新的 token 换上去，整窗按 token 重绘。
    let palette = signal(palette::initial(
        state::config().map(|config| config.palette).unwrap_or_default(),
    ));
    // 明暗（浅色 / 深色 / 跟随系统）与配色各管一层，互不影响。
    let theme_setting = signal(palette::initial_setting(
        state::config().map(|config| config.theme).unwrap_or_default(),
    ));
    // 日志面板的过滤档 / 跟随开关 / 滚动句柄：跨页存活，所以在根组件建。
    let ui = Ui {
        logs: LogView::new(),
        profiles: profile::ProfileUi::new(),
    };
    // 根组件只跑一次，窗口属性也在这里设一次。
    hide_system_title_bar();
    // 之后所有「活起来」的部分都在这里挂上：每秒刷新、托盘、关闭策略、
    // 单实例唤回。
    start_polling(live);
    install_tray(live);
    intercept_close();
    handle_quit_request();
    follow_logs(live, ui.logs);

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
                .child(content(page, live, error, ui)),
        )
        // 启停蒙版：整窗盖住，带 spinner 和「在等哪一步」的文案。
        .child(busy_mask(live))
}

/// 每秒重读一次内核。
///
/// `set_interval` 的回调在界面线程上跑（nsh 的 `owner.run`），所以可以直接写
/// 信号；写进去值没变时信号不会通知，界面也不会重绘——空闲时每秒一次读操作
/// 而已，不会让窗口一直重画。
fn start_polling(live: Signal<Live>) {
    set_interval(Duration::from_millis(1000), move || {
        live.set(Live::now());
    });
    // 单实例唤回：第二个进程只能给「显示窗口」的事件置位，真正把窗口叫到
    // 前台必须由本进程在界面线程上做（`WindowRef` 不是跨线程的东西）。
    // 400ms 轮询一次，双击图标到窗口浮出来的延迟用户感觉不到。
    set_interval(Duration::from_millis(400), || {
        if single_instance::take_show_request() {
            show_window();
        }
    });
}

/// 把主窗口显出来并聚焦（托盘点击、单实例唤回、托盘菜单共用）。
fn show_window() {
    let window = current_window();
    window.show();
    window.focus();
}

/// 托盘图标：程序自带的 `icon.png`，用绝对路径交给 nsh 的图片加载器。
///
/// 不用 `asset!("icon.png")`：那个宏编出来是 `asset:/@<包名>/icon.png`，
/// 运行时要在 `AssetBase` 里登记一个和包名对应的目录才解得开，我们没走那套
/// 打包流程，结果就是图标静默加载失败（只留一条 WARN，托盘空白）。
/// 平台认绝对路径，见 `nsh_core::image::loader::resolve`。
fn tray_icon() -> String {
    crate::core::paths::assets_dir()
        .join("icon.png")
        .display()
        .to_string()
}

/// 系统托盘：图标 + 状态提示 + 显示窗口 / 启停 / 退出。
///
/// 放在 `effect` 里：托盘要跟着服务状态重画（提示语和第二项菜单的文案都读
/// `live`），`effect` 会把读过的东西记成依赖，状态一变就重设一次。
fn install_tray(live: Signal<Live>) {
    effect(move || {
        let snapshot = live.get();
        let active = snapshot.is_active();
        let toggle = if active {
            t("停止隧道", "Stop tunnel")
        } else {
            t("启动隧道", "Start tunnel")
        };
        set_tray(
            tray()
                .icon(tray_icon())
                .tooltip(format!("Tun2Proxy · {}", snapshot.status_label()))
                .menu([
                    MenuItem::new(t("显示窗口", "Show window")).on_select(show_window),
                    MenuItem::new(t("隐藏到托盘", "Hide to tray"))
                        .on_select(|| current_window().hide()),
                    MenuItem::separator(),
                    MenuItem::new(toggle).on_select(toggle_tunnel),
                    MenuItem::separator(),
                    MenuItem::new(t("退出", "Quit")).on_select(quit_app),
                ])
                .on_click(|_: &TrayClickEvent| show_window()),
        );
    });
}

/// 托盘菜单里的「启动 / 停止」。
///
/// 走的是和界面上按钮同一条路（[`state::start`] / [`state::stop`]），所以托盘
/// 起停和界面起停的行为完全一致；失败没有提示条可显示，写一行日志。
fn toggle_tunnel() {
    if state::engine().is_some_and(|engine| engine.is_active()) {
        state::stop();
        return;
    }
    if let Err(error) = state::start() {
        state::log_line(error);
    }
}

/// 真正退出：先把隧道收掉，免得留下 TUN 网卡和路由。
fn quit_app() {
    state::stop();
    quit();
}

/// 点窗口的关闭按钮：按配置退出或收进托盘。
///
/// 光注册 [`on_quit_request`] 不够——那条只管「退出请求」（菜单 Quit、⌘Q /
/// Ctrl+Q，且 macOS 上才是全的）。右键点右上角那个 ×、Alt+F4、任务栏关闭
/// 走的都是「关窗」，nsh 在 `close_on_press: true`（默认）时直接销毁窗口，
/// 窗口没了 app 就结束，`on_quit_request` 根本收不到（Windows 上实测：
/// 进程随 WM_CLOSE 一起没了）。所以这里做两件事：
///
/// 1. [`hide_system_title_bar`] 里把 `close_on_press` 设成 false，关窗先问
///    我们；
/// 2. 用 `on_close_press` 接住这个请求，按配置决定隐藏还是真退。
fn intercept_close() {
    current_window().on_close_press(|_reason| match state::close_action() {
        CloseAction::Quit => quit_app(),
        _ => current_window().hide(),
    });
}

/// 用户点关闭按钮时按配置决定：[`CloseAction::Quit`] 退出，其余收进托盘。
///
/// 收进托盘而不是退出，是「隧道在跑，窗口只是看板」的用法：关掉看板不该
/// 顺手把网也断了。托盘菜单里还有「退出」，要真离开时走那一条。
fn handle_quit_request() {
    on_quit_request(|| match state::close_action() {
        CloseAction::Quit => quit_app(),
        _ => current_window().hide(),
    });
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
    // 关窗由 [`intercept_close`] 决定，不让 nsh 直接销毁窗口。
    attributes.close_on_press = Some(false);
    current_window().set(attributes);
}

/// 顶栏：品牌、状态灯、明暗切换、配色选择器、版本号。
///
/// 拖动区只覆盖**左半区**（品牌方块 + 名字 + 中间空白），右侧那串控件不在拖动
/// 区里。这是实测出来的边界，不是偏好：
///
/// * 拖动区如果只给「中间那段空白」，窗口一窄空白就被挤没，用户拖着标题栏
///   纹丝不动；
/// * 反过来把整条顶栏标成拖动区也不行——Windows 后端按下时先按标题栏处理，
///   里面那几个用 `dynamic()` 包着的色块和分段开关收不到 `on_press`，点了
///   没反应（`window_drag(false)` 标在容器上也没能救回来）。
///
/// 所以左边的牌子负责拖，右边的控件保持「普通元素」，两边都不吃亏。
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
        .bg(move || theme().sidebar)
        .border_b_1()
        .border_color(move || theme().sidebar_border)
        .child(
            view()
                .flex_1()
                .h_full()
                .flex_row()
                .items_center()
                .gap_3()
                .window_drag(true)
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
                ),
        )
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
                .bg(move || theme().accent)
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
        // 未启动时让光点呼吸：nsh 的 `animate-pulse`，不用自己起定时器；
        // 启动 / 停止这两个过渡态改成快一点的双闪，和呼吸区分开。
        // 尺寸都取 4 的倍数：125% 缩放下 6dp 是 7.5 个物理像素，圆点边缘
        // 会被抗锯齿拉成毛边；8dp 正好落在 10 个物理像素上。
        let light = view()
            .w(px(8.0))
            .h(px(8.0))
            .rounded_full()
            .bg(if active { ok_color() } else { warn_color() });
        let light = if snapshot.is_busy() {
            light.animate_ping()
        } else if active {
            light
        } else {
            light.animate_pulse()
        };
        view()
            .h(px(26.0))
            .flex_row()
            .items_center()
            .gap_2()
            .pl_2()
            .pr_3()
            .rounded_full()
            // 胶囊底用 `accent` 而不是 `muted`：四套主题里浅色的 muted 和顶栏
            // 的 sidebar 只差一两个色阶，没描边之后整块看不出来；accent 比
            // sidebar 深一档，深色下又比 muted 亮一档，两种明暗都站得住。
            .bg(move || theme().accent)
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
        .bg(move || theme().accent)
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
            // 没选中的格子悬停时先浮出一层底色，按下去再缩一点点。
            // 悬停色用 `secondary`：胶囊底已经是 `accent`，再用 accent 就看不
            // 出变化；secondary 在各套主题里都比 accent 浅一点（深色下是浅一
            // 档的灰），两种明暗都看得出鼠标在哪。
            .when(!chosen, |this| this.hover(|state| state.bg(theme().secondary)))
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
        .bg(move || theme().accent)
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
            .border_1()
            .border_color(move || {
                if palette.get() == item {
                    theme().foreground
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .cursor_pointer()
            // 悬停放大、按下缩回去：手指能摸到的反馈。
            // 140ms + ease-out：够快，跟得上手；有收尾，不像硬切。
            .transition_transform()
            .duration(140.0)
            .ease_out()
            .hover(|this| this.scale(1.18))
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
        let chosen = page.get() == page_item;
        let bar = view().w(px(4.0)).h(px(16.0)).rounded_full();
        let bar = if chosen {
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
        let row = if chosen {
            row.bg(theme().sidebar_accent)
        } else {
            row
        };
        row.child(bar).child(
            text(page_item.label())
                .text_sm()
                .font_weight(if chosen {
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
            .bg(move || theme().muted)
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

/// 内容区：页面标题 + 当前这一页。
fn content(
    page: Signal<Page>,
    live: Signal<Live>,
    error: Signal<Option<String>>,
    ui: Ui,
) -> impl IntoElement {
    view()
        .flex_1()
        .h_full()
        .flex_col()
        .p_6()
        .gap_5()
        .child(header(page, live))
        .child(dynamic(move || page_body(page.get(), live, error, ui)))
}

/// 按当前页给出内容。
///
/// 每一页都写成函数、统一转成 `AnyElement`：`dynamic` 的闭包只能有一个返回
/// 类型，`match` 出来的几个不同元素类型得先擦掉。
fn page_body(
    page: Page,
    live: Signal<Live>,
    error: Signal<Option<String>>,
    ui: Ui,
) -> AnyElement {
    match page {
        Page::Dashboard => dashboard(live, error).into_element(),
        Page::Profiles => profile::page(ui.profiles).into_element(),
        Page::Program => program(live, ui.logs).into_element(),
        other => pending_page(other).into_element(),
    }
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
                text(move || {
                    format!(
                        "{} · {}",
                        snapshot.status_label(),
                        live.get().binary_label()
                    )
                })
                .text_xs()
                .tabular_nums()
                .text_color(move || theme().muted_foreground)
            }),
        )
}

/// 仪表盘：提示条 + 状态卡 + 流量图 + 两张信息卡。
///
/// 外面套一层滚动：默认窗口（900×650）下一屏正好放得下，但用户把窗口拉小
/// 或加了提示条以后就不够了——不滚的话 flex 会把卡片压扁、互相盖住。
fn dashboard(live: Signal<Live>, error: Signal<Option<String>>) -> impl IntoElement {
    scroll()
        .flex_1()
        .child(
            view()
                .flex_col()
                .gap_3()
                .child(banners(live, error))
                .child(hero(live, error))
                .child(traffic_card(live))
                .child(
                    view()
                        .flex_row()
                        .gap_3()
                        .child(split_card())
                        .child(env_card()),
                ),
        )
}

/// 提示条：内核给的「启动被拦下」「会话打满」，以及本地校验的错误。
///
/// 三条都可能同时存在，各自一行；没有就完全不占位置（`when` 里什么都不给）。
fn banners(live: Signal<Live>, error: Signal<Option<String>>) -> impl IntoElement {
    view()
        .flex_col()
        .gap_2()
        .child(
            when(
                move || error.get().is_some(),
                move || {
                    let text_value = error.get().unwrap_or_default();
                    banner(
                        warn_color(),
                        t("启动没有成功", "Could not start"),
                        text_value,
                        Some(Rc::new(move || error.set(None))),
                    )
                },
            ),
        )
        .child(
            when(
                move || live.get().blocked.is_some(),
                move || {
                    let reason = live.with(|snapshot| snapshot.blocked.clone()).unwrap_or_default();
                    banner(
                        warn_color(),
                        t("上游代理连不通", "The upstream proxy is unreachable"),
                        reason,
                        None,
                    )
                },
            ),
        )
        .child(
            when(
                move || live.get().sessions_full,
                || {
                    banner(
                        info_color(),
                        t("会话名额已打满", "Session limit reached"),
                        t(
                            "新连接被丢弃，去「代理配置」页把「最大会话数」调大。",
                            "New connections are dropped; raise “max sessions” on the Profiles page.",
                        )
                        .to_string(),
                        None,
                    )
                },
            ),
        )
}

/// 一条提示：左边一道色条，右边标题 + 说明（+ 可选关闭）。
fn banner(
    accent: AdaptiveColor,
    title: &'static str,
    detail: String,
    dismiss: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
    let row = view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .p_3()
        .rounded_lg()
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(view().w(px(4.0)).h(px(28.0)).rounded_full().bg(accent))
        .child(
            view()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(
                    text(title)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(accent),
                )
                .child(
                    text(detail)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        );
    match dismiss {
        Some(dismiss) => row.child(
            view()
                .w(px(22.0))
                .h(px(22.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|this| this.bg(theme().accent))
                .on_press(move || dismiss())
                .child(
                    text("×")
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                ),
        ),
        None => row,
    }
}

/// 状态卡：顶上一条霓虹色带 + 大号状态 + 启动按钮 + 两路当前速率。
fn hero(live: Signal<Live>, error: Signal<Option<String>>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let active = snapshot.is_active();
        view()
            .w_full()
            .flex_col()
            .rounded_xl()
            .overflow_hidden()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().card)
            .child(
                // 状态色带：直连（青）→ 代理（紫），整个界面的「信号灯」。
                // 高度 4dp：3dp 在 125% 缩放下是 3.75 个物理像素，渐变边缘
                // 会压在半透明像素上，看着像脏了一条。
                view()
                    .w_full()
                    .h(px(4.0))
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
                    .p_5()
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
                                    .text_2xl()
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
                    ))
                    .child(tunnel_button(live, error)),
            )
    })
}

/// 主按钮：未运行时「启动」，运行中「停止」。
///
/// 启动 / 停止中不接点击：内核那两次调用都不是幂等的（连点会起两条检测线程，
/// 后一条必然回一句「服务已经在运行」，把前一条的结论盖掉）；这期间整窗还有
/// 蒙版挡着。
fn tunnel_button(live: Signal<Live>, error: Signal<Option<String>>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let busy = snapshot.is_busy();
        let active = snapshot.is_active();
        let label = if active {
            t("停止", "Stop")
        } else {
            t("启动", "Start")
        };
        view()
            .h(px(38.0))
            .px_6()
            .rounded_full()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .transition_transform()
            .transition_colors()
            .duration(150.0)
            .ease_out()
            // 悬停再抬一点，按下缩回去；不可点时保持原样，别骗用户的手指。
            .when(!busy, |this| this.hover(|this| this.scale(1.04)).active(|this| this.scale(0.96)))
            .bg(move || {
                if busy {
                    theme().muted
                } else if live.get().is_active() {
                    danger_color()
                } else {
                    theme().primary
                }
            })
            .on_press(move || {
                let snapshot = live.get();
                if snapshot.is_busy() {
                    return;
                }
                if snapshot.is_active() {
                    state::stop();
                    return;
                }
                match state::start() {
                    Ok(()) => error.set(None),
                    Err(message) => error.set(Some(message)),
                }
            })
            .child(
                text(label)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || {
                        if live.get().is_busy() {
                            theme().muted_foreground
                        } else {
                            theme().primary_foreground
                        }
                    }),
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

/// 流量图卡：两路下行速率堆叠面积（直连在下、代理在上）+ 图例。
///
/// 为什么是堆叠面积而不是两条折线：这张图回答的是「总流量里直连占多少、
/// 代理占多少」，堆叠面积把两者的比例画在同一个高度里，一眼看得出分流有没有
/// 在干活；两条折线要来回对比纵轴。
///
/// 每根柱子就是一列（1 秒一个点，窗口 120 秒），两段高度按同一刻度算，
/// 所以总高永远是「这一秒的总下行」。
fn traffic_card(live: Signal<Live>) -> impl IntoElement {
    card(
        t("实时流量", "Live traffic"),
        t(
            "最近两分钟，每秒一个点；直连在下、代理在上，两路合起来就是总下行。",
            "The last two minutes, one point per second: direct at the bottom, proxied on top.",
        ),
        view()
            .flex_col()
            .gap_3()
            .child(
                view()
                    .w_full()
                    .h(px(CHART_HEIGHT))
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(move || theme().muted)
                    .child(traffic_plot(live)),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_4()
                    .child(legend(
                        direct_color(),
                        t("直连 ↓", "Direct ↓"),
                        live,
                        true,
                    ))
                    .child(legend(
                        proxy_color(),
                        t("代理 ↓", "Proxy ↓"),
                        live,
                        false,
                    ))
                    .child(view().flex_1())
                    .child(
                        dynamic(move || {
                            let peak = live.get().peak();
                            text(move || {
                                format!(
                                    "{} {}",
                                    t("峰值", "peak"),
                                    rate_text(live.get().peak().max(peak))
                                )
                            })
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground)
                        }),
                    ),
            ),
    )
}

/// 图例：色块 + 名字 + 当前速率。
fn legend(
    accent: AdaptiveColor,
    label: &'static str,
    live: Signal<Live>,
    direct: bool,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(accent))
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || {
                let snapshot = live.get();
                let value = if direct {
                    snapshot.direct.1
                } else {
                    snapshot.proxy.1
                };
                rate_text(value)
            })
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .tabular_nums()
            .text_color(accent),
        )
}

/// 图本身：一列一列叠出来的面积。
fn traffic_plot(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        if snapshot.history.is_empty() {
            return view()
                .size_full()
                .items_center()
                .justify_center()
                .child(
                    text(t(
                        "开始运行后这里会画出两分钟内的流量。",
                        "Traffic for the last two minutes appears here once it runs.",
                    ))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                );
        }
        // 纵轴取两路之和的峰值：两段高度相加不超过图高，比例才是真的。
        let peak = snapshot.peak().max(1) as f32;
        let columns = snapshot
            .history
            .iter()
            .map(|sample| plot_column(sample, peak))
            .collect::<Vec<_>>();
        view().size_full().flex_row().items_end().children(columns)
    })
}

/// 一秒的柱子：下面直连、上面代理。
fn plot_column(sample: &RouteTraffic, peak: f32) -> AnyElement {
    let scaled = |bytes: u64| {
        let height = bytes as f32 / peak * CHART_HEIGHT;
        // 有一点流量就至少画 1dp，否则低速时整张图看起来是空的。
        let floor = if bytes > 0 { 1.0 } else { 0.0 };
        px(height.max(floor))
    };
    view()
        .flex_1()
        .h_full()
        .flex_col()
        .justify_end()
        .child(
            view()
                .w_full()
                .h(scaled(sample.proxy.down))
                .bg(proxy_color()),
        )
        .child(
            view()
                .w_full()
                .h(scaled(sample.direct.down))
                .bg(direct_color()),
        )
        .into_element()
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

/// 程序与日志页：上面管内核，下面看日志。
fn program(live: Signal<Live>, log_view: LogView) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .gap_3()
        .child(core_card(live))
        .child(log_card(live, log_view))
}

/// 内核卡：装没装、哪一版、要不要更新，以及那三个按钮。
///
/// 这一页的存在意义就是让「还没装 tun2proxy」的人能自己把事办完：启动失败时
/// 仪表盘的提示条会说「点下载并安装」，然后用户得能在这一页找到那个按钮。
fn core_card(live: Signal<Live>) -> impl IntoElement {
    view()
        .flex_col()
        .gap_3()
        .rounded_xl()
        .p_4()
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_4()
                .child(
                    view()
                        .flex_col()
                        .gap_1()
                        .child(
                            text(t("tun2proxy 内核", "tun2proxy core"))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().card_foreground),
                        )
                        .child(dynamic(move || core_summary(live.get()))),
                )
                .child(view().flex_1())
                .child(dynamic(move || core_actions(live.get()))),
        )
        .child(dynamic(move || core_progress(live.get())))
        .child(
            view()
                .flex_col()
                .gap_2()
                .pt_1()
                .child(path_row_full(t("二进制", "Binaries"), paths::bin_dir()))
                .child(path_row_full(t("日志", "Logs"), paths::logs_dir())),
        )
}

/// 内核那一行的说明：版本 + 更新结论。
fn core_summary(snapshot: Live) -> AnyElement {
    let installed = snapshot.installed();
    let (text_value, tone) = match &snapshot.update {
        Update::Available { latest } => (
            tr_format!(
                "有新版本 {} 可装（当前 {}）",
                "{} is available (now {})",
                latest,
                snapshot.binary_label()
            ),
            warn_color(),
        ),
        Update::Latest { version } => (
            tr_format!("已是最新（{}）", "up to date ({})", version),
            ok_color(),
        ),
        Update::Done { version } => (
            tr_format!("已更新到 {}", "updated to {}", version),
            ok_color(),
        ),
        Update::Failed(reason) => (reason.clone(), danger_color()),
        Update::Checking | Update::Downloading { .. } | Update::Installing => (
            snippet(snapshot.update.clone()),
            theme().muted_foreground,
        ),
        Update::Idle => {
            if installed {
                (
                    tr_format!(
                        "{}，点「检查更新」看看有没有新版",
                        "{}, press “Check for updates”",
                        snapshot.binary_label()
                    ),
                    theme().muted_foreground,
                )
            } else {
                (
                    t(
                        "还没装。首次使用先点「下载并安装」，装完就能启动隧道。",
                        "Not installed yet. Press “Download and install” first.",
                    )
                    .to_string(),
                    theme().muted_foreground,
                )
            }
        }
    };
    text(text_value)
        .text_xs()
        .text_color(tone)
        .into_element()
}

/// 正在忙时那一行文案。
fn snippet(update: Update) -> String {
    match update {
        Update::Checking => t("正在查询最新版本…", "Checking the latest release…").to_string(),
        Update::Downloading { got, total } => match total {
            Some(total) if total > 0 => tr_format!(
                "正在下载：{} / {}",
                "Downloading: {} / {}",
                bytes_text(got),
                bytes_text(total)
            ),
            _ => tr_format!(
                "正在下载：已下载 {}",
                "Downloading: {} so far",
                bytes_text(got)
            ),
        },
        Update::Installing => t("正在解压安装…", "Unpacking and installing…").to_string(),
        _ => String::new(),
    }
}

/// 内核卡右侧的按钮组。
fn core_actions(snapshot: Live) -> AnyElement {
    let busy = snapshot.update.is_busy();
    let running = snapshot.is_active();
    let can_install = !busy && !running;
    let install_label = match &snapshot.update {
        Update::Available { .. } => t("下载并安装", "Download and install"),
        _ if snapshot.installed() => t("重新安装", "Reinstall"),
        _ => t("下载并安装", "Download and install"),
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        // 隧道在跑时按钮是灰的，原因写在这里：不写的话用户只会觉得按钮坏了。
        .child(dynamic(move || {
            if running {
                text(t(
                    "隧道运行中，先停止才能换内核",
                    "Stop the tunnel before replacing the core",
                ))
                .text_xs()
                .text_color(move || theme().muted_foreground)
                .into_element()
            } else {
                view().into_element()
            }
        }))
        .child(ghost_button_maybe(
            t("检查更新", "Check for updates"),
            !busy,
            || state::check_update(),
        ))
        .child(primary_button_maybe(install_label, can_install, || {
            state::install_update()
        }))
        .into_element()
}

/// 下载进度条 / 安装提示；不忙时完全不占位置。
fn core_progress(snapshot: Live) -> AnyElement {
    if !snapshot.update.is_busy() {
        return view().into_element();
    }
    let ratio = snapshot.update_progress();
    view()
        .flex_col()
        .gap_2()
        .child(
            view()
                .w_full()
                .h(px(8.0))
                .rounded_full()
                .overflow_hidden()
                .bg(move || theme().muted)
                .child(match ratio {
                    Some(ratio) => view()
                        .w(relative(ratio))
                        .h_full()
                        .rounded_full()
                        .bg(move || theme().primary)
                        .into_element(),
                    // 没有总长时就给一条在动的，别让用户以为卡住了。
                    None => view()
                        .w(relative(0.35))
                        .h_full()
                        .rounded_full()
                        .bg(move || theme().primary)
                        .animate_pulse()
                        .into_element(),
                }),
        )
        .into_element()
}

/// 日志卡：标题 + 行数 + 清空，下面是虚拟列表。
fn log_card(live: Signal<Live>, log_view: LogView) -> impl IntoElement {
    let filter = log_view.filter;
    view()
        .flex_1()
        .flex_col()
        .gap_3()
        .rounded_xl()
        .p_4()
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    text(t("运行日志", "Runtime log"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().card_foreground),
                )
                .child(
                    // 过滤开着的时候说清楚「现在看到的是几行里挑出来的几行」，
                    // 否则用户会以为日志本身丢了。
                    dynamic(move || {
                        let chosen = filter.get();
                        let (kept, total) = live.with(|snapshot| {
                            (
                                snapshot
                                    .logs
                                    .iter()
                                    .filter(|line| chosen.keeps(line.kind))
                                    .count(),
                                snapshot.logs.len(),
                            )
                        });
                        let caption = if chosen == LogFilter::All {
                            tr_format!("{} 行", "{} lines", total)
                        } else {
                            tr_format!("{} / {} 行", "{} of {} lines", kept, total)
                        };
                        text(caption)
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground)
                    }),
                )
                .child(view().flex_1())
                .child(copy_button(live, log_view))
                .child(ghost_button(t("清空", "Clear"), || state::clear_logs())),
        )
        .child(log_pane(live, log_view))
        // 工具条放底部：过滤和「跟随」都是「看日志时才会动」的东西，压在标题
        // 边上会跟上面那张内核卡抢注意力，也会在窄窗口里把标题挤掉。
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(filter_segments(log_view))
                .child(view().flex_1())
                .child(follow_toggle(log_view))
                .child(ghost_button_with(
                    t("跳到最新", "Jump to latest"),
                    log_view.listing,
                    move |listing| listing.scroll_to_end(false),
                )),
        )
}

/// 「跟随最新」开关：开着时每来一行新日志就滚到底。
///
/// 默认开——日志面板的默认问题是「刚发生了什么」。要看历史就先关掉，否则
/// 每一次新行都会把视图拽回底部（滚动容器的位置读不回来，只能靠用户表明
/// 意图）。
fn follow_toggle(log_view: LogView) -> impl IntoElement {
    let follow = log_view.follow;
    dynamic(move || {
        let on = follow.get();
        view()
            .h(px(26.0))
            .px_3()
            .rounded_full()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .transition_colors()
            .duration(140.0)
            .ease_out()
            .when(!on, |this| this.hover(|state| state.bg(theme().accent)))
            .bg(move || {
                if follow.get() {
                    theme().primary
                } else {
                    theme().muted
                }
            })
            .on_press(move || {
                let next = !follow.get();
                follow.set(next);
                if next {
                    // 打开就立刻回到底部：用户点它的意思就是「带我去看最新的」。
                    log_view.listing.scroll_to_end(false);
                }
            })
            .child(
                text(t("跟随最新", "Follow"))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if follow.get() {
                            theme().primary_foreground
                        } else {
                            theme().muted_foreground
                        }
                    }),
            )
    })
}

/// 每来一行新日志就把列表滚到底（「跟随」关着时不动）。
///
/// 触发器是 `log_total` 而不是 `logs.len()`：后者涨到上限就不再变（见
/// [`Live::log_total`] 的注释），跟随会在跑到上限时悄悄停掉。
fn follow_logs(live: Signal<Live>, log_view: LogView) {
    // 从 0 起算而不是从当前行数起算：`initial_scroll_at_end` 是首帧之前那次
    // 布局里算的，行高还没定下来，经常停在半行上；让第一轮 effect 再滚一次，
    // 打开的瞬间就一定贴底。
    let seen = Cell::new(0u64);
    effect(move || {
        let total = live.get().log_total;
        let follow = log_view.follow.get();
        let previous = seen.replace(total);
        if !follow || total == previous {
            return;
        }
        log_view.listing.scroll_to_end(false);
    });
}

/// 日志过滤的三段控件。
fn filter_segments(log_view: LogView) -> impl IntoElement {
    view()
        .h(px(24.0))
        .flex_row()
        .items_center()
        .gap_1()
        .p_1()
        .rounded_full()
        .bg(move || theme().muted)
        .children(LogFilter::ALL.iter().map(|choice| {
            let choice = *choice;
            filter_segment(choice, log_view)
        }))
}

/// 过滤控件里的一格。
fn filter_segment(choice: LogFilter, log_view: LogView) -> impl IntoElement {
    let filter = log_view.filter;
    dynamic(move || {
        let chosen = filter.get() == choice;
        view()
            .h(px(18.0))
            .px_2()
            .rounded_full()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(!chosen, |this| this.hover(|state| state.bg(theme().accent)))
            .transition_colors()
            .duration(140.0)
            .ease_out()
            .bg(move || {
                if filter.get() == choice {
                    theme().primary
                } else {
                    AdaptiveColor::TRANSPARENT
                }
            })
            .on_press(move || {
                filter.set(choice);
                // 换档后行数变了，上一次的「已复制 N 行」不再对得上。
                log_view.copied.set(None);
            })
            .child(
                text(choice.label())
                    .text_xs()
                    .font_weight(if chosen {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(move || {
                        if filter.get() == choice {
                            theme().primary_foreground
                        } else {
                            theme().muted_foreground
                        }
                    }),
            )
    })
}

/// 把当前这一档的日志整段放进剪贴板（报问题时粘得出去）。
fn copy_logs(live: Signal<Live>, log_view: LogView) {
    let chosen = log_view.filter.get();
    let body = live.with(|snapshot| {
        snapshot
            .logs
            .iter()
            .filter(|line| chosen.keeps(line.kind))
            .map(|line| format!("{:<5} {}", kind_tag(line.kind), line.text))
            .collect::<Vec<_>>()
            .join("\n")
    });
    if body.is_empty() {
        return;
    }
    let count = body.lines().count();
    set_clipboard(body);
    // 这个动作没有别的可见结果：不回报一句，用户不知道到底成没成。
    //
    // 这里不用定时器让它自己消失：`set_timeout` 的任务挂在调用时的 owner 上，
    // 而按下按钮后这一格会立刻重绘、旧 owner 随之销毁，定时器跟着被取消——
    // 表现就是「已复制」一直挂着。改成「下一次复制或换档时才变」，行为是
    // 确定的，也不需要额外的定时任务。
    log_view.copied.set(Some(count));
}

/// 复制按钮：点完在按钮上回一句「已复制 N 行」，再点还是复制，换档时复位。
fn copy_button(live: Signal<Live>, log_view: LogView) -> impl IntoElement {
    dynamic(move || {
        let copied = log_view.copied.get();
        let label = match copied {
            Some(count) => tr_format!("已复制 {} 行", "Copied {}", count),
            None => t("复制", "Copy").to_string(),
        };
        view()
            .h(px(26.0))
            .px_3()
            .rounded_full()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .transition_colors()
            .duration(140.0)
            .ease_out()
            .hover(|this| this.bg(theme().accent))
            .active(|this| this.scale(0.96))
            .border_1()
            .border_color(move || {
                if copied.is_some() {
                    theme().primary
                } else {
                    theme().border
                }
            })
            .on_press(move || copy_logs(live, log_view))
            .child(
                text(label)
                    .text_xs()
                    .text_color(move || {
                        if copied.is_some() {
                            theme().primary
                        } else {
                            theme().muted_foreground
                        }
                    }),
            )
    })
}

/// 日志正文：虚拟列表，只在视野里那几十行存在。
fn log_pane(live: Signal<Live>, log_view: LogView) -> impl IntoElement {
    let filter = log_view.filter;
    view()
        .flex_1()
        .rounded_lg()
        .overflow_hidden()
        .bg(move || theme().background)
        .child(
            virtual_list(
                move || visible_count(live, filter.get()),
                move |at| log_row(live, filter, at.index()),
            )
            .element_ref(&log_view.listing)
            // 打开就看最新的一行：日志面板默认关心「刚刚发生了什么」。
            // 列表自己不撑高度：不给它 `h_full` 时高度是 0，界面上就是
            // 「有行数、一片空白」。
            .initial_scroll_at_end()
            .h_full()
            .empty(dynamic(move || {
                // 两片空白长得一样但意思不同：真的是空的，还是这一档没匹配上。
                // 说错一句用户就会以为记录停了。
                let kept = visible_count(live, filter.get());
                let total = live.with(|snapshot| snapshot.logs.len());
                let caption = if total == 0 {
                    t(
                        "还没有日志。启动隧道后这里会实时显示，落盘的那份在日志目录里。",
                        "No log yet. Start the tunnel and it shows up here; the file copy lives in the log folder.",
                    )
                } else if kept == 0 {
                    t(
                        "这一档没有匹配的行。换个过滤档看看。",
                        "Nothing in this filter; try another one.",
                    )
                } else {
                    ""
                };
                view().p_4().child(
                    text(caption)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
            })),
        )
}

/// 当前过滤档下有几行（虚拟列表的长度就用它）。
fn visible_count(live: Signal<Live>, filter: LogFilter) -> usize {
    live.with(|snapshot| {
        snapshot
            .logs
            .iter()
            .filter(|line| filter.keeps(line.kind))
            .count()
    })
}

/// 过滤后第 `index` 行在原始列表里的下标。
///
/// 虚拟列表拿到的是「过滤后的序号」，而行是按原始下标存的，所以每个可见行
/// 都要换算一次。`LOG_TAIL` 只有 400 行、一屏也就几十行，这个 O(n) 查找比
/// 每帧重建一份索引数组更省。
fn visible_at(live: &Live, filter: LogFilter, index: usize) -> Option<usize> {
    live.logs
        .iter()
        .enumerate()
        .filter(|(_, line)| filter.keeps(line.kind))
        .map(|(at, _)| at)
        .nth(index)
}

/// 一行日志：来源标签 + 正文（过长截断，完整内容在文件里）。
///
/// 三处宽度都得写死：虚拟列表的 cell 宽度由内容决定，`truncate` 拿不到可用
/// 宽度就会把整行压成 0 宽——表现是「有行数、一个字也看不见」。所以行是
/// `w_full`、标签是固定宽、正文外面套一层 `flex_1` 的裁剪盒。
fn log_row(live: Signal<Live>, filter: Signal<LogFilter>, index: usize) -> impl IntoElement {
    // 行是按原始下标存的，`index` 是过滤后的序号，先换算成原始行。
    let at = move || {
        live.with(|snapshot| {
            visible_at(snapshot, filter.get(), index)
                .and_then(|at| snapshot.logs.get(at))
                .cloned()
        })
    };
    view()
        .w_full()
        .h(px(18.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .child(
            text(move || at().map(|line| kind_tag(line.kind)).unwrap_or_default())
                .w(px(46.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || kind_color(at().map(|line| line.kind))),
        )
        .child(
            view().flex_1().overflow_hidden().child(
                text(move || at().map(|line| line.text).unwrap_or_default())
                    .w_full()
                    .text_xs()
                    .truncate()
                    .tabular_nums()
                    .text_color(move || theme().secondary_foreground),
            ),
        )
}

/// 日志来源的短标签。
fn kind_tag(kind: LogKind) -> String {
    match kind {
        LogKind::App => "APP",
        LogKind::Split => "SPLIT",
        LogKind::Output => "TUN",
        LogKind::Other => "···",
    }
    .to_string()
}

/// 日志来源的颜色：子进程输出最淡（量最大），分流判定用代理色。
fn kind_color(kind: Option<LogKind>) -> AdaptiveColor {
    match kind {
        Some(LogKind::Split) => proxy_color(),
        Some(LogKind::App) => direct_color(),
        Some(LogKind::Output) => theme().muted_foreground,
        _ => theme().muted_foreground,
    }
}

/// 还没搬到 nsh 的页面：明确说「这一版还没有」，而不是给一片空白。
fn pending_page(_page: Page) -> impl IntoElement {
    // 不套 `card()`：那一页还没有内容，一张带边框的大卡片只会是一片空白，
    // 标题还会和页头重复一次。给一块安静的无边框提示就够。
    view()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .bg(move || theme().muted)
        .child(
            text(t(
                "这一版的界面还在搬迁中：代理配置、分流规则、网络参数三页还没有搬过来。",
                "This page is still being ported: profiles, split rules and network settings are not here yet.",
            ))
            .text_xs()
            .text_color(move || theme().secondary_foreground),
        )
        .child(
            text(t(
                "仪表盘与「程序与日志」已经可用。",
                "The dashboard and “Program” pages work already.",
            ))
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
}

/// 启停蒙版：整窗盖住，中间 spinner + 「在等哪一步」。
///
/// 必须能等得住：启动要走「探上游 → 清残留网卡 → 起分流器 → 拉子进程」，
/// 停止要等子进程退出和网卡回收，都是几秒级的。只把按钮变灰的话，用户会以为
/// 程序没反应而反复点。
fn busy_mask(live: Signal<Live>) -> impl IntoElement {
    when(move || live.get().is_busy(), move || {
        overlay()
            .absolute()
            .size_full()
            .items_center()
            .justify_center()
            // 半透明深色：看得到后面的界面，但读不出细节，焦点落在中间那张卡上。
            .bg(rgba(0x04060E_C8))
            .child(
                view()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p_6()
                    .rounded_xl()
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .child(spinner())
                    .child(
                        text(live.get().busy_label())
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(t(
                            "这一步会清理残留网卡与进程，请稍等。",
                            "This cleans up leftover adapters and processes; it takes a few seconds.",
                        ))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    ),
            )
    })
}

/// 转圈：一个渐变圆环原地旋转（nsh 的 `animate-spin`）。
///
/// 没有画「缺口」——nsh 的圆靠 `border` 画，四边不同色要额外属性；渐变
/// 从主色转到 `muted`，转起来一眼就是「在动」，够用且不挑主题。
fn spinner() -> impl IntoElement {
    view()
        .w(px(30.0))
        .h(px(30.0))
        .rounded_full()
        .bg_linear_to_r()
        .gradient_from(theme().primary)
        .gradient_to(theme().muted)
        .animate_spin()
}

/// 一张信息卡：标题、说明，加一块调用方拼好的内容。
fn card(title: &'static str, hint: &'static str, rows: impl IntoElement) -> impl IntoElement {
    view()
        .flex_1()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_xl()
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

/// 次要按钮：只有描边，没有底色。
fn ghost_button(label: &'static str, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_3()
        .rounded_full()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .transition_colors()
        .duration(140.0)
        .ease_out()
        .hover(|this| this.bg(theme().accent))
        .active(|this| this.scale(0.96))
        .border_1()
        .border_color(move || theme().border)
        .on_press(on_press)
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

/// 次要按钮，点击交给调用方处理（这里要的是「点一下让某个元素滚到底」）。
fn ghost_button_with(
    label: &'static str,
    target: ElementRef,
    on_press: impl Fn(ElementRef) + 'static,
) -> impl IntoElement {
    view()
        .h(px(26.0))
        .px_3()
        .rounded_full()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .transition_colors()
        .duration(140.0)
        .ease_out()
        .hover(|this| this.bg(theme().accent))
        .active(|this| this.scale(0.96))
        .border_1()
        .border_color(move || theme().border)
        .on_press(move || on_press(target))
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

/// 次要按钮的可禁用版本：不接点击、颜色变灰、不给悬停反馈。
///
/// 「灰着但还能点」是最容易让人点半天没反应的做法，所以禁用时连 `on_press`
/// 都不挂——点了什么也不会发生。
fn ghost_button_maybe(
    label: &'static str,
    enabled: bool,
    on_press: impl Fn() + 'static + Clone,
) -> impl IntoElement {
    let mut button = view()
        .h(px(26.0))
        .px_3()
        .rounded_full()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(move || theme().border)
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        );
    if enabled {
        let handler = on_press.clone();
        button = button
            .cursor_pointer()
            .transition_colors()
            .duration(140.0)
            .ease_out()
            .hover(|this| this.bg(theme().accent))
            .active(|this| this.scale(0.96))
            .on_press(move || handler());
    }
    button
}

/// 主按钮的可禁用版本：实色底，禁用时变灰并且不接点击。
fn primary_button_maybe(
    label: &'static str,
    enabled: bool,
    on_press: impl Fn() + 'static + Clone,
) -> impl IntoElement {
    let mut button = view()
        .h(px(30.0))
        .px_4()
        .rounded_full()
        .items_center()
        .justify_center()
        .bg(move || {
            if enabled {
                theme().primary
            } else {
                theme().muted
            }
        })
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if enabled {
                        theme().primary_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        );
    if enabled {
        let handler = on_press.clone();
        button = button
            .cursor_pointer()
            .transition_colors()
            .transition_transform()
            .duration(140.0)
            .ease_out()
            .hover(|this| this.scale(1.03))
            .active(|this| this.scale(0.96))
            .on_press(move || handler());
    }
    button
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

/// 路径行：一行放下「名字 + 路径」（过长截断）。
///
/// 原先名字在上、路径在下，三份路径就是六行，默认窗口里放不下；路径尾巴才是
/// 有用的部分（`config.json` / `bin` / `logs`），挤在一行里反而更好认。
fn path_row(label: &'static str, path: std::path::PathBuf) -> impl IntoElement {
    path_line(label, short_path(&path))
}

/// 和 [`path_row`] 一样，但要完整路径。
///
/// 程序页的用途不同：用户会照着这几行去文件管理器里找 tun2proxy.exe 和今天的
/// 日志，`…\bin` 这种缩写在那里等于没说。
fn path_row_full(label: &'static str, path: std::path::PathBuf) -> impl IntoElement {
    path_line(label, path.display().to_string())
}

/// 路径行的实际排版：名字靠左、路径贴右、过长截断。
fn path_line(label: &'static str, value: String) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            text(label)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(view().flex_1())
        .child(
            text(value)
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
