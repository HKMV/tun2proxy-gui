//! 仪表盘首屏卡右侧的流量折线图。
//!
//! 用 GPUI 的 `canvas` 而不是一排 `div` 画柱状图：折线需要「点与点之间」的
//! 插值，而布局层只认识矩形。`canvas` 能拿到精确的本地坐标，画出来的才是折线。
//!
//! gpuikit 没有图表组件，所以这里不另起一套配色和尺度，而是**全部走它的
//! token**：高度挂到 `ControlScale`（3 个 Large 控件高），网格线用
//! `border_subtle()`（比卡片轮廓轻一档，是「卡片内部」的语义色），两条色带
//! 用 `success()` 与 `accent()`，图例字号用 12 档、色块尺寸跟着控件圆角走。
//! 换主题时图表跟着换，不会变成界面里唯一一块「别人家的图」。
//!
//! # 两张图：下行一张、上行一张
//!
//! 流量本身只有一个总数，但用户真正想知道的是「这些字节走的是直连还是代理」，
//! 而且上下行的量级差得很远（下到几 MB/s，上常常只有几十 KB/s）。所以拆成
//! **两个方向的图**，每张图里再画**堆叠**的两条带：直连（成功色）贴着基线，
//! 代理（强调色）叠在它上面，两层合起来就是这个方向这一秒的速率。
//!
//! 合成一张图行不行？不行：上下行共用一个纵轴时，上行那条会被压成贴着底边的
//! 一条线，等于没画；而堆成四层（直连下/直连上/代理下/代理上）又会把「哪条路
//! 跑得多」这个主信息埋进四段色带里。两张图各自按自己的峰值自适应，上行哪怕
//! 只有 40 KB/s 也占满一整张图的高度。
//!
//! 纵轴按窗口内的峰值自适应——固定比例尺会让低流量永远贴着底边。
//!
//! 采样是「每秒一个点」，最多 120 个点（最近两分钟）。点不足 2 个时只画基线：
//! 一条平的线会被读成「确实没流量」，而空图 + 灰字「—」才是「还没数据」。

use gpui::{
    AnyElement, Bounds, Div, Hsla, IntoElement, ParentElement, PathBuilder, Styled, Window, canvas,
    div, fill, point, px, size,
};
use gpuikit::theme::{ControlSize, Theme, Themeable};

use crate::core::i18n::t;
use crate::core::split::TrendBucket;
use crate::core::RouteTraffic;

use super::theme::{CONTROL_HEIGHT_LARGE, TEXT_SMALL};
use super::widgets::{hint, human_bytes, SPACE_S, SPACE_XS};

/// 流量图里**一个方向**（下行 / 上行）的高度：2.5 个控件高（32 × 2.5 = 80）。
///
/// 原来是「上下行合计」的一张 96dp 图。拆成两张之后各自收一档：两张叠起来
/// 不至于把卡片顶得太高，80dp 里两条带也仍然分得清。
///
/// 和控件同源（见 [`CONTROL_HEIGHT_LARGE`]），旁边按钮的高度改一档时图表
/// 跟着改，不会出现「图表比按钮矮半头」这种只有肉眼能发现的对不齐。
pub const TRAFFIC_CHART_HEIGHT: f32 = CONTROL_HEIGHT_LARGE * 2.5;

/// 分流趋势图的高度：比流量缩略图高一档。
///
/// 流量图是首屏卡右侧的「读数」，只占 208dp 宽；分流趋势画在整行宽的卡片里，
/// 用同样的高度会显得又扁又空。
pub const SPLIT_CHART_HEIGHT: f32 = CONTROL_HEIGHT_LARGE * 4.0;

/// 空态高度：没数据时只放一句说明，占满 [`TRAFFIC_CHART_HEIGHT`] 会空出一大片。
///
/// 两张图的空态用同一个高度，取「一个控件高 + 上下各一档 [`SPACE_S`]」：
/// 一句话居中看着就是个提示条，而不是一张画坏了的空图。有数据后高度换成
/// 各自的图表高度，这个跳动只在启动那一刻发生一次。
pub const EMPTY_CHART_HEIGHT: f32 = CONTROL_HEIGHT_LARGE + SPACE_S * 2.0;

