//! 可复用的视图小部件与设计常量。
//!
//! 版式规则只有三条，全部取自 gpuikit 自己的尺度，不另起一套：
//!
//! 1. **尺寸走 `ControlScale`**：按钮、图标按钮、徽标的高度 / 圆角 / 字号都
//!    从 `theme.control(size)` 取，和 gpuikit 的 `Button`、`IconButton`、
//!    `Badge` 逐像素对齐——摆在同一行不会一大一小；
//! 2. **颜色一律走主题语义色**（`theme.fg()` / `theme.surface()` …），
//!    卡片里不写十六进制，换肤时整窗一起变；
//! 3. **可换行的文字要占满剩余宽度**（`flex_1` + `min_w_0`），
//!    否则行内文字会按「最宽不可断词」测量，中文会被压成一列竖排的字。
//!
//! gpuikit 有的组件一律用它的（按钮、图标按钮、勾选框、输入框、下拉框、
//! 对话框、徽标）。只有三样东西它没有，才自己画，且形状贴着它的 `Card`：
//! **强调色实心主按钮**、**图标 + 文字的行内动作**、**设置分组卡片**。

use gpui::{
    AnyElement, App, BorderStyle, Bounds, ClickEvent, DispatchPhase, Div, ElementId, Entity, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ScrollHandle, SharedString, Stateful,
    Svg, Window, canvas, div, point, px, prelude::*, quad, rems, size, transparent_black,
};
use gpuikit::elements::checkbox::{CheckState, checkbox_box};
use gpuikit::elements::icon_button::icon_button;
use gpuikit::elements::tooltip::tooltip;
use gpuikit::input::InputState;
use gpuikit::theme::{ControlSize, Theme, Themeable};
use gpuikit::traits::control_sized::ControlSized;
use std::cell::Cell;
use std::rc::Rc;

use super::theme::{
    TEXT_BODY, TEXT_GROUP, TEXT_HERO, TEXT_MONO, TEXT_SMALL, TEXT_TITLE, WEIGHT_SEMIBOLD,
};

/// 日志区高度（逻辑像素）。
///
/// 日志是唯一需要「自己滚」的区域：固定一个高度，页面滚动时它不动，
/// 免得一边翻设置一边把日志推出视野。
pub const LOG_PANE_HEIGHT: f32 = 220.0;

/// 滚动条滑块的宽度（逻辑像素）。
///
/// 8dp：比系统滚动条（17dp）细，但比原来那 4dp 粗一倍——4dp 在 125% 缩放下
/// 会被抗锯齿吃掉半个像素，看起来像一条脏线，而且几乎点不中。
pub const SCROLL_BAR_WIDTH: f32 = 8.0;

/// 滚动条滑块的不透明度（静止态）。
///
/// 0.75：`fg_muted` 按 55% 压到输入框底色上时，深色只有约 3.4:1、浅色约
/// 2.0:1——8dp 的细条在满屏卡片里像一条脏线，不凑近看不出「这里还能滚」。
/// 提到 75% 后深色约 5.2:1、浅色约 2.7:1，两套主题下都能一眼看见，又还没到
/// 抢正文的地步。页面条、日志条、输入框条共用这一个数。
pub const SCROLL_THUMB_OPACITY: f32 = 0.75;

/// 滚动条属于哪一个滚动区域。
///
/// 所有滚动条共用 `App` 上的一份拖动状态（见 [`ScrollDrag`]），按下时必须
/// 记住「是谁按下的」：否则拖着日志条往下走时，页面那条也会认为自己在被
/// 拖，把整页滚到日志的偏移量上——看起来就是「拖日志，页面自己跟着跑」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollBarKey {
    /// 某一页的页面滚动（下标与 `App::page_scroll` 对齐）。
    Page(usize),
    /// 仪表盘 / 程序页的日志窗格。
    Log,
    /// 多行输入框（绕过列表）。`&'static str` 是输入框的名字。
    Input(&'static str),
}

/// 正在被拖动的滚动条：属于哪一条、按下那一刻的指针 y 与滚动偏移。
///
/// 这份状态必须活在 `App` 上。渲染函数每帧都会重新调一次 [`scroll_bar`]，
/// 把 `Rc<Cell>` 建在函数体里的话，`on_mouse_down` 写进去的那份和下一帧
/// `canvas` 里读到的根本不是同一个——按下有反应，拖动永远没反应。
pub type ScrollDrag = Rc<Cell<Option<(ScrollBarKey, f32, f32)>>>;

