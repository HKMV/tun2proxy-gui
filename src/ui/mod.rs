//! 界面模块：状态定义、视图构建、消息处理。
//!
//! 分工：
//!
//! | 文件 | 内容 |
//! |------|------|
//! | [`state`] | [`state::App`]、[`state::Msg`]、导航页 [`state::Page`]、数字输入草稿 |
//! | [`view`] | 整窗骨架：左侧边栏 + 右侧内容区 |
//! | [`sidebar`] | 侧边栏：品牌、状态、导航、常驻的启停按钮 |
//! | [`pages`] | 每页的标题与卡片组合 |
//! | [`cards`] | 各张设置卡片（服务状态 / 代理 / 网络 / 高级 / 程序 / 日志） |
//! | [`widgets`] | 可复用的文字样式与小部件 |
//! | [`update`] | 处理 [`state::Msg`]，修改状态、调用内核 |

pub mod cards;
pub mod chart;
#[cfg(test)]
mod layout_probe;
pub mod pages;
pub mod sidebar;
pub mod state;
pub mod update;
pub mod view;
pub mod widgets;

pub use state::{App, Msg};
pub use update::update;
pub use view::view;

/// 窗口最小尺寸（逻辑像素）。
///
/// 这个数字不是拍脑袋定的：ZSUI 的布局契约是「控件与文字**永不**压到平台度量
/// 以下，装不下就由父级裁剪」（`src/view/layout.rs` 里
/// `allocate_grid_track_lengths` 的注释写得很清楚）。也就是说窗口缩到比内容
/// 的内在最小宽度还窄时，右侧内容会被直接切掉——不是换行，是消失。
///
/// 内容最宽的一页是「代理配置」：两个文本框并排（`field_pair`），
/// 每个文本框的最小宽度约 200dp，加上行间距 12 与卡片留白 32 合计约 448dp；
/// 再加侧边栏 224dp 与页面留白 32dp，整窗最少要 704dp。
/// 取 740 留一点余量，免得控件贴死在边界上。
///
/// `ui::layout_probe::no_page_overflows_the_minimum_window` 会盯着这个约束：
/// 以后有人往卡片里塞更宽的控件，测试会先报错，而不是让用户看到切边。
pub const MIN_WINDOW_WIDTH: f32 = 780.0;

/// 窗口最小高度（逻辑像素）。
///
/// 比宽度宽松：竖向装不下只会出现滚动条，不会丢内容。
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;
