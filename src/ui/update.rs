//! 消息处理：所有状态修改都发生在这里。
//!
//! 输入类消息只更新字段值（不清空提示，避免打字时提示闪烁）；
//! 「启动」「保存」这类动作消息才会校验、落盘并调用内核。
//!
//! 这里最需要小心的是**同步**：文本输入框、下拉框是有状态的实体
//! （[`gpuikit::input::InputState`] / [`SelectState`]），程序改完配置以后
//! 必须把新值写回实体，否则界面还显示旧值。所以每个改动配置的分支末尾都要
//! 明确回答一个问题：「这次改动是程序改的，还是用户改的？」
//!
//! - 用户打字 → 实体已经是新的，**不要**回写（会把光标打回开头）；
//! - 程序改值（切配置、填默认值、增删配置、换语言）→ 回写实体。

use gpui::{AppContext, Context, Window};
use gpuikit::elements::dialog::{DialogState, dialog};

use crate::core::i18n::{apply as apply_language, t};
use crate::core::watchdog;

use super::state::{App, Msg, Page};

/// 处理一条界面消息。
pub fn update(app: &mut App, msg: Msg, cx: &mut Context<App>) {
    // 卡顿探针：这一段跑超过 3 秒就写进 `<数据目录>/stall.log`。
    // 只做一次原子写，不分配内存，不影响热路径。
    let _guard = watchdog::enter(section_of(&msg));

    match msg {
        // —— 导航 ——
        Msg::PageSelected(page) => {
            // 离开配置页时回到列表态：下次进来应该先看到「有哪几套」。
            if page != Page::Profiles {
                app.profile_editing = false;
            }
            app.page = page;
        }
        Msg::ToggleSidebar => app.sidebar_collapsed = !app.sidebar_collapsed,

        // —— 配置切换 ——
        Msg::ProfileSelected(index) => {
            app.select_profile(index);
            // 运行中切换配置不会影响已启动的进程，只提示下次启动生效。
            if app.engine.is_active() {
                app.notice = Some(
                    t(
                        "已切换配置，重启服务后生效",
                        "Profile switched; restart the service to apply it",
                    )
                    .to_string(),
                );
            }
            // 输入框显示的是「当前这套配置」，跟着换一套就得整组刷新。
            app.sync_inputs(cx);
            app.sync_selects(cx);
        }
        Msg::ProfileEditStart(index) => {
            app.select_profile(index);
            app.profile_editing = true;
            // 从仪表盘点铅笔进来的：要真把人送到配置页，否则「编辑」只改了
            // 一个不可见的布尔值，界面停在原地什么也没发生。
            app.page = Page::Profiles;
            app.sync_inputs(cx);
            app.sync_selects(cx);
        }
        Msg::ProfileEditDone => app.profile_editing = false,
        Msg::ProfileRemovedAt(index) => {
            if app.config.remove(index) {
                app.after_profile_created();
                // 删到只剩一套（或删的正是正在编辑的那套）时回列表，
                // 免得详情页还挂着一个已经不存在的配置。
                app.profile_editing = false;
                // 下拉框的选项是建实体时定死的，配置少了必须整组重建。
                app.rebuild_selects(cx);
                app.sync_inputs(cx);
            } else {
                app.notice = Some(t("至少要保留一套配置", "Keep at least one profile").to_string());
            }
        }
        Msg::ProfileAdded => {
            app.config.add_profile();
            app.after_profile_created();
            app.rebuild_selects(cx);
            app.sync_inputs(cx);
        }
        Msg::ProfileDuplicated => {
            app.config.duplicate_active();
            app.after_profile_created();
            app.rebuild_selects(cx);
            app.sync_inputs(cx);
        }
        Msg::ProfileRenamed(name) => app.profile_mut().name = name,

        // —— 代理设置 ——
        Msg::ProxyUrl(value) => app.profile_mut().proxy_url = value,
        Msg::SetDns(mode) => app.profile_mut().dns = mode,
        Msg::DnsAddr(value) => app.profile_mut().dns_addr = value,
        Msg::VirtualPool(value) => app.profile_mut().virtual_dns_pool = value,
        Msg::BypassChanged(value) => app.profile_mut().bypass = value,
        Msg::BypassDefaults => {
            app.profile_mut().bypass = crate::core::config::DEFAULT_BYPASS.to_string();
            // 「填入默认值」是程序改的：多行输入框里的文字得跟着换。
            app.sync_inputs(cx);
        }

        // —— 网络参数 ——
        Msg::SetIpv6(value) => app.profile_mut().ipv6 = value,
        Msg::MtuChanged(value) => app.draft.mtu = value,
        Msg::TunName(value) => app.profile_mut().tun_name = value,
        Msg::SetSetupRoutes(value) => app.profile_mut().setup = value,

        // —— 高级选项 ——
        Msg::SetVerbosity(level) => app.profile_mut().verbosity = level,
        Msg::SetUnshare(value) => app.profile_mut().unshare = value,
        Msg::UdpgwChanged(value) => app.profile_mut().udpgw_server = value,
        Msg::UdpgwConnections(value) => app.draft.udpgw_connections = value,
        Msg::TcpTimeout(value) => app.draft.tcp_timeout = value,
        Msg::UdpTimeout(value) => app.draft.udp_timeout = value,
        Msg::MaxSessions(value) => app.draft.max_sessions = value,
        Msg::SetExitOnFatal(value) => app.profile_mut().exit_on_fatal = value,

        // —— 操作 ——
        Msg::Start => {
            app.notice = None;
            app.error = None;
            // 先补上名字（用户可能清空了输入框），再校验并启动。
            app.config.normalize();
            // 启动前先探上游代理：连不通就不启动（tun2proxy 起来后会接管
            // 路由，上游写错等于把网切断了）。检测在后台线程跑，结论由
            // 引擎写进共享状态，界面读 `snapshot.blocked`。
            let result = app
                .apply_draft()
                .and_then(|()| app.engine.start_with_precheck(app.config.active_profile()));
            match result {
                Ok(()) => {
                    let _ = app.config.save();
                    // 名字被 `normalize()` 补过的话，输入框要跟着显示新名字。
                    app.sync_inputs(cx);
                }
                Err(err) => app.error = Some(err),
            }
        }
        Msg::Stop => {
            app.notice = None;
            app.engine.stop();
        }
        Msg::SaveConfig => {
            app.error = None;
            app.config.normalize();
            match app.apply_draft().and_then(|()| app.config.save()) {
                Ok(path) => {
                    app.notice = Some(format!(
                        "{} {}",
                        t("配置已保存到", "Configuration saved to"),
                        path.display()
                    ));
                    app.sync_inputs(cx);
                }
                Err(err) => app.error = Some(err),
            }
        }
        Msg::ClearLogs => app.engine.clear_logs(),
        Msg::RelaunchElevated => {
            // 先把内核收掉：否则旧进程退出后，新进程会碰到还在跑的 tun2proxy
            // （TUN 网卡与路由都还挂在旧进程名下）。
            app.engine.shutdown_blocking();
            if crate::core::elevate::relaunch_elevated() {
                std::process::exit(0);
            }
            app.error = Some(
                t(
                    "以管理员身份重启失败，请手动右键「以管理员身份运行」。",
                    "Restarting as administrator failed. Right-click the app and choose “Run as administrator”.",
                )
                .to_string(),
            );
        }
        Msg::CheckProxy => app.engine.check_proxy(app.profile().proxy_url.clone()),
        Msg::CheckUpdate => app.engine.check_update(app.config.release_repo.clone()),
        Msg::InstallUpdate => app.engine.install_update(app.config.release_repo.clone()),

        // —— 窗口与托盘 ——
        Msg::SetConfirmCloseToTray(value) => {
            app.config.confirm_close_to_tray = value;
            // 这个开关就在设置页上，改完立刻落盘：它决定「关窗时会不会被拦」，
            // 不能等用户再点一次保存。
            if let Err(err) = app.config.save() {
                app.error = Some(err);
            }
        }
        Msg::MinimizeToTray => app.minimize_to_tray(cx),
        Msg::SetLanguage(value) => {
            app.config.language = value;
            // 先切语言再存盘：视图会在下一次重绘时读到新语言，
            // 存盘失败也只影响下次启动，不该拦住切换。
            apply_language(value);
            if let Err(err) = app.config.save() {
                app.error = Some(err);
            }
            // 占位文字是建输入框时定死的，换语言要重建；下拉框的选项同理。
            // 但下拉框**不能在这里重建**：这条消息正是它自己发出来的，
            // 在它的回调里把它的实体换掉，等于把自己的地基抽走。推到本次事件
            // 处理完再重建。
            app.rebuild_inputs(cx);
            cx.defer(|cx| {
                super::state::with_app(cx, |app, cx| app.rebuild_selects(cx));
            });
        }
        Msg::SetTheme(value) => {
            app.config.theme = value;
            // 主题是「一点就想看到结果」的东西：先换，再存盘。存盘失败只影响
            // 下次启动，不该拦住这次切换。
            super::theme::reinstall(cx, value);
            if let Err(err) = app.config.save() {
                app.error = Some(err);
            }
        }
    }

    // 状态改完了，让根视图重画一帧。
    cx.notify();
}

