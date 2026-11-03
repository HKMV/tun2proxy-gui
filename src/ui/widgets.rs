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
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, IntoElement, SharedString, Stateful, Svg,
    Window, div, px, prelude::*, rems,
};
use gpuikit::elements::checkbox::{CheckState, checkbox_box};
use gpuikit::elements::icon_button::icon_button;
use gpuikit::elements::tooltip::tooltip;
use gpuikit::theme::{ControlSize, Theme, Themeable};
use gpuikit::traits::control_sized::ControlSized;

use super::theme::{
    TEXT_BODY, TEXT_GROUP, TEXT_HERO, TEXT_MONO, TEXT_SMALL, TEXT_TITLE, WEIGHT_SEMIBOLD,
};

/// 日志区高度（逻辑像素）。
///
/// 日志是唯一需要「自己滚」的区域：固定一个高度，页面滚动时它不动，
/// 免得一边翻设置一边把日志推出视野。
pub const LOG_PANE_HEIGHT: f32 = 220.0;

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

/// 错误提示：危险色。
pub fn error_text(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(TEXT_SMALL))
        .text_color(theme.danger())
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
        div().flex_col().w_full().children(children),
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
