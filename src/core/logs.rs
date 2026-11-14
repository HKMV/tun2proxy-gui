//! 运行日志：`tracing` + `tracing-subscriber` + `tracing-appender`。
//!
//! 写法与 devroute 的 `libs/logs.rs` 保持一致：一个 subscriber 带两层，
//!
//! - **文件层**：按天滚动写 `<数据目录>/logs/tun2proxy-gui.YYYY-MM-DD.log`，
//!   只保留最近 [`KEEP_DAYS`] 天（`tracing-appender` 自己清理）；
//! - **界面层**（[`UiLayer`]）：把同一条事件格式化成一行推进
//!   [`Shared::push_log`]，也就是界面「程序与日志」那一块。
//!
//! 两层吃的是同一批事件，只是文件层会丢掉子进程的 INFO 行（见下）；点「清空
//! 日志」只清界面那一份，文件是事后要查的证据。
//!
//! # 三种来源，三种排版
//!
//! | target | 来源 | 界面/文件里的样子 |
//! |--------|------|------------------|
//! | [`TARGET_OUTPUT`] | tun2proxy 子进程的 stdout/stderr | `YYYY-MM-DD HH:MM:SS 级别 [tun2proxy] 消息` |
//! | [`TARGET_APP`] | 本程序的提示（启动、下载、托盘…） | `YYYY-MM-DD HH:MM:SS 消息` |
//! | [`TARGET_SPLIT`] | 分流器每条连接的判定 | `YYYY-MM-DD HH:MM:SS 消息` |
//! | 其它 | `log` / `tracing` 宏（含 gpuikit） | `YYYY-MM-DD HH:MM:SS 级别 消息` |
//!
//! # 子进程那一层为什么要改写时间
//!
//! tun2proxy 的行自带 `2026-11-12T08:03:18.123456Z INFO  tun2proxy: ...`，是
//! **UTC**；本程序的提示是本地时间。同一份文件里两套差 8 小时的时间，读的人
//! 第一眼就会怀疑哪条是哪一层写的。所以 [`relocalize`] 把上游那个时间戳换成
//! 本地时间，级别与目标照抄，行首只剩一套时间，整份日志按时间排得整齐。
//!
//! 直接原样写、再套一层我们的时间戳也不行：那是「两个时间、两个级别」。
//!
//! # 文件只留 WARN 以上
//!
//! 子进程按连接打日志（一次连接 `Beginning` + `Ending` 两行），实测每秒 6 KB，
//! 一天几百 MB，而事后要查的通常只有报错。所以文件层丢掉子进程的 INFO 及以下
//! （[`Level::WARN`] 以上照写），界面层照旧全显示——面板是用来盯现场的，文件
//! 是用来事后翻的，两边要的东西不一样。
//!
//! 界面上「仅分流判定」那一档过滤（`ui::state::LogFilter`）解决的是另一个
//! 问题：分流判定自己也是 INFO，会被子进程的这两行刷走。
//!
//! # 第三方库怎么进来
//!
//! `tracing-subscriber` 默认带 `tracing-log` 特性，`try_init` 时会把 `log`
//! crate 的 logger 换成转发器，于是 gpuikit / gpui 里的 `log::error!` 也进了
//! 同一份日志——以前那份手写 `log::set_logger` 就多余了。

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use time::format_description::well_known::Rfc3339;
use time::format_description::FormatItem;
use time::OffsetDateTime;
use time::UtcOffset;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::rolling;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::core::engine::Shared;
use crate::core::paths;

/// tun2proxy 子进程的原样输出。
pub const TARGET_OUTPUT: &str = "tun2proxy";
/// 本程序自己的提示。
pub const TARGET_APP: &str = "app";
/// 分流器的连接判定。
pub const TARGET_SPLIT: &str = "split";

/// 一行日志的来源。
///
/// 界面要按来源过滤（「仅分流判定」这一档），所以来源必须跟着行走——
/// 行已经格式化成一整串字符串，事后没法再猜是哪来的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    /// 本程序自己的提示。
    App,
    /// 分流器每条连接的判定。
    Split,
    /// tun2proxy 子进程的输出。
    Output,
    /// `log` / `tracing` 宏与第三方库。
    Other,
}

/// 一行日志：来源 + 已经排版好的文字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

