//! 消息处理：所有状态修改都发生在这里。
//!
//! 输入类消息只更新字段值（不清空提示，避免打字时提示闪烁）；
//! 「启动」「保存」这类动作消息才会校验、落盘并调用内核。

use zsui::AppCx;

use super::state::{App, Msg};
use crate::core::i18n::t;
use crate::core::watchdog;

/// 处理一条界面消息。
pub fn update(app: &mut App, msg: Msg, cx: &mut AppCx) {
    // 卡顿探针：这一段跑超过 3 秒就写进 `<数据目录>/stall.log`。
    // 只做一次原子写，不分配内存，不影响热路径。
    let _guard = watchdog::enter(section_of(&msg));
    match msg {
        // —— 导航 ——
        Msg::PageSelected(page) => {
            // 离开配置页时回到列表态：下次进来应该先看到「有哪几套」。
            if page != super::state::Page::Profiles {
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
                app.notice = Some("已切换配置，重启服务后生效".to_string());
            }
        }
        Msg::ProfileEditStart(index) => {
            app.select_profile(index);
            app.profile_editing = true;
            // 从仪表盘点铅笔进来的：要真把人送到配置页，否则「编辑」只改了
            // 一个不可见的布尔值，界面停在原地什么也没发生。
            app.page = super::state::Page::Profiles;
        }
        Msg::ProfileEditDone => app.profile_editing = false,
        Msg::ProfileRemovedAt(index) => {
            if app.config.remove(index) {
                app.after_profile_created();
                // 删到只剩一套（或删的正是正在编辑的那套）时回列表，
                // 免得详情页还挂着一个已经不存在的配置。
                app.profile_editing = false;
            } else {
                app.notice = Some("至少要保留一套配置".to_string());
            }
        }
        Msg::ProfileAdded => {
            app.config.add_profile();
            app.after_profile_created();
        }
        Msg::ProfileDuplicated => {
            app.config.duplicate_active();
            app.after_profile_created();
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
                Ok(path) => app.notice = Some(format!("配置已保存到 {}", path.display())),
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
            app.error = Some("以管理员身份重启失败，请手动右键「以管理员身份运行」。".to_string());
        }
        Msg::CheckProxy => app.engine.check_proxy(app.profile().proxy_url.clone()),
        Msg::CheckUpdate => app.engine.check_update(app.config.release_repo.clone()),
        Msg::InstallUpdate => app.engine.install_update(app.config.release_repo.clone()),

        // —— 窗口与托盘 ——
        Msg::CloseRequested => {
            // 托盘菜单的「退出」不是用户在关窗：那一路上已经确认过意图了。
            if crate::core::tray::quit_armed() {
                cx.quit();
            } else if app.config.confirm_close_to_tray && crate::core::window::can_hide() {
                app.close_prompt = true;
            } else {
                minimize_to_tray(app, cx);
            }
        }
        Msg::CloseDialog(result) => {
            app.close_prompt = false;
            match result {
                // 主按钮 = 留在托盘（默认、回车）。
                zsui::ZsContentDialogResult::Primary => minimize_to_tray(app, cx),
                // 关闭按钮（右上叉 / Esc）= 真退出。
                _ => cx.quit(),
            }
        }
        Msg::SetConfirmCloseToTray(value) => {
            app.config.confirm_close_to_tray = value;
            // 这个开关就在设置页上，改完立刻落盘：它决定「关窗时会不会被拦」，
            // 不能等用户再点一次保存。
            if let Err(err) = app.config.save() {
                app.error = Some(err);
            }
        }
        Msg::MinimizeToTray => minimize_to_tray(app, cx),
        Msg::SetLanguage(value) => {
            app.config.language = value;
            // 先切语言再存盘：视图会在下一次重建时读到新语言，
            // 存盘失败也只影响下次启动，不该拦住切换。
            crate::core::i18n::apply(value);
            if let Err(err) = app.config.save() {
                app.error = Some(err);
            }
        }
    }

    let _ = cx;
}

/// 把窗口藏进托盘。
///
/// 藏不住就真退出，而不是假装成功——窗口没藏起来而用户又看到弹窗消失，
/// 会以为程序卡了。
///
/// 用 `hide_main_deferred`：`ShowWindow(SW_HIDE)` 会同步把 `WM_SIZE` 派回
/// ZSUI 的窗口过程，而我们此刻还在 `update()` 里、视图状态锁还没放。
fn minimize_to_tray(app: &mut App, cx: &mut AppCx) {
    if crate::core::window::hide_main_deferred() {
        app.notice = Some(
            t(
                "已最小化到托盘，双击托盘图标可恢复窗口",
                "Minimized to the tray. Double-click the tray icon to restore the window.",
            )
            .to_string(),
        );
    } else {
        cx.quit();
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
        Msg::CloseRequested => watchdog::MSG_CLOSE_REQUESTED,
        Msg::CloseDialog(_) => watchdog::MSG_CLOSE_DIALOG,
        Msg::MinimizeToTray => watchdog::MSG_MINIMIZE_TO_TRAY,
        _ => watchdog::OTHER,
    }
}