/// 最小间距：图标与文字、开关与说明之间。
pub const SPACE_XS: f32 = 4.0;
/// 同一组元素之间。
pub const SPACE_S: f32 = 8.0;
/// 卡片内部的行间距。
pub const SPACE_M: f32 = 12.0;
/// 大区块之间的间距。
pub const SPACE_XL: f32 = 24.0;

/// 内容区四周留白。
///
/// 比卡片间距大一圈：卡片与窗口边缘之间要有空气，卡片才像「浮在底色上」，
/// 而不是贴边铺满。
pub const CONTENT_PADDING: f32 = 16.0;

/// 行内边距。取 gpuikit `Card` 的 `p(0.75rem)`：分组卡片里的一行，
/// 与一张普通卡片的留白是同一个数。
pub const ROW_PADDING: f32 = 12.0;
/// 卡片圆角。gpuikit 的 `Card` 是 `rounded(0.5rem)`。
pub const CARD_RADIUS: f32 = 8.0;
/// 分组卡片的圆角：与浮动卡片同值，两类卡片在同一屏里才是同一种圆。
pub const GROUP_RADIUS: f32 = CARD_RADIUS;
/// 行前导图标的尺寸。取 `ControlSize::Large` 的 `ink`。
pub const ROW_ICON: f32 = 16.0;
/// 行右侧控件的常用宽度（下拉框、输入框）。
///
/// 任务管理器里右侧控件是一列对齐的窄控件，而不是铺满整行：
/// 铺满会让「标题」和「它的值」隔着一整个屏幕，读起来要来回扫。
pub const ROW_CONTROL: f32 = 220.0;
/// 窄控件（数字输入框）的宽度。
pub const ROW_CONTROL_NARROW: f32 = 120.0;

// —— 文字 ——

/// 页面标题（顶栏用）。
pub fn page_title(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_TITLE))
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(theme.fg())
        .whitespace_nowrap()
        .child(text.into())
}

/// 首屏大标题。
pub fn hero_title(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_HERO))
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(theme.fg())
        .child(text.into())
}

/// 分组标题：卡片外面那行加粗小字。
pub fn group_label(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_GROUP))
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(theme.fg())
        .child(text.into())
}

/// 次要色的说明文字，可以换行。
pub fn hint(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_SMALL))
        .text_color(theme.fg_muted())
        .child(text.into())
}

/// 带颜色的说明文字（结论类：检测成功 / 失败）。
///
/// 颜色由调用方给（成功 / 危险 / 警告），所以这里的 `theme` 用不上——
/// 留着参数是为了让所有文字样式的签名一致，调用处不必记住「哪些要主题」。
pub fn hint_tone(text: impl Into<SharedString>, color: Hsla, _theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_SMALL))
        .text_color(color)
        .child(text.into())
}

/// 日志行：等宽字体，方便对齐时间戳与级别。
pub fn log_line(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_MONO))
        .font_family(super::theme::MONO_FAMILY)
        .text_color(theme.fg_muted())
        .child(text.into())
}

/// 不换行的说明文字：用在「一行里和按钮/开关并排」的场合。
pub fn caption_nowrap(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_BODY))
        .text_color(theme.fg())
        .whitespace_nowrap()
        .child(text.into())
}

/// 行右侧的「值」：次要色、单行、过长就省略。
pub fn row_value(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_BODY))
        .text_color(theme.fg_muted())
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
        .child(text.into())
}

// —— 容器 ——