impl App {
    /// 把一条消息交给 [`update`]。
    ///
    /// 包一层是为了让视图层写 `this.handle(msg, cx)` 就够了，不必到处
    /// `super::update::update(this, msg, cx)`——闭包里那行会很长。
    pub fn handle(&mut self, msg: Msg, cx: &mut Context<Self>) {
        update(self, msg, cx);
    }

    /// 窗口的叉被按下时调用：决定「真的关」还是「藏进托盘」。
    ///
    /// 返回 `true` 表示放行关闭。三种情况：
    ///
    /// - 托盘菜单点了「退出」→ 用户在别处已经确认过意图，直接退出；
    /// - 开了「关闭时询问」且能藏窗口 → 弹窗，等用户选；
    /// - 其余 → 直接藏进托盘（藏不住就退出，见 [`App::minimize_to_tray`]）。
    pub fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let _guard = watchdog::enter(watchdog::MSG_CLOSE_REQUESTED);

        // 托盘菜单的「退出」不是用户在关窗：那一路上已经确认过意图了。
        if crate::core::tray::quit_armed() {
            cx.quit();
            return true;
        }

        if self.config.confirm_close_to_tray && crate::core::window::can_hide() {
            // 对话框正文随服务状态变：**没在跑**的时候说「会停掉隧道」是在骗人
            // （根本没有隧道），用户会怀疑这个提示是模板话术。
            let running = self.engine.is_active();
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

            let state = cx.new(|_| {
                DialogState::new(
                    dialog("close-to-tray")
                        .confirm(t("最小化到托盘？", "Minimize to the tray?"), body)
                        // 两个答案都不破坏数据，别用红色主按钮喊「危险」。
                        .destructive(false)
                        .confirm_label(t("最小化到托盘", "Minimize to tray"))
                        .cancel_label(t("退出程序", "Quit"))
                        .on_confirm(|_window, cx| {
                            super::state::with_app(cx, |app, cx| {
                                let _guard = watchdog::enter(watchdog::MSG_CLOSE_DIALOG);
                                app.minimize_to_tray(cx);
                            });
                        })
                        .on_cancel(|_window, cx| {
                            let _guard = watchdog::enter(watchdog::MSG_CLOSE_DIALOG);
                            cx.quit();
                        }),
                )
            });
            state.update(cx, |state, cx| state.open(window, cx));
            self.close_dialog = Some(state);
            // 拦下这次关闭：窗口继续活着，只是先问一句。
            return false;
        }

