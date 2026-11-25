//! 「程序与日志」页要的数据与动作。
//!
//! 这一页是「程序本身」而不是「隧道」：内核装没装、要不要更新、关窗怎么办、
//! 配置文件与日志落在哪个目录。运行中的日志行与内核版本不在这里——它们每秒
//! 都在变，属于 [`super::Snapshot`]。
//!
//! # 关窗与语言为什么也走这里
//!
//! 两者都是**要落盘的界面设置**。界面自己记住没有意义（下次启动就没了），
//! 所以存盘这件事放后端，前端只管调。

use serde::Serialize;

use crate::core::config::CloseAction;
use crate::core::i18n::{Lang, LangSetting};
use crate::core::paths;

use super::state;

/// 一个「三选一」的选项（关窗行为、界面语言）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    /// 稳定的字符串 id（`ask` / `minimize` / `quit`、`auto` / `zh` / `en`）。
    pub id: &'static str,
    pub label: &'static str,
}

/// 这一页除日志以外的全部数据。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramView {
    /// 本程序版本（顶栏那个 `v0.6.5` 也来自它）。
    pub version: &'static str,
    pub close_action: &'static str,
    pub close_options: Vec<OptionView>,
    pub language: &'static str,
    pub language_options: Vec<OptionView>,
    /// 路径：配置、数据、内核二进制、日志。
    pub config_path: String,
    pub data_dir: String,
    pub bin_dir: String,
    pub logs_dir: String,
}

impl ProgramView {
    /// 读一次配置。
    pub fn now() -> Self {
        let config = state::config().unwrap_or_default();
        Self {
            version: env!("CARGO_PKG_VERSION"),
            close_action: close_id(config.close_action),
            close_options: CloseAction::ALL
                .iter()
                .map(|action| OptionView {
                    id: close_id(*action),
                    label: action.label(),
                })
                .collect(),
            language: lang_id(config.language),
            language_options: LangSetting::ALL
                .iter()
                .map(|setting| OptionView {
                    id: lang_id(*setting),
                    label: setting.label(),
                })
                .collect(),
            config_path: paths::config_path().display().to_string(),
            data_dir: paths::app_dir().display().to_string(),
            bin_dir: paths::bin_dir().display().to_string(),
            logs_dir: paths::logs_dir().display().to_string(),
        }
    }
}

fn close_id(action: CloseAction) -> &'static str {
    match action {
        CloseAction::Ask => "ask",
        CloseAction::Minimize => "minimize",
        CloseAction::Quit => "quit",
    }
}

fn lang_id(setting: LangSetting) -> &'static str {
    match setting {
        LangSetting::Auto => "auto",
        LangSetting::Zh => "zh",
        LangSetting::En => "en",
    }
}

/// 改关窗行为，返回改后的 id（认不出来就原样返回旧值）。
pub fn set_close_action(id: &str) -> &'static str {
    let action = match id {
        "ask" => CloseAction::Ask,
        "minimize" => CloseAction::Minimize,
        "quit" => CloseAction::Quit,
        _ => return close_id(state::close_action()),
    };
    state::remember_close_action(action);
    close_id(action)
}

/// 当前**实际**在用的界面语言（`auto` 已经解析成具体语言）。
///
/// 界面骨架的文案在 TSX 那边，得知道该写中文还是英文；`ProgramView.language`
/// 是设置项（可能是 `auto`），拿它判断会把「跟随系统」当成中文。
pub fn resolved_language() -> &'static str {
    match crate::core::i18n::lang() {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

/// 改界面语言，返回 `(改后的 id, 要不要重建界面)`。
///
/// 「重建」不是因为语言是构建期取的——文案全是运行期给的，改完下一帧就是新语言。
/// 留这个返回值是给界面一个「要不要重置滚动位置」的信号（TSX 那边换语言会回顶：
/// 断行全变了，停在原偏移上会卡在半行字上）。
pub fn set_language(id: &str) -> (&'static str, bool) {
    let setting = match id {
        "auto" => LangSetting::Auto,
        "zh" => LangSetting::Zh,
        "en" => LangSetting::En,
        _ => return (lang_id(state::config().map(|c| c.language).unwrap_or_default()), false),
    };
    let changed = state::remember_language(setting);
    (lang_id(setting), changed)
}

/// 清空内存里的日志（磁盘上的日志文件保留）。
pub fn clear_logs() {
    state::clear_logs();
}

/// 检查 tun2proxy 有没有新版本（异步，结论从快照的 `update` 读）。
pub fn check_update() {
    state::check_update();
}

/// 下载并安装（重新安装）tun2proxy（异步，进度从快照的 `update` 读）。
pub fn install_update() {
    state::install_update();
}