/// 画一张分流去向折线图：直连 / 走代理 / 失败三条线。
///
/// 和流量图共用同一套画法（[`stroke`] / [`columns`]），只是**宽度不传参**：
/// 它铺满所在卡片的整行，横坐标在 paint 阶段从 `bounds` 里取。传一个假想
/// 宽度再让画布自适应，线就会跟容器对不上（右边留一截空白）。
///
/// 三条线：直连（成功色）、走代理（强调色）、失败（危险色）。失败通常贴着
/// 底线——这正是要的：**平贴底线才说明没出事**，一旦翘起来一眼就能看见。
pub fn split_trend_chart(buckets: &[TrendBucket], theme: &Theme) -> AnyElement {
    let accent = theme.accent();
    let direct = theme.success();
    let failed = theme.danger();
    let border = theme.border();
    let grid = theme.surface_tertiary();

    // 没有数据时不画空框：一条贴着底线的平线会被读成「确实一条都没直连」，
    // 而此刻其实是「还没开始」。
    if buckets.iter().all(TrendBucket::is_empty) {
        return div()
            .flex_none()
            .w_full()
            .h(px(EMPTY_CHART_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(border)
            .child(hint(
                t("最近 2 分钟没有连接经过分流器", "No split decisions in the last 2 minutes"),
                theme,
            ))
            .into_any_element();
    }

    // 三条线共用一个纵轴：分成三个刻度的话，「两条线交叉」就再也说明不了
    // 任何事。上限在外面算，右边的刻度要用它。
    let peak = buckets
        .iter()
        .map(TrendBucket::peak)
        .max()
        .unwrap_or(0)
        .max(1);
    // 刻度取「上限 / 一半 / 0」三档，画布里的网格线也照着这份刻度画：线上的
    // 数字和线本身必须同源，否则「7 条」会标在曲线的半山腰上。
    let scale = ticks(SPLIT_CHART_HEIGHT, peak);
    let entries: Vec<(f32, String)> = scale
        .iter()
        .map(|&(y, value)| {
            // 0 那一档只写「0」：写「0 条」在基线旁边像个多余的单位。
            let text = if value == 0 {
                "0".to_string()
            } else {
                format!("{value} 条")
            };
            (y, text)
        })
        .collect();

    let buckets = buckets.to_vec();
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(px(SPLIT_CHART_HEIGHT))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                    // 右边留出刻度文字的位置，网格线、基线、曲线都不许越界。
                    let width: f32 = bounds.size.width.into();
                    let plot = (width - AXIS_WIDTH).max(8.0);
                    let height = SPLIT_CHART_HEIGHT;
                    let baseline = height - 1.0;

                    let peak = peak as f64;
                    let usable = (height - 3.0).max(1.0) as f64;

                    // 网格线 = 刻度线：0 那一档由下面的基线代表，不再重复画。
                    for (y, _) in &scale {
                        if *y < baseline - 0.5 {
                            window.paint_quad(fill(rect(bounds, 0.0, *y, plot, 1.0), grid));
                        }
                    }
                    window.paint_quad(fill(rect(bounds, 0.0, baseline, plot, 1.0), border));
                    // 时间竖线压在最底下：它只回答「这是第几秒」，不该比曲线显眼。
                    paint_time_guides(window, bounds, plot, baseline, grid.opacity(0.7));

                    // 画法和流量图保持一致：连续折线，主色那条下面垫一层淡面积。
                    //
                    // 之前这里是「稀疏」画法（两端都是 0 的区间直接跳过），窗口里
                    // 大部分格子是空的时候，曲线会变成一根根孤立的竖刺，跟上面那张
                    // 流量图并排放着像两种图表。连续画法更好读：线贴着基线，就是
                    // 「这段时间没有连接经过」，形态和流量图的峰谷是同一套语言。
                    //
                    // 面积只垫在直连那条下面：三条线各垫一层会糊成一块色斑；直连
                    // 是这里的多数派，垫它就够把「高矮」读出来。
                    let ys = |pick: fn(&TrendBucket) -> u64| -> Vec<f32> {
                        buckets
                            .iter()
                            .map(|bucket| y_for(pick(bucket) as f64, peak, usable, baseline))
                            .collect()
                    };
                    let direct_ys = ys(|bucket| bucket.direct);
                    let proxy_ys = ys(|bucket| bucket.proxy);

                    fill_area(window, bounds, &direct_ys, plot, baseline, direct);
                    stroke(window, bounds, &proxy_ys, plot, accent);
                    stroke(window, bounds, &direct_ys, plot, direct);
                    // 失败线最后画，压在上面；整个窗口一条失败都没有时干脆不画——
                    // 红线贴着底线常驻，会让人以为一直在报错。
                    if buckets.iter().any(|bucket| bucket.failed > 0) {
                        let failed_ys = ys(|bucket| bucket.failed);
                        stroke(window, bounds, &failed_ys, plot, failed);
                    }
                },
            )
            .w_full()
            .h(px(SPLIT_CHART_HEIGHT)),
        )
        .child(axis_column(AXIS_WIDTH, SPLIT_CHART_HEIGHT, entries, theme))
        .into_any_element()
}

