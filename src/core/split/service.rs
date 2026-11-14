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

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};

use crate::core::i18n::t;
use crate::core::traffic::RouteBytes;
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
///
/// 前四个是「几条连接」，后四个是「几个字节」：字节按路累加（直连 / 代理 ×
/// 上下行），给仪表盘的流量图拆两条曲线用。都是单调累加，只在分流器启动时清零。
#[derive(Debug, Default)]
pub struct Counters {
    pub direct: AtomicU64,
    pub proxy: AtomicU64,
    pub failed: AtomicU64,
    pub sessions: AtomicU64,
    pub direct_down: AtomicU64,
    pub direct_up: AtomicU64,
    pub proxy_down: AtomicU64,
    pub proxy_up: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            direct: self.direct.load(Ordering::Relaxed),
            proxy: self.proxy.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            sessions: self.sessions.load(Ordering::Relaxed),
            bytes: self.bytes(),
        }
    }

    /// 累计字节（不含连接数），给每秒一次的速度采样读。
    fn bytes(&self) -> RouteBytes {
        RouteBytes {
            direct_down: self.direct_down.load(Ordering::Relaxed),
            direct_up: self.direct_up.load(Ordering::Relaxed),
            proxy_down: self.proxy_down.load(Ordering::Relaxed),
            proxy_up: self.proxy_up.load(Ordering::Relaxed),
        }
    }

    fn clear_bytes(&self) {
        self.direct_down.store(0, Ordering::Relaxed);
        self.direct_up.store(0, Ordering::Relaxed);
        self.proxy_down.store(0, Ordering::Relaxed);
        self.proxy_up.store(0, Ordering::Relaxed);
    }

    /// 记一块字节。中继循环里每搬一块就调一次，所以只是 `fetch_add`，不取锁。
    fn add(&self, lane: Lane, down: bool, bytes: u64) {
        let counter = match (lane, down) {
            (Lane::Direct, true) => &self.direct_down,
            (Lane::Direct, false) => &self.direct_up,
            (Lane::Proxy, true) => &self.proxy_down,
            (Lane::Proxy, false) => &self.proxy_up,
        };
        counter.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// 计数快照（给界面读）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterSnapshot {
    pub direct: u64,
    pub proxy: u64,
    pub failed: u64,
    pub sessions: u64,
    /// 累计字节（按路、按方向分开）。
    pub bytes: RouteBytes,
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

    /// 累计字节（按路、按方向分开）。流量图每秒读一次做差分。
    pub fn route_bytes(&self) -> RouteBytes {
        self.counters.bytes()
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
        self.counters.clear_bytes();
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
                    relay(client, remote_stream, dispatch.counters.clone(), Lane::Direct).await;
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
            relay(client, remote, dispatch.counters.clone(), Lane::Proxy).await;
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
    // `remote` 已经交给 `relay`，中继结束时连接随之关闭。
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

/// 这条连接走的是哪条路——决定中继时把字节记到哪一对计数器上。
///
/// 和 [`DecisionRoute`] 分开：那个是给界面读的「判定结论」，这个只在记账时用，
/// 不做序列化、不上界面，多一个字段也没人看得见。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Direct,
    Proxy,
}

/// 中继时一次搬多少字节。
///
/// 和 tokio 的 `copy_bidirectional` 默认缓冲一样是 8 KB：一是那条路径本来就在
/// 用这个尺寸，换成更大的值没有实测收益；二是每条连接两个方向各一份，整机流量
/// 下这个常量直接乘在内存占用上。
const COPY_BUFFER: usize = 8 * 1024;

/// 双向中继，直到两边都关，**顺便把字节记到这条路账上**。
///
/// 方向：`client` 是 tun2proxy（本机），`remote` 是对端。client → remote 是
/// 上行（系统要发出去的），remote → client 是下行，和 TUN 网卡计数的方向语义
/// 一致，两张图才读得对。
///
/// 用自己写的搬运循环而不是 `copy_bidirectional`：后者只在最后返回总量，而
/// 界面要的是「现在每秒多少」——必须边搬边记账。
async fn relay(client: TcpStream, remote: TcpStream, counters: Arc<Counters>, lane: Lane) {
    let _ = client.set_nodelay(true);
    let _ = remote.set_nodelay(true);
    let (mut client_read, mut client_write) = client.into_split();
    let (mut remote_read, mut remote_write) = remote.into_split();

    // 两个方向各起一个任务：一边先关（半关闭）时另一边还得继续，直到对端也关，
    // 所以不能串着做。这里不用 `tokio::join!`——那个宏要开 tokio 的 `macros`
    // feature，而两个方向本来就可以当独立任务跑，`spawn` 已经让它们并发了。
    let up = {
        let counters = counters.clone();
        tokio::spawn(async move {
            let _ = copy_counted(&mut client_read, &mut remote_write, |bytes| {
                counters.add(lane, false, bytes)
            })
            .await;
        })
    };
    let down = tokio::spawn(async move {
        let _ = copy_counted(&mut remote_read, &mut client_write, |bytes| {
            counters.add(lane, true, bytes)
        })
        .await;
    });
    let _ = up.await;
    let _ = down.await;
}

/// 搬一个方向的数据，每搬成功一块就把字节数交给 `account`。
async fn copy_counted<R, W>(
    reader: &mut R,
    writer: &mut W,
    account: impl Fn(u64),
) -> io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut moved = 0u64;
    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) => return Err(err),
        };
        // 只记「真的写出去了」的字节：写失败说明这条连接已经传不下去，
        // 那些还压在缓冲里的字节既没到对端，也不该算进速率。
        writer.write_all(&buffer[..read]).await?;
        account(read as u64);
        moved += read as u64;
    }
    let _ = writer.shutdown().await;
    Ok(moved)
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

    /// 给端到端用例的每一步套个超时：卡住时要报出「卡在哪一步」，而不是让
    /// CI 挂到全局超时才失败。
    async fn within<F: std::future::Future>(step: &str, future: F) -> F::Output {
        match tokio::time::timeout(Duration::from_secs(5), future).await {
            Ok(value) => value,
            Err(_) => panic!("5 秒超时：{step}"),
        }
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

    /// 四条字节计数各记各的：两条路 × 上下行，串了就说明方向或路径写错了，
    /// 界面上的两条色带会跟着串。
    #[test]
    fn byte_counters_keep_the_two_lanes_apart() {
        let counters = Counters::default();
        counters.add(Lane::Direct, true, 1);
        counters.add(Lane::Direct, false, 10);
        counters.add(Lane::Proxy, true, 100);
        counters.add(Lane::Proxy, false, 1_000);

        assert_eq!(
            counters.bytes(),
            RouteBytes {
                direct_down: 1,
                direct_up: 10,
                proxy_down: 100,
                proxy_up: 1_000,
            }
        );
        // 清零是「每跑一次重新记账」，四条要一起归零。
        counters.clear_bytes();
        assert_eq!(counters.bytes(), RouteBytes::default());
    }

    /// 分流器重启会清计数：流量图的基线跟着归零，不能留着上一轮的数字。
    #[test]
    fn route_bytes_start_fresh_on_each_run() {
        let service = SplitService::new();
        service.counters.add(Lane::Proxy, true, 42);
        assert_eq!(service.route_bytes().proxy_down, 42);
        service.reset_counters();
        assert_eq!(service.route_bytes(), RouteBytes::default());
    }

    /// 搬运循环必须把「真的写出去了」的字节交出来：流量图全靠这个数字，
    /// 少记就会显示得比实际慢，多记就是凭空造流量。
    ///
    /// 手动建运行时而不是 `#[tokio::test]`：那个宏要开 tokio 的 `macros`
    /// feature，为一个测试加 feature 不值得。
    #[test]
    fn copying_counts_every_byte_it_moves() {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("建得起单线程运行时");
        runtime.block_on(async {
            // 缓冲开得比总量大：这里只验记账，不想测背压导致的阻塞。
            let (mut source, mut source_tail) = tokio::io::duplex(8 * 1024);
            let (mut sink, mut sink_tail) = tokio::io::duplex(8 * 1024);
            let counted = Arc::new(AtomicU64::new(0));

            let task_counted = counted.clone();
            let handle = tokio::spawn(async move {
                copy_counted(&mut source_tail, &mut sink, move |bytes| {
                    task_counted.fetch_add(bytes, Ordering::Relaxed);
                })
                .await
            });

            let payload = vec![7u8; 2_000];
            for _ in 0..3 {
                source.write_all(&payload).await.expect("写进管道");
            }
            // 关掉写端 = 半关闭，搬运循环读到 EOF 就该收工。
            source.shutdown().await.expect("关掉写端");

            let moved = handle.await.expect("搬运任务没 panic").expect("搬运成功");
            assert_eq!(moved, 6_000, "三块 2000 字节");
            assert_eq!(counted.load(Ordering::Relaxed), moved, "记账要和实际搬运一致");

            let mut received = Vec::new();
            sink_tail
                .read_to_end(&mut received)
                .await
                .expect("读回对端");
            assert_eq!(received.len(), 6_000, "字节要真的到了对端");
            assert!(received.iter().all(|&byte| byte == 7));
        });
    }

    /// 端到端：一条真的经过分流器的连接，字节要记在它**实际走的那条路**上。
    ///
    /// 单测能证明搬运循环会数数，但证明不了「代理这条分支传的是 Proxy 那条
    /// 账本」——那正是流量图会不会把两条路搞反的关键，所以这里用假上游跑一遍
    /// 完整的 SOCKS5：客户端 → 分流器 → 上游 → 回显。
    #[test]
    fn relayed_bytes_land_on_the_proxy_lane() {
        // 上游端口先用标准库占住：分流器得在它自己的运行时里 bind，不能嵌在
        // 别人的 `block_on` 里调用（那会 panic「Cannot start a runtime from
        // within a runtime」）。
        let upstream = std::net::TcpListener::bind("127.0.0.1:0").expect("绑上游端口");
        let upstream_addr = upstream.local_addr().expect("上游地址");
        upstream.set_nonblocking(true).expect("切非阻塞");

        let service = SplitService::new();
        // 规则集是空的：任何目标都「未命中清单」→ 走代理，正是要测的那条路。
        let plan = StartPlan {
            listen: "127.0.0.1:0".parse().expect("常量地址"),
            upstream: Upstream::parse(&format!("socks5://{upstream_addr}")).expect("上游地址合法"),
            rules: RuleSet::build(&Vec::<Rule>::new()).0,
            rule_count: 0,
            direct_dns: "223.5.5.5:53".parse().expect("常量 DNS"),
            outlet_hint: String::new(),
            fake_pool: None,
        };
        let addr = service.start(plan, Arc::new(Quiet)).expect("分流器起来");

        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("建得起单线程运行时");
        // 句柄要拷一份进去：`service` 本身得留在外面收尾（`stop()`）。
        let handle = service.clone();
        runtime.block_on(async move {
            // 假上游：只认一条连接，握手后把收到的字节原样吐回去。
            let upstream = TcpListener::from_std(upstream).expect("接管上游监听");
            let upstream_task = tokio::spawn(async move {
                let (mut socket, _) = upstream.accept().await.expect("接受上游连接");
                let mut greeting = [0u8; 2];
                socket.read_exact(&mut greeting).await.expect("读问候");
                let mut methods = vec![0u8; greeting[1] as usize];
                socket.read_exact(&mut methods).await.expect("读方法");
                socket.write_all(&[0x05, 0x00]).await.expect("回方法");
                let mut request = [0u8; 4];
                socket.read_exact(&mut request).await.expect("读请求头");
                let address_len = match request[3] {
                    0x01 => 4,
                    0x04 => 16,
                    0x03 => {
                        // 域名前面那个长度字节已经在这里读掉了，剩下的就只是域名。
                        let mut length = [0u8; 1];
                        socket.read_exact(&mut length).await.expect("读域名长度");
                        length[0] as usize
                    }
                    _ => 0,
                };
                let mut rest = vec![0u8; address_len + 2];
                socket.read_exact(&mut rest).await.expect("读地址与端口");
                socket
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await
                    .expect("回应答");
                let mut buffer = [0u8; 512];
                while let Ok(read) = socket.read(&mut buffer).await {
                    if read == 0 || socket.write_all(&buffer[..read]).await.is_err() {
                        break;
                    }
                }
            });

            let mut client = within("连上分流器", TcpStream::connect(addr))
                .await
                .expect("连上分流器");
            within("发问候", client.write_all(&[0x05, 0x01, 0x00]))
                .await
                .expect("问候");
            let mut greeting = [0u8; 2];
            within("收问候应答", client.read_exact(&mut greeting))
                .await
                .expect("收问候应答");
            assert_eq!(greeting, [0x05, 0x00]);

            let host = b"example.com";
            let mut request = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
            request.extend_from_slice(host);
            request.extend_from_slice(&80u16.to_be_bytes());
            within("发 CONNECT", client.write_all(&request))
                .await
                .expect("发 CONNECT");
            let mut reply = [0u8; 10];
            within("收 CONNECT 应答", client.read_exact(&mut reply))
                .await
                .expect("收应答");
            assert_eq!(reply[1], 0x00, "上游应答应是成功");

            // 中继阶段的字节：100 字节上行，回显 100 字节下行。
            let payload = vec![0x5au8; 100];
            within("发数据", client.write_all(&payload))
                .await
                .expect("发数据");
            let mut echoed = vec![0u8; 100];
            within("收回显", client.read_exact(&mut echoed))
                .await
                .expect("收回显");
            assert_eq!(echoed, payload);

            // 记账发生在 `write_all` 返回之后那一行，客户端可能抢在此之前就读到
            // 了回显——所以给一小段重试窗口，别赌谁先跑完。
            let deadline = Instant::now() + Duration::from_secs(2);
            let bytes = loop {
                let bytes = handle.route_bytes();
                if (bytes.proxy_up == 100 && bytes.proxy_down == 100) || Instant::now() >= deadline {
                    break bytes;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            assert_eq!(bytes.proxy_up, 100, "这 100 字节走的是代理那条路");
            assert_eq!(bytes.proxy_down, 100, "回显也算在下行里");
            assert_eq!(bytes.direct_up, 0, "直连那条不该有任何字节");
            assert_eq!(bytes.direct_down, 0);
            assert_eq!(handle.counters().proxy, 1, "一条连接算一次代理判定");

            // 客户端断开后假上游也该收工：它要是 panic 了，上面那些断言就成了
            // 「碰巧过了」，所以在这里把结果收回来。
            drop(client);
            within("假上游退出", upstream_task)
                .await
                .expect("假上游正常结束");
        });
        service.stop();
    }
}
