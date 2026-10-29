//! 可复用的视图小部件与设计常量。
//!
//! 所有间距、字号、颜色都从这里取，卡片里不再出现散落的魔法数字：
//! 改一处就能让五个页面一起对齐。样式规则只有三条：
//!
//! 1. **间距用 4 的倍数**（[`SPACE_XS`] … [`SPACE_XL`]）；
//! 2. **可换行的文字必须把 wrap 写进样式**（见 [`wrapped`]）；
//! 3. **行内标签一律不换行**（见 [`caption_nowrap`]），否则会被压成竖条。

use zsui::{
    button, column, icon_button, row, spacer, styled_text, toolbar_button, ColorRole, Dp,
    HorizontalAlign, SemanticTextStyle, TextRole, TextWeight, TextWrap, ThemeColorToken, ViewAlign,
    ViewNode, ZsBaseControlMetrics, ZsIcon,
};

/// 内容区滚动容器的控件 id。
///
/// ZSUI 的滚轮事件是「按 id 投递」的：容器没有 id 就收不到滚动，
/// 而且滚动位置也靠 id 在重建之间恢复。
pub const CONTENT_SCROLL_ID: u64 = 0x7475_6e32_0001;

/// 日志区滚动容器的控件 id。
pub const LOG_SCROLL_ID: u64 = 0x7475_6e32_0002;

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
/// 页面留白、卡片之间的间距。
/// 大区块之间的间距。
pub const SPACE_XL: f32 = 24.0;

/// 内容区四周留白。
///
/// 比卡片间距（[`SPACE_L`]）大一圈：卡片与窗口边缘之间要有空气，
/// 卡片才像「浮在底色上」，而不是贴边铺满。
pub const CONTENT_PADDING: f32 = 16.0;

/// 程序图标（64×64 RGBA，从 `assets/brand-64.rgba` 编进来）。
///
/// 编成裸 RGBA 而不是 PNG：ZSUI 的 `image()` 只收解码好的帧，
/// 而 PNG 解码在 ZSUI 里是异步的（`ZsImagePreviewState`），第一帧会是空白。
/// 64×64×4 = 16KB，比拖一个解码器划算。
/// 允许换行的文字样式：在可用宽度内按词换行。
///
/// 注意：`ViewNode::text_wrap()` 只对**多行输入框**生效，对普通文字无效，
/// 换行必须写进 [`SemanticTextStyle::wrap`]，否则文字会一直单行、
/// 超出卡片后被窗口裁掉。
fn wrapped(mut style: SemanticTextStyle) -> SemanticTextStyle {
    style.wrap = TextWrap::Word;
    style
}

// —— 文字 ——
//
// 字号只有四档，改字号时只改这里，不要在各个卡片里单独写 `TextRole`：
//
// | 用途 | 角色 | 字号 |
// | --- | --- | --- |
// | 页标题（顶栏） | `Title` | 28 |
// | 分组标题、品牌名 | `BodyLarge` | 18 |
// | 正文、行标题、说明、胶囊 | `Body` | 14 |
// | 日志 | `Monospace` | 13 |
//
// 说明文字不再用 `Caption`（12）：12px 的次要文字在 125% 缩放下还能读，
// 在 100% 缩放下就偏小了，而界面里最常读的恰好是这些「灰说明」。
// 层级靠颜色（`SecondaryText`）和粗细区分，不再靠 12 与 14 这 2dp 的差距——
// 2dp 的差距读者根本分辨不出来，只会一起显得小。

/// 页面标题（顶栏用）。
///
/// 用 `Title`（28）而不是 `Subtitle`（20）：顶栏要和内容区拉开差距，
/// 否则跟分组标题（18）只差 2dp，层级糊成一片。
pub fn page_title<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(
        text,
        SemanticTextStyle {
            weight: TextWeight::Semibold,
            ..SemanticTextStyle::for_role(TextRole::Title)
        },
    )
}

/// 1dp 发丝线：分隔顶栏与内容区、卡片内的分区。
///
/// ZSUI 没有描边 API，但 `spacer` 支持 `bg`——一条 1dp 高、铺满宽度的
/// 浅灰长条就是视觉上的一条线，比给容器加边框便宜得多。
pub fn hairline<M: 'static>() -> ViewNode<M> {
    spacer()
        .height(Dp::new(1.0))
        .flex(0.0)
        .bg(ThemeColorToken::Border)
}

