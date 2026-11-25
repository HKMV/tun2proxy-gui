//! nsh 适配器：把 [`crate::api`] 翻译成 TSX 能调的东西。
//!
//! 整个仓库里**只有这一个文件和 `ui/`** 依赖 nsh。业务（`core/`）和门面
//! （`api/`）都不认识它——所以以后要换别的界面框架，删掉这一层换新的适配器
//! 就行，业务一行不动。
//!
//! TSX 侧的用法：
//!
//! ```tsx
//! import { Backend } from "tun2proxy/native";
//! const state = JSON.parse(Backend.snapshot());
//! Backend.start();
//! ```
//!
//! 约定：
//!
//! * 读数据一律返回**一份 JSON 文本**（结构见 [`crate::api::Snapshot`]）；
//! * 动作一律返回 `{"ok":true}` 或 `{"ok":false,"error":"…"}`，错误文案已经是
//!   能直接显示给用户的中文/英文，前端不用再翻译一遍。

use nsh_core::prelude::*;
use serde::Serialize;

use crate::api::{Snapshot, state};

/// 后端接口（JS 侧的名字是 `Backend`）。
#[module]
pub struct Backend;

#[module]
impl Backend {
    /// 界面启动时要的那一份：当前配色 / 明暗 / 语言 / 关窗行为，当前配色的
    /// JSON 原文（前端 `JSON.parse` 后交给 nsh 的 `setTheme()`），和程序版本。
    pub fn boot() -> String {
        state::ensure_installed();
        let config = state::config().unwrap_or_default();
        json(&Boot {
            palette: config.palette.id().to_string(),
            theme: id_of(&config.theme),
            language: id_of(&config.language),
            close_action: id_of(&config.close_action),
            theme_json: crate::api::theme::source(config.palette),
            version: env!("CARGO_PKG_VERSION"),
        })
    }

    /// 某个配色的 JSON 原文（换配色时用：`setTheme(JSON.parse(...))`）。
    ///
    /// 只读、不落盘——要记住用户的选择就接着调 [`Self::set_palette`]。
    pub fn palette_json(palette: String) -> String {
        match parse::<crate::core::config::Palette>(&palette) {
            Ok(palette) => crate::api::theme::source(palette).to_string(),
            Err(error) => {
                log::warn!("取配色失败：{error}");
                "{}".to_string()
            }
        }
    }

    /// 这一刻的全部实时数据。
    pub fn snapshot() -> String {
        state::ensure_installed();
        json(&Snapshot::now().with_version_detail())
    }

    /// 启动隧道。本地校验没过时 `ok` 为 false，`error` 是可以直接显示的原因
    /// （「还没下载 tun2proxy」这种）；「上游不通」要等后台探测，从快照的
    /// `blocked` 里读。
    pub fn start() -> String {
        state::ensure_installed();
        outcome(state::start())
    }

    /// 停止隧道（不等结果，状态从快照里读）。
    pub fn stop() -> String {
        state::ensure_installed();
        state::stop();
        outcome(Ok(()))
    }

    /// 这几样设置都只影响界面，但要落盘——所以走后端，前端只管调。
    pub fn set_close_action(action: String) -> String {
        state::ensure_installed();
        match parse::<crate::core::config::CloseAction>(&action) {
            Ok(action) => {
                state::remember_close_action(action);
                outcome(Ok(()))
            }
            Err(error) => outcome(Err(error)),
        }
    }

    /// 换界面语言（要重建整棵树才生效，前端自己刷新）。
    pub fn set_language(language: String) -> String {
        state::ensure_installed();
        match parse::<crate::core::i18n::LangSetting>(&language) {
            Ok(setting) => {
                let changed = state::remember_language(setting);
                json(&Changed { changed })
            }
            Err(error) => outcome(Err(error)),
        }
    }

    /// 换配色（`cyber` / `aurora` / `ember` / `matrix`）。
    pub fn set_palette(palette: String) -> String {
        state::ensure_installed();
        match parse::<crate::core::config::Palette>(&palette) {
            Ok(palette) => {
                state::remember_palette(palette);
                outcome(Ok(()))
            }
            Err(error) => outcome(Err(error)),
        }
    }

    /// 换明暗（`auto` / `light` / `dark`）。
    pub fn set_theme(theme: String) -> String {
        state::ensure_installed();
        match parse::<crate::core::config::ThemeSetting>(&theme) {
            Ok(setting) => {
                state::remember_theme(setting);
                outcome(Ok(()))
            }
            Err(error) => outcome(Err(error)),
        }
    }

    // —— 代理配置页 ——

    /// 全部配置 + 当前是哪套 + 配置文件路径。
    pub fn profiles() -> String {
        state::ensure_installed();
        json(&crate::api::profiles::ProfilesView::now())
    }