        self.minimize_to_tray(cx);
        false
    }

    /// 把窗口藏进托盘。
    ///
    /// 藏不住就真退出，而不是假装成功——窗口没藏起来而用户又看到提示，
    /// 会以为程序卡了。
    ///
    /// 用 `hide_main_deferred`：此刻还在窗口过程里（`WM_CLOSE` 还没返回），
    /// 同步 `ShowWindow(SW_HIDE)` 会把 `WM_SHOWWINDOW` / `WM_SIZE` 立刻派回
    /// 同一个窗口过程，和正在处理的关闭消息搅在一起。推到另一个线程稍等再藏，
    /// 消息到达时这次关闭处理早已返回。
    pub fn minimize_to_tray(&mut self, cx: &mut Context<Self>) {
        if crate::core::window::hide_main_deferred() {
            self.notice = Some(
                t(
                    "已最小化到托盘，双击托盘图标可恢复窗口",
                    "Minimized to the tray. Double-click the tray icon to restore the window.",
                )
                .to_string(),
            );
            cx.notify();
        } else {
            cx.quit();
        }
    }
}

/// 把消息映射成卡顿探针里的段落名（见 `core/watchdog.rs`）。
///
/// 只给「可能真的卡住」的动作类消息起名字；改字段那类消息执行时间是微秒级，
/// 统一记成 `other` 就够了，不必为它们各写一行。
fn section_of(msg: &Msg) -> watchdog::Section {
    match msg {
        Msg::Start => watchdog::MSG_START,
        Msg::Stop => watchdog::MSG_STOP,
        Msg::SaveConfig => watchdog::MSG_SAVE_CONFIG,
        Msg::CheckProxy => watchdog::MSG_CHECK_PROXY,
        Msg::CheckUpdate => watchdog::MSG_CHECK_UPDATE,
        Msg::InstallUpdate => watchdog::MSG_INSTALL_UPDATE,
        Msg::RelaunchElevated => watchdog::MSG_RELAUNCH_ELEVATED,
        Msg::MinimizeToTray => watchdog::MSG_MINIMIZE_TO_TRAY,
        _ => watchdog::OTHER,
    }
}