/// 竖发丝线：两侧区域之间的 1dp 分隔。
///
/// 横向拉伸交给父级 `row` 的 `align(Stretch)`：它会把这条线拉成整列高，
/// 不需要（也不应该）写死高度。
pub fn v_hairline<M: 'static>() -> ViewNode<M> {
    column([])
        .width(Dp::new(1.0))
        .flex(0.0)
        .bg(ThemeColorToken::Border)
        .align(ViewAlign::Stretch)
}

/// 不换行的说明文字：用在「一行里和按钮/开关并排」的场合。
///
/// 可换行的文字在行内会被压缩到很窄（行布局按内在宽度分配），
/// 于是标签被折成竖条，所以行内的标签一律用这个。
///
/// 字号是 `Body`（14）而不是 `Caption`（12）：这个函数产出的都是要读的句子
/// （顶栏右侧的页面说明、实时流量的标签），不是标注。
pub fn caption_nowrap<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(text, SemanticTextStyle::for_role(TextRole::Body))
}

/// 正文大小的次要色文字，用于状态提示。可以换行。
pub fn hint<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(
        text,
        wrapped(SemanticTextStyle {
            color: ColorRole::SecondaryText,
            ..SemanticTextStyle::body()
        }),
    )
}

/// 错误提示：危险色。
pub fn error_text<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(
        text,
        wrapped(SemanticTextStyle {
            color: ColorRole::Danger,
            ..SemanticTextStyle::body()
        }),
    )
}

/// 日志行：等宽字体，方便对齐时间戳与级别。
pub fn log_line<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    // 日志行可能很长（带路径），允许换行，避免撑宽整张卡片。
    styled_text(
        text,
        wrapped(SemanticTextStyle {
            role: TextRole::Monospace,
            color: ColorRole::SecondaryText,
            ..SemanticTextStyle::for_role(TextRole::Monospace)
        }),
    )
}

// —— 容器 ——

/// 胶囊标签：浅底 + 小字，用于在标题上方标出状态（未连接 / 运行中）。
///
/// 两侧用 0 宽度的 `spacer` 撑出水平留白——ZSUI 的 `padding` 是四边等宽的，
/// 直接给 4dp 会让胶囊左右过于局促，看起来不像胶囊而像一块补丁。
///
/// 字用 `Body`（14）：胶囊里的「未运行」「tun2proxy v0.8.6」是要读的状态，
/// 12px 时在侧边栏那一小块灰底上几乎糊成一团。
pub fn pill<M: 'static>(text: impl Into<String>, color: ColorRole) -> ViewNode<M> {
    row([
        spacer().width(Dp::new(SPACE_XS)),
        styled_text(
            text,
            SemanticTextStyle {
                color,
                ..SemanticTextStyle::for_role(TextRole::Body)
            },
        ),
        spacer().width(Dp::new(SPACE_XS)),
    ])
    .gap(Dp::new(0.0))
    .padding(Dp::new(SPACE_XS))
    .radius(Dp::new(999.0))
    // 底色用 `Border` 而不是 `Control`：卡片底色是 `SurfaceRaised`（白），
    // `Control` 也是白调，胶囊贴在卡片上会「看不见」——只有文字浮在半空。
    // `Border` 是可见的浅灰，在白底与侧边栏上都能看清边界。
    .bg(ThemeColorToken::Border)
    .align(ViewAlign::Center)
    .flex(0.0)
}

