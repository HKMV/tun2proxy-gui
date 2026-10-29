//! tun2proxy 官方二进制的下载、校验与安装。
//!
//! GUI **不内置** tun2proxy，也不链接它的库：首次运行时从 GitHub Release
//! 下载当前平台对应的 zip，解压到 [`paths::bin_dir`]，之后随时可以检测新版
//! 并原地替换。好处是 GUI 的发布节奏不必跟随 tun2proxy，也不必为每个平台
//! 重新打包二进制。
//!
//! 下载依赖系统自带的 `curl`（Windows 10+ / macOS / 主流 Linux 都有），
//! 缺失时依次尝试 `wget`、Windows 上的 PowerShell。解压用纯 Rust 的 `zip`
//! crate，不再依赖外部解压工具。
//!
//! Release 包内除主程序 `tun2proxy-bin` 外，还包含 Windows 必需的
//! `wintun.dll`，所以安装时**整个压缩包一起解压**，而不是只挑主程序。

use std::io::Read;
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::core::i18n::t;
use crate::core::paths;
use crate::tr_format;

const USER_AGENT: &str = "tun2proxy-gui";
const API_LATEST: &str = "https://api.github.com/repos/{repo}/releases/latest";

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// Entry point of the downloaded CLI (`tun2proxy-bin` inside the archive).
pub fn exe_path() -> PathBuf {
    let name = if cfg!(windows) {
        "tun2proxy-bin.exe"
    } else {
        "tun2proxy-bin"
    };
    paths::bin_dir().join(name)
}

pub fn is_installed() -> bool {
    exe_path().is_file()
}

/// Runs `<bin> --version` and returns the bare version number.
pub fn installed_version() -> Option<String> {
    if !is_installed() {
        return None;
    }
    // 必须用 `command()` 而不是裸的 `Command::new`：Windows 上每次探测版本
    // 都会弹一下控制台黑窗（CREATE_NO_WINDOW 没设），启动瞬间很刺眼。
    let output = command(exe_path())
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version(&stdout).or_else(|| parse_version(&String::from_utf8_lossy(&output.stderr)))
}

/// Extracts `1.2.3` out of `tun2proxy 1.2.3 (abc 2025-01-01 00:00:00)`.
pub fn parse_version(text: &str) -> Option<String> {
    for token in text.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        {
            return Some(token.to_string());
        }
    }
    None
}

pub fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// 当前平台在 Release 中的目标三元组，用于拼出资产文件名。
/// 例如 `x86_64-pc-windows-msvc` → `tun2proxy-x86_64-pc-windows-msvc.zip`。
pub fn platform_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("windows", "x86") => Some("i686-pc-windows-msvc"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("linux", "arm") => Some("armv7-unknown-linux-musleabihf"),
        ("linux", "x86") => Some("i686-unknown-linux-musl"),
        _ => None,
    }
}

/// 从 Release 中挑出当前平台对应的压缩包。
pub fn asset_for_this_platform(release: &Release) -> Option<Asset> {
    let target = platform_target()?;
    release
        .assets
        .iter()
        .find(|asset| asset.name == format!("tun2proxy-{target}.zip"))
        .cloned()
}

/// 查询仓库最新 Release（GitHub API `/releases/latest`）。
pub fn fetch_release(repo: &str) -> Result<Release, String> {
    let url = API_LATEST.replace("{repo}", repo);
    let body = http_get(&url)?;
    serde_json::from_slice(&body).map_err(|err| {
        tr_format!(
            "解析 GitHub 响应失败：{err}",
            "Failed to parse the GitHub response: {err}"
        )
    })
}

/// 用系统自带工具发一次 GET，返回响应体。
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let mut errors = Vec::new();

    // 单次最多等 15 秒：三种方式都失败时，用户等 45 秒就能看到错误，
    // 而不是对着「正在查询最新版本…」发呆三分钟。
    match capture(
        "curl",
        &[
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "15",
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-H",
            "Accept: application/vnd.github+json",
            url,
        ],
    ) {
        Ok(output) => return Ok(output),
        // curl 的报错里带 HTTP 状态码和网络原因，直接透出便于用户判断
        // （403 通常是代理拦截，28 是超时）。
        Err(err) => errors.push(format!("curl：{err}")),
    }

    match capture(
        "wget",
        &[
            "-q",
            "-O",
            "-",
            "--timeout=15",
            &format!("--user-agent={USER_AGENT}"),
            url,
        ],
    ) {
        Ok(output) => return Ok(output),
        Err(err) => errors.push(format!("wget：{err}")),
    }

    #[cfg(windows)]
    {
        let script = format!(
            "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12;\
             $ProgressPreference='SilentlyContinue';\
             (Invoke-WebRequest -UseBasicParsing -UserAgent '{USER_AGENT}' -Uri '{url}').Content"
        );
        if let Ok(output) = capture(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        ) {
            return Ok(output);
        }
        errors.push(t("powershell：请求失败", "powershell: request failed").to_string());
    }

    Err(tr_format!(
        "无法访问 {}（{}）。可以手动下载后放到 {}",
        "Cannot reach {} ({}). You can download it manually into {}",
        url,
        errors.join("；"),
        paths::bin_dir().display()
    ))
}