    /// 某套配置的表单草稿（进编辑态时取）。
    pub fn profile_draft(index: usize) -> String {
        state::ensure_installed();
        match crate::api::profiles::draft(index) {
            Some(draft) => json(&draft),
            None => "null".to_string(),
        }
    }

    /// 切到某套配置（启动隧道用的是它）。
    pub fn select_profile(index: usize) -> String {
        state::ensure_installed();
        outcome(if state::select_profile(index) {
            Ok(())
        } else {
            Err("这套配置已经不在了".to_string())
        })
    }

    /// 复制当前这套配置，返回新那套的下标。
    pub fn duplicate_profile() -> String {
        state::ensure_installed();
        json(&Added::of(state::duplicate_profile()))
    }

    /// 新建一套配置，返回它的下标。
    pub fn add_profile() -> String {
        state::ensure_installed();
        json(&Added::of(state::add_profile()))
    }

    /// 删掉一套（只有一套时后端会拒绝）。
    pub fn remove_profile(index: usize) -> String {
        state::ensure_installed();
        outcome(if state::remove_profile(index) {
            Ok(())
        } else {
            Err("删不掉：至少要留一套配置".to_string())
        })
    }

    /// 保存一套配置。`draft` 是表单 JSON（数字字段是字符串），失败时 `error`
    /// 是能直接显示的原因（「MTU 要在 576 到 9000 之间」这种）。
    pub fn save_profile(index: usize, draft: String) -> String {
        state::ensure_installed();
        outcome(crate::api::profiles::save(index, &draft))
    }

    /// 检测一个代理地址能不能用（异步，结论从快照的 `probes` 里读）。
    pub fn check_proxy(url: String) -> String {
        state::ensure_installed();
        state::check_proxy(url.trim());
        outcome(Ok(()))
    }

    /// 表单里那些「一键填默认」的值。
    pub fn defaults() -> String {
        state::ensure_installed();
        json(&Defaults {
            bypass: crate::core::config::DEFAULT_BYPASS,
        })
    }

    // —— 分流规则页 ——

    /// 分流设置：总开关、三组清单、自定义规则、直连出口。
    pub fn split_view() -> String {
        state::ensure_installed();
        json(&crate::api::split::SplitView::now())
    }

    /// 规则明细。`filter` 是查找词（空串表示不过滤），`limit` 截断条数。
    pub fn split_rules(filter: String, limit: usize) -> String {
        state::ensure_installed();
        json(&crate::api::split::rules(&filter, limit))
    }

    /// 开关总闸，返回改后的值。
    pub fn split_toggle_enabled() -> String {
        state::ensure_installed();
        json(&Enabled {
            enabled: crate::api::split::toggle_enabled(),
        })
    }

    /// 开关某一组内置清单（`local` / `chinaIp` / `chinaDomain`）。
    pub fn split_set_group(group: String, enabled: bool) -> String {
        state::ensure_installed();
        json(&Enabled {
            enabled: crate::api::split::set_group(&group, enabled),
        })
    }

    /// 翻转单条规则，返回改后的状态。
    pub fn split_toggle_row(key: String) -> String {
        state::ensure_installed();
        json(&Enabled {
            enabled: crate::api::split::toggle_row(&key),
        })
    }

    /// 保存自定义规则（一行一条）。
    pub fn split_save_custom(custom: String) -> String {
        state::ensure_installed();
        outcome(crate::api::split::save_custom(&custom))
    }

    /// 保存直连出口（DNS + 网卡）。
    pub fn split_save_outlet(direct_dns: String, direct_interface: String) -> String {
        state::ensure_installed();
        outcome(crate::api::split::save_outlet(
            &direct_dns,
            &direct_interface,
        ))
    }

    /// 从 APNIC / v2fly 重新拉清单（异步，进度看快照的 `splitUpdate`）。
    pub fn split_update_lists() -> String {
        state::ensure_installed();
        crate::api::split::update_lists();
        outcome(Ok(()))
    }

    // —— 网络与绕过页 ——

    /// 当前那套配置的网络参数。
    pub fn network_view() -> String {
        state::ensure_installed();
        json(&crate::api::network::NetworkView::now())
    }

    /// 开关 IPv6 / 系统路由 / 卸载模式，返回改后的值。
    pub fn network_set_flag(flag: String, value: bool) -> String {
        state::ensure_installed();
        json(&Value {
            value: crate::api::network::set_flag(&flag, value),
        })
    }

    /// 保存 UDP 转发的三项（连接数与保活间隔是表单里的字符串）。
    pub fn network_save_udpgw(server: String, connections: String, keepalive: String) -> String {
        state::ensure_installed();
        outcome(crate::api::network::save_udpgw(
            &server,
            &connections,
            &keepalive,
        ))
    }