/// 滚动条。
///
/// gpui 的 `overflow_y_scroll` 只负责滚动，**不画滚动条**：内容超出视口时
/// 界面上没有任何提示，用户只能靠滚轮试探。这里按 `ScrollHandle` 的偏移量
/// 画一条，跟着滚动一起动，也能点、能拖。
///
/// 视觉取 gpuikit 的「滑块」语义而不是系统滚动条：没有凹槽、没有箭头，
/// 只有一颗 8dp 的圆头滑块贴着视口右边缘（`right: 2dp`），静止时是
/// `fg_muted` 的 [`SCROLL_THUMB_OPACITY`]——两套主题下都能一眼看见，又不会
/// 在满屏卡片里多出一条竖线。
///
/// 交互是浏览器那套：轨道（整条 8dp 竖列）上按下，滑块中心先跳到指针处，
/// 接着按住拖就跟手走。轨道本身透明，指针压上来才浮出一层 12% 的底，
/// 用来告诉用户「这条能抓」。
///
/// 滑块用 `canvas` 画而不是塞一个 `div`：拖动时改的是 `ScrollHandle` 的
/// 偏移量，`App` 不会重新 render，元素树里的 `top(...)` 是按下那一刻的旧值，
/// 结果就是「内容在动、滑块不动」。绘制阶段每次都能读到最新偏移量。
///
/// 拖拽走窗口级事件（`window.on_mouse_event`）而不是元素自己的
/// `on_mouse_move`：后者的 hitbox 只有这条 8dp 的窄条那么大，指针一动就跑出去了。
/// 起点记在 [`ScrollDrag`] 里（`App` 上的字段），不能是局部变量——见那里的注释。
///
/// 内容没超出视口时什么也不画（判断放在绘制阶段，见函数体里的注释）。
pub fn scroll_bar(
    id: impl Into<ElementId>,
    key: ScrollBarKey,
    handle: &ScrollHandle,
    theme: &Theme,
    drag: &ScrollDrag,
) -> AnyElement {
    let thumb_color = theme.fg_muted().opacity(SCROLL_THUMB_OPACITY);
    let lane = theme.fg_muted().opacity(0.12);

    let press_handle = handle.clone();
    let press_drag = drag.clone();
    let paint_handle = handle.clone();
    let move_handle = handle.clone();
    let move_drag = drag.clone();

    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(2.0))
        .w(px(SCROLL_BAR_WIDTH))
        .cursor_pointer()
        .hover(move |style| style.bg(lane))
        .on_mouse_down(MouseButton::Left, move |event, window, _cx| {
            let (viewport, thumb, travel) = geometry(&press_handle);
            let thumb_top = progress(&press_handle) * travel;
            // 指针 y 换算成轨道内的位置，用来判断点在滑块上还是空白处。
            let local = (event.position.y.as_f32()
                - press_handle.bounds().origin.y.as_f32())
            .clamp(0.0, viewport);
            let offset = if local >= thumb_top && local <= thumb_top + thumb {
                // 抓的就是滑块本身：不动它，从当前偏移接着拖。
                press_handle.offset().y.as_f32()
            } else {
                // 点空白处：滑块中心先过来，再接着拖（和浏览器一致）。
                let target = ((local - thumb * 0.5) / travel.max(1.0)).clamp(0.0, 1.0);
                let offset = -target * press_handle.max_offset().y.as_f32();
                press_handle.set_offset(point(press_handle.offset().x, px(offset)));
                window.refresh();
                offset
            };
            press_drag.set(Some((key, event.position.y.as_f32(), offset)));
        })
        .child(
            canvas(|_, _, _| (), move |bounds: Bounds<_>, _, window, _| {
                // 「内容有没有超出视口」必须在这里判：render 阶段读到的是
                // **上一帧**的布局结果，新切到一页时它还是 0，滚动条会缺席到
                // 用户滚一下为止。prepaint 已经算完布局，paint 里读才是最新的。
                if paint_handle.max_offset().y.as_f32() > 0.5 {
                    let (_, thumb, travel) = geometry(&paint_handle);
                    let top = progress(&paint_handle) * travel;
                    let bar = Bounds::new(
                        point(bounds.origin.x, bounds.origin.y + px(top)),
                        size(bounds.size.width, px(thumb)),
                    );
                    window.paint_quad(quad(
                        bar,
                        gpui::Corners::all(px(bounds.size.width.as_f32() * 0.5)),
                        thumb_color,
                        gpui::Edges::all(px(0.0)),
                        transparent_black(),
                        BorderStyle::default(),
                    ));
                }

                // 一次绘制里挂上窗口级监听：指针离开这条 8dp 的窄条也不会断。
                let drag = move_drag.clone();
                let handle = move_handle.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let Some((start_key, start_y, start_offset)) = drag.get() else {
                        return;
                    };
                    // 同一时刻只有一条滚动条在拖：别的那几条也有监听器，
                    // 不认领就直接退出。
                    if start_key != key {
                        return;
                    }
                    // 抬起没被这条滚动条收到时（指针跑出窗口），
                    // `dragging()` 会变假，顺手结束这次拖拽。
                    if !event.dragging() {
                        drag.set(None);
                        return;
                    }
                    // 滑块走完整条轨道 = 内容滚完整段距离。
                    //
                    // 符号：偏移量是 `[-max, 0]`，往下拖要把内容往上推，
                    // 也就是偏移量更**负**——这里必须是减，写成加会被 layout
                    // 钳回 0，界面上一点反应都没有。
                    let (_, _, travel) = geometry(&handle);
                    let max = handle.max_offset().y.as_f32();
                    let moved = event.position.y.as_f32() - start_y;
                    let delta = -moved * (max / travel.max(1.0));
                    handle.set_offset(point(handle.offset().x, px(start_offset + delta)));
                    window.refresh();
                });
            })
            .absolute()
            .size_full(),
        )
        .into_any_element()
}