fn kind_of(target: &str) -> LogKind {
    match target {
        TARGET_OUTPUT => LogKind::Output,
        TARGET_APP => LogKind::App,
        TARGET_SPLIT => LogKind::Split,
        _ => LogKind::Other,
    }
}

/// 日志文件名前缀，同时也是 panic 日志的前缀。
const LOG_PREFIX: &str = "tun2proxy-gui";
/// 保留最近多少天的日志（`tracing-appender` 按文件名日期清理）。
const KEEP_DAYS: usize = 7;
/// 默认记录到 INFO：DEBUG 会把 tokio / gpui 的内部细节一起倒进界面。
const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

/// tun2proxy 在并发会话名额打满、丢弃新连接时打的日志片段。
///
/// 原文是 `Too many sessions that over {limit}, dropping new session`（tun2proxy
/// 0.8.6 `src/lib.rs`）。只认后半句：数字会随配置变，而且这里也不需要它。
///
/// 打满时这条会跟着**每一条**被丢的连接重复，几百行里夹一句，用户不可能看见；
/// 所以界面层顺手在共享状态里留一个标记，交给提示条去说。
const SESSIONS_FULL_MARKER: &str = "dropping new session";

/// 文件里的时间：精确到毫秒，跨天时能看出是哪一天的那一条。
const FILE_TS: &[FormatItem<'static>] = time::macros::format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
);

/// 界面行首的时间：日期 + 时分秒，精确到秒。
///
/// 日期不能省：程序常开着不重启，面板里会横跨零点；只写时分秒的话，前一天的
/// 「23:59」和今天的「00:01」看起来是同一个瞬间。毫秒留给文件——面板里对齐
/// 读的是「什么时候发生了什么」，不是亚秒级的先后。
const UI_TS: &[FormatItem<'static>] =
    time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// 本地时区偏移，进程内只算一次。
///
/// 首选系统给的偏移；`time` crate 取不到时（不在主线程、或平台不支持）退回
/// `+8`——这是本程序和 devroute 作者所在时区，日志时间只是给人看的，宁可
/// 固定东八区也不要整份日志差 8 小时。
static LOCAL_OFFSET: LazyLock<UtcOffset> =
    LazyLock::new(|| UtcOffset::current_local_offset().unwrap_or(time::macros::offset!(+8)));

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(*LOCAL_OFFSET)
}

/// 初始化日志系统，返回当天日志文件的路径。
///
/// 调用一次就好（第二次会因为全局 subscriber 已存在而什么也不做）。目录建不
/// 出来时返回错误，但**界面层仍然装上**：拿不到历史文件不该让程序连日志都
/// 没有。
pub fn init(shared: Arc<Mutex<Shared>>, dirty: Arc<AtomicBool>) -> io::Result<PathBuf> {
    let dir = paths::logs_dir();
    let ui = UiLayer { shared, dirty };
    let file_layer = match file_layer(&dir) {
        Ok(layer) => layer,
        Err(err) => {
            // 只装界面层：日志还能看，只是没有历史文件。
            let _ = tracing_subscriber::registry()
                .with(targets())
                .with(ui)
                .try_init();
            log::set_max_level(log::LevelFilter::Trace);
            return Err(err);
        }
    };

    install_panic_hook(&dir);
    let _ = tracing_subscriber::registry()
        .with(targets())
        .with(file_layer)
        .with(ui)
        .try_init();
    // log 宏自己有一道 `max_level` 短路，设成 Trace 才不会在进 tracing 之前
    // 就被截掉——真正决定记不记的是上面的 `Targets`。
    log::set_max_level(log::LevelFilter::Trace);
    Ok(current_file(&dir))
}

/// 当天的日志文件路径。
///
/// 命名规则来自 [`rolling`]：`{前缀}.{日期}.{后缀}`。启动时把这条路径打进
/// 日志，用户就不用猜「日志到底写哪儿去了」。
///
/// 日期必须和 `rolling` 用同一套历法：它按 **UTC** 换日（`tracing-appender`
/// 内部就是拿到 `SystemTime` 直接取 UTC 日期），而日志**内容**里的时间戳是
/// 本地时间（见 [`now`]）。东八区凌晨 0 点到 8 点之间这两者差一天——按本地日期
/// 拼出来的路径那会儿并不存在，用户照着这句提示去打开日志只会扑空。
pub fn current_file(dir: &Path) -> PathBuf {
    let day = OffsetDateTime::now_utc()
        .format(&time::macros::format_description!("[year]-[month]-[day]"))
        .unwrap_or_default();
    dir.join(format!("{LOG_PREFIX}.{day}.log"))
}