/// 右侧刻度列的宽度，两张图共用一个值。
///
/// 必须一样宽：两张图现在上下放在同一张卡片里，绘图区宽度一样，时间竖线才会
/// 落在同一个 x 上（`64` 是按最长的一行「12.4 MB/s」定的，「120 条」用它只是
/// 左右多点留白，不是浪费）。
const AXIS_WIDTH: f32 = 64.0;

/// 时间轴上的竖参考线位置（占绘图区宽度的比例）：30 秒 / 60 秒 / 90 秒。
///
/// 两张图的时间窗都是 2 分钟，竖线都按同一组比例画，所以扫一眼就能把「流量
/// 高峰」和「分流高峰」对到同一刻。
const TIME_GUIDES: [f32; 3] = [0.25, 0.5, 0.75];

/// 竖参考线：比横网格再轻一档，只用来读「这是第几秒」。
fn paint_time_guides(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    plot: f32,
    baseline: f32,
    color: Hsla,
) {
    for ratio in TIME_GUIDES {
        let x = (plot * ratio).round();
        window.paint_quad(fill(rect(bounds, x, 0.0, 1.0, baseline), color));
    }
}

/// 画实时流量：下行一张、上行一张，每张各自堆叠「直连在下、代理在上」。
///
/// `samples` 是「旧 → 新」的采样序列，最新的点画在最右边。宽度跟着容器走
/// （和分流趋势图一样），几张图的时间轴才能对齐。
pub fn traffic_chart(samples: &[RouteTraffic], theme: &Theme) -> AnyElement {
    // 还没采到点：一句话说明，别画两个空框。
    if samples.len() < 2 {
        return div()
            .flex_none()
            .w_full()
            .h(px(EMPTY_CHART_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(theme.border())
            .child(hint(
                t("启动后显示实时流量", "Live traffic appears after start"),
                theme,
            ))
            .into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(SPACE_S))
        .child(direction_chart(samples, Direction::Down, theme))
        .child(direction_chart(samples, Direction::Up, theme))
        .into_any_element()
}

/// 图上的一个方向。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Up,
}

impl Direction {
    /// 采样里这个方向、这条路的速率。
    fn rate(self, sample: RouteTraffic, direct: bool) -> u64 {
        let lane = if direct { sample.direct } else { sample.proxy };
        match self {
            Direction::Down => lane.down,
            Direction::Up => lane.up,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Direction::Down => t("下行", "Down"),
            Direction::Up => t("上行", "Up"),
        }
    }
}

/// 一个方向的小图：一行标签 + 堆叠色带 + 右侧刻度。
///
/// 两张图的画法完全一样，只有取值方向不同，所以这里只写一遍——两份几乎一样
/// 的画布代码，改一处忘一处的时候就是对不齐的时候。
fn direction_chart(samples: &[RouteTraffic], direction: Direction, theme: &Theme) -> AnyElement {
    // 两条带要能一眼分开，还要和正下方那张「连接去向」图用同一套颜色语言：
    // 绿色 = 直连，强调色 = 代理。换个色系（比如两个蓝），读者就得记住
    // 「哪张图的绿是哪条路」。
    let direct = theme.success();
    let proxy = theme.accent();
    let border = theme.border();
    // 参考线取 `surface_tertiary()`：比卡片底亮两档，1dp 画上去刚好看得见，
    // 又不至于像网格纸。`border_subtle()` 和卡片同色（它就是「卡片内部不画
    // 线」的语义），`border()` 又只比卡片亮一档，两者画上去都等于没画。
    let grid = theme.surface_tertiary();

    // 纵轴上限只算**这一个方向**：下行几 MB/s、上行几十 KB/s 是常态，共用一个
    // 上限的话上行那张永远是一条贴底的线。
    let peak = samples
        .iter()
        .map(|sample| direction.rate(*sample, true) + direction.rate(*sample, false))
        .max()
        .unwrap_or(0)
        .max(1);

    // 刻度取「上限 / 一半 / 0」：小图上再多一条就成了网格纸。
    let scale = ticks(TRAFFIC_CHART_HEIGHT, peak);
    let entries: Vec<(f32, String)> = scale
        .iter()
        .map(|&(y, value)| {
            let text = if value == 0 {
                "0".to_string()
            } else {
                format!("{}/s", human_bytes(value))
            };
            (y, text)
        })
        .collect();

    // 标签右边那个读数是「此刻这个方向一共多快」。
    //
    // 两张图各自按自己的峰值定纵轴，光看刻度要凑近了才能发现比例尺不是一套；
    // 把读数写在标签后面，一眼就是「这张图画的是现在 1.5 MB/s 这个量级」，
    // 也顺手把「两个方向分着画」这件事说清楚了。
    let current = samples.last().copied().unwrap_or_default();
    let current_rate = direction.rate(current, true) + direction.rate(current, false);

    let samples = samples.to_vec();
    let grid_ticks = scale.clone();
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(SPACE_XS))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .w_full()
                .child(hint(direction.label(), theme))
                .child(
                    div()
                        .text_size(px(TEXT_SMALL))
                        .text_color(theme.fg())
                        .whitespace_nowrap()
                        .child(format!("{}/s", human_bytes(current_rate))),
                ),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .w_full()
                .h(px(TRAFFIC_CHART_HEIGHT))
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                            let width: f32 = bounds.size.width.into();
                            // 画图只占左边那一块，右边留给刻度文字；网格线和基线也跟着缩。
                            let plot = (width - AXIS_WIDTH).max(8.0);
                            let height = TRAFFIC_CHART_HEIGHT;
                            let baseline = height - 1.0;

                            let peak = peak as f64;
                            // 顶部留 2dp：贴着上边缘的线会被画布裁掉半个像素。
                            let usable = (height - 3.0).max(1.0) as f64;

                            // 水平参考线跟着刻度走（上限那档就是曲线的最高点），0 那档
                            // 由下面的基线代表。只看折线能看出「有起伏」，看不出「起伏
                            // 多大」；线比外轮廓轻一档，不抢折线。
                            for (y, _) in &grid_ticks {
                                if *y < baseline - 0.5 {
                                    window.paint_quad(fill(rect(bounds, 0.0, *y, plot, 1.0), grid));
                                }
                            }

                            // 基线：一条发丝线，给折线一个「地面」。
                            window.paint_quad(fill(rect(bounds, 0.0, baseline, plot, 1.0), border));
                            paint_time_guides(window, bounds, plot, baseline, grid.opacity(0.7));

                            let direct_ys: Vec<f32> = samples
                                .iter()
                                .map(|sample| {
                                    y_for(direction.rate(*sample, true) as f64, peak, usable, baseline)
                                })
                                .collect();
                            let total_ys: Vec<f32> = samples
                                .iter()
                                .map(|sample| {
                                    let both = direction.rate(*sample, true)
                                        + direction.rate(*sample, false);
                                    y_for(both as f64, peak, usable, baseline)
                                })
                                .collect();

                            // 先铺两条带（在线下面，否则会把线盖住）：直连贴地，代理
                            // 从直连的顶边填到总量的顶边。这样高矮和面积都能读，
                            // 「大半走代理」一眼就看得出来。
                            fill_area(window, bounds, &direct_ys, plot, baseline, direct);
                            fill_band(window, bounds, &total_ys, &direct_ys, plot, proxy);
                            // 两条交界线各描一道：代理那条其实是「总量」的轮廓。
                            stroke(window, bounds, &total_ys, plot, proxy);
                            stroke(window, bounds, &direct_ys, plot, direct);
                        },
                    )
                    .w_full()
                    .h(px(TRAFFIC_CHART_HEIGHT)),
                )
                .child(axis_column(AXIS_WIDTH, TRAFFIC_CHART_HEIGHT, entries, theme)),
        )
        .into_any_element()
}