/// 视口高度、滑块高度、滑块可走的距离。
///
/// 三处都要用（绘制、按下、拖动），算一处比抄三遍可靠：视口高度取
/// `ScrollHandle` 上一次布局的结果，比调用方传一个可能过期的数字可靠。
fn geometry(handle: &ScrollHandle) -> (f32, f32, f32) {
    let viewport = handle.bounds().size.height.as_f32().max(1.0);
    let content = viewport + handle.max_offset().y.as_f32();
    // 内容越长滑块越短，但不短于 32dp：再短就点不中也看不见。
    let thumb = (viewport * viewport / content).max(32.0).min(viewport);
    (viewport, thumb, (viewport - thumb).max(0.0))
}

/// 当前滚动进度，0 到 1。
///
/// 偏移量是 `[-max, 0]`（顶部 0、底部 -max），取负再除就翻成了 0..1。
fn progress(handle: &ScrollHandle) -> f32 {
    let max = handle.max_offset().y.as_f32();
    if max <= 0.5 {
        return 0.0;
    }
    (-handle.offset().y.as_f32() / max).clamp(0.0, 1.0)
}

/// 多行输入框的滚动条。
///
/// gpui 的 `text_area` 和滚动容器一样只滚不画，而 gpuikit 的 `Textarea`
/// 没有留出画条的位置。这里把条叠在输入框的**内边距**上：`inset` 是内容区
/// 相对外框的上下内缩（内边距 + 1dp 描边），条因此正好贴住文字的视口，
/// 不会跑进圆角；横向落在右边距里，按上来不会被 `Input` 当成「点到文字」，
/// 不会顺手把光标挪走。
///
/// 滚动量由 [`InputState`] 自己持有，所以几何每帧现读：视口高度、内容高度
/// 和偏移都从实体里取，`App` 不必再存一份，也不会和输入框的真实布局对不上。
///
/// 拖动状态与 [`scroll_bar`] 共用一份，靠 [`ScrollBarKey`] 认领。
pub fn input_scroll_bar(
    id: impl Into<ElementId>,
    key: ScrollBarKey,
    state: &Entity<InputState>,
    inset: f32,
    theme: &Theme,
    drag: &ScrollDrag,
) -> AnyElement {
    let thumb_color = theme.fg_muted().opacity(SCROLL_THUMB_OPACITY);
    let lane = theme.fg_muted().opacity(0.12);

    let press_state = state.clone();
    let press_drag = drag.clone();
    let paint_state = state.clone();
    let move_state = state.clone();
    let move_drag = drag.clone();

    div()
        .id(id)
        .absolute()
        .top(px(inset))
        .bottom(px(inset))
        .right(px(2.0))
        .w(px(SCROLL_BAR_WIDTH))
        .cursor_pointer()
        .hover(move |style| style.bg(lane))
        .child(
            canvas(|_, _, _| (), move |bounds: Bounds<_>, _, window, cx| {
                let (viewport, content, offset) = input_metrics(&paint_state, cx);
                let max = (content - viewport).max(0.0);
                // 和 `scroll_bar` 一样在绘制阶段判断：render 阶段读到的布局
                // 是上一帧的，切换页面时会漏画一帧滚动条。
                if max > 0.5 && viewport > 0.5 {
                    let (_, thumb, travel) = thumb_geometry(viewport, content);
                    let top = (offset / max).clamp(0.0, 1.0) * travel;
                    let bar = Bounds::new(
                        point(bounds.origin.x, bounds.origin.y + px(top)),
                        size(bounds.size.width, px(thumb)),
                    );
                    window.paint_quad(quad(
                        bar,
                        gpui::Corners::all(px(bounds.size.width.as_f32() * 0.5)),
                        thumb_color,
                        gpui::Edges::all(px(0.0)),
                        transparent_black(),
                        BorderStyle::default(),
                    ));
                }

                // 按下：点在轨道空白处，滑块中心先过来；点在滑块上就只接着拖。
                let press = press_state.clone();
                let press_drag = press_drag.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !bounds.contains(&event.position)
                    {
                        return;
                    }
                    let (viewport, content, offset) = input_metrics(&press, cx);
                    let max = (content - viewport).max(0.0);
                    if max <= 0.5 || viewport <= 0.5 {
                        return;
                    }
                    let (_, thumb, travel) = thumb_geometry(viewport, content);
                    let local = (event.position.y.as_f32() - bounds.origin.y.as_f32())
                        .clamp(0.0, viewport);
                    let thumb_top = (offset / max).clamp(0.0, 1.0) * travel;
                    let start = if local >= thumb_top && local <= thumb_top + thumb {
                        offset
                    } else {
                        let target = ((local - thumb * 0.5) / travel.max(1.0)).clamp(0.0, 1.0);
                        let next = target * max;
                        press.update(cx, |input, cx| {
                            input.set_scroll_offset(px(next), cx);
                        });
                        next
                    };
                    press_drag.set(Some((key, event.position.y.as_f32(), start)));
                    window.refresh();
                });

                // 拖动：和 `scroll_bar` 一样挂窗口级监听，指针出了窄条也不断。
                let mover = move_state.clone();
                let move_drag = move_drag.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let Some((start_key, start_y, start_offset)) = move_drag.get() else {
                        return;
                    };
                    if start_key != key {
                        return;
                    }
                    if !event.dragging() {
                        move_drag.set(None);
                        return;
                    }
                    let (viewport, content, _) = input_metrics(&mover, cx);
                    let max = (content - viewport).max(0.0);
                    if max <= 0.5 || viewport <= 0.5 {
                        return;
                    }
                    let (_, _, travel) = thumb_geometry(viewport, content);
                    // 偏移是 `[0, max]`（顶部 0、底部 max）：指针往下走，内容
                    // 跟着往上推，偏移变大——方向和 `scroll_bar` 相反。
                    let moved = event.position.y.as_f32() - start_y;
                    let next = (start_offset + moved * (max / travel.max(1.0))).clamp(0.0, max);
                    mover.update(cx, |input, cx| {
                        input.set_scroll_offset(px(next), cx);
                    });
                    window.refresh();
                });
            })
            .absolute()
            .size_full(),
        )
        .into_any_element()
}