/// 首屏主卡片：状态胶囊 + 主操作 + 大标题 + 一句说明。
///
/// 仪表盘用这张卡代替普通的页面标题栏——同一块区域既回答「现在什么状态」，
/// 又给出「下一步做什么」，不必让用户在两处之间来回找。
/// 标题必须允许换行：窄窗口下 `TitleLarge` 一行放不下，不换行就会被窗口裁掉。
pub fn hero<M: 'static>(
    badge: ViewNode<M>,
    title: impl Into<String>,
    desc: impl Into<String>,
    action: Option<ViewNode<M>>,
    extras: impl IntoIterator<Item = ViewNode<M>>,
    side: Option<ViewNode<M>>,
) -> ViewNode<M> {
    // 主操作放在文字块下面、靠左：它属于「这一块内容」，不属于窗口。
    // 摆在右上角时，鼠标要先滑到卡片另一端才能启动，视觉上还会把标题和
    // 说明切断；左侧文字列读完就是按钮，动线是一条直线。
    //
    // 文字列必须 `align(Stretch)`：`row` 里的可换行文字按「最宽不可断词」
    // 测量，列里的子项如果是 `Start` 就各自取这个最小宽度，标题会被切在
    // 第一个空格处（“Tun2Proxy 未”）。拉成 Stretch 后文字拿到列的完整宽度，
    // 才能按真正的行宽换行。胶囊不能跟着拉伸，所以用一行包住。
    let mut lines = vec![
        row([badge]).align(ViewAlign::Start),
        styled_text(
            title,
            wrapped(SemanticTextStyle {
                weight: TextWeight::Semibold,
                ..SemanticTextStyle::for_role(TextRole::TitleLarge)
            }),
        ),
        hint(desc),
    ];
    if let Some(action) = action {
        lines.push(row([action]).align(ViewAlign::Start));
    }

    let text = column(lines)
        // 胶囊→标题、标题→说明都留 12dp：比行间距宽一点，三行才不会挤成一段。
        .gap(Dp::new(SPACE_M))
        .align(ViewAlign::Stretch)
        .flex(1.0);

    let mut head = vec![text];
    if let Some(side) = side {
        head.push(side);
    }

    let mut children = vec![row(head).gap(Dp::new(SPACE_XL)).align(ViewAlign::Center)];
    children.extend(extras);

    stroked(
        column(children)
            // 说明行（报错 / 提示 / 权限）与上面那一组之间拉开距离，
            // 否则它们会读成「标题说明的第四行」。
            .gap(Dp::new(SPACE_M))
            .padding(Dp::new(SPACE_XL))
            .radius(Dp::new(CARD_RADIUS - CARD_STROKE))
            .bg(ThemeColorToken::SurfaceRaised)
            .align(ViewAlign::Stretch),
    )
}

/// 一行带图标的提示（权限不足、注意事项）。
///
/// 图标负责一眼分类，文字保持 caption 尺寸，不抢首屏卡的主标题。
///
/// 文字要包一层 `column(...).align(Stretch).flex(1.0)`：`row` 里的可换行文字
/// 按「最宽不可断词」测量，直接放进去会被压成一列竖排的汉字。
pub fn notice<M: 'static>(icon: ZsIcon, tone: ColorRole, text: impl Into<String>) -> ViewNode<M> {
    row([
        zsui::icon(icon)
            .icon_color(tone)
            .width(Dp::new(16.0))
            .height(Dp::new(16.0))
            .min_width(Dp::new(16.0))
            .min_height(Dp::new(16.0))
            .flex(0.0),
        column([hint(text)]).align(ViewAlign::Stretch).flex(1.0),
    ])
    .gap(Dp::new(SPACE_S))
    .align(ViewAlign::Start)
}

/// 卡片圆角。
pub const CARD_RADIUS: f32 = 12.0;
/// 卡片描边宽度。
pub const CARD_STROKE: f32 = 1.0;

/// 给任意节点套一圈 1dp 描边（`Border` 色），圆角用卡片圆角。
fn stroked<M: 'static>(inner: ViewNode<M>) -> ViewNode<M> {
    stroked_with(inner, CARD_RADIUS)
}

/// 给任意节点套一圈 1dp 描边（`Border` 色），圆角可指定。
fn stroked_with<M: 'static>(inner: ViewNode<M>, radius: f32) -> ViewNode<M> {
    column([inner])
        .padding(Dp::new(CARD_STROKE))
        .radius(Dp::new(radius))
        .bg(ThemeColorToken::Border)
        .align(ViewAlign::Stretch)
        .flex(0.0)
}

// —— 设置分组（任务管理器的版式）——
//
// 一个分组 = 卡片外一行加粗小标题 + 一张白卡；卡片里是若干「行」，
// 行与行之间夹一条左右缩进的发丝线。行内部从左到右是：
//
// ```text
// [图标]  标题            [右侧控件]
//         说明文字
// ```
//
// 为什么不用一张卡一行：设置项多了以后，卡片之间的 16dp 间距会把页面拉得很长，
// 而且每张卡都要单独描边，屏幕上全是框。分组把「同一类」的设置收进一张卡，
// 靠行内的层级（标题 + 灰说明 + 右侧控件）区分，信息密度高得多。

