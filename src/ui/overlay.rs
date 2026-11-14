//! 全局忙碌蒙版：点「启动 / 停止」之后立刻铺一层遮罩 + 转圈 + 一句说明。
//!
//! # 为什么需要它
//!
//! 点「启动」到侧边栏的状态变成「正在启动…」之间隔着好几件事：探一次上游
//! 代理（`connect_timeout` 最长 3 秒）、清理残留的隧道网卡、起本机分流器、
//! 最后才 `Command::spawn` 拉起子进程。这一段全部跑在**后台线程**里
//! （见 `core::engine::start_with_precheck` 与 `core::engine::start`），界面
//! 线程并没有被卡住——但屏幕上什么都不会变：状态还是「未运行」，按钮还是
//! 「启动」。用户看到的就是「点了没反应」，于是再点一次。
//!
//! 所以蒙版在按下按钮的**同一个消息里**立起来：一帧之内出现，等这次启停真的
//! 有了结论再撤掉。它不参与任何业务判断，也不写内核状态，只读一份快照
//! （[`super::state::App::snapshot`]）来判断「该不该撤」。
//!
//! # 为什么复用 gpuikit 的 `loading_indicator`
//!
//! gpui 有 `AnimationExt::with_animation`，但 `.repeat()` 的动画会通过
//! `Window::request_animation_frame` 让**整个窗口**按显示器刷新率重绘
//! （见 `vendor/gpuikit/src/elements/loading_indicator.rs` 的模块注释）。
//! gpuikit 的 `LoadingIndicator` 改用一条共享的低频时钟，只在帧真的变了时
//! 通知一次正在显示它的视图——每秒 5～8 次，正好是转圈需要的频率。所以这里
//! 复用它，既不新增依赖，也不自己另造一条定时器。
//!
//! # 它挡不住退出，也不该挡
//!
//! 蒙版只是窗口里的一个普通元素：关窗走平台层的 `on_window_should_close`
//! （见 [`super::view::install_window_hooks`]），托盘「退出」走 `cx.quit()`，
//! 两条路都不经过这个元素。所以蒙版挂着的时候，关窗、退出、藏进托盘都和
//! 平时一样，它不会把关闭流程挂住。

use std::time::Duration;

use gpui::{AnyElement, BoxShadow, Context, IntoElement, div, hsla, px, prelude::*};
use gpuikit::elements::loading_indicator::{LoadingIndicatorSize, loading_indicator};
use gpuikit::theme::{Theme, Themeable};

use crate::core::Status;
use crate::core::i18n::t;

use super::state::App;
use super::theme::{TEXT_BODY, WEIGHT_SEMIBOLD};
use super::widgets::{CARD_RADIUS, SPACE_M, SPACE_XL, SPACE_XS, hint};

/// 「这次动作有结果了吗」的检查间隔。
///
/// 200ms 与引擎那条重绘通知（`core::engine` 里 180ms 的 `ui-notifier`）同一
/// 量级：晚一帧撤掉蒙版用户看不出来，而查得再勤也只是白跑一趟锁。
const SETTLE_POLL: Duration = Duration::from_millis(200);

/// 蒙版最多挂多久（兜底）。
///
/// 正常路径上蒙版由「有结论」撤掉；这个上限防的是意料之外的卡死（比如某个
/// 系统调用再也不返回）——宁可撤掉蒙版让人继续操作，也不能把界面永久盖住。
const SETTLE_LIMIT: Duration = Duration::from_secs(60);

/// 一次「点了要等一会儿」的启停动作。
///
/// 决定蒙版上说哪一句话，也决定「什么才算有结论」（见 [`settled`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAction {
    /// 启动：从按下按钮到隧道跑起来（或失败）为止。
    Starting,
    /// 停止：从按下按钮到子进程真的退出为止。
    Stopping,
}

impl PendingAction {
    /// 蒙版上那一句标题。
    pub fn label(&self) -> &'static str {
        match self {
            PendingAction::Starting => t("正在启动隧道…", "Starting the tunnel…"),
            PendingAction::Stopping => t("正在停止隧道…", "Stopping the tunnel…"),
        }
    }

    /// 蒙版上那一句说明：告诉用户「在等哪一步」，而不是只说「请稍候」。
    pub fn detail(&self) -> &'static str {
        match self {
            PendingAction::Starting => t(
                "检测上游代理、清理残留网卡、拉起 tun2proxy，通常几秒内完成",
                "Checking the upstream proxy, clearing leftover adapters, launching tun2proxy",
            ),
            PendingAction::Stopping => t(
                "正在请求 tun2proxy 退出并回收虚拟网卡",
                "Asking tun2proxy to exit and releasing the TUN adapter",
            ),
        }
    }
}