fn capture(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(tr_format!(
            "退出码 {:?}：{}",
            "Exit code {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// 下载 `url` 到 `dest`，并持续回调已下载字节数（用于进度条）。
pub fn download(url: &str, dest: &Path, progress: impl Fn(u64)) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            tr_format!(
                "无法创建 {}：{err}",
                "Cannot create {}: {err}",
                parent.display()
            )
        })?;
    }
    let _ = std::fs::remove_file(dest);
    let dest_text = dest.to_string_lossy().to_string();

    let mut child = match command("curl")
        .args([
            "-fL",
            "--retry",
            "2",
            "--connect-timeout",
            "15",
            // 连接建立了但完全没有进度（低于 1KB/s 持续 30 秒）就放弃，
            // 避免卡在半死不活的连接上。
            "--speed-limit",
            "1024",
            "--speed-time",
            "30",
            "--max-time",
            "1800",
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-o",
            &dest_text,
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return Err(tr_format!(
                "无法运行 curl（{err}），请手动下载 {} 到 {}",
                "Cannot run curl ({err}); download {} into {} manually",
                url,
                paths::bin_dir().display()
            ));
        }
    };

    loop {
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(meta) = std::fs::metadata(dest) {
            progress(meta.len());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return Err(tr_format!(
                    "下载失败（退出码 {:?}）：{}",
                    "Download failed (exit code {:?}): {}",
                    status.code(),
                    stderr.trim()
                ));
            }
            Ok(None) => {}
            Err(err) => {
                return Err(tr_format!(
                    "下载进程异常：{err}",
                    "The download process failed: {err}"
                ))
            }
        }
    }
}

/// 校验 SHA256。`digest` 形如 `sha256:xxxx`，来自 GitHub API 的资产元数据。
pub fn verify_sha256(path: &Path, digest: &str) -> Result<(), String> {
    let expected = digest
        .trim()
        .trim_start_matches("sha256:")
        .to_ascii_lowercase();
    if expected.len() != 64 {
        return Ok(());
    }
    let mut file = std::fs::File::open(path).map_err(|err| {
        tr_format!(
            "无法读取下载文件：{err}",
            "Cannot read the downloaded file: {err}"
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| {
            tr_format!(
                "读取下载文件失败：{err}",
                "Reading the downloaded file failed: {err}"
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(tr_format!(
            "校验失败：期望 {expected}，实际 {actual}",
            "Verification failed: expected {expected}, got {actual}"
        ))
    }
}

/// 解压 Release 包到 bin 目录，返回新装好的 tun2proxy 版本号。
///
/// 整个包一起解压（主程序 + `wintun.dll` 等），然后在 bin 目录里执行
/// `tun2proxy-bin --version` 做一次安装后自检。
pub fn install_zip(archive: &Path) -> Result<String, String> {
    extract_zip(archive, &paths::bin_dir())?;
    installed_version().ok_or_else(|| {
        t(
            "安装完成，但无法执行 tun2proxy 获取版本号",
            "Installed, but tun2proxy could not be run to read its version",
        )
        .to_string()
    })
}

/// 把压缩包里的所有文件解压到 `dir`（主程序与 `wintun.dll` 之类要放在一起）。
///
/// 单独拆出来是为了能对「解压」这一步写离线单元测试。
fn extract_zip(archive: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| {
        tr_format!(
            "无法创建 {}：{err}",
            "Cannot create {}: {err}",
            dir.display()
        )
    })?;

    let file = std::fs::File::open(archive)
        .map_err(|err| tr_format!("无法打开压缩包：{err}", "Cannot open the archive: {err}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|err| tr_format!("压缩包损坏：{err}", "The archive is corrupt: {err}"))?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|err| {
            tr_format!(
                "读取压缩包条目失败：{err}",
                "Cannot read an archive entry: {err}"
            )
        })?;
        let Some(name) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };
        let target = dir.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).ok();
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|err| {
            tr_format!(
                "解压 {} 失败：{err}",
                "Failed to extract {}: {err}",
                name.display()
            )
        })?;
        std::fs::write(&target, &bytes).map_err(|err| {
            tr_format!(
                "写入 {} 失败：{err}",
                "Failed to write {}: {err}",
                target.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解压时会把条目里的目录层级原样展开，并写进目标目录。
    #[test]
    fn extract_zip_writes_entries() {
        let dir = std::env::temp_dir().join(format!("t2p-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("fake.zip");

        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("tun2proxy-bin.exe", options).unwrap();
            zip.write_all(b"stub").unwrap();
            zip.start_file("wintun.dll", options).unwrap();
            zip.write_all(b"dll").unwrap();
            zip.finish().unwrap();
        }

        let out = dir.join("bin");
        extract_zip(&archive, &out).unwrap();
        assert_eq!(
            std::fs::read(out.join("tun2proxy-bin.exe")).unwrap(),
            b"stub"
        );
        assert_eq!(std::fs::read(out.join("wintun.dll")).unwrap(), b"dll");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `v0.8.6` 与 `0.8.6` 视为同一版本，避免每次启动都提示更新。
    #[test]
    fn tag_and_version_compare_equal() {
        assert_eq!(normalize_tag("v0.8.6"), "0.8.6");
        assert_eq!(normalize_tag("0.8.6"), "0.8.6");
        assert_eq!(
            parse_version("tun2proxy 0.8.6 (abc 2025-01-01)"),
            Some("0.8.6".to_string())
        );
    }
}