/// 纵轴某个比例对应的画布纵坐标。
///
/// 和画布里那几条参考线共用同一套几何（基线 1dp、顶部留 2dp），否则刻度文字
/// 会和它标的那条线差半个字高。
fn axis_y(height: f32, ratio: f32) -> f32 {
    let baseline = height - 1.0;
    let usable = (height - 3.0).max(1.0);
    baseline - usable * ratio
}

/// 右侧的刻度列：`(纵坐标, 文字)`，坐标按画布的本地坐标给。
///
/// 文字画在画布**外面**（同一个相对定位盒子里绝对定位），而不是往画布里挤：
/// canvas 里画字要自己算字形，字体一换就歪；用 `div` 排出来的字跟着主题走。
///
/// 首尾两档的纵坐标会贴着画布的上下边缘（上限在 y≈2、0 在 y=height-1），
/// 居中对齐会让字溢出盒子一半，所以按 `height` 把它推回盒子里。
fn axis_column(width: f32, height: f32, entries: Vec<(f32, String)>, theme: &Theme) -> Div {
    // 12px 的字实际占 14dp 左右，用它当夹取的正文字高。
    let label_height = 14.0;
    let mut column = div()
        .absolute()
        .top_0()
        .right_0()
        .w(px(width))
        .h_full();
    for (y, text) in entries {
        let top = (y - label_height / 2.0).clamp(0.0, (height - label_height).max(0.0));
        column = column.child(
            div()
                .absolute()
                .right_0()
                // 竖直居中到那条线上。
                .top(px(top))
                .text_size(px(TEXT_SMALL))
                .text_color(theme.fg_muted())
                .whitespace_nowrap()
                .child(text),
        );
    }
    column
}