/// 多行输入框的（可见高度、内容高度、离顶部多远）。
fn input_metrics(state: &Entity<InputState>, cx: &App) -> (f32, f32, f32) {
    let input = state.read(cx);
    (
        input.visible_height().as_f32(),
        input.content_height().as_f32(),
        input.distance_from_top().as_f32(),
    )
}

/// 视口高度、滑块高度、滑块可走的距离。
fn thumb_geometry(viewport: f32, content: f32) -> (f32, f32, f32) {
    let viewport = viewport.max(1.0);
    // 内容越长滑块越短，但不短于 32dp：再短就点不中也看不见。
    let thumb = (viewport * viewport / content.max(1.0))
        .max(32.0)
        .min(viewport);
    (viewport, thumb, (viewport - thumb).max(0.0))
}

/// 1dp 发丝线：分隔顶栏与内容区、卡片内的分区。
///
/// 颜色用 `border_subtle()`，和 gpuikit 的 `Separator` 同一个色。
pub fn hairline(theme: &Theme) -> Div {
    div()
        .h(px(1.0))
        .w_full()
        .flex_none()
        .bg(theme.border_subtle())
}

/// 竖发丝线：两侧区域之间的 1dp 分隔。
///
/// 比 [`hairline`] 深一档：侧边栏与内容区是两块**并列**的面板，
/// 深色主题下 `border_subtle()` 的分量不足以让边界立住。
pub fn v_hairline(theme: &Theme) -> Div {
    div().w(px(1.0)).h_full().flex_none().bg(theme.border())
}

