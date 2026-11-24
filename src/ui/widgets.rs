//! 各页共用的小件：卡片外壳、标题行、字段行、输入框、分段控件。
//!
//! 集中在这里不是为了「抽框架」，而是为了让卡片只有一种画法：底色、描边、
//! 圆角、内边距各页各写一遍时，层级很容易叠脏（一个卡片里套一个带描边的
//! 卡片、底色又差一档，边缘就发毛）。改样式只改这一个文件。

use nsh_base::{input, theme};
use nsh_core::prelude::*;

/// 「正常」的颜色：`success` token，没有就退到 `chart-3`。
pub fn ok_color() -> AdaptiveColor {
    theme().color("success").unwrap_or(theme().chart_3)
}

/// 「需注意」的颜色：`warning` token，没有就退到 `chart-4`。
pub fn warn_color() -> AdaptiveColor {
    theme().color("warning").unwrap_or(theme().chart_4)
}

/// 「点一下就好」的提示色：`info` token，没有就退到 `chart-2`。
pub fn info_color() -> AdaptiveColor {
    theme().color("info").unwrap_or(theme().chart_2)
}

/// 「危险 / 停止」的颜色：`destructive` token，没有就退到 `chart-4`。
pub fn danger_color() -> AdaptiveColor {
    theme().color("destructive").unwrap_or(theme().chart_4)
}

/// 「直连」的颜色：主题里的 `direct` token，没有就退到 `chart-1`。
pub fn direct_color() -> AdaptiveColor {
    theme().color("direct").unwrap_or(theme().chart_1)
}

/// 「代理」的颜色：主题里的 `proxy` token，没有就退到 `chart-2`。
pub fn proxy_color() -> AdaptiveColor {
    theme().color("proxy").unwrap_or(theme().chart_2)
}

/// 一张卡片的外壳：底色、描边、圆角、内边距，纵向排布。
pub fn card_shell() -> View {
    view()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_xl()
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
}

/// 卡片标题行：标题 + 一句说明 + 右侧动作区。
pub fn card_head(title: &'static str, hint: Option<String>, actions: impl IntoElement) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            view()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(
                    text(title)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().card_foreground),
                )
                .child(
                    text(hint.unwrap_or_default())
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(actions)
}

/// 字段行：左边标签，右边控件 + 一句说明。
///
/// 说明放在**控件下面**而不是标签下面：标签列只有 140dp，一句中文说明会被折
/// 成三行，把整行的行高撑开、右边的输入框跟着上下不齐。放在控件下面能占满
/// 剩余宽度，一行说完。
pub fn field(label: &'static str, hint: Option<String>, control: impl IntoElement) -> View {
    field_with(label, hint, control, true)
}

/// 和 [`field`] 一样，但说明**不截断**：允许多行。
///
/// 给那些「一句说完会丢信息」的说明用（比如内置清单每一组到底包含什么）。
/// 截断的单行说明适合短提示，长文案被切一半比换行更难看懂。
pub fn field_wrapped(label: &'static str, hint: Option<String>, control: impl IntoElement) -> View {
    field_with(label, hint, control, false)
}

fn field_with(
    label: &'static str,
    hint: Option<String>,
    control: impl IntoElement,
    truncate_hint: bool,
) -> View {
    let mut hint_text = text(hint.unwrap_or_default())
        .text_xs()
        .text_color(move || theme().muted_foreground);
    if truncate_hint {
        hint_text = hint_text.truncate();
    }
    view()
        .flex_row()
        .items_start()
        .gap_4()
        .child(
            view().w(px(140.0)).pt_1().child(
                text(label)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().secondary_foreground),
            ),
        )
        .child(
            view()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(control)
                .child(hint_text),
        )
}

/// 单行输入框的高度（4 的倍数，125% 缩放下不会发毛）。
const INPUT_HEIGHT: f32 = 28.0;