/// 纵轴的刻度：「上限 / 一半 / 0」三档，`(纵坐标, 数值)` 自上而下。
///
/// 按**数值**去重：连接数的峰值是 1 条时，「一半」舍入之后还是 1，两档刻度会
/// 叠在一起，右边就会写出两个「1 条」（旧版按固定比例 75% / 50% / 0 取整，
/// 峰值是 1 时三档全变 0，整列成了「0 条 / 0 条 / 0」）。
fn ticks(height: f32, peak: u64) -> Vec<(f32, u64)> {
    let peak = peak.max(1);
    let mut out: Vec<(f32, u64)> = Vec::with_capacity(3);
    for value in [peak, peak / 2, 0] {
        if out.iter().any(|&(_, seen)| seen == value) {
            continue;
        }
        let ratio = value as f32 / peak as f32;
        out.push((axis_y(height, ratio), value));
    }
    out
}

/// 数值 → 纵坐标（自上而下，0 在顶部）。
fn y_for(value: f64, peak: f64, usable: f64, baseline: f32) -> f32 {
    let ratio = (value / peak).clamp(0.0, 1.0);
    baseline - (ratio * usable) as f32
}

/// 折线本身：把采样点连成一条路径，交给光栅器描边。
///
/// 之前是「一列画一根 1dp 竖条」：竖条的高度取相邻两点的落差，横坐标落在
/// 整数 dp 上。那样画出来是一条**台阶**——斜率一大就一格一格，放大看是
/// 锯齿；每帧还要画上千个四边形。描边路径是一条真正的斜线，斜边由 lyon
/// 自己抗锯齿，没有台阶，也只有一个图元。
fn stroke(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    ys: &[f32],
    width: f32,
    color: Hsla,
) {
    let points = line_points(bounds, ys, width);
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(LINE_WIDTH));
    builder.move_to(points[0]);
    for point in &points[1..] {
        builder.line_to(*point);
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// 采样序列 → 折线上的点（一个采样一个点，窗口坐标）。
fn line_points(
    bounds: Bounds<gpui::Pixels>,
    ys: &[f32],
    width: f32,
) -> Vec<gpui::Point<gpui::Pixels>> {
    let count = ys.len();
    if count < 2 {
        return Vec::new();
    }
    let span = (width - 1.0).max(1.0);
    ys.iter()
        .enumerate()
        .map(|(index, &y)| {
            let x = index as f32 / (count - 1) as f32 * span;
            point(bounds.origin.x + px(x), bounds.origin.y + px(y))
        })
        .collect()
}

/// 折线的线宽（dp）。
///
/// 比 1dp 略粗一点：1dp 在 1.25 倍缩放的屏上正好落在 1.25 个物理像素，
/// 抗锯齿会把整条线摊成 60% 的淡色，看着发灰。
const LINE_WIDTH: f32 = 1.5;

/// 折线下方的淡色面积：折线 + 底线围成的多边形，一次填充。
///
/// 同样是「一列一个方块」的写法换过来的：逐列填充的边缘跟折线一样是台阶，
/// 而且每帧几百个半透明四边形叠在一起，边缘接缝会一点点深下去。
fn fill_area(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    ys: &[f32],
    width: f32,
    baseline: f32,
    color: Hsla,
) {
    let points = line_points(bounds, ys, width);
    if points.len() < 2 {
        return;
    }
    let bottom = bounds.origin.y + px(baseline);
    let mut builder = PathBuilder::fill();
    builder.move_to(points[0]);
    for point in &points[1..] {
        builder.line_to(*point);
    }
    let last = points[points.len() - 1];
    let first = points[0];
    builder.line_to(point(last.x, bottom));
    builder.line_to(point(first.x, bottom));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color.opacity(0.14));
    }
}

