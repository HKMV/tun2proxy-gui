//! 界面层（nsh）。
//!
//! nsh 只给「无样式的基础组件 + 引擎」，外观全归应用自己：这一层就是本应用
//! 的那套 ui 层——主题 token、样式化控件、各页的布局，全部在这里，特性页只
//! 组合它们，不直接给 nsh 的基础组件上色。
//!
//! | 文件 | 内容 |
//! |------|------|
//! | [`page`] | 导航页枚举 [`page::Page`]：名字、副标题、顺序 |
//! | [`state`] | 界面层要用的内核句柄：配置、引擎 |
//! | [`palette`] | 配色方案：`themes/*.json` → 设计 token，可运行中切换 |
//! | [`shell`] | 整窗骨架：侧边栏 + 内容区 |

pub mod live;
pub mod page;
pub mod palette;
pub mod profile;
pub mod shell;
pub mod split;
pub mod state;
pub mod widgets;

/// 窗口最小尺寸（逻辑像素）。
///
/// 侧边栏 176dp + 内容区留白 32dp + 卡片内两栏 ≈ 780dp，取整。
pub const MIN_WINDOW_WIDTH: f32 = 780.0;

/// 窗口最小高度（逻辑像素）。
///
/// 比宽度宽松：竖向装不下只会出现滚动条，不会丢内容。
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;
