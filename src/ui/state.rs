//! 界面层要用的内核句柄：配置与引擎。
//!
//! nsh 的根组件是「跑一次、绑定按信号重跑」的模型，没有 GPUI 那种
//! `Entity<App>` 可以随手塞字段；启动流程又在窗口之前（提权、单实例都在
//! 那里），所以配置与引擎由 [`install`] 在 `main` 里装进来，界面从这里取。
//!
//! 配置是**可变**的：配色和明暗都能在界面上换，换完要写回配置文件。因此这里
//! 存的是 `Mutex<Config>`，改一项就改在同一份上——各写各的副本会把别的字段
//! 悄悄覆盖回旧值（换配色时把刚选的明暗写回默认）。
//! `Engine` 内部自己用 `Arc` + 锁维护状态，这里不用再包一层。

use std::sync::{Mutex, OnceLock};

use crate::core::config::{CloseAction, Palette, ThemeSetting};
use crate::core::i18n::t;
use crate::core::logs::LogKind;
use crate::core::{Config, Engine};

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static ENGINE: OnceLock<Engine> = OnceLock::new();

/// 日志面板的过滤档位（只影响显示，不落盘）。
///
/// 子进程按连接打日志（一条连接两行），分流判定自己也是 INFO，两者很容易互相
/// 冲掉：想确认「这几条连接走了哪边」时，得能把子进程那一层摘掉。文件里不用
/// 管这件事——文件层本来就只留 WARN 以上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFilter {
    /// 全都显示。
    All,
    /// 摘掉 tun2proxy 子进程的输出，留下程序提示与分流判定。
    NoOutput,
    /// 只看分流判定。
    SplitOnly,
}

impl LogFilter {
    /// 分段控件里的顺序。
    pub const ALL: [LogFilter; 3] = [
        LogFilter::All,
        LogFilter::NoOutput,
        LogFilter::SplitOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogFilter::All => t("全部", "All"),
            LogFilter::NoOutput => t("隐藏 TUN 输出", "Hide TUN output"),
            LogFilter::SplitOnly => t("仅分流", "Split only"),
        }
    }

    /// 这一档要不要留下 `kind` 这种行。
    pub fn keeps(self, kind: LogKind) -> bool {
        match self {
            LogFilter::All => true,
            LogFilter::NoOutput => kind != LogKind::Output,
            LogFilter::SplitOnly => kind == LogKind::Split,
        }
    }
}

/// 记下本次运行要用的配置与引擎（只在 `main` 里调一次）。
pub fn install(config: Config, engine: Engine) {
    let _ = CONFIG.set(Mutex::new(config));
    let _ = ENGINE.set(engine);
}

/// 本次运行配置的一份副本；界面在 `install` 之前渲染时返回 `None`。
///
/// 返回副本而不是引用：调用方多半只取一两个字段（配色、明暗），拿着副本既
/// 不会把锁带到界面渲染里，也不用关心别处正在改哪一项。
pub fn config() -> Option<Config> {
    CONFIG.get().map(|config| lock(config).clone())
}

/// 本次运行的引擎；界面在 `install` 之前渲染时返回 `None`。
pub fn engine() -> Option<&'static Engine> {
    ENGINE.get()
}

/// 记住用户这次选的配色，并写回配置文件。
pub fn remember_palette(palette: Palette) {
    let Some(config) = CONFIG.get() else {
        return;
    };
    let mut config = lock(config);
    if config.palette == palette {
        return;
    }
    config.palette = palette;
    if let Err(error) = config.save() {
        log::warn!("保存配色失败：{error}");
    }
}

/// 记住用户这次选的明暗，并写回配置文件。
pub fn remember_theme(setting: ThemeSetting) {
    let Some(config) = CONFIG.get() else {
        return;
    };
    let mut config = lock(config);
    if config.theme == setting {
        return;
    }
    config.theme = setting;
    if let Err(error) = config.save() {
        log::warn!("保存明暗设置失败：{error}");
    }
}

/// 启动隧道：先补齐配置、校验，再交给内核去探上游、起子进程。
///
/// 返回的错误是**本地校验**的结论（比如还没下载 tun2proxy），可以当场显示；
/// 「上游不通」这类结论要等后台探测，会由内核写进 `Shared::start_blocked`，
/// 界面从快照里读。从托盘菜单调到它时没有提示条可用，所以失败也写一行日志。
pub fn start() -> Result<(), String> {
    let Some(engine) = ENGINE.get() else {
        return Err("引擎还没准备好".to_string());
    };
    let Some(config) = CONFIG.get() else {
        return Err("配置还没读进来".to_string());
    };
    let mut config = lock(config);
    // 名字被清空过的话在这里补回来，否则界面上会留着一个没有名字的配置。
    config.normalize();
    let profile = config.active_profile().clone();
    let split = config.split.clone();
    let result = engine.start_with_precheck(&profile, &split);
    if result.is_ok() {
        // 启动成功才落盘：名字是 `normalize()` 补的，用户会以为它本来就在。
        if let Err(error) = config.save() {
            log::warn!("保存配置失败：{error}");
        }
    }
    if let Err(error) = &result {
        engine.log_line(error.clone());
    }
    result
}

/// 停止隧道。
///
/// `stop()` 只发中断信号，状态立刻变成「正在停止」；子进程真正退出、虚拟网卡
/// 回收由内核的监督线程负责，界面靠每秒刷新看到结果。
pub fn stop() {
    if let Some(engine) = ENGINE.get() {
        engine.stop();
    }
}

/// 清空界面上的日志（内存里的那份；按天写的文件保留，那是事后要查的证据）。
pub fn clear_logs() {
    if let Some(engine) = ENGINE.get() {
        engine.clear_logs();
    }
}

/// 去 GitHub 查一下 tun2proxy 有没有新版本。
///
/// 走后台线程，结论写进共享状态（[`crate::core::engine::Update`]），界面每秒
/// 从快照里读到——所以这里不返回任何东西，也不该在调用后阻塞。
pub fn check_update() {
    if let Some(engine) = ENGINE.get() {
        engine.check_update(release_repo());
    }
}

/// 下载并安装最新版 tun2proxy。
///
/// 隧道在跑的时候内核会直接拒绝（Windows 上换不了正在运行的 exe），这个结论
/// 同样以 `Update::Failed` 的形式回到界面。
pub fn install_update() {
    if let Some(engine) = ENGINE.get() {
        engine.install_update(release_repo());
    }
}

/// Release 从哪个仓库取：配置里可改（`release_repo`），默认官方仓库。
fn release_repo() -> String {
    config()
        .map(|config| config.release_repo)
        .unwrap_or_else(|| "tun2proxy/tun2proxy".to_string())
}

/// 写一行本程序自己的提示（托盘菜单这类没有提示条的地方用）。
pub fn log_line(line: impl Into<String>) {
    if let Some(engine) = ENGINE.get() {
        engine.log_line(line.into());
    }
}

/// 用户点关闭按钮时想干什么（配置里的 `close_action`）。
pub fn close_action() -> CloseAction {
    config()
        .map(|config| config.close_action)
        .unwrap_or(CloseAction::Ask)
}

/// 取锁；上一次持锁的线程 panic 了也照常用里面的值（配置不该因此失效）。
fn lock(config: &Mutex<Config>) -> std::sync::MutexGuard<'_, Config> {
    config.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
