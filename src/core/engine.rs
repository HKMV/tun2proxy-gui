//! tun2proxy 子进程引擎。
//!
//! 职责：把下载好的 tun2proxy 作为子进程拉起，持续读取它的 stdout/stderr，
//! 并把「运行状态、日志尾部、版本号、更新进度」写进一份共享状态，界面线程
//! 只读这份状态来渲染。
//!
//! 线程模型（没有任何 async runtime）：
//!
//! ```text
//!  UI 线程 ──start()/stop()──▶ Engine ──spawn──▶ tun2proxy 子进程
//!      ▲                         │
//!      │  request_rebuild()       ├─ log-reader 线程  ×2  ─┐
//!      └──── ui-notifier 线程 ◀────┴─ supervisor 线程     ├─▶ Shared(Mutex)
//!                                   └─ 下载/更新线程     ─┘
//! ```
//!
//! `ui-notifier` 把高频的日志/进度变化合并成最多 180ms 一次的重绘请求，
//! 避免子进程刷日志时把界面拖死。

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::Level;

use crate::core::binary;
use crate::core::config::Profile;
use crate::core::i18n;
use crate::core::i18n::t;
use crate::core::logs;
use crate::core::paths;
use crate::core::probe::{self, ProbeReport};
use crate::core::split::ipset::Cidr;
use crate::core::split::library::Library;
use crate::core::split::rules::RuleSet;
use crate::core::split::service::{
    CounterSnapshot, Decision, SplitService, SplitSink, StartPlan, TrendBucket,
};
use crate::core::split::settings::SplitSettings;
use crate::core::split::upstream::Upstream;
use crate::core::traffic::{self, RouteMeter, RouteTraffic, TrafficMeter};
use crate::tr_format;

const MAX_LOGS: usize = 2000;

/// tun2proxy 子进程的生命周期状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed(String),
}

impl Status {
    /// 是否处于「运行中或正在切换」的状态。
    ///
    /// 界面用它决定侧边栏按钮显示「启动」还是「停止」。
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Starting | Status::Running | Status::Stopping)
    }
}

/// 版本检查 / 下载安装的进度状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    Idle,
    Checking,
    Available { latest: String },
    Latest { version: String },
    Downloading { got: u64, total: Option<u64> },
    Installing,
    Done { version: String },
    Failed(String),
}

impl Update {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Update::Checking | Update::Downloading { .. } | Update::Installing
        )
    }
}

/// 界面线程与后台线程共享的可变状态。
///
/// 所有写入都经过 [`Engine`]，界面只读快照。
pub struct Shared {
    pub status: Status,
    pub update: Update,
    pub version: Option<String>,
    /// 最近的日志行（已经排版好，带来源——界面要按来源过滤）。
    pub logs: VecDeque<logs::LogLine>,
    /// 最近两分钟的流量采样（每秒一个点），按「直连 / 代理」分成两条，给仪表盘
    /// 的折线图用。
    ///
    /// 放在共享状态里而不是引擎自己算：界面每帧重建，图上的点必须能重复读到
    /// 同一份历史，否则「上一帧的曲线」和「这一帧的曲线」接不上。
    pub traffic: VecDeque<RouteTraffic>,
    /// 最新一次采样的瞬时速率（同样按两条路分）。
    pub rate: RouteTraffic,
    /// 上游代理连通性检测的状态（界面上那一行「检测」按钮的结果）。
    ///
    /// 和日志/流量一样放在共享状态里：检测在后台线程跑，界面每帧重建，
    /// 结果必须能重复读到同一份，否则按钮点完界面不知道该显示什么。
    pub probe: Option<ProbeState>,
    /// 「启动被拦下」的原因（一次性提示）。
    ///
    /// 启动前的连通性检测在后台线程跑，检测不过就写这里；界面把它当成
    /// 一条错误横幅显示，直到下一次启动尝试把它清掉。放在共享状态里
    /// 而不是 `App::error`：写它的是后台线程，拿不到界面状态。
    pub start_blocked: Option<String>,
    /// 会话名额被打满过（tun2proxy 在日志里报了 `dropping new session`）。
    ///
    /// 名额打满不会让隧道退出（本程序从不传 `--exit-on-fatal-error`），只是
    /// 新连接被丢：用户看到的现象是「某些网站打不开」，而那条 WARN 夹在几百
    /// 上千行子进程输出里，没人会翻到。这里只存一个「发生过」的标记，界面用
    /// 它挂一条提示条——打满正是最需要用户去调大上限的时刻。
    ///
    /// 检测点在 [`crate::core::logs`] 的界面层：子进程输出进内存只有那一条路。
    /// 上游那条是 WARN，所以日志级别被调成「错误」时不会产生，检测自然也不
    /// 生效——那是用户自己关掉了可见性，不属于这里的漏洞。
    pub sessions_full: bool,
    /// 子进程最后一条 ERROR 级别的输出。
    ///
    /// 单独存一份而不只是留在 `logs` 里：进程退出时状态机要拿它当「为什么
    /// 停了」的答案。tun2proxy 的致命错误都打在自己的 stderr 上（比如
    /// `Error: cannot open the tun device`），只报退出码等于什么也没说。
    pub last_error: Option<String>,
    /// 累计写入过多少行日志（不随 [`Shared::logs`] 的裁剪回退）。
    ///
    /// 界面靠它判断「有没有新行」：`logs.len()` 涨到上限就不再变，用它的话
    /// 日志框会停在半路再也不跟新，看起来就是「日志不滚」。
    pub log_total: u64,
    /// 重绘回调：后台线程改完状态后调用它，让界面重建一次。
    ///
    /// 用裸回调而不是直接存 GUI 句柄，是为了让 core 不依赖任何界面框架。
    pub rebuild: Option<Arc<dyn Fn() + Send + Sync>>,
    /// 分流器的运行情况（每秒同步一次）。
    pub split: SplitRuntime,
    /// 清单在线更新的进度。
    pub split_update: SplitUpdate,
}