/// 行内边距（上下左右等宽）。
///
/// ZSUI 的 `padding` 是四边等宽的，所以这里只能选一个「上下不显肥、左右不显瘦」
/// 的值：14dp 配上 32dp 左右的内容高度，行高约 60dp，和任务管理器相当。
pub const ROW_PADDING: f32 = 14.0;
/// 分组卡片的圆角（比浮动卡片小，分组更像列表）。
pub const GROUP_RADIUS: f32 = 8.0;
/// 行前导图标的尺寸。
pub const ROW_ICON: f32 = 20.0;
/// 行右侧控件的常用宽度（下拉框、输入框）。
///
/// 任务管理器里右侧控件是一列对齐的窄控件，而不是铺满整行：
/// 铺满会让「标题」和「它的值」隔着一整个屏幕，读起来要来回扫。
pub const ROW_CONTROL: f32 = 200.0;

/// 分组标题：卡片外面那行加粗小字。
///
/// `BodyLarge`（18）：分组标题是页面的第二层结构（页标题 28 → 分组 18 → 行 14），
/// 和行标题同为 14 时只能靠「加粗」区分，扫读时几乎看不出来。
pub fn group_label<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(
        text,
        SemanticTextStyle {
            weight: TextWeight::Semibold,
            ..SemanticTextStyle::for_role(TextRole::BodyLarge)
        },
    )
}

/// 行的说明文字：小号、次要色、可换行。
fn row_desc<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    row_desc_tone(text, ColorRole::SecondaryText)
}

/// 带颜色的行说明：检测成功/失败这类结论要能一眼分出颜色。
///
/// 用 `Body`（14）而不是 `Caption`：行说明是设置项的主要阅读内容
/// （「这些地址不走代理」「流量经它转发」），12px 太小。
fn row_desc_tone<M: 'static>(text: impl Into<String>, color: ColorRole) -> ViewNode<M> {
    styled_text(
        text,
        wrapped(SemanticTextStyle {
            color,
            ..SemanticTextStyle::for_role(TextRole::Body)
        }),
    )
}

/// 行的前导图标。
pub fn row_icon<M: 'static>(icon: ZsIcon) -> ViewNode<M> {
    zsui::icon(icon)
        .width(Dp::new(ROW_ICON))
        .height(Dp::new(ROW_ICON))
        .min_width(Dp::new(ROW_ICON))
        .min_height(Dp::new(ROW_ICON))
        .flex(0.0)
}

/// 一行设置。
///
/// `leading` 一般是 [`row_icon`] 或单选圈，`trailing` 是控件或 [`row_value`]。
/// 两者都可省略：省略 `trailing` 就是纯说明行（比如日志块）。
pub fn setting_row<M: 'static>(
    leading: Option<ViewNode<M>>,
    title: impl Into<String>,
    desc: Option<String>,
    trailing: Option<ViewNode<M>>,
) -> ViewNode<M> {
    setting_row_tone(leading, title, desc, None, trailing)
}

/// 和 [`setting_row`] 一样，但可以给说明文字指定颜色（结论类文字用）。
pub fn setting_row_tone<M: 'static>(
    leading: Option<ViewNode<M>>,
    title: impl Into<String>,
    desc: Option<String>,
    desc_tone: Option<ColorRole>,
    trailing: Option<ViewNode<M>>,
) -> ViewNode<M> {
    let mut texts = vec![styled_text(
        title,
        SemanticTextStyle::for_role(TextRole::Body),
    )];
    if let Some(desc) = desc {
        texts.push(match desc_tone {
            Some(color) => row_desc_tone(desc, color),
            None => row_desc(desc),
        });
    }

    let mut children = Vec::with_capacity(3);
    if let Some(leading) = leading {
        children.push(leading);
    }
    // 文字列必须 `Stretch`：`Start` 会让可换行的说明取「最小宽度」（最长单词），
    // 中文没有空格，于是整句只剩前两个字。拉伸后它占满剩余宽度再按宽度换行。
    children.push(
        column(texts)
            .gap(Dp::new(SPACE_XS))
            .align(ViewAlign::Stretch)
            .flex(1.0),
    );
    if let Some(trailing) = trailing {
        children.push(trailing);
    }

    row(children)
        .gap(Dp::new(SPACE_M))
        .padding(Dp::new(ROW_PADDING))
        .align(ViewAlign::Center)
}

