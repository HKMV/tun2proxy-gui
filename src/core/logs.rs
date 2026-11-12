//! 运行日志：`tracing` + `tracing-subscriber` + `tracing-appender`。
//!
//! 写法与 devroute 的 `libs/logs.rs` 保持一致：一个 subscriber 带两层，
//!
//! - **文件层**：按天滚动写 `<数据目录>/logs/tun2proxy-gui.YYYY-MM-DD.log`，
//!   只保留最近 [`KEEP_DAYS`] 天（`tracing-appender` 自己清理）；
//! - **界面层**（[`UiLayer`]）：把同一条事件格式化成一行推进
//!   [`Shared::push_log`]，也就是界面「程序与日志」那一块。
//!
//! 两层吃的是同一批事件，所以界面看到的和文件里写的永远对得上；点「清空日志」
//! 只清界面那一份，文件是事后要查的证据。
//!
//! # 三种来源，三种排版
//!
//! | target | 来源 | 界面/文件里的样子 |
//! |--------|------|------------------|
//! | [`TARGET_OUTPUT`] | tun2proxy 子进程的 stdout/stderr | **原样**（它自己带 ISO 时间戳和级别） |
//! | [`TARGET_APP`] | 本程序的提示（启动、下载、托盘…） | `HH:MM:SS 消息` |
//! | [`TARGET_SPLIT`] | 分流器每条连接的判定 | `HH:MM:SS 消息` |
//! | 其它 | `log` / `tracing` 宏（含 gpuikit） | `HH:MM:SS 级别 消息` |
//!
//! 子进程那一类必须原样写：tun2proxy 的行本来就是
//! `2026-11-12T08:03:18.123456Z INFO tun2proxy::...`，再套一层我们的时间戳，
//! 文件里就成了「两个时间、两个级别」，读的人第一眼就会怀疑是哪一层的。
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

/// 日志文件名前缀，同时也是 panic 日志的前缀。
const LOG_PREFIX: &str = "tun2proxy-gui";
/// 保留最近多少天的日志（`tracing-appender` 按文件名日期清理）。
const KEEP_DAYS: usize = 7;
/// 默认记录到 INFO：DEBUG 会把 tokio / gpui 的内部细节一起倒进界面。
const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

/// 文件里的时间：精确到毫秒，跨天时能看出是哪一天的那一条。
const FILE_TS: &[FormatItem<'static>] = time::macros::format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
);

/// 界面行首只放 `HH:MM:SS`：日志面板窄，日期在文件名里已经有。
const CLOCK_TS: &[FormatItem<'static>] =
    time::macros::format_description!("[hour]:[minute]:[second]");

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
pub fn current_file(dir: &Path) -> PathBuf {
    let day = now()
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
fn targets() -> Targets {
    Targets::new()
        .with_default(DEFAULT_LEVEL)
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
            shared.push_log(line);
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
            return writeln!(writer, "{message}");
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

/// 界面里的一行。
///
/// 三种来源三种排版（见模块文档的表格）：带级别的用 `{level:<5}` 对齐；程序
/// 自己的提示与分流器判定不带级别——它们本来就是「结果」而不是「事件」。
fn ui_line(target: &str, level: Level, message: &str) -> String {
    if is_passthrough(target) {
        return message.to_string();
    }
    let clock = now().format(CLOCK_TS).unwrap_or_default();
    match target {
        TARGET_APP | TARGET_SPLIT => format!("{clock} {message}"),
        _ => format!("{clock} {level:<5} {message}"),
    }
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
    fn ui_lines_carry_a_clock_except_for_raw_output() {
        // 程序自己的提示：只有时间，没有级别（级别在这一行里没有信息量）。
        let app = ui_line(TARGET_APP, Level::INFO, "配置目录 C:\\x");
        assert!(app.ends_with(" 配置目录 C:\\x"), "{app:?}");
        assert!(!app.contains("INFO"), "{app:?}");
        assert_eq!(&app[2..3], ":", "{app:?}");

        // 分流器同理。
        assert!(!ui_line(TARGET_SPLIT, Level::INFO, "直连 1.1.1.1").contains("INFO"));

        // 第三方 / log 宏：带上级别。
        let other = ui_line("gpuikit", Level::ERROR, "保存失败");
        assert!(other.contains("ERROR 保存失败"), "{other:?}");

        // 子进程输出原样，连时间也不加——它自己带。
        assert_eq!(
            ui_line(
                TARGET_OUTPUT,
                Level::INFO,
                "2026-11-12T08:00:00Z INFO tun2proxy: hi"
            ),
            "2026-11-12T08:00:00Z INFO tun2proxy: hi"
        );
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
    fn the_file_layer_writes_raw_output_verbatim() {
        // 用内存 writer 走一遍真实的 FormatEvent：子进程的行不许被加上前缀。
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
            tracing::info!(target: TARGET_OUTPUT, "2026-11-12T08:00:00Z INFO tun2proxy: hi");
        });

        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert!(lines[0].contains("[app] 配置目录 D:/x"), "{text:?}");
        assert_eq!(
            lines[1], "2026-11-12T08:00:00Z INFO tun2proxy: hi",
            "子进程的行要原样落盘：{text:?}"
        );
    }
}
