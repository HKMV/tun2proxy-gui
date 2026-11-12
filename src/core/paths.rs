//! 程序的数据目录约定。
//!
//! 所有可写数据都放在系统标准的用户配置目录下，方便打包 / 卸载：
//!
//! ```text
//! Windows  %APPDATA%\tun2proxy-gui\
//! macOS    ~/Library/Application Support/tun2proxy-gui/
//! Linux    ~/.config/tun2proxy-gui/   （遵循 $XDG_CONFIG_HOME）
//! ```
//!
//! 目录内包含：`config.json`（用户配置）、`bin/`（下载下来的 tun2proxy 及其
//! 依赖的 wintun.dll 等）、`logs/`（按天一份运行日志）、
//! `tun2proxy-download.zip`（临时下载文件）。
//! 设置环境变量 `TUN2PROXY_GUI_HOME` 可以把整个目录搬到别处（便携版）。

use std::path::PathBuf;

/// GUI 的数据根目录。
pub fn app_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TUN2PROXY_GUI_HOME") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Some(base) = std::env::var_os("APPDATA") {
            return PathBuf::from(base).join("tun2proxy-gui");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("tun2proxy-gui");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(base).join("tun2proxy-gui");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("tun2proxy-gui");
        }
    }
    std::env::temp_dir().join("tun2proxy-gui")
}

/// 配置文件 `config.json` 的路径。
pub fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

/// tun2proxy 二进制的存放目录（解压后的整个 Release 包都在这里）。
pub fn bin_dir() -> PathBuf {
    app_dir().join("bin")
}

/// 运行日志的存放目录：`tun2proxy-gui.YYYY-MM-DD.log`（按天滚动）。
///
/// 落盘逻辑在 [`crate::core::logs`]，这里只负责路径——
/// `TUN2PROXY_GUI_HOME` 搬目录时日志跟着一起搬。
pub fn logs_dir() -> PathBuf {
    app_dir().join("logs")
}