/// 单行文本输入。
///
/// nsh 的 `input` 是无样式的宿主元素，描边和底色由外面这层 `view` 给；输入框
/// 自己只负责文本、光标和选区。
///
/// `enabled = false` 时用 `readonly` 而不是只画成灰的：灰着还能改的框，用户
/// 改完发现不生效会更糊涂。
pub fn text_field(
    value: impl Fn() -> String + 'static,
    placeholder: &'static str,
    enabled: bool,
    on_change: impl Fn(String) + 'static,
) -> View {
    let mut area = input()
        .w_full()
        .h(px(INPUT_HEIGHT - 8.0))
        .text_xs()
        .text_color(move || {
            if enabled {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        })
        .placeholder(placeholder)
        .readonly(!enabled)
        .value(value);
    if enabled {
        area = area.on_change(move |text: &str| on_change(text.to_string()));
    }
    field_box(enabled).child(area)
}

/// 多行文本输入（绕过地址那种一行一条的列表）。
pub fn text_area(
    value: impl Fn() -> String + 'static,
    placeholder: &'static str,
    height: f32,
    on_change: impl Fn(String) + 'static,
) -> View {
    view()
        .h(px(height))
        .w_full()
        .p_2()
        .rounded_lg()
        .bg(move || theme().background)
        .border_1()
        .border_color(move || theme().border)
        .child(
            input()
                .w_full()
                .h(px(height - 16.0))
                .text_xs()
                .text_color(move || theme().foreground)
                .placeholder(placeholder)
                .multiline(true)
                .value(value)
                .on_change(move |text: &str| on_change(text.to_string())),
        )
}

/// 输入框外面那层：底色比卡片深一档，用描边把边界说清楚。
fn field_box(enabled: bool) -> View {
    view()
        .h(px(INPUT_HEIGHT))
        .w_full()
        .px_2()
        .rounded_lg()
        .bg(move || {
            if enabled {
                theme().background
            } else {
                theme().muted
            }
        })
        .border_1()
        .border_color(move || theme().border)
        .items_center()
}

/// 分段控件里的一格（选中的那格用主色实心）。
pub fn chip(label: &'static str, chosen: bool, on_press: impl Fn() + 'static) -> View {
    let mut chip = view()
        .h(px(20.0))
        .px_2()
        .rounded_full()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .transition_colors()
        .duration(140.0)
        .ease_out()
        .bg(move || {
            if chosen {
                theme().primary
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .child(
            text(label)
                .text_xs()
                .font_weight(if chosen {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(move || {
                    if chosen {
                        theme().primary_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        );
    if !chosen {
        chip = chip.hover(|this| this.bg(theme().accent));
    }
    chip.on_press(on_press)
}

/// 分段控件的容器：一排 chip 装在一个胶囊里。
pub fn chips(children: impl IntoIterator<Item = View>) -> View {
    view()
        .h(px(24.0))
        .flex_row()
        .items_center()
        .gap_1()
        .p_1()
        .rounded_full()
        // 只包住里面的 chip：撑满整行会在右边留一条空灰条，浅色主题下尤其显脏。
        .self_start()
        .bg(move || theme().muted)
        .children(children)
}

/// 次要按钮：只有描边，没有底色。
pub fn ghost_button(label: &'static str, on_press: impl Fn() + 'static) -> View {
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

/// 次要按钮的可禁用版本：禁用时不接点击、颜色变灰、不给悬停反馈。
///
/// 「灰着但还能点」是最容易让人点半天没反应的做法，所以禁用时连 `on_press`
/// 都不挂——点了什么也不会发生。
pub fn ghost_button_maybe(
    label: &'static str,
    enabled: bool,
    on_press: impl Fn() + 'static,
) -> View {
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
        button = button
            .cursor_pointer()
            .transition_colors()
            .duration(140.0)
            .ease_out()
            .hover(|this| this.bg(theme().accent))
            .active(|this| this.scale(0.96))
            .on_press(on_press);
    }
    button
}

/// 主按钮：实色底。
pub fn primary_button(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .h(px(28.0))
        .px_4()
        .rounded_full()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .transition_colors()
        .transition_transform()
        .duration(140.0)
        .ease_out()
        .hover(|this| this.scale(1.03))
        .active(|this| this.scale(0.96))
        .bg(move || theme().primary)
        .on_press(on_press)
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// 开关：一条胶囊，圆点靠左（关）或靠右（开）。
///
/// 没有用「开 / 关」两个字的按钮：这一页有几处开关是**组**级别的（三组内置
/// 清单），一排「开」字读下来分不清哪个是哪个，圆点的位置一眼就能扫。
pub fn toggle(on: bool, on_press: impl Fn() + 'static) -> View {
    view()
        .w(px(40.0))
        .h(px(22.0))
        .p(px(3.0))
        .flex_row()
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .transition_colors()
        .transition_transform()
        .duration(160.0)
        .ease_out()
        .active(|this| this.scale(0.94))
        .bg(move || {
            if on {
                theme().primary
            } else {
                // 关态用 `border` 而不是 `muted`：`muted` 在深色主题下与卡片底
                // 几乎同色，胶囊会整个消失，只剩一个飘着的灰点。
                theme().border
            }
        })
        .justify_start()
        .when(on, |this| this.justify_end())
        .on_press(on_press)
        .child(
            view()
                .w(px(16.0))
                .h(px(16.0))
                .rounded_full()
                .bg(move || {
                    if on {
                        theme().primary_foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

/// 一行右对齐的操作按钮（保存这类动作用）。
///
/// **别把按钮放进 `card_head` 的 actions**：页面滚动之后，卡片头那一行的命中
/// 区还停在没滚动时的位置，点了没反应（内容流里的控件不受影响）。按钮放在
/// 输入框下面既能点得动，也更符合「改完再保存」的顺序。
pub fn action_row(buttons: Vec<View>) -> View {
    view()
        .flex_row()
        .items_center()
        .justify_end()
        .gap_2()
        .children(buttons)
}

/// 一句提示条（错误红、成功绿、普通灰）。
pub fn note(text_value: String, tone: AdaptiveColor) -> View {
    view().child(text(text_value).text_xs().text_color(move || tone))
}

/// 一次保存的结论：绿色是「存下了」，红色是「没存」。
#[derive(Clone, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    pub bad: bool,
}

impl Note {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bad: false,
        }
    }

    pub fn bad(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bad: true,
        }
    }
}

/// 把一次保存的结论画成一行（没有结论时什么都不画）。
pub fn note_result(value: Option<Note>) -> View {
    match value {
        Some(result) => note(
            result.text,
            if result.bad {
                danger_color()
            } else {
                ok_color()
            },
        ),
        None => view(),
    }
}