fn file_layer<S>(
    dir: &Path,
) -> io::Result<
    tracing_subscriber::fmt::Layer<
        S,
        tracing_subscriber::fmt::format::DefaultFields,
        FileFormat,
        rolling::RollingFileAppender,
    >,
>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fs::create_dir_all(dir)?;
    let appender = rolling::Builder::new()
        .filename_prefix(LOG_PREFIX)
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .rotation(rolling::Rotation::DAILY)
        .build(dir)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .event_format(FileFormat)
        .with_writer(appender))
}

/// 全局级别与降噪。
///
/// 第三方的 DEBUG 是纯噪声（每一次绘制、每一个字形都可能打一行），它们在界面
/// 上会直接把用户要看的那几条冲走，所以单独压到 WARN。
///
/// 子进程那一类反而放到 DEBUG：级别由它自己行里的级别决定（见
/// [`upstream_level`]），把门开到 DEBUG 才接得住 `--verbosity debug` 的输出。
fn targets() -> Targets {
    Targets::new()
        .with_default(DEFAULT_LEVEL)
        .with_target(TARGET_OUTPUT, Level::DEBUG)
        .with_target("gpui", Level::WARN)
        .with_target("gpui_platform", Level::WARN)
        .with_target("gpuikit", Level::WARN)
        .with_target("accesskit", Level::WARN)
}

/// 界面层：把事件格式化成一行塞进共享状态。
struct UiLayer {
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
}

impl<S: Subscriber> Layer<S> for UiLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        let line = ui_line(meta.target(), *meta.level(), &message);
        if let Ok(mut shared) = self.shared.lock() {
            if message.contains(SESSIONS_FULL_MARKER) {
                shared.sessions_full = true;
            }
            shared.push_log(line.0, line.1);
        }
        self.dirty.store(true, Ordering::Relaxed);
    }
}

/// 文件层：`2026-11-12 16:03:18.123 INFO  [app] 消息`，子进程输出原样。
struct FileFormat;

impl<S, F> FormatEvent<S, F> for FileFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    F: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, F>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        if is_passthrough(meta.target()) {
            // 子进程的 INFO 及以下不落盘：一次连接两行，一天几百 MB，而事后要
            // 查的通常只有报错。界面层不受这条限制——面板是用来盯现场的。
            if *meta.level() > Level::WARN {
                return Ok(());
            }
            return writeln!(writer, "{}", relocalize(&message, FILE_TS));
        }
        writeln!(
            writer,
            "{} {:<5} [{}] {message}",
            now().format(FILE_TS).unwrap_or_default(),
            meta.level(),
            meta.target()
        )
    }
}

/// 只取 `message` 字段。
///
/// 默认的字段格式化会把 `key=value` 一个不落地拼上，日志行越堆越长；这里只要
/// 人写的那句话，需要额外信息就把它写进消息里。
struct MessageVisitor<'a>(&'a mut String);

impl Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            use fmt::Write as _;
            let _ = write!(self.0, "{value:?}");
        }
    }
}

fn is_passthrough(target: &str) -> bool {
    target == TARGET_OUTPUT
}

/// 子进程这一行自己的级别（拿不到就算 INFO）。
///
/// 文件层只留 WARN 以上，靠的就是它：读行本身比让调用方传级别可靠——子进程
/// 的级别只写在行里，`tracing` 那边记录时统一是同一个级别。
pub fn upstream_level(line: &str) -> Level {
    let Some((head, _)) = line.strip_prefix('[').and_then(|rest| rest.split_once(']')) else {
        // `main` 直接返回的 `Error: ...` 没有方括号，但那是「进程要退出了」的
        // 一条，按 ERROR 记，文件里必须留下。
        return if line.trim_start().starts_with("Error:") {
            Level::ERROR
        } else {
            Level::INFO
        };
    };
    match head.split_whitespace().nth(1) {
        Some("ERROR") => Level::ERROR,
        Some("WARN") => Level::WARN,
        Some("DEBUG") => Level::DEBUG,
        Some("TRACE") => Level::TRACE,
        _ => Level::INFO,
    }
}

