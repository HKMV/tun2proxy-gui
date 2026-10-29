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
//! 依赖的 wintun.dll 等）、`tun2proxy-download.zip`（临时下载文件）。
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

/// 把内嵌的图标落到数据目录，返回它的绝对路径。
///
/// 窗口图标是按**文件路径**加载的（zsui 的 `icon_path`），而图标已经编进
/// 二进制了；每次启动比对字节写一遍（二十来 KB）比「要求用户把 assets/ 目录
/// 跟 exe 放在一起」或者「依赖工作目录」都可靠——从快捷方式、命令行、
/// 资源管理器双击启动，工作目录可以完全不同。
///
/// 写不进去（比如目录只读）就返回 `None`：没有图标不影响程序能用，
/// 窗口会用系统默认图标，不值得为此启动失败。
///
/// 实现搬到 [`crate::core::icon`]：图标只有一个真源，三处取法必须一致。
pub fn install_icon() -> Option<PathBuf> {
    crate::core::icon::install_to_app_dir()
}
