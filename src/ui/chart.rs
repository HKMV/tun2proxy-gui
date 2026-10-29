//! 仪表盘首屏卡右侧的流量折线图。
//!
//! 用 ZSUI 的 Canvas 而不是一排 `ViewNode` 画柱状图：折线需要「点与点之间」的
//! 插值，而布局层只认识矩形。Canvas 能拿到精确的本地坐标，画出来的才是折线。
//!
//! 两条线：下行（[`ColorRole::Accent`]）与上行（[`ColorRole::Success`]），
//! 下行线下方铺一层很淡的面积色，让「量」在余光里也能看出来。
//! 纵轴按窗口内的峰值自适应——固定比例尺会让低流量永远贴着底边。
//!
//! 采样是「每秒一个点」，最多 60 个点（最近一分钟）。点不足 2 个时只画基线：
//! 一条平的线会被读成「确实没流量」，而空图 + 灰字「—」才是「还没数据」。

use zsui::{
    canvas, ColorRole, Dp, NativeDrawFill, SemanticTextStyle, TextRole, ViewNode,
    ZsCanvasPrimitive, ZsCanvasRect, ZsCanvasScene,
};

use crate::core::i18n::t;
use crate::core::TrafficSample;

/// 折线图高度（逻辑像素）。
pub const CHART_HEIGHT: f32 = 68.0;

/// 画一张流量折线图。
///
/// `samples` 是「旧 → 新」的采样序列，最新的点画在最右边。
pub fn traffic_chart<M: 'static>(samples: &[TrafficSample], width: f32) -> ViewNode<M> {
    let width = width.max(24.0);
    let scene = build_scene(samples, width, CHART_HEIGHT);
    canvas(scene)
        .width(Dp::new(width))
        .height(Dp::new(CHART_HEIGHT))
        .flex(0.0)
}

fn build_scene(samples: &[TrafficSample], width: f32, height: f32) -> ZsCanvasScene {
    let mut scene = ZsCanvasScene::new();
    let baseline = height - 1.0;

    // 基线：一条发丝线，给折线一个「地面」。
    scene.push(ZsCanvasPrimitive::fill_rect(
        rect(0.0, baseline, width, 1.0),
        NativeDrawFill::Role(ColorRole::Border),
    ));

    if samples.len() < 2 {
        // 空图必须自己说明白：只有一条基线的方框会被读成「画坏了」，
        // 一行灰字才能说清「这里将来会有东西」。
        scene.push(ZsCanvasPrimitive::text(
            t("启动后显示实时流量", "Live traffic appears after start"),
            rect(0.0, (height - 16.0) / 2.0, width, 16.0),
            SemanticTextStyle {
                color: ColorRole::SecondaryText,
                ..SemanticTextStyle::for_role(TextRole::Body)
            },
        ));
        return scene;
    }

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
    fill_area(&mut scene, &down, width, baseline);
    stroke(
        &mut scene,
        &down,
        width,
        NativeDrawFill::Role(ColorRole::Accent),
    );
    stroke(
        &mut scene,
        &up,
        width,
        NativeDrawFill::Role(ColorRole::Success),
    );
    scene
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
fn stroke(scene: &mut ZsCanvasScene, ys: &[f32], width: f32, fill: NativeDrawFill) {
    for (x, y) in columns(ys, width) {
        scene.push(ZsCanvasPrimitive::fill_rect(
            rect(x, y - 0.75, 1.0, 1.5),
            fill,
        ));
    }
}

/// 折线下方的淡色面积。
fn fill_area(scene: &mut ZsCanvasScene, ys: &[f32], width: f32, baseline: f32) {
    let fill = NativeDrawFill::RoleWithAlpha {
        role: ColorRole::Accent,
        alpha: 26,
    };
    for (x, y) in columns(ys, width) {
        let top = y.min(baseline);
        let height = (baseline - top).max(0.0);
        if height <= 0.0 {
            continue;
        }
        scene.push(ZsCanvasPrimitive::fill_rect(
            rect(x, top, 1.0, height),
            fill,
        ));
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

fn rect(x: f32, y: f32, width: f32, height: f32) -> ZsCanvasRect {
    ZsCanvasRect::new(Dp::new(x), Dp::new(y), Dp::new(width), Dp::new(height))
}

/// 图例上的一个小方块 + 名称（给 `cards.rs` 拼图例用）。
pub fn legend_swatch<M: 'static>(role: ColorRole) -> ViewNode<M> {
    // 直接用 Canvas 画 8×8 的圆角块：比找一个「纯色方块」图标稳定。
    let mut scene = ZsCanvasScene::new();
    scene.push(ZsCanvasPrimitive::round_fill(
        rect(0.0, 0.0, 8.0, 8.0),
        NativeDrawFill::Role(role),
        Dp::new(2.0),
    ));
    canvas(scene)
        .width(Dp::new(8.0))
        .height(Dp::new(8.0))
        .flex(0.0)
}

/// 图例文字（灰字）。
///
/// 字号跟正文一致（`Body` 14）：图例讲的是「哪条线下行、哪条线上行」，
/// 是读的内容，不是脚注。
pub fn legend_text<M: 'static>(text: impl Into<String>) -> ViewNode<M> {
    zsui::styled_text(
        text,
        SemanticTextStyle {
            color: ColorRole::SecondaryText,
            ..SemanticTextStyle::for_role(TextRole::Body)
        },
    )
}
