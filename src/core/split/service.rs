//! 本机 SOCKS5 分流器：tun2proxy 的上游，按规则决定「直连」还是「转给真代理」。
//!
//! ```text
//!  tun2proxy 子进程  --proxy socks5://127.0.0.1:<随机端口>
//!        │  CONNECT www.qq.com:443
//!        ▼
//!  本模块的 SOCKS5 服务端
//!        ├─ 命中「直连」 → 出口网卡拨号（[`super::outlet`]）→ 中继
//!        └─ 其余         → 连真上游（[`super::upstream`]）→ 中继
//! ```
//!
//! # 为什么插在上游位置
//!
//! fake-IP（默认 DNS 策略）下 tun2proxy 会把**域名**塞进 SOCKS5 CONNECT 的
//! 地址字段——这是整条链路上唯一还能看到域名的位置。插在这里既不用改
//! tun2proxy，也不用动系统路由，改规则还能热更新。
//!
//! # 只做 CONNECT
//!
//! UDP ASSOCIATE 与 BIND 一律回「命令不支持」：tun2proxy 的 UDP 走它自己的
//! `--udpgw-server` 通道，不经过这里。硬做一条半吊子的 UDP 中继只会让 QUIC
//! 之类的流量变成时通时不通，比明确不支持更难排查。
//!
//! # 失败就回退
//!
//! 直连拨号失败（网卡没找到、网线拔了、目标拒绝）时**不会**悄悄改走代理：
//! 那样用户看到的是「有时很慢」而不是「这条直连不通」，且同一条连接可能被
//! 拨两次。这里直接给客户端回错误码，由它决定要不要重试。

use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};

use crate::core::i18n::t;
use crate::tr_format;

use super::ipset::Cidr;
use super::outlet::{self, Outlet};
use super::rules::{Action, RuleSet};
use super::upstream::{Target, Upstream};

/// 直连拨号的超时。比系统默认的 21 秒短得多：分流器在关键路径上，
/// 一条连不通的直连不该把浏览器卡这么久。
const DIAL_TIMEOUT: Duration = Duration::from_secs(8);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// 日志出口：由引擎注入（写进界面那份日志），内核不认识界面框架。
pub trait SplitSink: Send + Sync {
    fn log(&self, line: String);
}

/// 这条连接走了哪条路。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionRoute {
    Direct,
    Proxy,
}

impl DecisionRoute {
    pub fn label(self) -> &'static str {
        match self {
            DecisionRoute::Direct => t("直连", "Direct"),
            DecisionRoute::Proxy => t("代理", "Proxy"),
        }
    }
}

/// 一条判定记录。
///
/// 和日志分开存：日志只打前 20 条（整机流量下必须限流），而界面上的「最近
/// 判定」要的是**最近**几条、条条都在，还要能一眼看出结论和原因。所以这里存
/// 的是结构化的一份，日志仍按原来的规则走。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// 全局序号，从 1 开始单调递增（界面靠它判断有没有新记录）。
    pub seq: u64,
    pub route: DecisionRoute,
    /// 目标，形如 `www.qq.com:443`。
    pub target: String,
    /// 真的打通了没有。`false` 表示「判定这么走，但没连上」。
    pub ok: bool,
    /// 直连时是对端地址，走代理时是命中原因，失败时是失败原因。
    pub detail: String,
}

/// 最近判定的环形缓冲。
///
/// 容量固定：整机流量下每秒几百条连接，无上限地堆下去就是一个内存泄漏。
/// 满了以后丢最旧的一条，界面永远看到最近 [`DECISION_CAPACITY`] 条。
pub struct Decisions {
    capacity: usize,
    seq: AtomicU64,
    items: Mutex<VecDeque<Decision>>,
}

/// 最多记住多少条判定。界面一次只看十几条，留 4 倍余量足够翻回去看。
pub const DECISION_CAPACITY: usize = 64;

impl Decisions {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            seq: AtomicU64::new(0),
            items: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
        }
    }

    fn record(&self, route: DecisionRoute, ok: bool, target: &str, detail: String) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let Ok(mut items) = self.items.lock() else {
            return;
        };
        if items.len() >= self.capacity {
            items.pop_front();
        }
        items.push_back(Decision {
            seq,
            route,
            target: target.to_string(),
            ok,
            detail,
        });
    }

    fn clear(&self) {
        if let Ok(mut items) = self.items.lock() {
            items.clear();
        }
        self.seq.store(0, Ordering::Relaxed);
    }

    /// 最近的 `limit` 条，**新的在前**（界面从上往下读就是时间倒序）。
    fn recent(&self, limit: usize) -> Vec<Decision> {
        let Ok(items) = self.items.lock() else {
            return Vec::new();
        };
        items.iter().rev().take(limit).cloned().collect()
    }
}

impl Default for Decisions {
    fn default() -> Self {
        Self::new(DECISION_CAPACITY)
    }
}