/// 胶囊标签：状态色 15% 底 + 同色文字，用于在标题上方标出状态。
///
/// 形状照抄 gpuikit 的 `Badge`（高度 / 内边距 / 圆角 / 字号都取自
/// `ControlScale`），只是把底色从 `accent()` 换成调用方给的状态色——
/// 「未运行」和「运行中」必须是两种颜色，这一点 `Badge` 的四个固定变体
/// 表达不了。
pub fn pill(text: impl Into<SharedString>, color: Hsla, theme: &Theme) -> Div {
    let metrics = theme.control(ControlSize::Medium);
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(metrics.height)
        .px(metrics.padding_x)
        .rounded(metrics.radius)
        .text_size(metrics.text_size)
        .font_weight(WEIGHT_SEMIBOLD)
        .line_height(metrics.line_height)
        .bg(color.opacity(0.15))
        .text_color(color)
        .whitespace_nowrap()
        .child(text.into())
}

/// 首屏卡右侧的流量色点。
pub fn dot(color: Hsla) -> Div {
    div()
        .flex_none()
        .w(px(8.0))
        .h(px(8.0))
        .rounded(px(4.0))
        .bg(color)
}

/// 卡片外壳：surface 底 + 1dp 描边 + 0.5rem 圆角。
///
/// 与 gpuikit 的 `Card` 完全同一套：不再用「底色套一圈描边」的旧做法，
/// 那种双层盒子在深色主题下会露出一圈亮边。
fn stroked(inner: Div, radius: f32, theme: &Theme) -> Div {
    div()
        .flex_none()
        .rounded(px(radius))
        .border_1()
        .border_color(theme.border())
        .bg(theme.surface())
        .overflow_hidden()
        .child(inner)
}

// —— 设置分组（任务管理器的版式）——
//
// 一个分组 = 卡片外一行加粗小标题 + 一张 surface 卡片；卡片里是若干「行」，
// 行与行之间夹一条左右缩进的发丝线。行内部从左到右是：
//
// ```text
// [图标]  标题            [右侧控件]
//         说明文字
// ```
//
// 为什么不用一张卡一行：设置项多了以后，卡片之间的间距会把页面拉得很长，
// 而且每张卡都要单独描边，屏幕上全是框。分组把「同一类」的设置收进一张卡，
// 靠行内的层级（标题 + 灰说明 + 右侧控件）区分，信息密度高得多。

/// 把一个 `Svg` 画出来：尺寸与颜色都落在 `Svg` 自己身上。
///
/// gpui 的 `Svg` 是**叶子元素**，不从父容器继承宽高。外面套一层
/// `w(18).h(18)` 的 `div` 只把盒子撑开，里面的图标量出来仍是 0×0——
/// 于是图标整片「消失」，而布局上看不出任何异常。gpuikit 自己的组件
/// （`icon_button` 的图标、`Select` 的箭头）也都是在 `Svg` 上
/// `.size()` + `.text_color()`，这里照同一个口径来。
pub fn icon(svg: Svg, size: f32, color: Hsla) -> impl IntoElement {
    svg.flex_none().w(px(size)).h(px(size)).text_color(color)
}

/// 行的前导图标。
pub fn row_icon(svg: Svg, theme: &Theme) -> impl IntoElement {
    icon(svg, ROW_ICON, theme.fg_muted())
}

/// 一行设置。
///
/// `leading` 一般是 [`row_icon`]，`trailing` 是控件或 [`row_value`]。
/// 两者都可省略：省略 `trailing` 就是纯说明行（比如日志块）。
pub fn setting_row(
    leading: Option<AnyElement>,
    title: impl Into<SharedString>,
    desc: Option<String>,
    desc_tone: Option<Hsla>,
    trailing: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    let mut texts = div().flex_col().gap(px(SPACE_XS)).flex_1().min_w_0().child(
        div()
            .text_size(px(TEXT_BODY))
            .text_color(theme.fg())
            .child(title.into()),
    );
    if let Some(desc) = desc {
        texts = texts.child(match desc_tone {
            Some(color) => hint_tone(desc, color, theme),
            None => hint(desc, theme),
        });
    }

    let mut row = div()
        .flex()
        .items_center()
        .gap(px(SPACE_M))
        .px(px(ROW_PADDING))
        .py(px(ROW_PADDING));
    if let Some(leading) = leading {
        row = row.child(leading);
    }
    row = row.child(texts);
    if let Some(trailing) = trailing {
        row = row.child(div().flex_none().child(trailing));
    }
    row
}