/// 把子进程那一行改写成「本地时间 + 级别 + 目标」。
///
/// 上游的行长这样（UTC、级别和目标挤在方括号里）：
///
/// ```text
/// [2026-11-12T09:55:29Z INFO  tun2proxy] Beginning #1146 TCP 10.0.0.33:1 -> 198.18.0.4:443
/// ```
///
/// 改写成和本程序其它日志同一个样子，行首只剩一套时间：
///
/// ```text
/// 2026-11-12 17:55:29.123 INFO  [tun2proxy] Beginning #1146 TCP 10.0.0.33:1 -> 198.18.0.4:443
/// ```
///
/// 认不出这个形状（上游换了格式、或者压根不是 tracing 输出的裸文本）就只往前
/// 补一个本地时间，原文一个字不动——宁可多一个时间戳，也不要把它丢掉。
fn relocalize(line: &str, format: &[FormatItem<'static>]) -> String {
    let clock = || now().format(format).unwrap_or_default();
    let Some((head, body)) = line.strip_prefix('[').and_then(|rest| rest.split_once("] ")) else {
        return format!("{} {line}", clock());
    };
    let mut parts = head.split_whitespace();
    let (Some(stamp), Some(level)) = (parts.next(), parts.next()) else {
        return format!("{} {line}", clock());
    };
    let target = parts.next().unwrap_or(TARGET_OUTPUT);
    let local = OffsetDateTime::parse(stamp, &Rfc3339)
        .map(|at| at.to_offset(*LOCAL_OFFSET))
        .ok()
        .and_then(|at| at.format(format).ok())
        .unwrap_or_else(clock);
    format!("{local} {level:<5} [{target}] {body}")
}

/// 界面里的一行：来源 + 排版好的文字。
///
/// 三种来源三种排版（见模块文档的表格）：带级别的用 `{level:<5}` 对齐；程序
/// 自己的提示与分流器判定不带级别——它们本来就是「结果」而不是「事件」。
fn ui_line(target: &str, level: Level, message: &str) -> (LogKind, String) {
    let kind = kind_of(target);
    if kind == LogKind::Output {
        return (kind, relocalize(message, UI_TS));
    }
    let clock = now().format(UI_TS).unwrap_or_default();
    let text = match kind {
        LogKind::App | LogKind::Split => format!("{clock} {message}"),
        _ => format!("{clock} {level:<5} {message}"),
    };
    (kind, text)
}

/// panic 写进 `tun2proxy-gui.panic.log`。
///
/// panic 之后进程就没了，界面上什么都留不下；这个文件是唯一的现场。release
/// 是 `panic = "abort"`，hook 照样会在 abort 之前跑完。
fn install_panic_hook(dir: &Path) {
    let path = dir.join(format!("{LOG_PREFIX}.panic.log"));
    std::panic::set_hook(Box::new(move |info| {
        let text = format!(
            "{} {info}\n{}\n",
            now().format(FILE_TS).unwrap_or_default(),
            std::backtrace::Backtrace::force_capture()
        );
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(text.as_bytes());
        }
        // 控制台里也留一份：开发时不用去翻文件。
        eprintln!("{text}");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_lines_carry_a_clock_and_a_kind() {
        // 程序自己的提示：只有时间，没有级别（级别在这一行里没有信息量）。
        let (kind, app) = ui_line(TARGET_APP, Level::INFO, "配置目录 C:\\x");
        assert_eq!(kind, LogKind::App);
        assert!(app.ends_with(" 配置目录 C:\\x"), "{app:?}");
        assert!(!app.contains("INFO"), "{app:?}");
        // 行首是 `YYYY-MM-DD HH:MM:SS`：带日期，跨零点时也分得清是哪一天。
        assert_eq!(&app[4..5], "-", "{app:?}");
        assert_eq!(&app[10..11], " ", "{app:?}");
        assert_eq!(&app[13..14], ":", "{app:?}");

        // 分流器同理。
        let (kind, split) = ui_line(TARGET_SPLIT, Level::INFO, "直连 1.1.1.1");
        assert_eq!(kind, LogKind::Split);
        assert!(!split.contains("INFO"));

        // 第三方 / log 宏：带上级别。
        let (kind, other) = ui_line("gpuikit", Level::ERROR, "保存失败");
        assert_eq!(kind, LogKind::Other);
        assert!(other.contains("ERROR 保存失败"), "{other:?}");

        // 子进程那一层：时间换成本地时钟，级别与目标照抄。
        let (kind, output) = ui_line(
            TARGET_OUTPUT,
            Level::INFO,
            "[2026-11-12T08:00:00Z INFO  tun2proxy] Beginning #1",
        );
        assert_eq!(kind, LogKind::Output);
        let tail = " INFO  [tun2proxy] Beginning #1";
        assert!(output.ends_with(tail), "{output:?}");
        // 前 19 个字符是 `YYYY-MM-DD HH:MM:SS`：既不带上游那个 UTC 时间戳，
        // 也不重复时间。
        assert_eq!(output.len() - tail.len(), 19, "{output:?}");
    }

    #[test]
    fn upstream_times_are_rewritten_into_local_time() {
        let text = relocalize(
            "[2026-11-12T08:00:00Z INFO  tun2proxy] Beginning #1",
            FILE_TS,
        );
        // 同一个瞬间换个时区表示；不写死 +8，测试跟着机器时区走。
        let expected = OffsetDateTime::parse("2026-11-12T08:00:00Z", &Rfc3339)
            .unwrap()
            .to_offset(*LOCAL_OFFSET)
            .format(FILE_TS)
            .unwrap();
        assert_eq!(text, format!("{expected} INFO  [tun2proxy] Beginning #1"));
    }

    #[test]
    fn unknown_shapes_keep_their_text_and_still_get_a_clock() {
        let text = relocalize("Error: bad config", FILE_TS);
        assert!(text.ends_with(" Error: bad config"), "{text:?}");
        // 行首是 `YYYY-MM-DD HH:MM:SS.mmm`：第 5 位是 `-`，第 11 位是空格。
        assert_eq!(&text[4..5], "-", "{text:?}");
        assert_eq!(&text[10..11], " ", "{text:?}");
    }

    #[test]
    fn the_upstream_level_comes_from_the_line_itself() {
        assert_eq!(
            upstream_level("[2026-11-12T09:55:29Z INFO  tun2proxy] hi"),
            Level::INFO
        );
        assert_eq!(
            upstream_level("[2026-11-12T09:55:29Z WARN  tun2proxy] hi"),
            Level::WARN
        );
        assert_eq!(
            upstream_level("[2026-11-12T09:55:29Z ERROR tun2proxy] hi"),
            Level::ERROR
        );
        // `main` 直接返回的那种没有方括号，但它是死因，必须算 ERROR。
        assert_eq!(upstream_level("Error: main returned 1"), Level::ERROR);
        assert_eq!(upstream_level("something odd"), Level::INFO);
    }

    #[test]
    fn the_file_path_names_the_current_day() {
        let path = current_file(Path::new("/tmp/logs"));
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("tun2proxy-gui."), "{name}");
        assert!(name.ends_with(".log"), "{name}");
        // tun2proxy-gui.2026-11-12.log：去掉前缀和后缀，中间正好是 10 个字符的日期。
        let day = name
            .trim_start_matches("tun2proxy-gui.")
            .trim_end_matches(".log");
        assert_eq!(day.len(), 10, "{name}");
        assert_eq!(day.matches('-').count(), 2, "{name}");
    }

    #[test]
    fn the_file_layer_localises_upstream_lines_and_drops_their_info() {
        // 用内存 writer 走一遍真实的 FormatEvent：子进程的 INFO 不落盘，
        // WARN 以上落盘并且换成本地时间。
        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl Write for Buffer {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
            type Writer = Self;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buffer = Buffer::default();
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .event_format(FileFormat)
            .with_writer(buffer.clone());
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: TARGET_APP, "配置目录 D:/x");
            tracing::info!(
                target: TARGET_OUTPUT,
                "[2026-11-12T08:00:00Z INFO  tun2proxy] Beginning #1"
            );
            tracing::warn!(
                target: TARGET_OUTPUT,
                "[2026-11-12T08:00:01Z WARN  tun2proxy] Too many sessions that over 200"
            );
        });

        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert!(lines[0].contains("[app] 配置目录 D:/x"), "{text:?}");
        assert!(
            lines[1].ends_with(" WARN  [tun2proxy] Too many sessions that over 200"),
            "子进程的 WARN 要落盘，并且换成本地时间：{text:?}"
        );
    }
}