impl Shared {
    /// 追加一行日志并维持长度上限。
    ///
    /// 直接用 `logs.push_back` 会漏掉 [`Shared::log_total`]，界面就不知道
    /// 又来了新行，所以三条写入路径（应用提示、分流器、子进程输出）都走这里。
    ///
    /// **只写内存**：落盘由 [`crate::core::logs`] 的格式化层负责，两者吃的是
    /// 同一批 tracing 事件，所以界面与文件不会对不上（唯一的差别是文件层丢掉
    /// 了子进程的 INFO 行）。点「清空日志」也只清这里，文件保留——那是事后
    /// 要查的证据。
    pub fn push_log(&mut self, kind: logs::LogKind, line: impl Into<String>) {
        self.logs.push_back(logs::LogLine {
            kind,
            text: line.into(),
        });
        self.log_total += 1;
        while self.logs.len() > MAX_LOGS {
            self.logs.pop_front();
        }
    }

    /// 取最后 `count` 行日志（给界面用）。
    pub fn log_tail(&self, count: usize) -> Vec<logs::LogLine> {
        let skip = self.logs.len().saturating_sub(count);
        self.logs.iter().skip(skip).cloned().collect()
    }
}

/// 代理检测的状态机。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    /// 正在检测（按钮显示「检测中…」）。
    Running,
    /// 出结论了。
    Done(ProbeReport),
}

/// 分流器当前的运行情况（给界面读的那一份）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitRuntime {
    pub running: bool,
    pub addr: Option<std::net::SocketAddr>,
    pub rules: usize,
    pub direct: u64,
    pub proxy: u64,
    pub failed: u64,
    pub sessions: u64,
    /// 最近的判定，**新的在前**（界面上的「最近判定」列表直接按顺序画）。
    pub decisions: Vec<Decision>,
    /// 分流去向的时序，**旧 → 新**，最后一格是「现在」。
    ///
    /// 长度固定（[`crate::core::split::TREND_BUCKETS`]）：界面画折线时按位置
    /// 摊开，不用再自己补零或对齐时间轴。
    pub trend: Vec<TrendBucket>,
}

/// 界面上一屏能看到多少条判定：再多就要往回滚，滚动条自己会长出来。
pub const SPLIT_DECISIONS_SHOWN: usize = 20;

impl SplitRuntime {
    fn from_service(service: &SplitService) -> Self {
        let counters: CounterSnapshot = service.counters();
        Self {
            running: service.is_running(),
            addr: service.address(),
            rules: service.rule_count(),
            direct: counters.direct,
            proxy: counters.proxy,
            failed: counters.failed,
            sessions: counters.sessions,
            decisions: service.recent_decisions(SPLIT_DECISIONS_SHOWN),
            trend: service.trend(),
        }
    }
}

/// 清单在线更新的进度（给界面的「立即更新」按钮用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitUpdate {
    Idle,
    Working(String),
    Done(String),
    Failed(String),
}

impl SplitUpdate {
    pub fn is_busy(&self) -> bool {
        matches!(self, SplitUpdate::Working(_))
    }

    /// 界面上那一行状态文字。
    pub fn text(&self) -> Option<&str> {
        match self {
            SplitUpdate::Idle => None,
            SplitUpdate::Working(text) | SplitUpdate::Done(text) | SplitUpdate::Failed(text) => {
                Some(text)
            }
        }
    }
}

/// 把分流器的日志与状态接到引擎的共享状态上。
struct EngineSink {
    dirty: Arc<AtomicBool>,
}

impl SplitSink for EngineSink {
    fn log(&self, line: String) {
        // 走 tracing 而不是直接 push：同一条事件由日志层分发到界面和文件，
        // 分流器的判定才会出现在磁盘上（以前它只进内存）。
        tracing::info!(target: logs::TARGET_SPLIT, "{}", line);
        self.dirty.store(true, Ordering::Relaxed);
    }
}

/// 引擎句柄。内部全是 `Arc`，可以随意 clone 后丢给后台线程。
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    stopping: Arc<AtomicBool>,
    split: SplitService,
}

