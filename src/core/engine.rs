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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{Log, Metadata, Record};

use crate::core::binary;
use crate::core::config::Profile;
use crate::core::i18n;
use crate::core::i18n::t;
use crate::core::paths;
use crate::core::probe::{self, ProbeReport};
use crate::core::traffic::{self, TrafficMeter, TrafficSample};
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
    pub logs: VecDeque<String>,
    /// 最近一分钟的流量采样（每秒一个点），给仪表盘的折线图用。
    ///
    /// 放在共享状态里而不是引擎自己算：界面每帧重建，图上的点必须能重复读到
    /// 同一份历史，否则「上一帧的曲线」和「这一帧的曲线」接不上。
    pub traffic: VecDeque<TrafficSample>,
    /// 最新一次采样的瞬时速率（字节/秒）。
    pub rate: TrafficSample,
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
    /// 重绘回调：后台线程改完状态后调用它，让界面重建一次。
    ///
    /// 用裸回调而不是直接存 GUI 句柄，是为了让 core 不依赖任何界面框架。
    pub rebuild: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Shared {
    /// 取最后 `count` 行日志（给界面用）。
    pub fn log_tail(&self, count: usize) -> Vec<String> {
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

/// 引擎句柄。内部全是 `Arc`，可以随意 clone 后丢给后台线程。
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    stopping: Arc<AtomicBool>,
}

impl Engine {
    /// 创建引擎：安装日志桥接、启动重绘调度线程，并探测已安装版本。
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::Stopped,
            update: Update::Idle,
            version: binary::installed_version(),
            logs: VecDeque::new(),
            traffic: VecDeque::new(),
            rate: TrafficSample::default(),
            probe: None,
            start_blocked: None,
            rebuild: None,
        }));
        let dirty = Arc::new(AtomicBool::new(false));
        install_logger(shared.clone(), dirty.clone());

        let engine = Self {
            shared,
            dirty,
            child: Arc::new(Mutex::new(None)),
            stopping: Arc::new(AtomicBool::new(false)),
        };
        engine.spawn_notifier();
        engine.log_line(tr_format!(
            "配置目录 {}",
            "Config directory {}",
            paths::app_dir().display()
        ));
        engine
    }

    pub fn shared(&self) -> Arc<Mutex<Shared>> {
        self.shared.clone()
    }

    /// 注入「请求界面重绘」的回调（由 GUI 层提供，通常包装 zsui 的
    /// `UiInvalidationHandle::request_rebuild`）。
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

    pub fn clear_logs(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.logs.clear();
        }
        self.touch();
    }

    pub fn log_line(&self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("{line}");
        if let Ok(mut shared) = self.shared.lock() {
            shared.logs.push_back(line);
            while shared.logs.len() > MAX_LOGS {
                shared.logs.pop_front();
            }
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
    pub fn start_with_precheck(&self, profile: &Profile) -> Result<(), String> {
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

        let url = profile.proxy_url.clone();
        let profile = profile.clone();
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
                    if let Err(err) = engine.start(&profile) {
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
    pub fn start(&self, profile: &Profile) -> Result<(), String> {
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

        let args = profile.cli_args();
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
        self.spawn_supervisor();
        Ok(())
    }

    /// 请求子进程退出：先发中断信号让它自己清理路由，6 秒后仍未退出则强杀。
    pub fn stop(&self) {
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

    fn spawn_supervisor(&self) {
        let shared = self.shared.clone();
        let child = self.child.clone();
        let stopping = self.stopping.clone();
        let dirty = self.dirty.clone();

        std::thread::Builder::new()
            .name("tun2proxy-supervisor".to_string())
            .spawn(move || {
                let mut ticks = 0u32;
                let mut announced = false;
                let mut meter = TrafficMeter::new();
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
                        let status = match result {
                            Ok(status) if was_stopping || status.success() => Status::Stopped,
                            Ok(status) => Status::Failed(tr_format!(
                                "tun2proxy 已退出（{}）",
                                "tun2proxy exited ({})",
                                describe_exit(&status)
                            )),
                            Err(err) => Status::Failed(tr_format!(
                                "无法获取进程状态：{err}",
                                "Cannot read the process status: {err}"
                            )),
                        };
                        if let Ok(mut shared) = shared.lock() {
                            shared.status = status;
                            // 进程没了，速率归零；历史留着，曲线慢慢滑出视野。
                            shared.rate = TrafficSample::default();
                        }
                        meter.reset();
                        dirty.store(true, Ordering::Relaxed);
                        return;
                    }

                    // 每 4 个 tick（约 1 秒）采一次流量。
                    //
                    // 采样失败（进程刚起、读不到计数器）就跳过这一拍：折线图
                    // 少一个点比画一个假的尖峰好。
                    if ticks.is_multiple_of(4) {
                        let sample = {
                            let mut guard = match child.lock() {
                                Ok(guard) => guard,
                                Err(_) => return,
                            };
                            match guard.as_mut() {
                                Some(process) => meter.sample(process),
                                None => None,
                            }
                        };
                        if let Some(sample) = sample {
                            if let Ok(mut shared) = shared.lock() {
                                shared.rate = sample;
                                traffic::push(&mut shared.traffic, sample);
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
                if let Ok(mut shared) = shared.lock() {
                    shared.logs.push_back(line);
                    while shared.logs.len() > MAX_LOGS {
                        shared.logs.pop_front();
                    }
                }
                dirty.store(true, Ordering::Relaxed);
            }
        })
        .ok();
}

fn describe_exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => tr_format!("退出码 {code}", "Exit code {code}"),
        None => format!("{status}"),
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

// ---------------------------------------------------------------- logging --

struct SharedLogger {
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
}

impl Log for SharedLogger {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        let line = format!("{} {:<5} {}", timestamp(), record.level(), record.args());
        if let Ok(mut shared) = self.shared.lock() {
            shared.logs.push_back(line);
            while shared.logs.len() > MAX_LOGS {
                shared.logs.pop_front();
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn flush(&self) {}
}

fn install_logger(shared: Arc<Mutex<Shared>>, dirty: Arc<AtomicBool>) {
    static LOGGER: OnceLock<SharedLogger> = OnceLock::new();
    let logger = LOGGER.get_or_init(|| SharedLogger { shared, dirty });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}