/// 行右侧的「值」：右对齐、次要色、单行。
///
/// 值太长时会被压缩——所以调用方要传已经截短的文本（比如代理地址用摘要）。
pub fn row_value<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    styled_text(
        text,
        SemanticTextStyle {
            color: ColorRole::SecondaryText,
            horizontal_align: HorizontalAlign::End,
            ..SemanticTextStyle::body()
        },
    )
    .flex(0.0)
}

/// 行右侧的一组图标按钮。
///
/// 间距用 8 而不是 4：右侧常见的是「一长串数值 + 铅笔」这种组合，
/// 4dp 会让按钮看起来贴在文字上（比如上游代理那行的 `socks5://… · DNS 直连`）。
pub fn row_actions<M: 'static>(children: impl IntoIterator<Item = ViewNode<M>>) -> ViewNode<M> {
    row(children)
        .gap(Dp::new(SPACE_S))
        .align(ViewAlign::Center)
        .flex(0.0)
}

/// 动作行：右侧一排按钮，左边不占文字宽度。
///
/// 带说明文字的「行 + 多个按钮」在小窗口下会把按钮推出卡片（ZSUI 不压缩控件，
/// 直接裁掉），所以按钮多的时候单独占一行，行里只放按钮。
pub fn action_row<M: 'static>(actions: impl IntoIterator<Item = ViewNode<M>>) -> ViewNode<M> {
    let mut children = vec![push()];
    children.extend(actions);
    row(children)
        .gap(Dp::new(SPACE_S))
        .padding(Dp::new(ROW_PADDING))
        .align(ViewAlign::Center)
}

/// 行与行之间的发丝线，左右缩进一个 [`ROW_PADDING`]，和行内容对齐。
pub fn row_divider<M: 'static>() -> ViewNode<M> {
    row([
        spacer().width(Dp::new(ROW_PADDING)).flex(0.0),
        hairline().flex(1.0),
        spacer().width(Dp::new(ROW_PADDING)).flex(0.0),
    ])
    .gap(Dp::new(0.0))
    .align(ViewAlign::Center)
    .flex(0.0)
}

/// 一张分组卡片：白底 + 1dp 描边 + 小圆角，行之间夹发丝线。
pub fn group_card<M: 'static>(rows: impl IntoIterator<Item = ViewNode<M>>) -> ViewNode<M> {
    let mut children = Vec::new();
    for (index, node) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(row_divider());
        }
        children.push(node);
    }

    stroked_with(
        column(children)
            .gap(Dp::new(0.0))
            .radius(Dp::new(GROUP_RADIUS - CARD_STROKE))
            .bg(ThemeColorToken::SurfaceRaised)
            .align(ViewAlign::Stretch),
        GROUP_RADIUS,
    )
}

/// 一个分组：标题 + 卡片。
pub fn settings_group<M: 'static>(
    label: impl Into<String>,
    rows: impl IntoIterator<Item = ViewNode<M>>,
) -> ViewNode<M> {
    column([group_label(label), group_card(rows)])
        .gap(Dp::new(SPACE_S))
        .align(ViewAlign::Stretch)
        .flex(0.0)
}

/// 占满整行宽度的一块内容（多行输入框、日志窗格、进度条）。
///
/// 标题与说明在上，控件在下：控件需要横向空间时，不能挤在行的右边。
pub fn setting_block<M: 'static>(
    title: impl Into<String>,
    desc: Option<String>,
    body: ViewNode<M>,
) -> ViewNode<M> {
    let mut children = vec![styled_text(
        title,
        SemanticTextStyle::for_role(TextRole::Body),
    )];
    if let Some(desc) = desc {
        children.push(row_desc(desc));
    }
    children.push(body);

    column(children)
        .gap(Dp::new(SPACE_S))
        .padding(Dp::new(ROW_PADDING))
        .align(ViewAlign::Stretch)
}
/// 把后面的内容推到行的最右边。
///
/// `spacer()` 的固有宽度是 0，只有 `flex(1.0)` 才会吃掉行里剩下的空间——
/// 裸的 `spacer()` 什么都不会推，标题和右侧按钮会紧挨在一起。
pub fn push<M: 'static>() -> ViewNode<M> {
    spacer().flex(1.0)
}