impl Engine {
    /// 创建引擎：装好日志（tracing + 落盘 + 界面）、启动重绘调度线程，
    /// 并探测已安装版本。
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::Stopped,
            update: Update::Idle,
            version: binary::installed_version(),
            logs: VecDeque::new(),
            traffic: VecDeque::new(),
            rate: RouteTraffic::default(),
            probe: None,
            start_blocked: None,
            sessions_full: false,
            last_error: None,
            log_total: 0,
            rebuild: None,
            split: SplitRuntime::default(),
            split_update: SplitUpdate::Idle,
        }));
        let dirty = Arc::new(AtomicBool::new(false));
        // 日志要先于任何一条 `log_line` 装好，否则启动初期那几行只进了内存。
        // 建不出文件不算致命错误：界面日志照旧，下面提示一句原因。
        let log_error = logs::init(shared.clone(), dirty.clone()).err();

        let engine = Self {
            shared,
            dirty,
            child: Arc::new(Mutex::new(None)),
            stopping: Arc::new(AtomicBool::new(false)),
            split: SplitService::new(),
        };
        engine.spawn_notifier();
        engine.spawn_split_watcher();
        engine.log_line(tr_format!(
            "配置目录 {}",
            "Config directory {}",
            paths::app_dir().display()
        ));
        match log_error {
            None => engine.log_line(tr_format!(
                "运行日志 {}（按天一个文件，保留最近 7 天）",
                "Run log {} (one file per day, keeps the last 7)",
                logs::current_file(&paths::logs_dir()).display()
            )),
            Some(err) => engine.log_line(tr_format!(
                "无法创建运行日志文件：{err}（界面日志不受影响）",
                "Cannot create the run log file: {err} (the in-app log is unaffected)"
            )),
        }
        engine
    }

    /// 每秒把分流器的状态抄进共享状态：界面每帧都读同一份快照，
    /// 不让它直接去锁分流器的内部结构。
    fn spawn_split_watcher(&self) {
        let shared = self.shared.clone();
        let dirty = self.dirty.clone();
        let service = self.split.clone();
        std::thread::Builder::new()
            .name("split-watcher".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let next = SplitRuntime::from_service(&service);
                if let Ok(mut shared) = shared.lock() {
                    if shared.split != next {
                        shared.split = next;
                        drop(shared);
                        dirty.store(true, Ordering::Relaxed);
                    }
                }
            })
            .ok();
    }

    pub fn shared(&self) -> Arc<Mutex<Shared>> {
        self.shared.clone()
    }

    /// 注入「请求界面重绘」的回调（由 GUI 层提供，包装成一次
    /// `cx.notify()`；内核不知道界面框架是什么）。
    pub fn set_rebuild_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.rebuild = Some(hook);
        }
    }

    pub fn is_active(&self) -> bool {
        self.child
            .lock()
            .map(|child| child.is_some())
            .unwrap_or(false)
    }

    /// 当前状态（给托盘这类「不在界面线程上」的调用方用）。
    pub fn status(&self) -> Status {
        self.shared
            .lock()
            .map(|shared| shared.status.clone())
            .unwrap_or(Status::Stopped)
    }

    /// 在后台线程里检测上游代理是否可用。
    ///
    /// 必须异步：`TcpStream::connect_timeout` 最长要等 3 秒，同步跑会把界面
    /// 冻住。这里先写 `Running` 让按钮立刻变成「检测中…」，出结论后再写一次。
    pub fn check_proxy(&self, url: String) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.probe = Some(ProbeState::Running);
        }
        self.touch();
        self.request_rebuild();
        let shared = self.shared.clone();
        let dirty = self.dirty.clone();
        let target = url.clone();
        let spawned = std::thread::Builder::new()
            .name("proxy-probe".to_string())
            .spawn(move || {
                let report = probe::check(&target);
                if let Ok(mut shared) = shared.lock() {
                    shared.probe = Some(ProbeState::Done(report));
                }
                dirty.store(true, Ordering::Relaxed);
                // 检测线程自己也要唤醒界面：它不走 180ms 的批量调度，
                // 结论出来就该立刻显示。
                let hook = shared.lock().ok().and_then(|shared| shared.rebuild.clone());
                if let Some(hook) = hook {
                    hook();
                }
            });
        if spawned.is_err() {
            if let Ok(mut shared) = self.shared.lock() {
                shared.probe = Some(ProbeState::Done(ProbeReport {
                    ok: false,
                    detail: t("无法启动检测线程", "Cannot spawn the probe thread").to_string(),
                    millis: 0,
                }));
            }
        }
    }

    /// 立刻请求界面重绘（不等 180ms 的批量调度）。
    fn request_rebuild(&self) {
        let hook = self
            .shared
            .lock()
            .ok()
            .and_then(|shared| shared.rebuild.clone());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 只清界面上的日志，**不动**磁盘上的文件。
    ///
    /// 清空是「我要重新看一遍接下来发生什么」，而文件是事后排查的证据——
    /// 用户点一下就把刚过去的故障现场删掉，那是帮倒忙。
    pub fn clear_logs(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.logs.clear();
        }
        self.touch();
    }

    /// 打一条本程序自己的提示。
    ///
    /// 不再直接往共享状态里塞：走 tracing 之后，同一条记录会由日志层分别交给
    /// 界面（[`crate::core::logs`] 的界面层）和文件（按天滚动），两条线不会
    /// 各说各话。
    pub fn log_line(&self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("{line}");
        tracing::info!(target: logs::TARGET_APP, "{}", line);
    }

    /// 关掉「启动被拦下」的原因。
    ///
    /// 原因存在共享状态里（检测线程写的），只清界面那一份，下一帧从快照里
    /// 又会读出来——提示条上的 ✕ 得真的把它清掉。
    pub fn dismiss_start_blocked(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.start_blocked = None;
        }
        self.touch();
    }

    /// 关掉「会话名额已打满」的提示条。
    ///
    /// 和 [`Engine::dismiss_start_blocked`] 一样：标记在共享状态里，不清掉的话
    /// 下一帧又会从快照里读出来。
    pub fn dismiss_sessions_full(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.sessions_full = false;
        }
        self.touch();
    }

    fn touch(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Wakes the UI at most every 180 ms, so a chatty child process cannot
    /// trigger a rebuild per log line.
    fn spawn_notifier(&self) {
        let shared = self.shared.clone();
        let dirty = self.dirty.clone();
        std::thread::Builder::new()
            .name("ui-notifier".to_string())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(180));
                if dirty.swap(false, Ordering::Relaxed) {
                    let hook = shared.lock().ok().and_then(|shared| shared.rebuild.clone());
                    if let Some(hook) = hook {
                        hook();
                    }
                }
            })
            .ok();
    }

    fn set_status(&self, status: Status) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.status = status;
        }
        self.touch();
    }

    fn set_update(&self, update: Update) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.update = update;
        }
        self.touch();
    }

    fn set_version(&self, version: Option<String>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.version = version;
        }
        self.touch();
    }

    /// 启动前先探一次上游代理地址，不通就不启动。
    ///
    /// 为什么必须探：tun2proxy 一起来就接管 TUN 网卡和默认路由，上游写错时
    /// 流量是**静默黑洞**——用户看到的是「网断了」，而不是「代理连不上」，
    /// 比直接报错难查得多。所以宁可晚一两秒启动。
    ///
    /// 探针在后台线程跑（`connect_timeout` 最长 3 秒，同步跑会冻住界面）：
    /// 通过就接着 [`Engine::start`]，不通过就把原因写进 `Shared::start_blocked`
    /// 交给界面显示。返回值只表示「本地校验过了、检测已经开始」。
    pub fn start_with_precheck(
        &self,
        profile: &Profile,
        settings: &SplitSettings,
    ) -> Result<(), String> {
        if self.is_active() {
            return Err(i18n::t(
                t("服务已经在运行", "The service is already running"),
                "The service is already running",
            )
            .to_string());
        }
        // 本地能查的先查掉：没下载二进制就不必白探一次。
        if !binary::exe_path().is_file() {
            return Err(i18n::t(
                t(
                    "还没有下载 tun2proxy，请先到「程序与日志」页点「下载并安装」",
                    "tun2proxy is not installed yet. Download it on the Program page first.",
                ),
                "tun2proxy is not installed yet. Download it on the Program page first.",
            )
            .to_string());
        }
        // 分流开着但设置不合法（直连 DNS 写错了之类）时，与其起一条半残的隧道，
        // 不如现在就告诉用户哪里不对。
        if let Err(err) = settings.validate() {
            return Err(err);
        }

        let url = profile.proxy_url.clone();
        let profile = profile.clone();
        let settings = settings.clone();
        if let Ok(mut shared) = self.shared.lock() {
            shared.probe = Some(ProbeState::Running);
            shared.start_blocked = None;
        }
        self.touch();
        self.request_rebuild();

        let engine = self.clone();
        let spawned = std::thread::Builder::new()
            .name("start-precheck".to_string())
            .spawn(move || {
                let report = probe::check(&url);
                let ok = report.ok;
                let detail = report.detail.clone();
                if let Ok(mut shared) = engine.shared.lock() {
                    shared.probe = Some(ProbeState::Done(report));
                    if !ok {
                        shared.start_blocked = Some(format!(
                            "{} {detail}",
                            i18n::t(
                                t(
                                    "上游代理连不通，已取消启动：",
                                    "Upstream proxy unreachable, start cancelled:"
                                ),
                                "Upstream proxy unreachable, start cancelled:"
                            )
                        ));
                    }
                }
                if ok {
                    if let Err(err) = engine.start(&profile, &settings) {
                        if let Ok(mut shared) = engine.shared.lock() {
                            shared.start_blocked = Some(err);
                        }
                    }
                }
                engine.touch();
                engine.request_rebuild();
            });
        if let Err(err) = spawned {
            if let Ok(mut shared) = self.shared.lock() {
                shared.start_blocked = Some(format!(
                    "{} {err}",
                    i18n::t("无法启动检测线程：", "Cannot spawn the probe thread:")
                ));
            }
            self.request_rebuild();
        }
        Ok(())
    }

    /// 按一套代理配置启动 tun2proxy 子进程（不阻塞，失败时返回可读的中文错误）。
    ///
    /// 分流开着时，先在本机起 SOCKS5 分流器，再把 `--proxy` 指过去；其它参数
    /// 一个都不改。分流器起不来（网卡找不到、上游协议不支持…）时**自动回退**：
    /// 照原样启动 tun2proxy，并把原因写进日志——宁可这一轮没有分流，也不能
    /// 因为一个附加功能把整条隧道拖下水。
    pub fn start(&self, profile: &Profile, settings: &SplitSettings) -> Result<(), String> {
        if self.is_active() {
            return Err(t("服务已经在运行", "The service is already running").to_string());
        }
        let exe = binary::exe_path();
        if !exe.is_file() {
            return Err(t(
                "还没有下载 tun2proxy，请先到「程序与日志」页点「下载并安装」",
                "tun2proxy is not installed yet. Download it on the Program page first.",
            )
            .to_string());
        }

        let mut effective = profile.clone();
        // 网卡名在这里定死：用户填了就用填的，留空就本次随机一个。定下来之后
        // `cli_args` 传给 tun2proxy 的 `--tun` 和流量采样找网卡用的是同一个字符串。
        effective.tun_name = match profile.tun_name_configured() {
            Some(name) => name.to_string(),
            None => {
                let name = crate::core::config::random_tun_name();
                self.log_line(tr_format!(
                    "隧道网卡：{}（本次启动随机，可在「代理配置」里固定下来）",
                    "Tunnel adapter: {} (random for this run; pin it under Profiles)",
                    name
                ));
                name
            }
        };
        // 上一次被强杀留下的网卡（`t2p` 前缀、当前不在用）在起新隧道之前再清一次：
        // 程序启动时已经清过，这里是兜底（比如启动后被别的进程又留下了一张）。
        self.clean_stale_adapters();
        if settings.enabled {
            match self.start_split(profile, settings) {
                Ok(port) => {
                    effective.proxy_url = format!("socks5://127.0.0.1:{port}");
                    self.log_line(tr_format!(
                        "分流已启用：tun2proxy 的上游改为本机 127.0.0.1:{}",
                        "Split routing enabled: tun2proxy now uses the local proxy at 127.0.0.1:{}",
                        port
                    ));
                }
                Err(err) => {
                    self.log_line(tr_format!(
                        "分流器启动失败，本次启动按原配置走代理：{err}",
                        "The splitter failed to start; this run uses the proxy as configured: {err}"
                    ));
                }
            }
        } else {
            self.split.stop();
        }

        let args = effective.cli_args();
        // 上一次运行的报错不能留到这一次：状态机拿它解释「为什么停了」，
        // 留着的话新一次失败会报出旧原因。「会话打满」的提示同理，重启之后
        // 名额是新的，旧提示挂在界面上只会误导。
        if let Ok(mut shared) = self.shared.lock() {
            shared.last_error = None;
            shared.sessions_full = false;
        }
        self.log_line(tr_format!(
            "启动：{} {}",
            "Starting: {} {}",
            exe.display(),
            args.join(" ")
        ));

        let mut command = Command::new(&exe);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Own process group (so it can receive CTRL_BREAK later) and no
            // console window.
            command.creation_flags(0x0000_0200 | 0x0800_0000);
        }

        let mut child = command.spawn().map_err(|err| {
            tr_format!("无法启动 tun2proxy：{err}", "Cannot start tun2proxy: {err}")
        })?;

        if let Some(stdout) = child.stdout.take() {
            spawn_reader(stdout, self.shared.clone(), self.dirty.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_reader(stderr, self.shared.clone(), self.dirty.clone());
        }

        *self
            .child
            .lock()
            .map_err(|_| t("状态锁损坏", "The state lock is poisoned").to_string())? = Some(child);
        self.stopping.store(false, Ordering::Relaxed);
        self.set_status(Status::Starting);
        // 流量采样要按网卡名找 TUN：名字就是上面 `cli_args` 里传给 `--tun` 的
        // 那个（留空时已经换成随机名），两边取的是同一个字段。
        self.spawn_supervisor(effective.tun_name.clone());
        Ok(())
    }

    /// 清理本程序留下、当前没人要的隧道进程与网卡，把结果写进日志。
    ///
    /// 程序启动时和每次启动隧道前各调一次：网卡名是每次随机生成的，进程被强杀时
    /// 那张 Wintun 适配器和它的持有者都会留在系统里。
    ///
    /// 两步的顺序由 [`crate::core::orphans::sweep`] 写死：**先结束孤儿进程，再删
    /// 网卡**。反过来做不会报错，只是网卡一张也删不掉——孤儿攥着它，它的状态一直
    /// 是 Up，而 Up 的网卡按设计不删；等到下次启动撞名失败，才有人回头查原因。
    ///
    /// 没有管理员权限时 `remove_stale` 直接返回空结果，结束进程则只对本用户的
    /// 进程有效；两种情况都会写日志，不静默。
    pub fn clean_stale_adapters(&self) {
        // 自己拉着的子进程不是孤儿：停止 / 重启的那一瞬它还在跑。
        let held: Vec<u32> = self
            .child
            .lock()
            .ok()
            .and_then(|child| child.as_ref().map(std::process::Child::id))
            .into_iter()
            .collect();
        let (orphans, stale) = crate::core::orphans::sweep(
            || crate::core::orphans::terminate(&binary::exe_path(), &held),
            crate::core::adapters::remove_stale,
        );

        if !orphans.killed.is_empty() {
            self.log_line(tr_format!(
                "已结束残留的 tun2proxy 进程：PID {}",
                "Ended leftover tun2proxy process(es): PID {}",
                join_pids(&orphans.killed)
            ));
        }
        if !orphans.failed.is_empty() {
            self.log_line(tr_format!(
                "有 {} 个残留 tun2proxy 进程结束不掉（需要管理员权限）：PID {}",
                "Could not end {} leftover tun2proxy process(es) (administrator rights needed): PID {}",
                orphans.failed.len(),
                join_pids(&orphans.failed)
            ));
        }

        if !stale.removed.is_empty() {
            self.log_line(tr_format!(
                "已清理残留隧道网卡：{}",
                "Removed leftover tunnel adapters: {}",
                stale.removed.join("、")
            ));
        }
        if !stale.failed.is_empty() {
            self.log_line(tr_format!(
                "有 {} 张残留隧道网卡删不掉（需要管理员权限）：{}",
                "Could not remove {} leftover tunnel adapter(s) (administrator rights needed): {}",
                stale.failed.len(),
                stale.failed.join("、")
            ));
        }
    }

    /// 起本机分流器，返回它监听的端口。
    fn start_split(&self, profile: &Profile, settings: &SplitSettings) -> Result<u16, String> {
        let library = Library::load();
        let rules = library.direct_rules(settings)?;
        let rule_count = rules.len();
        if rule_count == 0 {
            return Err(t(
                "分流规则是空的：请至少打开一组清单，或写一条自定义规则",
                "The split rule set is empty: enable a list or add a custom rule",
            )
            .to_string());
        }
        let upstream = Upstream::parse(&profile.proxy_url)?;
        let direct_dns = settings.direct_dns_addr().ok_or_else(|| {
            t(
                "直连 DNS 需要填一个 IP 地址，例如 223.5.5.5",
                "The direct DNS must be an IP address, e.g. 223.5.5.5",
            )
            .to_string()
        })?;
        let fake_pool = Cidr::parse(profile.virtual_dns_pool.trim()).ok();
        self.log_line(tr_format!(
            "分流清单：{}（{}），规则 {} 条",
            "Split lists: {} ({}), {} rules",
            library.origin.label(),
            library.date,
            rule_count
        ));

        let plan = StartPlan {
            // 端口交给系统分配：固定端口会和用户机器上别的代理撞车。
            listen: "127.0.0.1:0".parse().expect("常量地址"),
            upstream,
            rules: RuleSet::build(&rules).0,
            rule_count,
            direct_dns,
            outlet_hint: settings.direct_interface.clone(),
            fake_pool,
        };
        let sink: Arc<dyn SplitSink> = Arc::new(EngineSink {
            dirty: self.dirty.clone(),
        });
        let addr = self.split.start(plan, sink)?;
        Ok(addr.port())
    }

    /// 规则变了（开关某一行、改了自定义规则）时热更新，不重启隧道。
    pub fn reload_split(&self, settings: &SplitSettings) -> Result<usize, String> {
        if !self.split.is_running() {
            return Ok(0);
        }
        let library = Library::load();
        let rules = library.direct_rules(settings)?;
        let count = rules.len();
        self.split.reload(RuleSet::build(&rules).0, count)?;
        Ok(count)
    }

    pub fn split_update(&self) -> SplitUpdate {
        self.shared
            .lock()
            .map(|shared| shared.split_update.clone())
            .unwrap_or(SplitUpdate::Idle)
    }

    /// 后台下载并替换清单；成功后立刻把新规则灌进正在跑的隧道。
    pub fn update_split_lists(&self, settings: SplitSettings) {
        if self.split_update().is_busy() {
            return;
        }
        let engine = self.clone();
        engine.set_split_update(SplitUpdate::Working(
            t("准备下载…", "Preparing to download…").to_string(),
        ));
        let spawned = std::thread::Builder::new()
            .name("split-update".to_string())
            .spawn(move || {
                let progress_engine = engine.clone();
                let progress = move |message: String| {
                    progress_engine.set_split_update(SplitUpdate::Working(message));
                };
                match crate::core::split::update::run(&progress) {
                    Ok(outcome) => {
                        let note = tr_format!(
                            "已更新 · {} · IPv4 {} 条 / IPv6 {} 条 / 域名 {} 条（{:.1} 秒）",
                            "Updated · {} · {} IPv4 / {} IPv6 / {} domain rules ({:.1}s)",
                            outcome.date,
                            outcome.ipv4,
                            outcome.ipv6,
                            outcome.domain,
                            outcome.seconds
                        );
                        engine.log_line(note.clone());
                        match engine.reload_split(&settings) {
                            Ok(count) if count > 0 => engine.log_line(tr_format!(
                                "分流规则已热更新：{} 条",
                                "Split rules hot-reloaded: {}",
                                count
                            )),
                            Ok(_) => {}
                            Err(err) => engine.log_line(tr_format!(
                                "清单已更新，但热更新失败（隧道需要重启一次）：{err}",
                                "The lists were updated but the hot reload failed (restart the tunnel once): {err}"
                            )),
                        }
                        engine.set_split_update(SplitUpdate::Done(note));
                    }
                    Err(err) => {
                        engine.log_line(tr_format!(
                            "清单更新失败：{err}",
                            "Updating the lists failed: {err}"
                        ));
                        // 失败不等于没有规则：内置快照一直可用，这句话必须跟着
                        // 错误一起出现，否则用户会以为分流现在没规则了。
                        engine.set_split_update(SplitUpdate::Failed(tr_format!(
                            "{err}（内置清单仍在生效，可以稍后重试）",
                            "{err} (the built-in lists are still in effect; retry later)"
                        )));
                    }
                }
            });
        if spawned.is_err() {
            self.set_split_update(SplitUpdate::Failed(
                t("无法启动下载线程", "Cannot spawn the download thread").to_string(),
            ));
        }
    }

    fn set_split_update(&self, update: SplitUpdate) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.split_update = update;
        }
        self.touch();
        self.request_rebuild();
    }

    /// 请求子进程退出：先发中断信号让它自己清理路由，6 秒后仍未退出则强杀。
    pub fn stop(&self) {
        // 先关分流器：tun2proxy 一停，它那个上游就没人连了。
        self.split.stop();
        let mut guard = match self.child.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let Some(child) = guard.as_mut() else {
            return;
        };
        self.stopping.store(true, Ordering::Relaxed);
        self.set_status(Status::Stopping);
        let pid = child.id();
        if !send_interrupt(pid) {
            self.log_line(t(
                "无法发送中断信号，直接结束进程",
                "Cannot send the interrupt signal, killing the process instead",
            ));
            let _ = child.kill();
        } else {
            self.log_line(t("已请求 tun2proxy 退出", "Asked tun2proxy to exit"));
        }
        drop(guard);

        // Hard deadline: if the child ignores the interrupt, kill it.
        let child = self.child.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(6));
            if let Ok(mut guard) = child.lock() {
                if let Some(child) = guard.as_mut() {
                    let _ = child.kill();
                }
            }
        });
    }

    /// 窗口关闭时调用：确保子进程已经退出，避免残留路由/网卡。
    pub fn shutdown_blocking(&self) {
        if !self.is_active() {
            return;
        }
        self.stop();
        let deadline = Instant::now() + Duration::from_secs(7);
        while Instant::now() < deadline {
            if !self.is_active() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }

    fn spawn_supervisor(&self, tun_name: String) {
        let shared = self.shared.clone();
        let child = self.child.clone();
        let stopping = self.stopping.clone();
        let dirty = self.dirty.clone();
        // 分路字节只有分流器知道（它是两条路的唯一中继点）：每秒问它要一次累计
        // 值，差分成两条速率。分流器没在跑时这个句柄照样能用——`is_running()`
        // 为 false，那时全部流量都算代理。
        let split = self.split.clone();

        std::thread::Builder::new()
            .name("tun2proxy-supervisor".to_string())
            .spawn(move || {
                let mut ticks = 0u32;
                let mut announced = false;
                let mut meter = TrafficMeter::new(&tun_name);
                let mut route_meter = RouteMeter::default();
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    ticks += 1;

                    let exit = {
                        let mut guard = match child.lock() {
                            Ok(guard) => guard,
                            Err(_) => return,
                        };
                        match guard.as_mut() {
                            Some(process) => match process.try_wait() {
                                Ok(Some(status)) => Some(Ok(status)),
                                Ok(None) => None,
                                Err(err) => Some(Err(err)),
                            },
                            None => return,
                        }
                    };

                    if let Some(result) = exit {
                        let was_stopping = stopping.load(Ordering::Relaxed);
                        if let Ok(mut guard) = child.lock() {
                            *guard = None;
                        }
                        // 读在写状态之前：解释失败原因要用它，而写状态时又
                        // 会拿同一把锁。
                        let reason = shared
                            .lock()
                            .ok()
                            .and_then(|shared| shared.last_error.clone());
                        let status = match result {
                            Ok(status) if was_stopping || status.success() => Status::Stopped,
                            Ok(status) => Status::Failed(explain_exit(
                                &describe_exit(&status),
                                reason.as_deref(),
                            )),
                            Err(err) => Status::Failed(tr_format!(
                                "无法获取进程状态：{}",
                                "Cannot read the process status: {}",
                                err
                            )),
                        };
                        if let Ok(mut shared) = shared.lock() {
                            shared.status = status;
                            // 进程没了，速率归零；历史留着，曲线慢慢滑出视野。
                            shared.rate = RouteTraffic::default();
                            // 「会话打满」的提示条只对「隧道还在跑、只是丢连接」
                            // 有意义。进程已经没了（用户点了停止，或崩了），
                            // 提示条再挂着就是一句过期的话。
                            shared.sessions_full = false;
                        }
                        meter.reset();
                        dirty.store(true, Ordering::Relaxed);
                        return;
                    }

                    // 每 4 个 tick（约 1 秒）采一次流量。
                    //
                    // 采样失败（网卡还没建出来、读不到计数器）就跳过这一拍：
                    // 折线图少一个点比画一个假的尖峰好。
                    if ticks.is_multiple_of(4) {
                        let sample = {
                            let guard = match child.lock() {
                                Ok(guard) => guard,
                                Err(_) => return,
                            };
                            match guard.as_ref() {
                                // 子进程还在才采：网卡是它建的，它没了网卡也就没了。
                                Some(process) => meter.sample(Some(process)),
                                None => None,
                            }
                        };
                        if let Some(sample) = sample {
                            // 网卡给的是隧道总数，分流器给的是「哪条路跑了多少」。
                            // 分流器在跑就用它的实测，没跑就把总数整份算代理——
                            // 这时每一份流量确实都从上游代理走（见 `RouteMeter`）。
                            let routed = route_meter.sample(
                                split.is_running(),
                                split.route_bytes(),
                                sample,
                            );
                            if let Ok(mut shared) = shared.lock() {
                                shared.rate = routed;
                                traffic::push(&mut shared.traffic, routed);
                            }
                            dirty.store(true, Ordering::Relaxed);
                        }
                    }

                    if !announced && ticks >= 3 {
                        announced = true;
                        let mut changed = false;
                        if let Ok(mut shared) = shared.lock() {
                            if shared.status == Status::Starting {
                                shared.status = Status::Running;
                                changed = true;
                            }
                        }
                        if changed {
                            dirty.store(true, Ordering::Relaxed);
                        }
                    }
                }
            })
            .ok();
    }

    /// 查询最新版本：只比对版本号，不下载。
    pub fn check_update(&self, repo: String) {
        if self.update_busy() {
            return;
        }
        self.set_update(Update::Checking);
        let engine = self.clone();
        std::thread::spawn(move || match binary::fetch_release(&repo) {
            Ok(release) => {
                let installed = engine
                    .shared
                    .lock()
                    .ok()
                    .and_then(|shared| shared.version.clone());
                let Some(asset) = binary::asset_for_this_platform(&release) else {
                    engine.set_update(Update::Failed(tr_format!(
                        "{} 没有适配当前平台的安装包",
                        "{} has no package for this platform",
                        release.tag_name
                    )));
                    return;
                };
                let latest = binary::normalize_tag(&release.tag_name);
                match installed {
                    Some(installed) if binary::normalize_tag(&installed) == latest => {
                        engine.set_update(Update::Latest { version: installed });
                    }
                    _ => {
                        let _ = asset;
                        engine.set_update(Update::Available {
                            latest: release.tag_name.clone(),
                        });
                    }
                }
            }
            Err(err) => engine.set_update(Update::Failed(err)),
        });
    }

    /// 下载并安装最新版本（需先停止服务，否则 Windows 上无法替换正在运行的 exe）。
    pub fn install_update(&self, repo: String) {
        if self.update_busy() {
            return;
        }
        if self.is_active() {
            self.set_update(Update::Failed(
                t("请先停止服务再更新", "Stop the service before updating").to_string(),
            ));
            return;
        }
        self.set_update(Update::Checking);
        let engine = self.clone();
        std::thread::spawn(move || {
            let result = install_latest(&engine, &repo);
            match result {
                Ok(version) => {
                    engine.set_version(Some(version.clone()));
                    engine.set_update(Update::Done { version });
                }
                Err(err) => engine.set_update(Update::Failed(err)),
            }
        });
    }

    fn update_busy(&self) -> bool {
        self.shared
            .lock()
            .map(|shared| shared.update.is_busy())
            .unwrap_or(false)
    }
}

