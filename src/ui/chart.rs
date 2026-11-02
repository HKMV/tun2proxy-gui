//! 仪表盘首屏卡右侧的流量折线图。
//!
//! 用 GPUI 的 `canvas` 而不是一排 `div` 画柱状图：折线需要「点与点之间」的
//! 插值，而布局层只认识矩形。`canvas` 能拿到精确的本地坐标，画出来的才是折线。
//!
//! 两条线：下行（强调色）与上行（成功色），下行线下方铺一层很淡的面积色，
//! 让「量」在余光里也能看出来。纵轴按窗口内的峰值自适应——固定比例尺会让
//! 低流量永远贴着底边。
//!
//! 采样是「每秒一个点」，最多 60 个点（最近一分钟）。点不足 2 个时只画基线：
//! 一条平的线会被读成「确实没流量」，而空图 + 灰字「—」才是「还没数据」。

use gpui::{
    AnyElement, Bounds, Hsla, IntoElement, ParentElement, Styled, Window, canvas, div, fill, point,
    px, size,
};
use gpuikit::theme::{Theme, Themeable};

use crate::core::i18n::t;
use crate::core::TrafficSample;

use super::theme::TEXT_BODY;
use super::widgets::{hint, SPACE_XS};

/// 折线图高度（逻辑像素）。
pub const CHART_HEIGHT: f32 = 68.0;

/// 画一张流量折线图。
///
/// `samples` 是「旧 → 新」的采样序列，最新的点画在最右边。
pub fn traffic_chart(samples: &[TrafficSample], width: f32, theme: &Theme) -> AnyElement {
    let width = width.max(24.0);
    // 两条线要能一眼分开。accent 在深色主题里是蓝、上行取 success 的绿，
    // 色相差得够远；同色系的两档（比如 accent + info 都是蓝）在 68dp 高的
    // 缩略图上会糊成一条。
    let accent = theme.accent();
    let upstream = theme.success();
    let border = theme.border();

    // 还没采到点：画一条基线 + 一句说明。
    //
    // 空图必须自己说明白：只有一个方框会被读成「画坏了」。
    if samples.len() < 2 {
        return div()
            .flex_none()
            .w(px(width))
            .h(px(CHART_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(border)
            .child(hint(
                t("启动后显示实时流量", "Live traffic appears after start"),
                theme,
            ))
            .into_any_element();
    }

    let samples = samples.to_vec();
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
            let height = CHART_HEIGHT;
            let baseline = height - 1.0;

            // 基线：一条发丝线，给折线一个「地面」。
            window.paint_quad(fill(
                rect(bounds, 0.0, baseline, width, 1.0),
                border,
            ));

            let peak = samples
                .iter()
                .map(|sample| sample.peak())
                .max()
                .unwrap_or(0)
                .max(1) as f64;
            // 顶部留 2dp：贴着上边缘的线会被画布裁掉半个像素。
            let usable = (height - 3.0).max(1.0) as f64;

            let down: Vec<f32> = samples
                .iter()
                .map(|sample| y_for(sample.down as f64, peak, usable, baseline))
                .collect();
            let up: Vec<f32> = samples
                .iter()
                .map(|sample| y_for(sample.up as f64, peak, usable, baseline))
                .collect();

            // 面积先画（在线下面），否则会把线盖住。
            fill_area(window, bounds, &down, width, baseline, accent);
            stroke(window, bounds, &down, width, accent);
            stroke(window, bounds, &up, width, upstream);
        },
    )
    .flex_none()
    .w(px(width))
    .h(px(CHART_HEIGHT))
    .into_any_element()
}

/// 数值 → 纵坐标（自上而下，0 在顶部）。
fn y_for(value: f64, peak: f64, usable: f64, baseline: f32) -> f32 {
    let ratio = (value / peak).clamp(0.0, 1.0);
    baseline - (ratio * usable) as f32
}

/// 逐列画折线：每列 1dp 宽、1.5dp 高，相邻两点的纵坐标之间线性插值。
///
/// Canvas 没有 polyline 图元，但「每列一个 1dp 宽的小矩形」在屏幕上就是一条
/// 连续的线——而且宽度随画布自适应，不依赖固定的采样点数。
fn stroke(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    ys: &[f32],
    width: f32,
    color: Hsla,
) {
    for (x, y) in columns(ys, width) {
        window.paint_quad(fill(rect(bounds, x, y - 0.75, 1.0, 1.5), color));
    }
}

/// 折线下方的淡色面积。
fn fill_area(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    ys: &[f32],
    width: f32,
    baseline: f32,
    color: Hsla,
) {
    let fill_color = color.opacity(0.14);
    for (x, y) in columns(ys, width) {
        let top = y.min(baseline);
        let height = (baseline - top).max(0.0);
        if height <= 0.0 {
            continue;
        }
        window.paint_quad(fill(rect(bounds, x, top, 1.0, height), fill_color));
    }
}

/// 把采样序列摊成「每一列的横坐标 + 纵坐标」。
fn columns(ys: &[f32], width: f32) -> Vec<(f32, f32)> {
    let count = ys.len();
    if count < 2 {
        return Vec::new();
    }
    let span = (width - 1.0).max(1.0);
    let mut out = Vec::with_capacity(width.round().max(1.0) as usize + 1);
    for index in 0..count - 1 {
        let x0 = index as f32 / (count - 1) as f32 * span;
        let x1 = (index + 1) as f32 / (count - 1) as f32 * span;
        let y0 = ys[index];
        let y1 = ys[index + 1];
        let start = x0.floor() as i32;
        let end = x1.ceil() as i32;
        for x in start..=end {
            let ratio = if x1 > x0 {
                ((x as f32 - x0) / (x1 - x0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            out.push((x as f32, y0 + (y1 - y0) * ratio));
        }
    }
    out
}

/// 画布本地坐标 → 窗口坐标。
fn rect(
    bounds: Bounds<gpui::Pixels>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Bounds<gpui::Pixels> {
    Bounds::new(
        point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
        size(px(width), px(height)),
    )
}

/// 图例上的一个小色块 + 名称。
pub fn legend_swatch(color: Hsla) -> impl IntoElement {
    div().flex_none().w(px(8.0)).h(px(8.0)).rounded(px(2.0)).bg(color)
}

/// 图例文字（灰字）。
///
/// 字号跟正文一致：图例讲的是「哪条线下行、哪条线上行」，是读的内容，不是脚注。
pub fn legend_text(text: impl Into<gpui::SharedString>, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(px(TEXT_BODY))
        .text_color(theme.fg_muted())
        .whitespace_nowrap()
        .child(text.into())
}

/// 图例之间的间距。
pub const LEGEND_GAP: f32 = SPACE_XS;