/// 蒙版状态：有没有在等一个动作、等的是哪一个。
///
/// 只存「在等」这件事，不存结论——结论永远去引擎的快照里读（见 [`settled`]），
/// 免得界面和内核各记一份「起来了没有」。每次等待带一个代数 [`Busy::seq`]：
/// 撤蒙版的后台轮询醒来时先核对代数，期间用户又点了别的（比如启动还没起完就
/// 按了停止）就直接放弃，不去动新那一次的蒙版。
#[derive(Default)]
pub struct Busy {
    action: Option<PendingAction>,
    seq: u64,
}

impl Busy {
    /// 正在等的那次动作；`None` 表示没在等。
    pub fn action(&self) -> Option<PendingAction> {
        self.action
    }

    /// 有没有一次启停正在进行中（侧栏据此把按钮置灰）。
    pub fn is_busy(&self) -> bool {
        self.action.is_some()
    }
}

/// 画蒙版；没在等任何动作时返回 `None`。
pub fn busy_overlay(app: &App, theme: &Theme) -> Option<AnyElement> {
    let action = app.busy.action()?;

    Some(
        div()
            .id("busy-overlay")
            // 铺满整窗，而不是只盖内容区：起隧道时侧栏的启停按钮也属于这件事，
            // 让一半界面还能点，会让人以为另一半也该有反应。
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            // 遮罩色走主题 token（gpuikit 的 `overlay()`，弹窗背面用的就是它），
            // 明暗两套主题都不必各写一个值。
            .bg(theme.overlay())
            // 挡住底下的点击：蒙版在的时候，点在设置页按钮上应该什么也不发生，
            // 而不是「看起来被挡住了，其实点到了」。
            .occlude()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SPACE_M))
                    .px(px(SPACE_XL))
                    .py(px(SPACE_M))
                    .rounded(px(CARD_RADIUS))
                    .border_1()
                    .border_color(theme.border())
                    .bg(theme.surface())
                    // 阴影：卡片和遮罩都是浅色，不靠一层投影分开就会糊成一片。
                    // 只画到四周，不影响布局（`shadow` 不占盒模型）。
                    .shadow(card_shadow())
                    .child(
                        // 转圈：8 帧的盲文点阵，一眼就是「在转」。颜色取强调色，
                        // 和主界面里的「正在进行」是同一档。尺寸用 28px 那一档
                        // （`XLarge`，本仓库给 gpuikit 补的，见 `vendor/gpuikit/PATCH.md`）：
                        // 上游最大的 `Large` 只有 20px，摆在这个铺满整窗的蒙版上，
                        // 静态看只剩两三个点，像标点不像动画。
                        loading_indicator()
                            .braille()
                            .size(LoadingIndicatorSize::XLarge)
                            .color(theme.accent()),
                    )
                    .child(
                        div()
                            .flex_col()
                            .gap(px(SPACE_XS))
                            .child(
                                div()
                                    .text_size(px(TEXT_BODY))
                                    .font_weight(WEIGHT_SEMIBOLD)
                                    .text_color(theme.fg())
                                    .whitespace_nowrap()
                                    .child(action.label()),
                            )
                            .child(hint(action.detail(), theme)),
                    ),
            )
            .into_any_element(),
    )
}

/// 蒙版卡片脚下那层淡阴影。
///
/// 颜色固定用黑色 + 低 alpha，不跟主题走：亮色主题下卡片浮起来，深色主题下
/// 几乎看不见——这两种结果都比「深色下变成一圈浅色光晕」正常（阴影在深色
/// 界面上本来就该消失，那是这个材质的物理行为）。
fn card_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(10.), hsla(0.0, 0.0, 0.0, 0.16)).blur_radius(px(28.)),
    ]
}

