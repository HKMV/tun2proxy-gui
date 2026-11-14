//! 界面模块：状态定义、视图构建、消息处理。
//!
//! 分工：
//!
//! | 文件 | 内容 |
//! |------|------|
//! | [`state`] | [`state::App`]（同时是 GPUI 根视图实体）、[`state::Msg`]、导航页 [`state::Page`]、输入框 / 下拉框实体 |
//! | [`view`] | `impl Render for App`：整窗骨架 + 关窗拦截 |
//! | [`theme`] | gpuikit 主题的装载与换肤，加上字号 / 字重常量 |
//! | [`sidebar`] | 侧边栏：品牌、状态、导航、常驻的启停按钮 |
//! | [`pages`] | 每页的标题栏与卡片组合 |
//! | [`cards`] | 各张设置卡片（服务状态 / 代理 / 网络 / 高级 / 程序 / 日志） |
//! | [`chart`] | 仪表盘首屏卡右侧的流量折线图（GPUI `canvas`） |
//! | [`overlay`] | 全局忙碌蒙版：启停期间盖住整窗，让「点了以后在等什么」看得见 |
//! | [`widgets`] | 可复用的文字样式与小部件 |
//! | [`update`] | 处理 [`state::Msg`]，修改状态、调用内核 |

pub mod cards;
pub mod chart;
pub mod overlay;
pub mod pages;
pub mod sidebar;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;
pub mod widgets;

pub use state::App;

/// 窗口最小尺寸（逻辑像素）。
///
/// 和改造前的硬约束不同，这里不再是「装不下就被裁掉」的下限：GPUI 的布局是
/// taffy（flexbox），窗口缩小时控件会先压缩、再换行，文字仍然读得到。
/// 这个下限只是为了保证**最窄的可用状态**：内容区还剩约 600dp，
/// 分组卡里「标题 + 右侧 220dp 控件」的两栏排布不至于挤成上下两行。
///
/// 侧边栏 176dp + 内容区留白 32dp + 卡片内两栏 ≈ 780dp，取整。
pub const MIN_WINDOW_WIDTH: f32 = 780.0;

/// 窗口最小高度（逻辑像素）。
///
/// 比宽度宽松：竖向装不下只会出现滚动条，不会丢内容。
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;