fn install_latest(engine: &Engine, repo: &str) -> Result<String, String> {
    let release = binary::fetch_release(repo)?;
    let asset = binary::asset_for_this_platform(&release).ok_or_else(|| {
        tr_format!(
            "{} 没有适配当前平台的安装包",
            "{} has no package for this platform",
            release.tag_name
        )
    })?;

    engine.log_line(tr_format!(
        "下载 {}",
        "Downloading {}",
        asset.browser_download_url
    ));
    let archive = paths::app_dir().join("tun2proxy-download.zip");
    let total = asset.size;
    // 先进入「下载中」：连接卡住、一个字节都没下来时，界面也不会一直停在
    // 「正在查询最新版本…」上。
    engine.set_update(Update::Downloading {
        got: 0,
        total: Some(total),
    });
    let progress = {
        let engine = engine.clone();
        move |got: u64| {
            engine.set_update(Update::Downloading {
                got,
                total: Some(total),
            });
        }
    };
    binary::download(&asset.browser_download_url, &archive, progress)?;

    engine.set_update(Update::Installing);
    if let Some(digest) = asset.digest.as_deref() {
        binary::verify_sha256(&archive, digest)?;
        engine.log_line(t("SHA256 校验通过", "SHA256 verified"));
    }
    let version = binary::install_zip(&archive)?;
    let _ = std::fs::remove_file(&archive);
    engine.log_line(tr_format!(
        "tun2proxy {version} 安装完成",
        "tun2proxy {version} installed"
    ));
    Ok(version)
}