// —— 操作 ——

/// 图标按钮：只显示图标，文字作为悬停提示与无障碍标签。
///
/// 用于「动作本身一眼就懂」的地方（新建 / 复制 / 删除 / 保存 / 清空），
/// 把横向空间留给内容；文字按钮留给有歧义或主操作。
pub fn icon_action<M: 'static + Clone>(
    label: impl Into<String>,
    icon: ZsIcon,
    message: M,
) -> ViewNode<M> {
    let label = label.into();
    icon_button(label.clone(), icon)
        .tooltip(label)
        .on_click(message)
}

/// 图标 + 文字的工具栏按钮，宽度贴着文字内容。
///
/// `toolbar_button` 的 `min_width` 里含「两个字形的 shaping 余量」
/// （`estimated_text_width_with_shaping_reserve`），那是给西文连笔留的余量，
/// 中文标签用不上；而 Toolbar 的标签是**左对齐**画的，多出来的宽度全落在
/// 按钮右边——看起来就是「按钮比文字宽一截、右侧一片空白」。
/// 这里把那段余量扣掉：图标保留，按钮宽度 = 文字 + 图标 + 内边距。
pub fn labeled_action<M: 'static + Clone>(
    label: impl Into<String>,
    icon: ZsIcon,
    message: M,
) -> ViewNode<M> {
    let label = label.into();
    let node = toolbar_button(label.clone(), icon);
    let reserve = ZsBaseControlMetrics::current().average_character_width.0 * 2.0;
    let node = match node.style.min_width {
        Some(width) => node.min_width(Dp::new((width.0 - reserve).max(0.0))),
        None => node,
    };
    // 不再挂 tooltip：按钮上已经写着这几个字，鼠标一停就弹一个同名气泡，
    // 只会盖住按钮本身。
    node.on_click(message)
}

/// 紧凑行：只有标题 + 右侧控件，没有前导图标、没有第二行说明。
///
/// 给「两三项、标题自己就说得清」的分组用（窗口与托盘、开关类）。
/// 带说明的行会把卡片撑成两倍高，而说明读起来比标题慢——一眼扫过去
/// 能看懂的设置不需要解释。
///
/// 行高跟单行控件（复选框 / 按钮）对齐，所以 `trailing` 尽量给矮控件。
pub fn compact_row<M: 'static>(
    title: impl Into<String>,
    trailing: Option<ViewNode<M>>,
) -> ViewNode<M> {
    let mut children =
        vec![styled_text(title, SemanticTextStyle::for_role(TextRole::Body)).flex(1.0)];
    if let Some(trailing) = trailing {
        children.push(trailing);
    }

    row(children)
        .gap(Dp::new(SPACE_M))
        .padding(Dp::new(ROW_PADDING))
        .align(ViewAlign::Center)
}

/// 纯文字按钮：宽度贴着文字，不带图标。
///
/// [`labeled_action`] 带图标，塞进一行设置里会和标题抢注意力；这一行的动作
/// 本身就写在标题旁边（「最小化到托盘」→ 按钮「最小化」），图标是冗余的。
pub fn text_action<M: 'static + Clone>(label: impl Into<String>, message: M) -> ViewNode<M> {
    button(label).min_width(Dp::new(0.0)).on_click(message)
}

/// 一个小圆点（直径 [DOT_SIZE]），用主题色填。
///
/// 图标栏（48dp）里放不下「色点 + 文字」，只能用色点表达状态。
/// 不用 `badge(Dot)`：那个胶囊是按「带底色的小标签」画的，在窄栏里
/// 会被居中成一个看不见的点，干脆自己画一个实心圆。
pub fn dot<M: 'static>(token: ThemeColorToken) -> ViewNode<M> {
    column([])
        .width(Dp::new(DOT_SIZE))
        .height(Dp::new(DOT_SIZE))
        .radius(Dp::new(DOT_SIZE / 2.0))
        .bg(token)
        .flex(0.0)
}

/// 状态色点的直径。
const DOT_SIZE: f32 = 10.0;

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