/// 两条折线之间的淡色带：堆叠图上层用它（从下层顶边填到本层顶边）。
///
/// `top` 的纵坐标必须比 `bottom` 小（屏幕坐标向下为正，值大的在上面）；
/// 两条线的点数不一致时直接不画——那说明调用方把序列错位了，画出来是一块
/// 歪七扭八的色斑，比不画更难查。
fn fill_band(
    window: &mut Window,
    bounds: Bounds<gpui::Pixels>,
    top: &[f32],
    bottom: &[f32],
    width: f32,
    color: Hsla,
) {
    if top.len() < 2 || top.len() != bottom.len() {
        return;
    }
    let top_points = line_points(bounds, top, width);
    let bottom_points = line_points(bounds, bottom, width);
    let mut builder = PathBuilder::fill();
    builder.move_to(top_points[0]);
    for point in &top_points[1..] {
        builder.line_to(*point);
    }
    // 回程顺着下层顶边倒着走，围成一个闭合的带子。
    for point in bottom_points.iter().rev() {
        builder.line_to(*point);
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color.opacity(0.14));
    }
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

/// 图例上的一个小色块。
///
/// 圆角取 `ControlSize::Small` 的 radius：图例是和 12px 文字并排的小记号，
/// 用 Medium/Large 的圆角会比文字还「重」，在余光里先看到方块而不是数字。
pub fn legend_swatch(color: Hsla, theme: &Theme) -> impl IntoElement {
    let radius = theme.control(ControlSize::Small).radius.0;
    div()
        .flex_none()
        .w(px(8.0))
        .h(px(8.0))
        .rounded(px(radius))
        .bg(color)
}

