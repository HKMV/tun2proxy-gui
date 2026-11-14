//! tun2proxy GUI 的「无界面内核」。
//!
//! 这一层完全不依赖 GUI 框架（GPUI / gpuikit），因此可以单独测试，也可以在
//! 以后换成其它界面框架时原样复用。它负责四件事：
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`paths`] | 配置文件、下载目录、日志文件的存放位置 |
//! | [`config`] | 用户配置（代理地址、DNS、MTU…）的读写、校验，以及转换成 CLI 参数 |
//! | [`binary`] | 从 GitHub Release 下载 / 校验 / 解压 tun2proxy 官方二进制，读取版本号 |
//! | [`engine`] | 把 tun2proxy 作为子进程拉起、收集日志、维护状态与更新进度 |
//! | [`logs`] | 运行日志（tracing）：按天写 `<数据目录>/logs/`，同时推给界面 |
//! | [`probe`] | 检测上游代理地址是否连得通（TCP + SOCKS5/HTTP 握手） |
//! | [`split`] | 分流规则：国内 IP / 域名的清单聚合、匹配，以及本机 SOCKS5 分流器 |
//! | [`traffic`] | 读 TUN 网卡的收发计数，给仪表盘的流量折线图用 |
//! | [`adapters`] | 启动前清理本程序留下、当前不在用的隧道网卡（Windows） |
//! | [`orphans`] | 启动前结束上一次留下的孤儿 tun2proxy 进程（否则网卡清不掉，Windows） |
//! | [`watchdog`] | 界面线程卡顿探针（假死时把「卡在哪一段」写进 stall.log） |
//! | [`icon`] | 程序图标的唯一真源（exe 资源 / 标题栏 / 托盘共用一份字节） |
//! | [`window`] | 主窗口句柄的查找与显隐（托盘唤回、最小化到托盘） |
//! | [`single_instance`] | 只允许开一个实例：第二次启动请已有实例自己把窗口显出来 |
//! | [`tray`] | 系统托盘图标（Windows）：状态提示、启停、显示窗口、退出 |
//! | [`elevate`] | 管理员 / root 权限检测与 Windows 上的自动提权重启 |
//!
//! 设计取舍：**不把 tun2proxy 编进程序，也不链接它的库**。用户首次运行时由
//! [`binary`] 下载官方 Release，之后可随时检测更新并原地替换，这样 GUI 的
//! 发布节奏不必跟着 tun2proxy 走，也不用重新打包各平台产物。
//!
//! 支持多套代理配置：应用级设置放在 [`config::Config`]，每套代理放在
//! [`config::Profile`]，界面只在两者之间做切换。

pub mod adapters;
pub mod binary;
pub mod config;
pub mod elevate;
pub mod engine;
pub mod i18n;
pub mod icon;
pub mod logs;
pub mod orphans;
pub mod paths;
pub mod platform;
pub mod probe;
pub mod single_instance;
pub mod split;
pub mod traffic;
pub mod tray;
pub mod watchdog;
pub mod window;

pub use config::{CloseAction, Config, DnsMode, Profile, ThemeSetting, Verbosity};
pub use engine::{Engine, Shared, Status, Update};
pub use traffic::{RouteTraffic, TrafficSample};

/// 本程序（GUI）的版本号。
///
/// 和侧栏底部那行「tun2proxy v0.8.6」不是一个东西：那是官方二进制的版本，
/// 会随「检查更新」变；这个是这份 GUI 自己发布时的版本，从 `Cargo.toml`
/// 编译进来，用来回答「你现在用的是哪一版前端」。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
