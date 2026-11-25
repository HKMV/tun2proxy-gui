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

use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use tproxy_config::IpCidr;

use crate::core::config::{CloseAction, Palette, Profile, ThemeSetting};
use crate::core::i18n::{LangSetting, t};
use crate::core::logs::LogKind;
use crate::core::split::Library;
use crate::core::split::library::snapshot_dir;
use crate::core::split::settings::SplitGroup;
use crate::core::{Config, Engine};
use crate::tr_format;

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static ENGINE: OnceLock<Engine> = OnceLock::new();

/// 三份清单，外加「解析它们时三份文件的最后修改时间」。
///
/// 域名清单一万六千多条，每次渲染都从磁盘解析一遍会让筛选框敲一下卡一次；
/// 但清单在线更新完必须能看到新内容，所以缓存按文件时间失效，而不是一存到底。
static LIBRARY: OnceLock<Mutex<Option<(Option<std::time::SystemTime>, Library)>>> = OnceLock::new();

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

/// 让后端就绪：配置、语言、内核、上次残留的隧道网卡。
///
/// 启动路径有两条：
///
/// * **发布版**（`src/main.rs`）：单实例、提权、DPI 之后自己 load 配置和内核，
///   再 [`install`]——所以这里进去会发现已经有了，直接返回；
/// * **`nsh run App.tsx`**（开发时的 QuickJS host）：进程不是我们自己的 `main`
///   起来的，没人做这一步，于是第一次取数据时由 [`crate::bridge`] 调这里补上。
///
/// 两条路都不会重复初始化：`Once` 保证只跑一次，`CONFIG` 已就位时闭包里立即
/// 返回。
pub fn ensure_installed() {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        if CONFIG.get().is_some() {
            return;
        }
        let config = Config::load();
        crate::core::i18n::apply(config.language);
        let engine = Engine::new();
        // 开发时每次重启都可能撞上一次被强杀留下的网卡，和发布版一样先清掉。
        engine.clean_stale_adapters();
        install(config, engine);
    });
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

/// 记住关窗口的行为，并写回配置文件。
///
/// 只在下次关窗口时生效，不用重启：`intercept_close` 每次回调都现读配置。
pub fn remember_close_action(action: CloseAction) {
    log::info!("关闭按钮的行为改为：{}", action.label());
    with_config(|config| {
        if config.close_action == action {
            return false;
        }
        config.close_action = action;
        true
    });
}

/// 记住界面语言并立刻切过去。
///
/// 语言是进程级的全局量（[`i18n`](crate::core::i18n)），已经画出来的文字不会
/// 自己变——界面文字是构建期取值的。所以调用方拿到返回值后还要 bump 一下整窗
/// 重建的信号，让所有 `t()` 重新取一遍。
pub fn remember_language(setting: LangSetting) -> bool {
    log::info!("界面语言改为：{}", setting.label());
    let saved = with_config(|config| {
        if config.language == setting {
            return false;
        }
        config.language = setting;
        true
    });
    crate::core::i18n::apply(setting);
    saved
}

// —— 代理配置 ——

/// 后台检测一个上游地址通不通（最长 3 秒，结论由内核写进 `Shared::probes`）。
pub fn check_proxy(url: &str) {
    if let Some(engine) = ENGINE.get() {
        engine.check_proxy(url.to_string());
    }
}

/// 切换到第 `index` 套配置（下次启动隧道用它）。
pub fn select_profile(index: usize) -> bool {
    with_config(|config| {
        if config.active == index || index >= config.profiles.len() {
            return false;
        }
        config.select(index);
        true
    })
}

/// 新建一套配置（默认参数起步），返回新下标。
pub fn add_profile() -> Option<usize> {
    let mut added = None;
    with_config(|config| {
        added = Some(config.add_profile());
        true
    });
    added
}

/// 复制当前这套配置，返回新下标。
pub fn duplicate_profile() -> Option<usize> {
    let mut added = None;
    with_config(|config| {
        added = Some(config.duplicate_active());
        true
    });
    added
}

/// 删除第 `index` 套配置；只剩一套时什么也不做，返回是否删掉了。
pub fn remove_profile(index: usize) -> bool {
    with_config(|config| config.remove(index))
}

/// 把改好的配置写回第 `index` 套并落盘。
///
/// 校验交给调用方（表单边输入边校验，错误显示在字段旁边）；这里只负责写，
/// 写完返回是否成功，界面据此决定要不要退出编辑态。
pub fn save_profile(index: usize, profile: Profile) -> bool {
    with_config(|config| {
        let Some(slot) = config.profiles.get_mut(index) else {
            return false;
        };
        if *slot == profile {
            // 一个字都没改：不写盘，但界面上「保存」也该算成功。
            return true;
        }
        *slot = profile;
        true
    })
}

