//! 业务接口层：界面**唯一**能碰的东西。
//!
//! # 为什么单独一层
//!
//! 这一层把「这个程序能做什么」和「它长什么样」彻底分开：
//!
//! ```text
//! core/    内核：拉起 tun2proxy、采样流量、读配置、判分流。不认识界面。
//! api/     门面：一串纯函数 + 可序列化的快照结构。**不依赖任何 UI 框架**
//!          （连 nsh 的 prelude 都不 use），谁想画界面就调它。
//! bridge/  适配器：把 api 的函数与快照翻译成 nsh 的 `#[module]`，给 TSX 用。
//! ui/      过渡期的 Rust 界面（逐页搬到 TSX，搬完删）。
//! ```
//!
//! 换界面时只换 `bridge/` + 前端的 `.tsx`：`core/` 和 `api/` 一行不动。反过来
//! 说，`api/` 里出现 `use nsh_…` 就是分层破了，编译期就该拦下来。
//!
//! # 数据怎么过河
//!
//! 快照一律走 JSON（[`snapshot`] 里的结构都是 `Serialize`）：一份 JSON 既喂得
//! 了 TSX 的 `JSON.parse`，也能被命令行、测试、将来的别的界面直接吃。函数签名
//! 也照这个口径——返回 `String`（JSON 文本）或 `Result<(), String>`（错误就是
//! 一句能直接显示的中文），不泄露 Rust 类型。
//!
//! # 状态放哪
//!
//! 配置与引擎是**进程级单例**（提权、单实例、托盘都在窗口之前），由 [`state`]
//! 的 `install` 在 `main` 里装一次。界面不持有它们，只调函数、读快照——所以
//! 界面什么时候重建、重建几次都不影响业务。

pub mod network;
pub mod profiles;
pub mod program;
pub mod snapshot;
pub mod split;
pub mod state;
pub mod theme;

pub use snapshot::{Snapshot, bytes_text, rate_text};
pub use state::LogFilter;