    /// 保存绕过清单。
    pub fn network_save_bypass(bypass: String) -> String {
        state::ensure_installed();
        outcome(crate::api::network::save_bypass(&bypass))
    }

    // —— 程序与日志页 ——

    /// 程序本身的信息：版本、关窗行为、语言、四个路径。
    pub fn program_view() -> String {
        state::ensure_installed();
        json(&crate::api::program::ProgramView::now())
    }

    /// 清空内存里的日志（磁盘文件保留）。
    pub fn clear_logs() -> String {
        state::ensure_installed();
        crate::api::program::clear_logs();
        outcome(Ok(()))
    }

    /// 检查 tun2proxy 有没有新版本（异步，结论看快照的 `update`）。
    pub fn check_update() -> String {
        state::ensure_installed();
        crate::api::program::check_update();
        outcome(Ok(()))
    }

    /// 下载并安装内核（异步，进度看快照的 `update`）。
    pub fn install_update() -> String {
        state::ensure_installed();
        crate::api::program::install_update();
        outcome(Ok(()))
    }

    // —— 运行环境 ——

    /// 权限不够时的一句话，够用时是空串（前端据此决定要不要提示）。
    ///
    /// 返回裸文本而不是 JSON：只有一句话，包一层反而要前端多解一次。
    pub fn privilege_hint() -> String {
        state::ensure_installed();
        crate::api::env::privilege_hint().unwrap_or_default()
    }

    /// 以管理员身份重新拉起自己。
    ///
    /// 成功时新进程已经起来了：前端收到 `ok` 就该退出本进程，否则会有两个窗口。
    pub fn relaunch_elevated() -> String {
        state::ensure_installed();
        if crate::api::env::relaunch_elevated() {
            outcome(Ok(()))
        } else {
            outcome(Err("没能以管理员身份启动：可能是 UAC 里点了「否」，或系统拒绝了这次提权。".to_string()))
        }
    }
}

/// `{"enabled":true}`：开关类动作的返回值。
#[derive(Serialize)]
struct Enabled {
    enabled: bool,
}

/// `{"value":true}`：单个布尔项的返回值。
#[derive(Serialize)]
struct Value {
    value: bool,
}

/// 默认值（[`Backend::defaults`]）。
#[derive(Serialize)]
struct Defaults {
    /// 局域网绕过清单的默认值（每行一个网段）。
    bypass: &'static str,
}

/// 新建 / 复制出来的下标。
#[derive(Serialize)]
struct Added {
    ok: bool,
    index: Option<usize>,
}

impl Added {
    fn of(index: Option<usize>) -> Self {
        Self {
            ok: index.is_some(),
            index,
        }
    }
}

/// `{"changed":true}`：有些调用要告诉前端「值真的变了」。
#[derive(Serialize)]
struct Changed {
    changed: bool,
}

/// 界面启动时要的那一份（[`Backend::boot`]）。
///
/// 键名跟 TSX 侧 `api.ts` 的 `Boot` 一字不差；改字段时两边一起改。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Boot {
    /// 当前配色：`cyber` / `matrix` / `ember` / `aurora`。
    palette: String,
    /// 明暗设置：`auto` / `light` / `dark`。
    theme: String,
    /// 界面语言设置：`auto` / `zh` / `en`。
    language: String,
    /// 点关闭按钮怎么办：`ask` / `minimize` / `quit`。
    close_action: String,
    /// 当前配色的 JSON 原文（`themes/<id>.json`）。
    theme_json: &'static str,
    /// 程序版本，顶栏常驻显示。
    version: &'static str,
}

/// 枚举 → 它在配置文件里的名字（`"dark"` 这种），不用给每个枚举手写一遍。
fn id_of<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 任何能序列化的东西 → JSON 文本。
fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        log::warn!("给界面的 JSON 序列化失败：{error}");
        "{\"error\":\"serialize failed\"}".to_string()
    })
}

/// JSON 字符串字面量 → 枚举值。
///
/// 这些枚举都是配置的一部分，本来就靠 serde 在 JSON 里读写（`"dark"`、
/// `"cyber"`），所以这里走同一条路——不用给每个枚举再手写一遍 `FromStr`，
/// 也就不会出现「界面上能选、反序列化对不上」的两套取值。
fn parse<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|_| format!("未知的取值：{raw}"))
}

/// `Ok` / `Err(文案)` → `{"ok":…}`。
fn outcome(result: Result<(), String>) -> String {
    match result {
        Ok(()) => "{\"ok\":true}".to_string(),
        Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
    }
}