fn spawn_reader(
    pipe: impl Read + Send + 'static,
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("tun2proxy-log".to_string())
        .spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                // tun2proxy 的报错两种写法都出现过：log crate 的
                // `[ts ERROR module] ...`，以及 `main` 直接返回的 `Error: ...`。
                // 后者才是「进程要退出了」的那一条，所以两种都收，后到的覆盖
                // 先到的——最后一条才是死因。
                let is_error = line.contains(" ERROR ") || line.trim_start().starts_with("Error:");
                // 按行里自己的级别记录：文件层只留 WARN 以上，判断依据就是这个
                // 级别（`upstream_level` 从行文本里读），界面层则照单全收。
                match logs::upstream_level(&line) {
                    Level::ERROR => tracing::error!(target: logs::TARGET_OUTPUT, "{}", line),
                    Level::WARN => tracing::warn!(target: logs::TARGET_OUTPUT, "{}", line),
                    Level::DEBUG => tracing::debug!(target: logs::TARGET_OUTPUT, "{}", line),
                    Level::TRACE => tracing::trace!(target: logs::TARGET_OUTPUT, "{}", line),
                    Level::INFO => tracing::info!(target: logs::TARGET_OUTPUT, "{}", line),
                }
                if is_error {
                    if let Ok(mut shared) = shared.lock() {
                        shared.last_error = Some(line);
                    }
                }
                dirty.store(true, Ordering::Relaxed);
            }
        })
        .ok();
}