/// 计数器。
#[derive(Debug, Default)]
pub struct Counters {
    pub direct: AtomicU64,
    pub proxy: AtomicU64,
    pub failed: AtomicU64,
    pub sessions: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            direct: self.direct.load(Ordering::Relaxed),
            proxy: self.proxy.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            sessions: self.sessions.load(Ordering::Relaxed),
        }
    }
}

/// 计数快照（给界面读）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterSnapshot {
    pub direct: u64,
    pub proxy: u64,
    pub failed: u64,
    pub sessions: u64,
}

/// 趋势每隔多久汇总一格（秒）。
///
/// 2 秒：5 分钟窗口里的曲线全挤在右边一块，看不出「刚才这几分钟」的节奏；
/// 缩到 2 分钟、格子也缩到 2 秒，点数和原来一样（60 个），横向却铺得开。
pub const TREND_BUCKET_SECS: u64 = 2;

/// 趋势窗口里有几格：60 × 2 秒 = 最近 2 分钟。
pub const TREND_BUCKETS: usize = 60;

/// 趋势里的一格：这 [`TREND_BUCKET_SECS`] 秒里各走了多少条。
///
/// 和 [`CounterSnapshot`] 的区别：计数器只有「本次运行一共多少」，画不出趋势；
/// 这里按时间切片，界面才能画出折线。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrendBucket {
    pub direct: u64,
    pub proxy: u64,
    pub failed: u64,
}

impl TrendBucket {
    /// 这一格里最高的一条线（定纵轴用）。
    pub fn peak(&self) -> u64 {
        self.direct.max(self.proxy).max(self.failed)
    }

    pub fn is_empty(&self) -> bool {
        self.peak() == 0
    }
}

/// 把一条连接算进哪一格。
#[derive(Clone, Copy)]
enum Counted {
    Direct,
    Proxy,
    Failed,
}

/// 分流去向的时序。
///
/// 每条连接按「第几格」累加，只留最近 [`TREND_BUCKETS`] 格，更早的直接丢——
/// 固定内存，整机流量下也不会涨。
///
/// 和计数器分开存：计数器是累加的原子量，没有时间信息；趋势要的是「什么时候
/// 走了哪条路」，所以要自己按格记账。两者在同一次判定里一起加（见
/// [`Dispatch`] 上的 `count_*`），不会各走各的。
pub struct Trend {
    /// 台账起点。格号 = 经过的秒数 / [`TREND_BUCKET_SECS`]。
    start: Instant,
    /// `(格号, 那一格的计数)`，按格号升序；空档不占位，读的时候补零。
    slots: Mutex<VecDeque<(u64, TrendBucket)>>,
}

impl Trend {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            slots: Mutex::new(VecDeque::with_capacity(TREND_BUCKETS)),
        }
    }

    fn clear(&self) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.clear();
        }
    }

    /// 现在落在第几格。
    fn slot_now(&self) -> u64 {
        self.start.elapsed().as_secs() / TREND_BUCKET_SECS
    }

    fn bump(&self, counted: Counted) {
        self.bump_at(self.slot_now(), counted);
    }

    /// 记在指定格上。抽出来是为了让单测能直接喂格号，不用真的等 5 秒。
    fn bump_at(&self, slot: u64, counted: Counted) {
        let Ok(mut slots) = self.slots.lock() else {
            return;
        };
        if let Some((last, bucket)) = slots.back_mut() {
            if *last == slot {
                add(bucket, counted);
                return;
            }
            // 时钟不会倒流，但真倒流了也别把队尾弄乱。
            if *last > slot {
                return;
            }
        }
        let mut bucket = TrendBucket::default();
        add(&mut bucket, counted);
        slots.push_back((slot, bucket));
        while slots.len() > TREND_BUCKETS {
            slots.pop_front();
        }
    }

    /// 最近 [`TREND_BUCKETS`] 格，**旧 → 新**（最后一格是「现在」）。
    ///
    /// 中间没有连接的格子补零：折线上不能出现「跳过去的点」，否则两段不相邻的
    /// 流量会被画成一条斜线，读起来像一直在跑。
    fn series(&self) -> Vec<TrendBucket> {
        let current = self.slot_now();
        let mut out = vec![TrendBucket::default(); TREND_BUCKETS];
        let Ok(slots) = self.slots.lock() else {
            return out;
        };
        for (slot, bucket) in slots.iter() {
            if *slot > current {
                continue;
            }
            let back = (current - slot) as usize;
            if back < TREND_BUCKETS {
                out[TREND_BUCKETS - 1 - back] = *bucket;
            }
        }
        out
    }
}

impl Default for Trend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl Trend {
    /// 造一条「已经跑了 `secs` 秒」的趋势：单测要往过去的格子里写，
    /// 又不能真的等上十分钟。
    fn aged(secs: u64) -> Self {
        Self {
            start: Instant::now()
                .checked_sub(Duration::from_secs(secs))
                .unwrap_or_else(Instant::now),
            slots: Mutex::new(VecDeque::with_capacity(TREND_BUCKETS)),
        }
    }
}

