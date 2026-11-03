//! 根视图：左侧边栏 + 右侧内容区（必要时再叠一层「关闭确认」弹窗）。
//!
//! 视图不是每帧重建的树：`App` 自己实现 [`Render`]，由 gpui 在需要时调用
//! `render()`。所以本文件只做三件事：
//!
//! 1. 取一份后台状态快照（避免各页各自加锁）；
//! 2. 把快照交给侧边栏与内容区去画；
//! 3. 把「关闭确认」对话框挂成最后一个子节点（gpui 的 `deferred` 会把它
//!    画在整棵树的上面，与顺序无关，但挂在最后读起来最清楚）。

use gpui::{App as GpuiApp, Context, IntoElement, Render, Window, div, prelude::*};
use gpuikit::theme::{ActiveTheme, ThemeVariant, Themeable};

use super::pages;
use super::sidebar;
use super::state::{App, UiHandle};

impl Render for App {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 卡顿探针：一帧超过 3 秒同样会记一笔（布局算炸了也能看出是哪一段）。
        let _guard = crate::core::watchdog::enter(crate::core::watchdog::VIEW);

        // 一次性把后台状态拷出来，避免各页各自加锁。
        let snapshot = self.snapshot();
        let theme = cx.theme().clone();

        // 日志行数变了才把日志窗格滚到底，否则用户往回翻会被拽回最新一行。
        //
        // 用累计条数而不是 `logs.len()`：后者到 200 行上限就不再变化，日志框
        // 会停在半途，看起来就是「日志滚不动」。
        self.follow_logs(snapshot.log_total);

        div()
            .flex()
            .size_full()
            // 底色铺满整窗：内容区是 `bg` + `surface` 卡片，侧边栏是 `surface`。
            // 两块面板靠这个色差分层，再加侧边栏右侧那条 1dp 竖线定边界。
            .bg(theme.bg())
            .child(sidebar::sidebar(self, &snapshot, &theme, cx))
            .child(pages::content(self, &snapshot, &theme, cx))
            .children(self.close_dialog.clone())
    }
}

/// 装好「界面实体全局句柄」与「关窗拦截」。
///
/// 两件事都必须发生在窗口建好之后：
///
/// - **全局句柄**：下拉框、对话框的回调只能拿到 `&mut App`，拿不到
///   `&mut Context<App>`。把界面实体存成全局，这些回调就能顺着它找回来
///   （见 [`super::state::with_app`]）。
/// - **关窗拦截**：tun2proxy 是常驻服务，点叉直接把隧道停了很容易误操作。
///   拦截函数返回 `false` 就否决这次关闭，窗口继续活着（可以只藏进托盘）。
pub fn install_window_hooks(entity: gpui::Entity<App>, window: &mut Window, cx: &mut GpuiApp) {
    cx.set_global(UiHandle(entity.downgrade()));

    // 原生标题栏跟着界面走。gpui 只在系统配色变化时同步 DWM 的深色标志，
    // 而我们的主题是按 `TUN2PROXY_GUI_THEME` 选的，可能与系统不一致。
    let dark = cx.theme().variant == ThemeVariant::Dark;
    crate::core::platform::set_dark_titlebar(dark);

    window.on_window_should_close(cx, move |window, cx| {
        entity.update(cx, |app, cx| app.on_close_requested(window, cx))
    });
}
