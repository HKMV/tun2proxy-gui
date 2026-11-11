//! 分流：国内 IP / 域名走本地网络直连，其余照常走代理。
//!
//! # 两层结构
//!
//! **规则层**（纯算法，只用标准库，可单测、可被工具复用）：
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`ipset`] | CIDR 解析、前缀索引（最长前缀查询）、区间合并与拆分 |
//! | [`rules`] | 规则模型（域名后缀 / 精确 / 关键字 / IP 段）、规则集编译、判定顺序 |
//! | [`lists`] | 把 APNIC 与 v2fly 的原始清单解析、聚合成规则 |
//! | [`builtin`] | 编译期内置的清单快照（离线兜底） |
//! | [`library`] | 清单来源选择：优先用下载到用户目录的那份 |
//! | [`settings`] | 用户设置（总开关、分组开关、单行开关、自定义规则、直连 DNS） |
//!
//! **运行层**（tokio + `socket2`）：
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`service`] | 本机 SOCKS5 服务端：接 tun2proxy 的 CONNECT，按规则选路并中继 |
//! | [`upstream`] | 上游代理客户端（socks5 / socks4 / http） |
//! | [`outlet`] | 直连出口绑定与直连 DNS（防「直连又掉回 TUN」的回环） |
//! | [`update`] | 清单在线更新（APNIC + v2fly，含子清单展开） |
//!
//! 另外还有一个只在测试构建里存在的 [`live_check`]：起真的分流器、扮演 tun2proxy
//! 发 SOCKS5 请求、用假上游记录「走了代理」的那几条，把判定结果和日志打出来。
//! 不需要管理员权限，也不需要装 tun2proxy（`cargo test --offline live_split --
//! --ignored --nocapture`）。
//!
//! # 开关是「全有或全无」
//!
//! [`settings::SplitSettings::enabled`] 默认 **false**：关掉时完全不起分流器，
//! tun2proxy 的命令行参数与没有这个功能时一字不差。打开后也只改 `--proxy` 的
//! 目标（指向本机分流器），其它参数、UDP 转发、绕过列表全部保持原样。
//!
//! # 判定顺序
//!
//! 域名 → 精确 → 最长后缀 → 关键字；IP → 最长前缀。都没有命中就走代理
//! （= 没有这个功能时的行为）。同长度前缀上 `proxy` 赢，所以用户在自定义规则里
//! 写一行 `proxy:xxx.cn` 总能盖住内置清单。
//!
//! 接入之后仍有若干「定义了但暂时没人调用」的方法（工具、预留的 API），
//! 所以这里继续允许 `dead_code`：那些告警是噪音，不是线索。
#![allow(dead_code)]

pub mod builtin;
pub mod ipset;
pub mod library;
pub mod lists;
pub mod outlet;
pub mod rules;
pub mod service;
pub mod settings;
pub mod update;
pub mod upstream;

/// 真跑自检：起真的分流器 + 假的 tun2proxy / 假上游，把判定结果打出来。
/// 只在测试构建里存在，默认不跑（见文件头）。
#[cfg(test)]
mod live_check;

#[allow(unused_imports)]
pub use ipset::{Cidr, IpSet};
#[allow(unused_imports)]
pub use library::{Library, Origin};
#[allow(unused_imports)]
pub use rules::{Action, Match, Matcher, MatcherKind, Rule, RuleSet, Stats};
#[allow(unused_imports)]
pub use service::{
    CounterSnapshot, Decision, DecisionRoute, SplitService, SplitSink, StartPlan, TREND_BUCKETS,
    TREND_BUCKET_SECS, TrendBucket,
};
#[allow(unused_imports)]
pub use settings::{SplitGroup, SplitSettings};
#[allow(unused_imports)]
pub use upstream::{Upstream, Scheme};