/// 行与行之间的发丝线，左右缩进一个 [`ROW_PADDING`]，和行内容对齐。
///
/// 用 `border()` 而不是 `border_subtle()`：卡片本身就是 `border()` 的描边，
/// 行分隔再淡一档，深色主题下整张卡会读成一块没有结构的色块。
pub fn row_divider(theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .flex_none()
        .px(px(ROW_PADDING))
        .child(
            div()
                .h(px(1.0))
                .w_full()
                .flex_none()
                .bg(theme.border()),
        )
}

/// 一张分组卡片：surface 底 + 1dp 描边 + 小圆角，行之间夹发丝线。
pub fn group_card(rows: Vec<AnyElement>, theme: &Theme) -> Div {
    let mut children: Vec<AnyElement> = Vec::with_capacity(rows.len() * 2);
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(row_divider(theme).into_any_element());
        }
        children.push(row);
    }

    stroked(
        // `flex_none()` 不能省：卡片内层是 flex 容器里的 auto 项，默认
        // `flex: 0 1 auto` 允许它被压缩。行多、页面挤的时候最后一行会被
        // 压扁，卡片底部就对不上内容高度。
        div().flex_col().flex_none().w_full().children(children),
        GROUP_RADIUS,
        theme,
    )
}

/// 一个分组：标题 + 卡片。
pub fn settings_group(label: impl Into<SharedString>, rows: Vec<AnyElement>, theme: &Theme) -> Div {
    div()
        .flex_col()
        .flex_none()
        .gap(px(SPACE_S))
        .child(group_label(label, theme))
        .child(group_card(rows, theme))
}

/// 占满整行宽度的一块内容（多行输入框、日志窗格、进度条）。
///
/// 标题与说明在上，控件在下：控件需要横向空间时，不能挤在行的右边。
pub fn setting_block(
    title: impl Into<SharedString>,
    desc: Option<String>,
    body: impl IntoElement,
    theme: &Theme,
) -> Div {
    let mut block = div()
        .flex_col()
        .gap(px(SPACE_S))
        .px(px(ROW_PADDING))
        .py(px(ROW_PADDING))
        .child(
            div()
                .text_size(px(TEXT_BODY))
                .text_color(theme.fg())
                .child(title.into()),
        );
    if let Some(desc) = desc {
        block = block.child(hint(desc, theme));
    }
    block.child(body)
}

/// 紧凑行：只有标题 + 右侧控件，没有前导图标、没有第二行说明。
///
/// 给「两三项、标题自己就说得清」的分组用（窗口与托盘、开关类）。
pub fn compact_row(
    title: impl Into<SharedString>,
    trailing: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(SPACE_M))
        .px(px(ROW_PADDING))
        .py(px(ROW_PADDING))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(TEXT_BODY))
                .text_color(theme.fg())
                .child(title.into()),
        );
    if let Some(trailing) = trailing {
        row = row.child(div().flex_none().child(trailing));
    }
    row
}

// —— 操作 ——

/// 主操作按钮：中性实心。
///
/// 不用 accent 填充。accent 是留给「选中 / 焦点 / 数据」的，一整个按钮铺上
/// 去，一屏里最大的色块就是它，比它要执行的操作本身还抢眼——尤其在深色主题
/// 下，亮色块压在暗底上会一直把视线拽回来。
///
/// 主按钮靠**对比度**立起来：`fg()` 作底、`bg()` 作字。深色主题是浅底深字，
/// 浅色主题自动反转成深底浅字，不用判断 `variant`，也不用为两套主题各写一份。
///
/// 尺寸、圆角、字号仍取 `ControlSize::Large` 的 metrics，和 gpuikit 的
/// `Button` 摆在一起看不出是两套实现。
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let metrics = theme.control(ControlSize::Large);
    let face = theme.fg();
    let ink = theme.bg();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(metrics.gap)
        .h(metrics.height)
        .px(metrics.padding_x)
        .rounded(metrics.radius)
        .bg(face)
        .text_size(metrics.text_size)
        .line_height(metrics.line_height)
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(ink)
        .whitespace_nowrap()
        .cursor_pointer()
        // 悬停/按下靠透明度往背景透，明暗两套主题都是「变淡一档」。
        .hover(move |style| style.bg(face.opacity(0.88)))
        .active(move |style| style.bg(face.opacity(0.76)))
        .on_click(handler)
        .child(label.into())
}

