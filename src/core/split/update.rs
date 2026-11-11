//! 清单在线更新：点一次「立即更新」，把三份清单换成今天的数据。
//!
//! # 数据从哪来
//!
//! | 数据 | 地址 |
//! |------|------|
//! | 国内 IP 段 | `https://ftp.apnic.net/apnic/stats/apnic/delegated-apnic-latest` |
//! | `.cn` 等 TLD | `https://raw.githubusercontent.com/v2fly/domain-list-community/master/data/tld-cn` |
//! | 国内域名主清单 | `.../data/geolocation-cn` |
//!
//! 主清单里大量 `include:`（实测 158 个一层 + 173 个二层），必须展开：本体只有
//! 一千多条，**`qq.com`、`taobao.com`、`bilibili.com` 这些全在子清单里**，只解析
//! 本体等于这个功能对主要站点完全无效。
//!
//! # 为什么不引 HTTP 库
//!
//! 程序已经在用系统自带的 `curl` 下 tun2proxy（见 [`crate::core::binary`]），
//! 这里沿用同一条路径：不增加二进制体积，也不引入 TLS 依赖。代价是要求系统有
//! `curl`——Windows 10 1803 之后、macOS 与主流 Linux 都自带。
//!
//! # 下载失败怎么办
//!
//! 三份文件全部下载并解析成功之后才落盘（先写 `.tmp` 再改名），中途失败时
//! 磁盘上原来的那份**一个字节都不动**，程序继续用旧清单。

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::core::i18n::t;
use crate::tr_format;

use super::lists::{
    china_ip_from_apnic, expand_domain_lists, parse_domain_list, render_cidr_snapshot,
    render_domain_snapshot,
};

const APNIC_URL: &str = "https://ftp.apnic.net/apnic/stats/apnic/delegated-apnic-latest";
const DATA_BASE: &str = "https://raw.githubusercontent.com/v2fly/domain-list-community/master/data/";
/// 根清单：`tld-cn` 给 `.cn` 这类 TLD，`geolocation-cn` 给其余国内域名。
const ROOT_LISTS: [&str; 2] = ["tld-cn", "geolocation-cn"];
/// 子清单的下载轮数上限。实测两层就收敛，留三层给上游加层级用。
const MAX_DOWNLOAD_ROUNDS: usize = 4;
const DOWNLOAD_THREADS: usize = 8;

/// 一次更新的结果。
#[derive(Clone, Debug)]
pub struct UpdateOutcome {
    pub ipv4: usize,
    pub ipv6: usize,
    pub domain: usize,
    pub files: usize,
    pub unresolved: usize,
    pub date: String,
    pub seconds: f32,
}

/// 执行一次更新。`progress` 会被反复调用来报进度（不要在里面做重活）。
pub fn run(progress: &(dyn Fn(String) + Send + Sync)) -> Result<UpdateOutcome, String> {
    let started = Instant::now();
    let dir = super::library::snapshot_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|err| tr_format!("无法创建 {}：{err}", "Cannot create {}: {err}", dir.display()))?;
    let cache = dir.join("v2fly");
    std::fs::create_dir_all(&cache).map_err(|err| {
        tr_format!("无法创建 {}：{err}", "Cannot create {}: {err}", cache.display())
    })?;

    // —— IP ——
    progress(t("下载 APNIC 委派数据…", "Downloading APNIC delegations…").to_string());
    let apnic = http_text(APNIC_URL)?;
    let china = china_ip_from_apnic(&apnic);
    if china.v4.is_empty() {
        return Err(t(
            "APNIC 数据里没有解析出任何国内 IPv4 段，已放弃本次更新",
            "No Chinese IPv4 blocks were parsed from the APNIC data; update aborted",
        )
        .to_string());
    }
    let date = today();

    // —— 域名：先把子清单都抓下来，再统一展开 ——
    progress(t("下载国内域名清单…", "Downloading the China domain lists…").to_string());
    for name in ROOT_LISTS {
        let path = cache.join(name);
        let text = http_text(&format!("{DATA_BASE}{name}"))?;
        std::fs::write(&path, text).map_err(|err| {
            tr_format!("无法写入 {}：{err}", "Cannot write {}: {err}", path.display())
        })?;
    }
    let downloaded = download_includes(&cache, progress)?;
    progress(tr_format!(
        "展开子清单（已下载 {} 个）…",
        "Expanding sub-lists ({} downloaded)…",
        downloaded
    ));

    let mut roots: Vec<(String, String)> = Vec::new();
    for name in ROOT_LISTS {
        let text = std::fs::read_to_string(cache.join(name)).map_err(|err| {
            tr_format!("读不到缓存里的 {name}：{err}", "Cannot read the cached {name}: {err}")
        })?;
        roots.push((name.to_string(), text));
    }
    let root_refs: Vec<(&str, &str)> = roots
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let cache_for_fetch = cache.clone();
    let mut fetch = |name: &str| -> Option<String> {
        std::fs::read_to_string(cache_for_fetch.join(name)).ok()
    };
    let expanded = expand_domain_lists(&root_refs, &mut fetch, "国内域名");
    if expanded.rules.is_empty() {
        return Err(t(
            "域名清单展开后是空的，已放弃本次更新",
            "The domain lists expanded to nothing; update aborted",
        )
        .to_string());
    }

    // —— 落盘：全部解析成功之后才动磁盘 ——
    progress(t("写入清单文件…", "Writing the list files…").to_string());
    let ipv4_note = tr_format!(
        "来源：APNIC delegated-apnic-latest，生成于 {date}",
        "Source: APNIC delegated-apnic-latest, generated {date}"
    );
    let domain_note = tr_format!(
        "来源：v2fly domain-list-community（{} 及子清单），生成于 {}",
        "Source: v2fly domain-list-community ({} and its sub-lists), generated {}",
        ROOT_LISTS.join(" / "),
        date
    );
    write_atomic(
        &dir.join("china-ipv4.txt"),
        &render_cidr_snapshot("中国大陆 IPv4 段（在线更新）", &ipv4_note, &china.v4),
    )?;
    write_atomic(
        &dir.join("china-ipv6.txt"),
        &render_cidr_snapshot("中国大陆 IPv6 段（在线更新）", &ipv4_note, &china.v6),
    )?;
    write_atomic(
        &dir.join("china-domain.txt"),
        &render_domain_snapshot("中国大陆域名（在线更新）", &domain_note, &expanded.rules),
    )?;

    Ok(UpdateOutcome {
        ipv4: china.v4.len(),
        ipv6: china.v6.len(),
        domain: expanded.rules.len(),
        files: expanded.files,
        unresolved: expanded.unresolved.len(),
        date,
        seconds: started.elapsed().as_secs_f32(),
    })
}