/// 改一次配置并落盘；`change` 返回「真的改了」才写盘。
fn with_config(change: impl FnOnce(&mut Config) -> bool) -> bool {
    let Some(config) = CONFIG.get() else {
        return false;
    };
    let mut config = lock(config);
    if !change(&mut config) {
        return false;
    }
    if let Err(error) = config.save() {
        log::warn!("保存配置失败：{error}");
    }
    true
}

// —— 分流规则 ——

/// 分流总开关。
///
/// 关掉时引擎**完全不起分流器**，与没有这个功能时一字不差；开着的改动要等
/// 下次启动隧道才生效，界面上那句状态说明就是这么写的。
pub fn set_split_enabled(enabled: bool) -> bool {
    with_config(|config| {
        if config.split.enabled == enabled {
            return false;
        }
        config.split.enabled = enabled;
        true
    })
}

/// 打开 / 关掉某一组内置清单。
pub fn set_split_group(group: SplitGroup, enabled: bool) -> bool {
    with_config(|config| {
        if config.split.group_enabled(group) == enabled {
            return false;
        }
        config.split.set_group(group, enabled);
        true
    })
}

/// 存自定义规则（整段文本），写完返回校验结论。
///
/// 校验失败也**要落盘**：用户可能只是暂时写错一行就想先存着，逼他改对才能
/// 保存会丢掉已经敲进去的几十行。规则解析失败时引擎那边按空处理，不影响启动。
pub fn save_split_custom(custom: String) -> Result<(), String> {
    let Some(config) = CONFIG.get() else {
        return Err(t("配置还没读进来", "The config is not loaded yet").to_string());
    };
    let mut config = lock(config);
    if config.split.custom == custom {
        return Ok(());
    }
    config.split.custom = custom;
    let verdict = config.split.parse_custom().map(|_| ());
    if let Err(error) = config.save() {
        log::warn!("保存配置失败：{error}");
    }
    verdict
}

/// 存直连出口：DNS 服务器与出口网卡。
pub fn save_split_outlet(direct_dns: String, direct_interface: String) -> Result<(), String> {
    let Some(config) = CONFIG.get() else {
        return Err(t("配置还没读进来", "The config is not loaded yet").to_string());
    };
    let mut config = lock(config);
    let dns = direct_dns.trim().to_string();
    let interface = direct_interface.trim().to_string();
    if config.split.direct_dns == dns && config.split.direct_interface == interface {
        return Ok(());
    }
    let previous = (config.split.direct_dns.clone(), config.split.direct_interface.clone());
    config.split.direct_dns = dns;
    config.split.direct_interface = interface;
    let verdict = config.split.validate().or_else(|error| {
        // 只有「直连 DNS 不是 IP」这类出口错误该由这一块负责；「一组清单都没开」
        // 是另一张卡片的事，别在这里拦着用户存出口。
        if config.split.direct_dns_addr().is_some() {
            Ok(())
        } else {
            Err(error)
        }
    });
    if verdict.is_err() {
        config.split.direct_dns = previous.0;
        config.split.direct_interface = previous.1;
        return verdict;
    }
    if let Err(error) = config.save() {
        log::warn!("保存配置失败：{error}");
    }
    Ok(())
}

/// 后台更新三份清单（走网络，结论由内核写进 `Shared::split_update`）。
pub fn update_split_lists() {
    let Some(engine) = ENGINE.get() else {
        return;
    };
    let settings = config().map(|config| config.split).unwrap_or_default();
    engine.update_split_lists(settings);
}

/// 内置清单的条数 `(局域网与保留, 国内 IP, 国内域名)`。
///
/// 取的是**编译进程序的快照**条数，不含运行时探测到的本机网卡段：那个数每次
/// 开机都可能变，摆在「内置清单」这一栏里反而让人以为清单被人改过。
pub fn split_counts() -> (usize, usize, usize) {
    let (local, ipv4, ipv6, domain) = crate::core::split::builtin::snapshot_size();
    (local, ipv4 + ipv6, domain)
}

