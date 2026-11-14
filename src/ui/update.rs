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

use gpui::{AppContext, Context, IntoElement, Window};
use gpuikit::elements::dialog::{DialogState, dialog};
use gpuikit::theme::ControlSize;
use gpuikit::traits::control_sized::ControlSized;

use crate::core::i18n::{apply as apply_language, t, tf};
use crate::core::watchdog;
use crate::core::CloseAction;

use super::overlay::{PendingAction, begin};
use super::state::{App, Msg, Page};
use super::widgets::RememberChoice;

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
            // 切完立刻落盘。只改内存的话，关掉程序再打开会回到上一套，
            // 而界面上没有任何「未保存」提示——用户会以为切换没生效。
            persist_active(app);
            // 运行中切换配置不会影响已启动的进程，只提示下次启动生效。
            if app.engine.is_active() {
                app.flash_notice(
                    t(
                        "已切换配置，重启服务后生效",
                        "Profile switched; restart the service to apply it",
                    ),
                    cx,
                );
            }
            // 输入框显示的是「当前这套配置」，跟着换一套就得整组刷新。
            app.sync_inputs(cx);
            app.sync_selects(cx);
        }
        Msg::ProfileEditStart(index) => {
            app.select_profile(index);
            persist_active(app);
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
                persist_active(app);
            } else {
                app.flash_notice(t("至少要保留一套配置", "Keep at least one profile"), cx);
            }
        }
        Msg::ProfileAdded => {
            app.config.add_profile();
            app.after_profile_created();
            app.rebuild_selects(cx);
            app.sync_inputs(cx);
            persist_active(app);
        }
        Msg::ProfileDuplicated => {
            app.config.duplicate_active();
            app.after_profile_created();
            app.rebuild_selects(cx);
            app.sync_inputs(cx);
            persist_active(app);
        }
        // 名字是「配置文件」下拉框的选项文字，改一个字下拉框就得换一个实体
        // （见 `App::refresh_profile_select`）。这条消息来自名称输入框，不是
        // 下拉框自己发出来的，所以可以在这里直接重建。
        Msg::ProfileRenamed(name) => {
            app.profile_mut().name = name;
            app.refresh_profile_select(cx);
        }

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

        // —— 操作 ——
        Msg::Start => {
            // 已经有一次启停在进行中：蒙版挡住了内容区、侧栏的按钮也置灰了，
            // 这里再兜一层——连点两次会起两条检测线程，后一条必然回一句「服
            // 务已经在运行」，把前一条的结论盖掉。
            if !app.busy.is_busy() {
                app.notice = None;
                app.error = None;
                // 先补上名字（用户可能清空了输入框），再校验并启动。
                app.config.normalize();
                // `normalize()` 补名字时下拉框的选项文字也跟着变（空名字 → 「配置 N」）。
                app.refresh_profile_select(cx);
                // 启动前先探上游代理：连不通就不启动（tun2proxy 起来后会接管
                // 路由，上游写错等于把网切断了）。检测在后台线程跑，结论由
                // 引擎写进共享状态，界面读 `snapshot.blocked`。
                let result = app.apply_draft().and_then(|()| {
                    app.engine
                        .start_with_precheck(app.config.active_profile(), &app.config.split)
                });
                match result {
                    Ok(()) => {
                        // 从这里到状态变成「正在启动…」还要经过探测、清理残留
                        // 网卡、起分流器和拉子进程——全在后台线程上，界面**不
                        // 会被卡住，但也不会有任何变化**。所以在这一帧就把蒙
                        // 版立起来，告诉用户「在做事」。
                        begin(app, PendingAction::Starting, cx);
                        let _ = app.config.save();
                        // 名字被 `normalize()` 补过的话，输入框要跟着显示新名字。
                        app.sync_inputs(cx);
                    }
                    // 本地校验就没过：错误当场就看得见（提示条），不必立蒙版。
                    Err(err) => app.error = Some(err),
                }
            }
        }
        Msg::Stop => {
            app.notice = None;
            // 停止同样要等：`stop()` 只发中断信号（状态立刻变成「正在停止…」），
            // 子进程真正退出、虚拟网卡回收是后台监督线程的事。已经停着的时候
            // 不立蒙版——那时 `stop()` 什么也不做，蒙版会闪一帧就没了。
            let was_active = app.engine.is_active();
            app.engine.stop();
            if was_active {
                begin(app, PendingAction::Stopping, cx);
            }
        }
        Msg::SaveConfig => {
            app.error = None;
            app.config.normalize();
            // 同上：保存时补上的名字也要出现在下拉框里。
            app.refresh_profile_select(cx);
            match app.apply_draft().and_then(|()| app.config.save()) {
                Ok(_) => {
                    // 只报结果，不报路径：写去哪儿是程序自己的事，把一整条
                    // `D:\...\config.json` 摊在提示条上只会挤掉真正要看的话。
                    app.flash_notice(t("配置已保存", "Configuration saved"), cx);
                    app.sync_inputs(cx);
                }
                Err(err) => app.error = Some(err),
            }
        }
        Msg::ClearLogs => app.engine.clear_logs(),
        Msg::SetLogFilter(filter) => app.log_filter = filter,
        Msg::CopyLogs => {
            // 复制的是引擎里保留的全部日志（2000 行），不是屏幕上那 200 行：
            // 用户要贴给别人看时，「为什么停了」多半在更早的位置。也不跟
            // 当前过滤档——贴出去的是完整现场。
            let text = app
                .shared
                .lock()
                .map(|shared| {
                    shared
                        .logs
                        .iter()
                        .map(|line| line.text.clone())
                        .collect::<Vec<String>>()
                        .join("\r\n")
                })
                .unwrap_or_default();
            if text.trim().is_empty() {
                app.flash_notice(t("还没有日志可复制", "There is no log to copy"), cx);
            } else {
                let lines = text.lines().count();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                let text = tf(
                    t("已复制 {lines} 行日志到剪贴板", "Copied {lines} log lines to the clipboard"),
                    &[("lines", &lines.to_string())],
                );
                app.flash_notice(text, cx);
            }
        }
        Msg::DismissNotice => {
            app.notice = None;
            app.error = None;
            // 「启动被拦下」和「会话打满」都存在引擎的共享状态里，不清掉下一帧
            // 又会冒出来。
            app.engine.dismiss_start_blocked();
            app.engine.dismiss_sessions_full();
        }
        Msg::RelaunchElevated => {
            // 先把内核收掉：否则旧进程退出后，新进程会碰到还在跑的 tun2proxy
            // （TUN 网卡与路由都还挂在旧进程名下）。
            app.engine.shutdown_blocking();
            // 再把「唯一实例」的名额让出去：新进程是接手的那个，不放开的话
            // 它会以为已经有一个在跑，然后把自己关掉。
            crate::core::single_instance::release();
            if crate::core::elevate::relaunch_elevated() {
                std::process::exit(0);
            }
            // 没拉起来：把名额拿回来，否则这会儿再开一个就成了两个实例。
            // `release` 连「显示窗口」事件一起交出去了，所以这里要重新开出来：
            // 不然这一路失败之后，双击 exe 就没人应，窗口再也叫不回来。
            if crate::core::single_instance::claim() {
                crate::core::single_instance::create_show_event();
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

        // —— 分流规则 ——
        //
        // 这一组都是「改完立刻落盘」：分流开关和规则的意图非常直接（勾上就该
        // 生效），再多要求一次「保存」只会让用户以为没点上。规则本身在隧道
        // 运行中热更新，总开关的改动要重启隧道才生效（引擎只在启动时决定
        // `--proxy` 指向哪里）。
        Msg::SetSplitEnabled(value) => {
            app.config.split.enabled = value;
            persist_active(app);
            app.flash_notice(
                if value {
                    t(
                        "分流已开启，重启服务后生效",
                        "Split routing enabled; restart the service to apply it",
                    )
                } else {
                    t(
                        "分流已关闭，重启服务后生效",
                        "Split routing disabled; restart the service to apply it",
                    )
                },
                cx,
            );
        }
        Msg::SetSplitGroup(group, value) => {
            app.config.split.set_group(group, value);
            reload_split(app, cx);
        }
        Msg::SplitCustomChanged(value) => app.config.split.custom = value,
        Msg::SplitRowToggled(key) => {
            app.config.split.toggle_row(&key);
            reload_split(app, cx);
        }
        Msg::SplitDns(value) => app.config.split.direct_dns = value,
        Msg::SplitInterface(value) => app.config.split.direct_interface = value,
        Msg::SplitApply => reload_split(app, cx),
        Msg::SplitUpdate => {
            app.engine.update_split_lists(app.config.split.clone());
            // 下载在后台线程跑；这里只等它出结果，然后把清单重新读一遍，
            // 让界面上的条数和日期立刻变成新的。
            let engine = app.engine.clone();
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(400))
                    .await;
                if engine.split_update().is_busy() {
                    continue;
                }
                let _ = this.update(cx, |app, cx| {
                    app.split_library = crate::core::split::Library::load();
                    cx.notify();
                });
                break;
            })
            .detach();
        }
        Msg::SplitReset => {
            match crate::core::split::update::reset() {
                Ok(()) => {
                    app.split_library = crate::core::split::Library::load();
                    app.flash_notice(
                        t(
                            "已换回内置清单",
                            "Reverted to the built-in lists",
                        ),
                        cx,
                    );
                    reload_split(app, cx);
                }
                Err(err) => app.error = Some(err),
            }
        }
        Msg::SplitFilter(text) => {
            // 纯显示状态：不进配置，也不用落盘，但要重画一帧，否则明细列表
            // 要等到下一次别的状态变化才跟着筛。
            app.split_filter = text;
            cx.notify();
        }

        // —— 窗口与托盘 ——
        Msg::SetCloseAction(value) => {
            app.config.close_action = value;
            // 这一项就在设置页上，改完立刻落盘：它决定「点叉时会不会被拦」，
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
            // 这条消息可能来自设置页的下拉框，也可能来自侧栏底部那一行。
            // 下拉框的选中态是它自己的状态，不跟着改就一直是旧值——推到本次
            // 事件处理完再同步：如果消息正是那个下拉框发出来的，在它的回调里
            // 动它自己的选中态并不合适。
            cx.defer(|cx| {
                super::state::with_app(cx, |app, cx| app.sync_theme_select(cx));
            });
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

    /// 请求删除某套配置：先弹一句确认，确认后才真的删。
    ///
    /// 垃圾桶按钮就在行尾、和「编辑」的铅笔只隔 12dp，而删除会丢掉整套参数
    /// 且没有撤销——误点一次就得从头再填一遍代理地址、DNS、绕过列表。所以
    /// 这一步不能省。
    ///
    /// 需要 `window`（对话框要拿焦点），所以和 [`App::on_close_requested`]
    /// 一样走实体方法而不是 `Msg`。
    pub fn ask_remove_profile(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let _guard = watchdog::enter(watchdog::MSG_REMOVE_PROFILE);

        if self.config.profiles.len() <= 1 {
            self.flash_notice(t("至少要保留一套配置", "Keep at least one profile"), cx);
            return;
        }
        let Some(name) = self
            .config
            .profiles
            .get(index)
            .map(|profile| profile.display_name())
        else {
            return;
        };

        let state = cx.new(|_| {
            DialogState::new(
                dialog("remove-profile")
                    .confirm(
                        tf(
                            t("删除「{name}」？", "Delete “{name}”?"),
                            &[("name", &name)],
                        ),
                        t(
                            "这套配置的参数会一起删掉，没有撤销。",
                            "Every setting in this profile is deleted, with no undo.",
                        ),
                    )
                    .control_size(ControlSize::Large)
                    // 这次是真删数据，用危险色：和关窗那句「都不破坏数据」
                    // 正好相反。
                    .destructive(true)
                    .confirm_label(t("删除", "Delete"))
                    .cancel_label(t("取消", "Cancel"))
                    .on_confirm(move |_window, cx| {
                        super::state::with_app(cx, |app, cx| {
                            let _guard = watchdog::enter(watchdog::MSG_REMOVE_PROFILE);
                            app.remove_dialog = None;
                            app.handle(Msg::ProfileRemovedAt(index), cx);
                        });
                    })
                    .on_cancel(|_window, cx| {
                        super::state::with_app(cx, |app, _cx| app.remove_dialog = None);
                    })
                    // 点外面 / Esc 只是「不删」，不是「取消并退出」。
                    .on_dismiss(|_window, cx| {
                        super::state::with_app(cx, |app, _cx| app.remove_dialog = None);
                    }),
            )
        });
        state.update(cx, |state, cx| state.open(window, cx));
        self.remove_dialog = Some(state);
    }

    /// 窗口的叉被按下时调用：决定「真的关」还是「藏进托盘」。
    ///
    /// 返回 `true` 表示放行关闭。四种情况：
    ///
    /// - 托盘菜单点了「退出」→ 用户在别处已经确认过意图，直接退出；
    /// - 上次在弹窗里勾了「记住这个选择」→ 不再问，直接照办；
    /// - 「每次询问」且能藏窗口 → 弹窗，等用户选；
    /// - 其余 → 直接藏进托盘（藏不住就退出，见 [`App::minimize_to_tray`]）。
    pub fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let _guard = watchdog::enter(watchdog::MSG_CLOSE_REQUESTED);

        // 托盘菜单的「退出」不是用户在关窗：那一路上已经确认过意图了。
        if crate::core::tray::quit_armed() {
            cx.quit();
            return true;
        }

        // 用户已经表过态（弹窗里勾了「记住这个选择」，或设置页里选过）：
        // 再问一遍就是明知故问。
        match self.config.close_action {
            CloseAction::Minimize => {
                self.minimize_to_tray(cx);
                return false;
            }
            CloseAction::Quit => {
                cx.quit();
                return true;
            }
            CloseAction::Ask => {}
        }

        if crate::core::window::can_hide() {
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

            // 「记住这个选择」只存勾没勾，真值在按下答案之后才写进配置。
            let remember = cx.new(|_| RememberChoice::default());

            let state = cx.new(|_| {
                let confirm_remember = remember.clone();
                let cancel_remember = remember.clone();
                let content_remember = remember.clone();
                DialogState::new(
                    dialog("close-to-tray")
                        .confirm(t("最小化到托盘？", "Minimize to the tray?"), body)
                        // 弹窗里的按钮要和窗口里其它控件同档：默认的 Medium
                        // 只有 24dp，在 32dp 的界面上看着像缩水了一圈。
                        .control_size(ControlSize::Large)
                        // 两个答案都不破坏数据，别用红色主按钮喊「危险」。
                        .destructive(false)
                        .confirm_label(t("最小化到托盘", "Minimize to tray"))
                        .cancel_label(t("退出程序", "Quit"))
                        // 正文区就是那句「记住这个选择」：两个答案各自带着它
                        // 落盘，下次点叉不再问。
                        .content(move |_window, _cx| content_remember.clone().into_any_element())
                        .on_confirm(move |_window, cx| {
                            super::state::with_app(cx, |app, cx| {
                                let _guard = watchdog::enter(watchdog::MSG_CLOSE_DIALOG);
                                if confirm_remember.read(cx).checked() {
                                    remember_close_action(app, CloseAction::Minimize, cx);
                                }
                                app.minimize_to_tray(cx);
                            });
                        })
                        .on_cancel(move |_window, cx| {
                            super::state::with_app(cx, |app, cx| {
                                let _guard = watchdog::enter(watchdog::MSG_CLOSE_DIALOG);
                                if cancel_remember.read(cx).checked() {
                                    remember_close_action(app, CloseAction::Quit, cx);
                                }
                                cx.quit();
                            });
                        })
                        // 点弹窗外面（或按 Esc）只是把这句话问完就走开：这个
                        // 弹窗的「取消」答案是**退出程序**，一次误点就把整个
                        // 工作区关掉。所以三种「不回答」的退出方式只关弹窗，
                        // 真正退出必须点到「退出程序」。
                        .on_dismiss(|_window, cx| {
                            let _guard = watchdog::enter(watchdog::MSG_CLOSE_DIALOG);
                            super::state::with_app(cx, |app, _cx| app.close_dialog = None);
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
            self.flash_notice(
                t(
                    "已最小化到托盘，双击托盘图标可恢复窗口",
                    "Minimized to the tray. Double-click the tray icon to restore the window.",
                ),
                cx,
            );
        } else {
            cx.quit();
        }
    }
}

/// 记住「以后点叉就怎么办」，并让设置页的下拉框跟上。
///
/// 必须立刻落盘：这条决定的就是「下次点叉会不会再问」，只留在内存里的话，
/// 用户关掉程序再打开又会被问一遍——那个勾等于没勾。
fn remember_close_action(app: &mut App, action: CloseAction, cx: &mut Context<App>) {
    app.config.close_action = action;
    if let Err(err) = app.config.save() {
        app.error = Some(err);
    }
    app.sync_close_action_select(cx);
}

/// 切完配置立刻把「当前用的是哪一套」写回磁盘。
///
/// 只改内存的话，关掉程序再打开会回到上一套，而界面上没有任何「未保存」
/// 提示——用户会以为切换没生效。写盘失败只影响下次启动，不该拦住这次切换，
/// 所以只报错、不回滚。
fn persist_active(app: &mut App) {
    if let Err(err) = app.config.save() {
        app.error = Some(err);
    }
}

/// 规则变了：校验 → 落盘 → 热更新分流器（隧道没跑时只有前两步）。
///
/// 规则的改动**不**要求重启隧道：分流器本来就是按连接查表的，换一套规则
/// 只要把表换掉，已有连接不受影响。只有总开关（`enabled`）要重启，因为
/// 它决定 tun2proxy 的 `--proxy` 指向哪里——那在进程启动时就固定了。
fn reload_split(app: &mut App, cx: &mut Context<App>) {
    if let Err(err) = app.config.split.validate() {
        app.error = Some(err);
        cx.notify();
        return;
    }
    if let Err(err) = app.config.save() {
        app.error = Some(err);
        cx.notify();
        return;
    }
    match app.engine.reload_split(&app.config.split) {
        // 0 条 = 分流器没在跑：改动已经写进配置，启动隧道时才生效。
        Ok(0) => app.flash_notice(
            t(
                "规则已保存，隧道启动后生效",
                "Rules saved; they apply once the tunnel starts",
            ),
            cx,
        ),
        Ok(count) => {
            let text = count.to_string();
            app.flash_notice(
                tf(
                    t("规则已生效：{count} 条", "Rules applied: {count}"),
                    &[("count", &text)],
                ),
                cx,
            );
        }
        Err(err) => app.error = Some(err),
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