fn add(bucket: &mut TrendBucket, counted: Counted) {
    match counted {
        Counted::Direct => bucket.direct += 1,
        Counted::Proxy => bucket.proxy += 1,
        Counted::Failed => bucket.failed += 1,
    }
}

/// 一次启动需要的全部参数。
pub struct StartPlan {
    /// 监听地址，通常传 `127.0.0.1:0` 让系统分配空闲端口。
    pub listen: SocketAddr,
    pub upstream: Upstream,
    pub rules: RuleSet,
    pub rule_count: usize,
    pub direct_dns: SocketAddr,
    /// 用户指定的出口网卡（留空 = 自动判断）。
    pub outlet_hint: String,
    /// fake-IP 池：落在这里的地址说明 tun2proxy 没给域名，绝不能直接拨。
    pub fake_pool: Option<Cidr>,
}

/// 运行中的分流器。
struct Running {
    addr: SocketAddr,
    runtime: Runtime,
    dispatch: Arc<RwLock<Arc<Dispatch>>>,
}

/// 一次判定的结论。
enum Route {
    Direct,
    Proxy(&'static str),
}

/// 一个连接要用到的全部运行期状态。
struct Dispatch {
    rules: RuleSet,
    upstream: Upstream,
    outlet: Option<Outlet>,
    direct_dns: SocketAddr,
    fake_pool: Option<Cidr>,
    sink: Arc<dyn SplitSink>,
    counters: Arc<Counters>,
    /// 计数器的时序版本（界面上的「连接去向」折线）。
    trend: Arc<Trend>,
    /// 最近的判定记录（界面读的那一份）。
    decisions: Arc<Decisions>,
    /// 已经打印过多少条「判定」日志（整机流量下不能每条都打）。
    logged: AtomicUsize,
}

impl Dispatch {
    /// 计数器和趋势必须一起加：分开写迟早有一处漏掉，界面上就会出现
    /// 「数字涨了但曲线上没有」这种谁也对不上的情况。
    fn count_session(&self) {
        self.counters.sessions.fetch_add(1, Ordering::Relaxed);
    }

    fn count_direct(&self) {
        self.counters.direct.fetch_add(1, Ordering::Relaxed);
        self.trend.bump(Counted::Direct);
    }

    fn count_proxy(&self) {
        self.counters.proxy.fetch_add(1, Ordering::Relaxed);
        self.trend.bump(Counted::Proxy);
    }

    fn count_failed(&self) {
        self.counters.failed.fetch_add(1, Ordering::Relaxed);
        self.trend.bump(Counted::Failed);
    }

    /// 记一条判定。和日志分开走：日志限流，这份不限（有自己的环形上限）。
    fn record(&self, route: DecisionRoute, ok: bool, target: &Target, detail: String) {
        self.decisions
            .record(route, ok, &target.describe(), detail);
    }

    /// 判定顺序：域名 → 域名规则；IP → 网段规则；都不中 → 走代理（= 现在的行为）。
    fn route(&self, target: &Target) -> Route {
        match target {
            Target::Domain(host, _) => match self.rules.match_domain(host) {
                Some(hit) if hit.action() == Action::Direct => Route::Direct,
                Some(_) => Route::Proxy(t("规则指定走代理", "the rule says proxy")),
                None => Route::Proxy(t("未命中清单", "not in any list")),
            },
            Target::Ip(addr) => {
                if let Some(pool) = self.fake_pool {
                    if pool.contains(addr.ip()) {
                        // fake-IP：tun2proxy 只给了假地址，没有域名就没法判断。
                        return Route::Proxy(t("fake-IP 缺少域名", "fake-IP without a domain"));
                    }
                }
                if addr.ip().is_loopback() {
                    return Route::Proxy(t("目标是本机地址", "the target is a local address"));
                }
                match self.rules.match_ip(addr.ip()) {
                    Some(hit) if hit.action() == Action::Direct => Route::Direct,
                    Some(_) => Route::Proxy(t("规则指定走代理", "the rule says proxy")),
                    None => Route::Proxy(t("未命中清单", "not in any list")),
                }
            }
        }
    }

    fn note(&self, line: String) {
        // 只打前 20 条：整机流量下每秒几百条连接，全打会把真正的错误淹掉。
        if self.logged.fetch_add(1, Ordering::Relaxed) < 20 {
            self.sink.log(line);
        }
    }
}

/// 分流器句柄。内部全是 `Arc`，可以 clone 后丢给任意线程。
#[derive(Clone)]
pub struct SplitService {
    running: Arc<Mutex<Option<Running>>>,
    counters: Arc<Counters>,
    trend: Arc<Trend>,
    decisions: Arc<Decisions>,
    address: Arc<Mutex<Option<SocketAddr>>>,
    failure: Arc<Mutex<Option<String>>>,
    rule_count: Arc<AtomicUsize>,
}

impl Default for SplitService {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitService {
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(None)),
            counters: Arc::new(Counters::default()),
            trend: Arc::new(Trend::default()),
            decisions: Arc::new(Decisions::default()),
            address: Arc::new(Mutex::new(None)),
            failure: Arc::new(Mutex::new(None)),
            rule_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.address
            .lock()
            .map(|addr| addr.is_some())
            .unwrap_or(false)
    }