/// 三份清单（进程内缓存，文件改过就重新解析）。
///
/// 走「传一个读闭包进来」而不是把 `&Library` 返回出去：缓存藏在锁里，
/// 返回引用要么泄漏、要么得把锁一起交出去；闭包让锁只在这几百微秒里被持有。
pub fn with_split_library<T>(read: impl FnOnce(&Library) -> T) -> T {
    let cell = LIBRARY.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|error| error.into_inner());
    let stamp = library_stamp();
    let stale = match guard.as_ref() {
        Some((cached, _)) => *cached != stamp,
        None => true,
    };
    if stale {
        *guard = Some((stamp, Library::load()));
    }
    match guard.as_ref() {
        Some((_, library)) => read(library),
        // 上面刚填过，走不到这里；真到了也退回内置快照而不是 panic。
        None => read(&Library::builtin()),
    }
}

/// 三份清单文件的最后修改时间；一份都没有时是 `None`（用的是内置快照）。
fn library_stamp() -> Option<std::time::SystemTime> {
    let dir = snapshot_dir();
    ["china-ipv4.txt", "china-ipv6.txt", "china-domain.txt"]
        .iter()
        .filter_map(|name| std::fs::metadata(dir.join(name)).ok()?.modified().ok())
        .max()
}

/// 关掉 / 打开清单里的某一条规则（键形如 `suffix:qq.com`）。
pub fn toggle_split_row(key: &str) -> bool {
    let key = key.to_string();
    with_config(move |config| {
        config.split.toggle_row(&key);
        true
    })
}

// —— 网络与绕过（都改当前那套配置） ——

/// 解析一个整数并检查范围（表单里的数字字段都走这里）。
///
/// 返回的错误是**给用户看的一句话**：哪一项、该填什么范围，直接摆在字段
/// 旁边，不让他去猜「填错了」是哪一种错。
pub fn parse_number(
    raw: &str,
    label: &'static str,
    low: u64,
    high: u64,
) -> Result<u64, String> {
    let value = raw
        .trim()
        .parse::<u64>()
        .map_err(|_| tr_format!("{label} 需要填数字", "{label} must be a number"))?;
    if value < low || value > high {
        return Err(tr_format!(
            "{label} 要在 {low} 到 {high} 之间",
            "{label} must be between {low} and {high}"
        ));
    }
    Ok(value)
}

/// 当前配置上的三个布尔开关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileFlag {
    /// 转发 IPv6（`--ipv6-enabled`）。
    Ipv6,
    /// 启动时自动配置系统路由。
    Setup,
    /// 卸载模式（`--unshare`）。
    Unshare,
}

/// 改当前配置上的一个开关。
pub fn set_profile_flag(flag: ProfileFlag, value: bool) -> bool {
    with_config(|config| {
        let profile = config.active_profile_mut();
        let slot = match flag {
            ProfileFlag::Ipv6 => &mut profile.ipv6,
            ProfileFlag::Setup => &mut profile.setup,
            ProfileFlag::Unshare => &mut profile.unshare,
        };
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    })
}

/// 存当前配置的 UDP 转发（udpgw）。服务器留空表示不用这个功能。
///
/// 服务器写错就**不写盘**：这三项是打包传给 tun2proxy 的，格式不对会让隧道
/// 直接起不来，而错误原因藏在子进程的输出里。
pub fn save_profile_udpgw(
    server: String,
    connections: usize,
    keepalive: u64,
) -> Result<(), String> {
    let server = server.trim().to_string();
    if !server.is_empty() && server.parse::<std::net::SocketAddr>().is_err() {
        return Err(t(
            "UDP 转发服务器要写成 host:port，例如 1.2.3.4:7300",
            "The UDP relay server must be host:port, e.g. 1.2.3.4:7300",
        )
        .to_string());
    }
    with_config(|config| {
        let profile = config.active_profile_mut();
        if profile.udpgw_server == server
            && profile.udpgw_connections == connections
            && profile.udpgw_keepalive == keepalive
        {
            return false;
        }
        profile.udpgw_server = server;
        profile.udpgw_connections = connections;
        profile.udpgw_keepalive = keepalive;
        true
    });
    Ok(())
}

/// 存当前配置的绕过列表。
pub fn save_profile_bypass(bypass: String) -> Result<(), String> {
    for line in bypass.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.parse::<std::net::IpAddr>().is_err() && IpCidr::from_str(line).is_err() {
            return Err(tr_format!(
                "「{}」不是合法的 IP 或 CIDR",
                "“{}” is not a valid IP or CIDR",
                line
            ));
        }
    }
    with_config(|config| {
        let profile = config.active_profile_mut();
        if profile.bypass == bypass {
            return false;
        }
        profile.bypass = bypass;
        true
    });
    Ok(())
}

/// 取锁；上一次持锁的线程 panic 了也照常用里面的值（配置不该因此失效）。
fn lock(config: &Mutex<Config>) -> std::sync::MutexGuard<'_, Config> {
    config.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