/// 行内动作按钮：图标 + 文字，surface 底。
///
/// 用于「新建 / 复制 / 保存 / 检测」这类次要动作：把横向空间留给内容，
/// 同时比纯图标好认。形状与 gpuikit 的 `Button`（Filled）一致——同样的
/// 高度、圆角、字号，同样的 `button_bg()` 三态，只是多了图标位。
pub fn labeled_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Svg,
    theme: &Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    action_button(id, label.into(), Some(icon), theme, handler)
}

/// 纯文字动作按钮：宽度贴着文字，不带图标。
pub fn text_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    action_button(id, label.into(), None, theme, handler)
}

fn action_button(
    id: impl Into<ElementId>,
    label: SharedString,
    svg: Option<Svg>,
    theme: &Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let metrics = theme.control(ControlSize::Large);
    let bg = theme.button_bg();
    let bg_hover = theme.button_bg_hover();
    let bg_active = theme.button_bg_active();
    let mut button = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(metrics.gap)
        .h(metrics.height)
        .px(metrics.padding_x)
        .rounded(metrics.radius)
        .bg(bg)
        .text_size(metrics.text_size)
        .line_height(metrics.line_height)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.fg())
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |style| style.bg(bg_hover))
        .active(move |style| style.bg(bg_active))
        .on_click(handler);
    if let Some(svg) = svg {
        button = button.child(icon(svg, 14.0, theme.fg_muted()));
    }
    button.child(label)
}

/// 图标按钮：只显示图标，文字作为悬停提示与无障碍标签。
///
/// 用于「动作本身一眼就懂」的地方（编辑 / 删除 / 返回 / 清空），
/// 把横向空间留给内容；文字按钮留给有歧义或主操作。
///
/// 外框和文字按钮一样落在 `Large` 档（32dp），图标仍是 16dp 的 `ink`：
/// 一排设置里既有「检测」这样的文字按钮、又有行尾的铅笔，两者高度必须
/// 相等，否则同一行会忽高忽低。
pub fn icon_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Svg,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    icon_button(id, icon)
        .box_size(rems(2.0))
        .icon_size(rems(1.0))
        .tooltip(tooltip(label.into()))
        .on_click(handler)
}

/// 当前不能用的图标按钮：灰一档，点了没反应。
///
/// 比「能点但点了弹一句不行」好：用户不会先点一次才知道这条规则。
/// 提示文字仍然保留，说明**为什么**不行。
pub fn icon_action_off(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Svg,
) -> impl IntoElement {
    icon_button(id, icon)
        .box_size(rems(2.0))
        .icon_size(rems(1.0))
        .disabled(true)
        .tooltip(tooltip(label.into()))
}

/// 勾选框：状态由调用方给，点击由调用方处理。
///
/// 不用 gpuikit 的 `Checkbox` 实体：那个实体自己存一份「勾没勾」，和配置里的
/// 布尔值会各说各话（切配置后就对不上了）。这里只要它**画**，真值永远来自
/// 配置——`checkbox_box` 正是 gpuikit 抽出来的「纯外观」那一层。
///
/// 档位必须是 `Large`：`checkbox_box` 自己就带一个 rung 高度的外框（框里画
/// `ink` 大小的小方块），默认的 `Medium` 只有 20dp，会和同一张卡片里 32dp
/// 的输入框、下拉框差出一整档——一行 44、下一行 56，卡片像被掰过。
pub fn toggle_control(
    id: impl Into<ElementId>,
    on: bool,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .hover(|style| style.opacity(0.75))
        .on_click(handler)
        .child(
            checkbox_box(if on {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            })
            .control_size(ControlSize::Large),
        )
}

/// 把字节数格式化成人类可读的大小。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