    pub fn address(&self) -> Option<SocketAddr> {
        self.address.lock().ok().and_then(|addr| *addr)
    }

    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|item| item.clone())
    }

    pub fn rule_count(&self) -> usize {
        self.rule_count.load(Ordering::Relaxed)
    }

    pub fn counters(&self) -> CounterSnapshot {
        self.counters.snapshot()
    }

    /// 最近的 `limit` 条判定，新的在前。
    pub fn recent_decisions(&self, limit: usize) -> Vec<Decision> {
        self.decisions.recent(limit)
    }

    /// 最近 [`TREND_BUCKETS`] 格的分流去向，**旧 → 新**（最后一格是「现在」）。
    pub fn trend(&self) -> Vec<TrendBucket> {
        self.trend.series()
    }

    pub fn reset_counters(&self) {
        self.counters.direct.store(0, Ordering::Relaxed);
        self.counters.proxy.store(0, Ordering::Relaxed);
        self.counters.failed.store(0, Ordering::Relaxed);
        self.counters.sessions.store(0, Ordering::Relaxed);
    }

    /// 启动监听，返回**实际**监听地址（端口可能是系统分配的）。
    pub fn start(&self, plan: StartPlan, sink: Arc<dyn SplitSink>) -> Result<SocketAddr, String> {
        self.stop();
        self.reset_counters();
        // 上次运行留下的判定记录与趋势对新的一次运行没有意义，序号也重新从 1 数。
        self.decisions.clear();
        self.trend.clear();
        if let Ok(mut failure) = self.failure.lock() {
            *failure = None;
        }

        // 出口网卡要在 tun2proxy 起来之前探测：那时默认路由还是干净的一条。
        let outlet = match outlet::detect(&plan.outlet_hint) {
            Ok(outlet) => {
                sink.log(tr_format!(
                    "分流器直连出口：{}（接口 {}）",
                    "Splitter direct outlet: {} (interface {})",
                    outlet.label,
                    outlet.index
                ));
                Some(outlet)
            }
            Err(err) => {
                sink.log(tr_format!(
                    "分流器找不到直连出口（{err}），所有「直连」将退回走代理",
                    "The splitter cannot find a direct outlet ({err}); every “direct” decision falls back to the proxy"
                ));
                None
            }
        };

        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("split-proxy")
            .build()
            .map_err(|err| tr_format!("无法创建分流器的运行时：{err}", "Cannot create the splitter runtime: {err}"))?;

        let listener = runtime
            .block_on(TcpListener::bind(plan.listen))
            .map_err(|err| {
                tr_format!(
                    "分流器无法监听 {}：{err}",
                    "The splitter cannot listen on {}: {err}",
                    plan.listen
                )
            })?;
        let addr = listener
            .local_addr()
            .map_err(|err| tr_format!("拿不到监听地址：{err}", "Cannot read the listening address: {err}"))?;

        let dispatch = Arc::new(Dispatch {
            rules: plan.rules,
            upstream: plan.upstream,
            outlet,
            direct_dns: plan.direct_dns,
            fake_pool: plan.fake_pool,
            sink: sink.clone(),
            counters: self.counters.clone(),
            trend: self.trend.clone(),
            decisions: self.decisions.clone(),
            logged: AtomicUsize::new(0),
        });
        self.rule_count.store(plan.rule_count, Ordering::Relaxed);

        let shared = Arc::new(RwLock::new(dispatch));
        let accept_state = shared.clone();
        let accept_sink = sink.clone();
        runtime.spawn(async move {
            accept_loop(listener, accept_state, accept_sink).await;
        });

        sink.log(tr_format!(
            "分流器已监听 {}，规则 {} 条",
            "Splitter listening on {}, {} rules",
            addr,
            plan.rule_count
        ));
        if let Ok(mut slot) = self.address.lock() {
            *slot = Some(addr);
        }
        if let Ok(mut slot) = self.running.lock() {
            *slot = Some(Running {
                addr,
                runtime,
                dispatch: shared,
            });
        }
        Ok(addr)
    }

    /// 热更新规则：不重启监听，正在跑的连接也不受影响。
    pub fn reload(&self, rules: RuleSet, rule_count: usize) -> Result<(), String> {
        let guard = self
            .running
            .lock()
            .map_err(|_| t("分流器状态锁损坏", "The splitter state lock is poisoned").to_string())?;
        let Some(running) = guard.as_ref() else {
            return Err(t("分流器没有在运行", "The splitter is not running").to_string());
        };
        let mut slot = running
            .dispatch
            .write()
            .map_err(|_| t("分流器规则锁损坏", "The splitter rules lock is poisoned").to_string())?;
        let next = Arc::new(Dispatch {
            rules,
            upstream: slot.upstream.clone(),
            outlet: slot.outlet.clone(),
            direct_dns: slot.direct_dns,
            fake_pool: slot.fake_pool,
            sink: slot.sink.clone(),
            counters: slot.counters.clone(),
            trend: slot.trend.clone(),
            decisions: slot.decisions.clone(),
            logged: AtomicUsize::new(0),
        });
        *slot = next;
        self.rule_count.store(rule_count, Ordering::Relaxed);
        Ok(())
    }

    /// 停掉监听。已经建立的连接随之断开（它们本来就活在 tun2proxy 的生命周期里）。
    pub fn stop(&self) {
        let running = self.running.lock().ok().and_then(|mut slot| slot.take());
        if let Some(running) = running {
            // 后台关停：不阻塞界面线程，正在跑的 relay 会被直接取消。
            running.runtime.shutdown_background();
            if let Ok(mut slot) = self.address.lock() {
                *slot = None;
            }
        }
    }

    fn fail(&self, message: String) {
        if let Ok(mut slot) = self.failure.lock() {
            *slot = Some(message);
        }
        if let Ok(mut slot) = self.address.lock() {
            *slot = None;
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    dispatch: Arc<RwLock<Arc<Dispatch>>>,
    sink: Arc<dyn SplitSink>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let dispatch = dispatch.clone();
                tokio::spawn(async move {
                    let current = dispatch.read().map(|slot| slot.clone()).ok();
                    if let Some(current) = current {
                        serve(stream, current).await;
                    }
                });
            }
            Err(err) => {
                sink.log(tr_format!(
                    "分流器接受连接失败：{err}",
                    "The splitter failed to accept a connection: {err}"
                ));
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// 处理一条客户端连接。
async fn serve(mut client: TcpStream, dispatch: Arc<Dispatch>) {
    let target = match read_request(&mut client).await {
        Ok(target) => target,
        Err(()) => return,
    };
    dispatch.count_session();

    let verdict = dispatch.route(&target);
    match verdict {
        Route::Direct => {
            // 没有可用出口时绝不裸连：直连的包会被 TUN 抓回自己，形成回环。
            let Some(outlet) = dispatch.outlet.clone() else {
                dispatch.count_proxy();
                dispatch.note(tr_format!(
                    "{} 判定为直连，但没有可用出口，改走代理",
                    "{} is direct but no outlet is available, using the proxy",
                    target.describe()
                ));
                let reason =
                    t("直连没有可用出口", "no direct outlet is available").to_string();
                proxy_only(client, &dispatch, &target, reason).await;
                return;
            };
            let remote = match resolve_target(&outlet, &dispatch, &target).await {
                Ok(addr) => addr,
                Err(reason) => {
                    dispatch.count_proxy();
                    dispatch.note(tr_format!(
                        "{} 无法解析（{reason}），改走代理",
                        "{} could not be resolved ({reason}), using the proxy",
                        target.describe()
                    ));
                    let reason = tr_format!(
                        "直连解析失败：{reason}",
                        "direct lookup failed: {reason}"
                    );
                    proxy_only(client, &dispatch, &target, reason).await;
                    return;
                }
            };
            match dial_direct(&outlet, remote).await {
                Ok(remote_stream) => {
                    dispatch.count_direct();
                    dispatch.note(tr_format!(
                        "直连 {} → {}",
                        "direct {} → {}",
                        target.describe(),
                        remote
                    ));
                    dispatch.record(DecisionRoute::Direct, true, &target, remote.to_string());
                    if !write_reply(&mut client, 0x00).await {
                        return;
                    }
                    relay(client, remote_stream).await;
                }
                Err(failure) => {
                    dispatch.count_failed();
                    let message = failure.message;
                    dispatch.sink.log(tr_format!(
                        "直连 {} 失败：{}",
                        "Direct connection to {} failed: {}",
                        target.describe(),
                        message
                    ));
                    // 不再加「拨号失败：」前缀：`message` 自己已经说清了是
                    // 哪一步出的错，套两层只会变成「拨号失败：出口拨号失败：…」。
                    dispatch.record(DecisionRoute::Direct, false, &target, message);
                    let _ = write_reply(&mut client, failure.reply).await;
                }
            }
        }
        Route::Proxy(reason) => {
            dispatch.count_proxy();
            dispatch.note(tr_format!(
                "代理 {}（{reason}）",
                "proxy {} ({reason})",
                target.describe()
            ));
            proxy_only(client, &dispatch, &target, reason.to_string()).await;
        }
    }
}

async fn proxy_only(
    mut client: TcpStream,
    dispatch: &Arc<Dispatch>,
    target: &Target,
    reason: String,
) {
    match dispatch.upstream.connect(target).await {
        Ok(remote) => {
            dispatch.record(DecisionRoute::Proxy, true, target, reason);
            if !write_reply(&mut client, 0x00).await {
                return;
            }
            relay(client, remote).await;
        }
        Err(err) => {
            dispatch.count_failed();
            dispatch.sink.log(tr_format!(
                "上游 {} 连不上 {}：{err}",
                "Upstream {} could not reach {}: {err}",
                dispatch.upstream.scheme.label(),
                target.describe()
            ));
            dispatch.record(
                DecisionRoute::Proxy,
                false,
                target,
                tr_format!("上游连不上：{err}", "upstream unreachable: {err}"),
            );
            let _ = write_reply(&mut client, 0x05).await;
        }
    }
    // `remote` 在这里被 drop，连接随之关闭。
}

/// 直连：域名先按出口做一次 DNS，IP 直接用。
async fn resolve_target(
    outlet: &Outlet,
    dispatch: &Arc<Dispatch>,
    target: &Target,
) -> Result<SocketAddr, String> {
    match target {
        Target::Ip(addr) => Ok(*addr),
        Target::Domain(host, port) => {
            let outlet = outlet.clone();
            let server = dispatch.direct_dns;
            let host = host.clone();
            let resolved = tokio::task::spawn_blocking(move || {
                outlet.resolve(server, &host, RESOLVE_TIMEOUT)
            })
            .await
            .map_err(|err| tr_format!("DNS 线程异常：{err}", "DNS task failed: {err}"))?;
            let addresses = resolved.map_err(|err| {
                tr_format!(
                    "直连 DNS {} 查询失败：{err}",
                    "Direct DNS query to {} failed: {err}",
                    dispatch.direct_dns
                )
            })?;
            // 优先 IPv4：多数国内站点在 v6 上路径更绕，直连反而更慢。
            let pick = addresses
                .iter()
                .find(|ip| ip.is_ipv4())
                .or_else(|| addresses.first());
            match pick {
                Some(IpAddr::V4(ip)) => Ok(SocketAddr::new((*ip).into(), *port)),
                Some(IpAddr::V6(ip)) => Ok(SocketAddr::new((*ip).into(), *port)),
                None => Err(t("没有解析结果", "the name did not resolve").to_string()),
            }
        }
    }
}

async fn dial_direct(outlet: &Outlet, target: SocketAddr) -> Result<TcpStream, DialFailure> {
    let outlet = outlet.clone();
    let stream = tokio::task::spawn_blocking(move || outlet.dial(target, DIAL_TIMEOUT))
        .await
        .map_err(|err| DialFailure {
            message: tr_format!("拨号线程异常：{err}", "Dial task failed: {err}"),
            reply: 0x01,
        })?;
    let stream = stream.map_err(|err| DialFailure {
        message: tr_format!("出口拨号失败：{err}", "The outlet refused the dial: {err}"),
        reply: reply_code(&err),
    })?;
    TcpStream::from_std(stream).map_err(|err| DialFailure {
        message: tr_format!("接管连接失败：{err}", "Cannot take over the socket: {err}"),
        reply: 0x01,
    })
}

/// 直连失败：给界面看的原因 + 回给客户端的 SOCKS5 回复码。
struct DialFailure {
    message: String,
    reply: u8,
}

/// 把 `io::ErrorKind` 翻成 SOCKS5 的 REP 码。
///
/// 以前不管什么失败都回 `0x05`（connection refused），于是 **8 秒超时在
/// tun2proxy 那边也显示成 `ConnectionRefused`**，日志里看起来像「目标端口关着」，
/// 实际上根本没人应答——方向完全跑偏，白查半天。细分之后：
/// 拒绝是真拒绝，超时是真超时。
fn reply_code(err: &io::Error) -> u8 {
    match err.kind() {
        io::ErrorKind::ConnectionRefused => 0x05,
        io::ErrorKind::TimedOut => 0x06,
        io::ErrorKind::NetworkUnreachable | io::ErrorKind::HostUnreachable => 0x04,
        // 其余（权限、地址不可用、socket 建不出来…）没有更贴切的码，
        // 按 RFC 1928 里的通用失败回 0x01。
        _ => 0x01,
    }
}

/// 双向中继，直到两边都关。
async fn relay(mut client: TcpStream, mut remote: TcpStream) {
    let _ = client.set_nodelay(true);
    let _ = remote.set_nodelay(true);
    let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
    let _ = client.shutdown().await;
    let _ = remote.shutdown().await;
}

/// 读 SOCKS5 问候与 CONNECT 请求，返回目标。
async fn read_request(stream: &mut TcpStream) -> Result<Target, ()> {
    let mut head = [0u8; 2];
    if stream.read_exact(&mut head).await.is_err() {
        return Err(());
    }
    if head[0] != 0x05 {
        // 不是 SOCKS5，直接关掉（tun2proxy 只会发 SOCKS5）。
        return Err(());
    }
    let methods = head[1] as usize;
    let mut buffer = vec![0u8; methods];
    if stream.read_exact(&mut buffer).await.is_err() {
        return Err(());
    }
    // 一律选「无认证」：监听在 127.0.0.1，只有本机的 tun2proxy 会连进来。
    if stream.write_all(&[0x05, 0x00]).await.is_err() {
        return Err(());
    }

    let mut request = [0u8; 4];
    if stream.read_exact(&mut request).await.is_err() {
        return Err(());
    }
    if request[1] != 0x01 {
        // BIND / UDP ASSOCIATE 不支持，明确回错误码而不是硬撑着。
        let _ = write_reply(stream, 0x07).await;
        return Err(());
    }
    let target = match request[3] {
        0x01 => {
            let mut octets = [0u8; 4];
            if stream.read_exact(&mut octets).await.is_err() {
                return Err(());
            }
            let mut port = [0u8; 2];
            if stream.read_exact(&mut port).await.is_err() {
                return Err(());
            }
            Target::Ip(SocketAddr::new(
                IpAddr::from(octets),
                u16::from_be_bytes(port),
            ))
        }
        0x04 => {
            let mut octets = [0u8; 16];
            if stream.read_exact(&mut octets).await.is_err() {
                return Err(());
            }
            let mut port = [0u8; 2];
            if stream.read_exact(&mut port).await.is_err() {
                return Err(());
            }
            Target::Ip(SocketAddr::new(
                IpAddr::from(octets),
                u16::from_be_bytes(port),
            ))
        }
        0x03 => {
            let mut length = [0u8; 1];
            if stream.read_exact(&mut length).await.is_err() {
                return Err(());
            }
            let mut name = vec![0u8; length[0] as usize];
            if stream.read_exact(&mut name).await.is_err() {
                return Err(());
            }
            let mut port = [0u8; 2];
            if stream.read_exact(&mut port).await.is_err() {
                return Err(());
            }
            let host = String::from_utf8_lossy(&name).to_string();
            Target::Domain(host, u16::from_be_bytes(port))
        }
        _ => {
            let _ = write_reply(stream, 0x08).await;
            return Err(());
        }
    };
    Ok(target)
}

/// 回一个 SOCKS5 应答。
async fn write_reply(stream: &mut TcpStream, code: u8) -> bool {
    // VER, REP, RSV, ATYP=IPv4, BND.ADDR=0.0.0.0, BND.PORT=0
    let reply = [0x05, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    stream.write_all(&reply).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::split::rules::{parse_rules_text, Rule};

    fn rule_set(text: &str) -> RuleSet {
        let (rules, errors) = parse_rules_text(text, Action::Direct, "test");
        assert!(errors.is_empty(), "{errors:?}");
        RuleSet::build(&rules).0
    }

    struct Quiet;
    impl SplitSink for Quiet {
        fn log(&self, _line: String) {}
    }

    fn dispatch(rules: RuleSet, fake_pool: Option<&str>) -> Arc<Dispatch> {
        Arc::new(Dispatch {
            rules,
            upstream: Upstream::parse("socks5://127.0.0.1:1").unwrap(),
            outlet: None,
            direct_dns: "223.5.5.5:53".parse().unwrap(),
            fake_pool: fake_pool.map(|text| Cidr::parse(text).unwrap()),
            sink: Arc::new(Quiet),
            counters: Arc::new(Counters::default()),
            trend: Arc::new(Trend::default()),
            decisions: Arc::new(Decisions::default()),
            logged: AtomicUsize::new(0),
        })
    }

    #[test]
    fn domains_follow_the_rule_set() {
        let dispatch = dispatch(rule_set("qq.com"), None);
        assert!(matches!(
            dispatch.route(&Target::Domain("www.qq.com".into(), 443)),
            Route::Direct
        ));
        assert!(matches!(
            dispatch.route(&Target::Domain("github.com".into(), 443)),
            Route::Proxy(_)
        ));
    }

    #[test]
    fn fake_ip_without_a_domain_always_uses_the_proxy() {
        // 198.18.0.0/15 是默认的 fake-IP 池：这里的地址只在本机有意义，
        // 拿去直连必然失败，所以哪怕网段规则说「直连」也不能直连。
        let dispatch = dispatch(rule_set("198.18.0.0/15"), Some("198.18.0.0/15"));
        assert!(matches!(
            dispatch.route(&Target::Ip("198.18.1.2:443".parse().unwrap())),
            Route::Proxy(_)
        ));
    }

    #[test]
    fn loopback_never_goes_direct() {
        let dispatch = dispatch(RuleSet::build(&Vec::<Rule>::new()).0, None);
        assert!(matches!(
            dispatch.route(&Target::Ip("127.0.0.1:8080".parse().unwrap())),
            Route::Proxy(_)
        ));
    }

    #[test]
    fn counters_start_at_zero() {
        let service = SplitService::new();
        assert!(!service.is_running());
        assert_eq!(service.counters(), CounterSnapshot::default());
        assert!(service.reload(RuleSet::build(&Vec::<Rule>::new()).0, 0).is_err());
    }

    #[test]
    fn decisions_keep_the_newest_and_number_them() {
        let records = Decisions::new(3);
        for index in 1..=5 {
            records.record(
                DecisionRoute::Direct,
                true,
                &format!("host{index}:443"),
                String::new(),
            );
        }
        let recent = records.recent(10);
        assert_eq!(recent.len(), 3, "容量 3，多了要把最旧的挤掉");
        // 新的在前。
        assert_eq!(recent[0].target, "host5:443");
        assert_eq!(recent[2].target, "host3:443");
        assert_eq!(recent[0].seq, 5);
        assert_eq!(records.recent(2).len(), 2);
    }

    #[test]
    fn decisions_are_cleared_between_runs() {
        let records = Decisions::new(4);
        records.record(DecisionRoute::Proxy, true, "a:1", "未命中清单".to_string());
        records.clear();
        assert!(records.recent(4).is_empty());
        records.record(DecisionRoute::Proxy, true, "b:2", String::new());
        assert_eq!(records.recent(4)[0].seq, 1, "清零后序号要重新数");
    }

    #[test]
    fn trend_counts_into_the_current_slot() {
        let trend = Trend::aged(600);        let base = trend.slot_now();
        trend.bump_at(base, Counted::Direct);
        trend.bump_at(base, Counted::Direct);
        trend.bump_at(base, Counted::Proxy);
        trend.bump_at(base, Counted::Failed);

        let series = trend.series();
        assert_eq!(series.len(), TREND_BUCKETS, "窗口长度固定");
        let now_bucket = series[TREND_BUCKETS - 1];
        assert_eq!(now_bucket.direct, 2);
        assert_eq!(now_bucket.proxy, 1);
        assert_eq!(now_bucket.failed, 1);
        // 窗口里其它格都是空的：新的一格不会写到上一格里去。
        assert!(series[..TREND_BUCKETS - 1].iter().all(TrendBucket::is_empty));
    }

    #[test]
    fn trend_keeps_the_window_and_pads_gaps_with_zero() {
        let trend = Trend::aged(6000);
        let base = trend.slot_now();
        // 三种情况一次覆盖：窗口外的老格（要丢掉）、中间空了几格（要补零）、
        // 以及窗内靠前的一格（要落在对应位置上）。按时间顺序写入，和真实
        // 连接到达的顺序一致。
        trend.bump_at(base - 200, Counted::Direct);
        trend.bump_at(base - 10, Counted::Proxy);
        trend.bump_at(base, Counted::Direct);

        let series = trend.series();
        assert_eq!(series[TREND_BUCKETS - 1].direct, 1, "最新一格是刚写入的");
        assert_eq!(series[TREND_BUCKETS - 11].proxy, 1, "往前第 10 格");
        assert!(series[TREND_BUCKETS - 2].is_empty(), "空档要补 0，不能跳过");
        // 200 格之前那条早就出窗了，窗口里不许再出现它的计数。
        let total: u64 = series.iter().map(|bucket| bucket.peak()).sum();
        assert_eq!(total, 2, "只该留下窗口里的两条");
    }

    #[test]
    fn trend_mirrors_the_counters() {
        // 计数器和趋势是同一个判定里一起加的：两边对不上，界面上就是
        // 「数字涨了但曲线上没有」。
        let dispatch = dispatch(RuleSet::build(&Vec::<Rule>::new()).0, None);
        dispatch.count_session();
        dispatch.count_direct();
        dispatch.count_proxy();
        dispatch.count_proxy();
        dispatch.count_failed();

        let counters = dispatch.counters.snapshot();
        let last = dispatch.trend.series()[TREND_BUCKETS - 1];
        assert_eq!(counters.sessions, 1);
        assert_eq!(counters.direct, last.direct);
        assert_eq!(counters.proxy, last.proxy);
        assert_eq!(counters.failed, last.failed);
    }

    #[test]
    fn trend_is_cleared_between_runs() {
        let service = SplitService::new();
        service.trend.bump(Counted::Direct);
        let last = |trend: &SplitService| *trend.trend().last().unwrap();
        assert!(!last(&service).is_empty());
        service.trend.clear();
        assert!(service.trend().iter().all(TrendBucket::is_empty));
    }

    #[test]
    fn reply_codes_tell_refusal_from_timeout() {
        // 超时以前和拒绝共用一个 0x05，tun2proxy 侧一律显示 ConnectionRefused，
        // 排查时最容易把「没人应答」读成「端口关着」。
        let code = |kind| reply_code(&io::Error::from(kind));
        assert_eq!(code(io::ErrorKind::ConnectionRefused), 0x05);
        assert_eq!(code(io::ErrorKind::TimedOut), 0x06);
        assert_eq!(code(io::ErrorKind::HostUnreachable), 0x04);
        assert_eq!(code(io::ErrorKind::NetworkUnreachable), 0x04);
        assert_eq!(code(io::ErrorKind::PermissionDenied), 0x01);
    }
}