/// 图例文字（灰字）。
///
/// 字号用 gpuikit 的 12 档：图例是辅助说明，和卡片里的 description 同级；
/// 用 14 会和正文抢位置。
pub fn legend_text(text: impl Into<gpui::SharedString>, theme: &Theme) -> impl IntoElement {
    div()
        .text_size(px(TEXT_SMALL))
        .text_color(theme.fg_muted())
        .whitespace_nowrap()
        .child(text.into())
}

/// 图例之间的间距。
pub const LEGEND_GAP: f32 = SPACE_XS;

#[cfg(test)]
mod tests {
    use super::*;

    /// 刻度数值自上而下：上限、一半、0。
    fn values(height: f32, peak: u64) -> Vec<u64> {
        ticks(height, peak).into_iter().map(|(_, v)| v).collect()
    }

    #[test]
    fn ticks_split_the_peak_in_half() {
        assert_eq!(values(70.0, 12), vec![12, 6, 0]);
        assert_eq!(values(70.0, 3), vec![3, 1, 0]);
    }

    /// 峰值是 1 条时「一半」还是 1，两档刻度必须合并，否则右边会写出两个
    /// 「1 条」——旧版按固定比例取整更糟，三档全成了 0。
    #[test]
    fn ticks_dedupe_when_the_peak_is_tiny() {
        let scale = values(70.0, 1);
        assert_eq!(scale, vec![1, 0]);
        assert_eq!(scale.len(), 2, "1 条时只留两档，别叠出两个「1 条」");
    }

    /// 折线上的点：一个采样一个点，横坐标铺满画布，纵坐标原样带过去。
    ///
    /// 这里是「台阶感」的源头之一：以前按像素列插值，一列一个方块；现在点
    /// 之间直接连直线，点必须落在 `0 ..= width - 1` 上，否则曲线会缩在中间
    /// 或者越出右边框，跟刻度对不上。
    #[test]
    fn line_points_span_the_whole_plot() {
        let width = 848.0;
        let ys: Vec<f32> = (0..120).map(|i| 40.0 + (i as f32 * 0.1).sin() * 30.0).collect();
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(width), px(96.0)));
        let points = line_points(bounds, &ys, width);
        assert_eq!(points.len(), ys.len(), "一个采样一个点，不多不少");
        assert_eq!(points.first().map(|p| p.x), Some(px(10.0)));
        assert_eq!(
            points.last().map(|p| p.x),
            Some(px(10.0 + width - 1.0)),
            "最后一个点要落在右边框上"
        );
        for (index, point) in points.iter().enumerate() {
            assert_eq!(point.y, px(20.0 + ys[index]), "纵坐标不能被改写");
        }
    }

    #[test]
    fn ticks_stay_inside_the_canvas() {
        let height = SPLIT_CHART_HEIGHT;
        for peak in [1, 2, 3, 7, 120, 5000] {
            let scale = ticks(height, peak);
            assert_eq!(scale.first().map(|&(_, v)| v), Some(peak));
            assert_eq!(scale.last().map(|&(_, v)| v), Some(0));
            for (y, _) in scale {
                assert!(y >= 0.0 && y <= height - 1.0, "y={y} 越界");
            }
        }
    }
}