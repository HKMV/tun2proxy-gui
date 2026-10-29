//! 视图构建：左侧边栏 + 右侧内容区（必要时再叠一层「关闭确认」弹窗）。
//!
//! 每次状态变化都会整棵重建（ZSUI 的声明式做法），因此这里不做缓存，
//! 只保证函数是纯的：同样的 [`App`] 一定渲染出同样的界面。

use zsui::{
    content_dialog, row, Dp, ThemeColorToken, ViewAlign, ViewNode, WidgetId, ZsContentDialogButton,
    ZsContentDialogSpec,
};

use crate::core::i18n::t;

use super::pages;
use super::sidebar::sidebar;
use super::state::{App, Msg};

/// 「要不要最小化到托盘」弹窗的标识。
const CLOSE_DIALOG_ID: u64 = 0x7475_6e32_636c_6f73;

/// 构建整个窗口的内容。
pub fn view(app: &App) -> ViewNode<Msg> {
    // 卡顿探针：视图重建也在这条线程上，一帧里超过 3 秒同样会记一笔。
    let _guard = crate::core::watchdog::enter(crate::core::watchdog::VIEW);
    // 一次性把后台状态拷出来，避免各页各自加锁。
    let snapshot = app.snapshot();

    let root = row([
        // 侧边栏：品牌、导航、常驻的启动/停止按钮。
        sidebar(app, &snapshot),
        // 内容区：随导航切换。
        pages::content(app, &snapshot),
    ])
    .gap(Dp::new(0.0))
    .align(ViewAlign::Stretch)
    // 底色铺满整窗：内容区是灰底 + 白卡片（卡片自然浮起来），
    // 侧边栏是白底——两块颜色不同，分区不靠分隔线也看得出来。
    .bg(ThemeColorToken::Surface);

    if app.close_prompt {
        content_dialog(
            WidgetId::new(CLOSE_DIALOG_ID),
            true,
            close_dialog_spec(snapshot.status.is_active()),
            root,
        )
        .on_dialog_result(Msg::CloseDialog)
    } else {
        root
    }
}

/// 「关窗还是留在托盘」的弹窗内容。
///
/// 文案随服务状态变：**没在跑**的时候说「会停掉隧道」是在骗人（根本没有
/// 隧道），用户会怀疑这个提示是模板话术，进而不信任其它提示。
///
/// 主按钮（默认、回车）= 留在托盘，关闭按钮 = 真退出：常驻服务误关窗口的
/// 代价比多点一次「退出」大，所以默认落在安全的那一边。
fn close_dialog_spec(running: bool) -> ZsContentDialogSpec {
    // 正文只有一段：ZSUI 的 content dialog 不换行显示 `\n\n`，多段会被
    // 拼成一行，所以干脆写成一句流畅的话。
    let body = if running {
        t(
            "Tun2Proxy 正在代理流量，关掉窗口会把隧道一并停掉、网络恢复成直连。最小化到托盘可以继续代理，双击托盘图标就能把窗口叫回来。",
            "Tun2Proxy is proxying traffic. Closing the window stops the tunnel and the network falls back to a direct connection. Minimize to the tray to keep proxying; double-click the tray icon to bring the window back.",
        )
    } else {
        t(
            "服务当前没有运行，关掉窗口只是退出程序。最小化到托盘可以让它留在后台，双击托盘图标就能把窗口叫回来。",
            "The service is not running, so closing the window simply exits the app. Minimize to the tray to keep it in the background; double-click the tray icon to bring the window back.",
        )
    };

    ZsContentDialogSpec::new(body, t("退出程序", "Quit"))
        .title(t("最小化到托盘？", "Minimize to the tray?"))
        .primary_button(t("最小化到托盘", "Minimize to tray"))
        .default_button(ZsContentDialogButton::Primary)
}
