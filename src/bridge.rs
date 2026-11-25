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