fn describe_exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => tr_format!("退出码 {}", "Exit code {}", code),
        None => format!("{status}"),
    }
}

/// 把「怎么退的」和子进程最后一条报错拼成一句话。
///
/// tun2proxy 的致命错误都打在自己的 stderr 上，只报退出码等于什么也没说。
/// 曾经这里还专门翻译「会话打满」——那需要传 `--exit-on-fatal-error`，而本
/// 程序不再传它（见 [`crate::core::config::Profile::cli_args`]），所以名额
/// 打满时隧道不会退出，这条分支也就没有存在意义了。
fn explain_exit(code: &str, reason: Option<&str>) -> String {
    match reason {
        Some(reason) => tr_format!("{}：{}", "{}: {}", code, reason),
        None => code.to_string(),
    }
}

#[cfg(unix)]
fn send_interrupt(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, libc::SIGINT) == 0 }
}

#[cfg(windows)]
fn send_interrupt(pid: u32) -> bool {
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
        CTRL_BREAK_EVENT,
    };

    // The GUI has no console of its own: borrow the child's console just long
    // enough to deliver CTRL_BREAK to its process group, which is what makes
    // tun2proxy restore the routing table before exiting.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return false;
        }
        SetConsoleCtrlHandler(None, 1);
        let ok = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) != 0;
        std::thread::sleep(Duration::from_millis(250));
        SetConsoleCtrlHandler(None, 0);
        FreeConsole();
        ok
    }
}

#[cfg(not(any(unix, windows)))]
fn send_interrupt(_pid: u32) -> bool {
    false
}

/// 日志里的进程号列表：`1234、5678`。
///
/// 中文顿号而不是逗号：和同一行里并列的网卡名（t2p-ab12cd34、t2p-ef56gh78）
/// 保持同一种分隔，读起来是「一串东西」而不是「一个数后面跟一段话」。
fn join_pids(pids: &[u32]) -> String {
    pids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("、")
}

// 日志本身（tracing 订阅端、按天落盘、界面分发）都在 `crate::core::logs`。
// 这里只负责在正确的时机产生事件：应用提示走 `TARGET_APP`、分流器走
// `TARGET_SPLIT`、子进程的原始输出走 `TARGET_OUTPUT`。