/// 开始一次「点了要等一会儿」的动作：立刻立起蒙版，并在后台盯着它结束。
///
/// 调用方（[`super::update`]）在这之后会照常 `cx.notify()`，蒙版因此在**下一
/// 帧**就出现——这正是用户按下按钮后看到的第一样东西。
pub fn begin(app: &mut App, action: PendingAction, cx: &mut Context<App>) {
    app.busy.action = Some(action);
    app.busy.seq = app.busy.seq.wrapping_add(1);
    let seq = app.busy.seq;

    cx.spawn(async move |this, cx| {
        let mut waited = Duration::ZERO;
        loop {
            cx.background_executor().timer(SETTLE_POLL).await;
            waited += SETTLE_POLL;

            let done = this.update(cx, |app, cx| {
                // 已经换了一次动作（或蒙版被别处撤掉了）：这条轮询作废。
                if app.busy.seq != seq || app.busy.action.is_none() {
                    return true;
                }
                // 有结论，或者等得太久了：撤蒙版。
                if waited >= SETTLE_LIMIT || settled(app) {
                    app.busy.action = None;
                    cx.notify();
                    return true;
                }
                false
            });

            // 视图没了（窗口销毁）就别再转下去了。
            if !matches!(done, Ok(false)) {
                return;
            }
        }
    })
    .detach();
}

/// 这次动作已经有结论了吗？
///
/// 「未运行」有两个意思，必须分开看：一个是**还没开始**（探上游代理的那几秒
/// 里状态就是「未运行」），一个是**启动被拦下了**（上游连不通、子进程拉不
/// 起来）。后者才算有结论，判据是引擎写下的那句拦截原因
/// （`Snapshot::blocked`，失败时同样会出现在内容区的提示条上，错误没有
/// 被吞掉）。
fn settled(app: &App) -> bool {
    let snapshot = app.snapshot();
    let Some(action) = app.busy.action else {
        return true;
    };
    // 本地校验 / 落盘就失败的那条路上没有后台线程，错误当场写进 `error`。
    action_settled(
        action,
        &snapshot.status,
        snapshot.blocked.is_some(),
        app.error.is_some(),
    )
}

/// [`settled`] 的判断本体：不碰任何实体，纯函数，便于单测（见文件末尾）。
fn action_settled(action: PendingAction, status: &Status, blocked: bool, errored: bool) -> bool {
    if errored {
        return true;
    }
    match action {
        PendingAction::Starting => match status {
            // 还没到「正在创建网卡」：探测与清理仍在进行，蒙版继续挂着。
            // 只有「被打断并给出原因」的未运行才算有结论。
            Status::Stopped => blocked,
            Status::Starting => false,
            // 跑起来了、被叫停了、或者失败了：这一段等待都结束了。
            Status::Running | Status::Stopping | Status::Failed(_) => true,
        },
        // 停止：进程句柄一交出去（状态不再是活动态）就算停完。
        PendingAction::Stopping => !status.is_active(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探测与清理的那几秒里状态仍是「未运行」——这一段蒙版**必须**留着，
    /// 否则它会在按下的同一帧撤掉，等于没做。
    #[test]
    fn a_start_that_is_still_probing_stays_covered() {
        assert!(!action_settled(
            PendingAction::Starting,
            &Status::Stopped,
            false,
            false
        ));
    }

    /// 上游连不通时引擎写下拦截原因、状态停在「未运行」：这时才算有结论。
    #[test]
    fn a_blocked_start_is_settled() {
        assert!(action_settled(
            PendingAction::Starting,
            &Status::Stopped,
            true,
            false
        ));
    }

    /// 三种终态都撤蒙版：跑起来、失败、被叫停。
    #[test]
    fn every_terminal_status_dismisses_the_overlay() {
        for status in [
            Status::Running,
            Status::Failed("上游连不通".to_string()),
            Status::Stopping,
        ] {
            assert!(
                action_settled(PendingAction::Starting, &status, false, false),
                "{status:?} 该撤掉启动蒙版"
            );
        }
    }

    /// 「正在创建网卡」不是终态：网卡没建完之前隧道还不能用。
    #[test]
    fn starting_keeps_the_overlay() {
        assert!(!action_settled(
            PendingAction::Starting,
            &Status::Starting,
            false,
            false
        ));
    }

    /// 停止正好相反：只要还是活动态就没停完。
    #[test]
    fn a_stop_waits_until_the_tunnel_is_no_longer_active() {
        for status in [Status::Running, Status::Starting, Status::Stopping] {
            assert!(
                !action_settled(PendingAction::Stopping, &status, false, false),
                "{status:?} 还没停下来"
            );
        }
        for status in [Status::Stopped, Status::Failed("退出码 1".to_string())] {
            assert!(
                action_settled(PendingAction::Stopping, &status, false, false),
                "{status:?} 已经停了"
            );
        }
    }

    /// 本地校验就没过（二进制没下载、MTU 写错…）：错误当场可见，蒙版立撤。
    #[test]
    fn a_local_error_settles_either_action() {
        for action in [PendingAction::Starting, PendingAction::Stopping] {
            assert!(action_settled(action, &Status::Stopped, false, true));
        }
    }
}