/// 删掉下载下来的清单，回到内置快照。
pub fn reset() -> Result<(), String> {
    let dir = super::library::snapshot_dir();
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|err| {
        tr_format!("无法删除 {}：{err}", "Cannot remove {}: {err}", dir.display())
    })
}

/// 把 `include:` 引到的子清单全部下到 `cache`，返回这一轮新增的文件数。
///
/// 边下边扫：每一轮把「已下载文件里提到、但本地没有」的名字都抓一遍，
/// 直到没有新名字为止（上游清单会互相 include，深度不可预设）。
fn download_includes(
    cache: &Path,
    progress: &(dyn Fn(String) + Send + Sync),
) -> Result<usize, String> {
    let mut total = 0usize;
    for round in 0..MAX_DOWNLOAD_ROUNDS {
        let missing = missing_includes(cache);
        if missing.is_empty() {
            break;
        }
        progress(tr_format!(
            "下载子清单：第 {} 轮，{} 个",
            "Downloading sub-lists: round {}, {} files",
            round + 1,
            missing.len()
        ));
        let names: Vec<String> = missing.into_iter().collect();
        let done = AtomicUsize::new(0);
        let failures = AtomicUsize::new(0);
        let count = names.len();
        std::thread::scope(|scope| {
            let chunks: Vec<&[String]> = names.chunks(count.div_ceil(DOWNLOAD_THREADS).max(1)).collect();
            for chunk in chunks {
                let done = &done;
                let failures = &failures;
                scope.spawn(move || {
                    for name in chunk {
                        let url = format!("{DATA_BASE}{name}");
                        match http_text(&url) {
                            Ok(text) => {
                                let _ = std::fs::write(cache.join(name), text);
                            }
                            Err(_) => {
                                // 上游确实会删掉一部分仍被引用的名字（实测 40 个），
                                // 404 属于正常现象，不该让整次更新失败。
                                failures.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        let fetched = done.load(Ordering::Relaxed) - failures.load(Ordering::Relaxed);
        total += fetched;
        if fetched == 0 {
            // 一个都没下成：再扫也不会变，直接收手，别把用户晾在这里。
            break;
        }
    }
    Ok(total)
}

/// 扫一遍已下载的文件，挑出「被 include 了但本地还没有」的名字。
fn missing_includes(cache: &Path) -> BTreeSet<String> {
    let mut missing = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(cache) else {
        return missing;
    };
    for entry in entries.flatten() {
        if !entry.path().is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for include in parse_domain_list(&text, "cache").includes {
            if !cache.join(&include).is_file() {
                missing.insert(include);
            }
        }
    }
    missing
}

/// 先写 `.tmp` 再改名：中途失败时旧文件不会被写坏。
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text)
        .map_err(|err| tr_format!("无法写入 {}：{err}", "Cannot write {}: {err}", temp.display()))?;
    std::fs::rename(&temp, path).map_err(|err| {
        tr_format!(
            "无法替换 {}：{err}",
            "Cannot replace {}: {err}",
            path.display()
        )
    })
}

/// 用系统 `curl` 取一份文本。
///
/// `--connect-timeout` 是必须的：没有它，一个被丢弃的连接要等满 `--max-time`
/// 才失败，而 `--retry 2` 会把这个等待乘三——用户看到的是「点了更新以后
/// 四分钟毫无反应」。`--max-time` 给到 150 秒：APNIC 那份有 4 MB 多，慢一点的
/// 宽带也得留够时间。
fn http_text(url: &str) -> Result<String, String> {
    let mut command = Command::new("curl");
    command
        .args([
            "-fsSL",
            "--connect-timeout",
            "12",
            "--retry",
            "2",
            "--retry-delay",
            "1",
            "--max-time",
            "150",
            url,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|err| {
        tr_format!(
            "无法运行 curl（{err}）：请确认系统里有 curl",
            "Cannot run curl ({err}): make sure curl is installed"
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(tr_format!(
            "下载 {} 失败：{}",
            "Downloading {} failed: {}",
            url,
            curl_reason(&stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 把 curl 的 stderr 压成一句能读的话。
///
/// 原样转出去是一屏英文：`--retry 2` 会让同一句错误重复三遍，每句还带着
/// 「with 3129091 out of 4355860 bytes received」这种对用户毫无意义的数字。
/// 这里只认退出码，给一句中文；认不出来的才把第一行原样留下。
fn curl_reason(stderr: &str) -> String {
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let code = first
        .strip_prefix("curl: (")
        .and_then(|rest| rest.split(')').next())
        .unwrap_or("");
    let hint = match code {
        "28" => Some(t(
            "网络超时（下载太慢或连不上）",
            "network timeout (slow link or unreachable)",
        )),
        "6" => Some(t("域名解析失败", "DNS lookup failed")),
        "7" => Some(t("连不上服务器", "cannot connect to the server")),
        "22" => Some(t(
            "服务器返回了错误状态（可能是 404）",
            "the server returned an error status (maybe 404)",
        )),
        "35" | "60" => Some(t(
            "TLS 证书校验失败（可能被中间设备拦截）",
            "TLS verification failed (possibly intercepted)",
        )),
        _ => None,
    };
    match hint {
        // 退出码留在括号里：用户报问题时能直接对上 curl 的文档，也不必去翻日志。
        Some(hint) => format!("{hint} (curl {code})"),
        None if first.is_empty() => t("curl 返回了错误码", "curl returned an error").to_string(),
        None => first.to_string(),
    }
}

/// 今天的日期（本地时间按 UTC 计，差一天不影响「清单多旧」这个判断）。
pub fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0) as i64;
    let days = seconds / 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// 天数 → 公历年月日（Howard Hinnant 的算法，纯整数运算）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_match_known_days() {
        // 1970-01-01 是第 0 天。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2024-02-29（闰日）与 2026-11-11。
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(20_768), (2026, 11, 11));
    }

    #[test]
    fn curl_errors_are_condensed_into_one_sentence() {
        let stderr = "curl: (28) Operation timed out after 90009 milliseconds with 3129091 out of 4355860 bytes received\n\
                      curl: (28) Operation timed out after 90009 milliseconds with 3014403 out of 4355860 bytes received\n";
        let reason = curl_reason(stderr);
        assert!(reason.contains("超时") || reason.contains("timeout"), "{reason}");
        assert!(reason.contains("28"), "{reason}");
        assert!(!reason.contains("bytes received"), "{reason}");

        // 认不出来的错误原样保留第一行，别把信息吞掉。
        let reason = curl_reason("curl: (97) something odd\n");
        assert_eq!(reason, "curl: (97) something odd");
    }

    #[test]
    fn today_looks_like_a_date() {
        let date = today();
        assert_eq!(date.len(), 10, "{date}");
        assert!(date.starts_with("20"), "{date}");
        assert_eq!(&date[4..5], "-");
    }

    #[test]
    fn missing_include_scan_reads_the_cache_dir() {
        let dir = std::env::temp_dir().join("tun2proxy-gui-split-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), "qq.com\ninclude:b\ninclude:c\n").unwrap();
        std::fs::write(dir.join("b"), "baidu.com\n").unwrap();
        let missing = missing_includes(&dir);
        // `b` 已下载，`c` 没有。
        assert_eq!(missing.into_iter().collect::<Vec<_>>(), vec!["c".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
