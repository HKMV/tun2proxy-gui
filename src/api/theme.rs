//! 配色源文件。
//!
//! 四套配色就是仓库里的 `themes/*.json`（nsh 的 `Theme` 结构：
//! `{"cssVars":{"theme":…,"light":…,"dark":…}}`）。这里只做「名字 → 文件内容」
//! 的映射，**不解析**：Rust 界面把 JSON 交给 nsh 的 `Theme::from_json_lenient`，
//! TSX 界面自己 `JSON.parse` 后调 `setTheme()`。两边吃的是同一份文件，改颜色
//! 只改 JSON。
//!
//! 编译期 `include_str!`：发布版不用带 `themes/` 目录。

use crate::core::config::Palette;

/// 一套配色对应的 JSON 原文。
pub fn source(palette: Palette) -> &'static str {
    match palette {
        Palette::Cyber => include_str!("../../themes/cyber.json"),
        Palette::Matrix => include_str!("../../themes/matrix.json"),
        Palette::Ember => include_str!("../../themes/ember.json"),
        Palette::Aurora => include_str!("../../themes/aurora.json"),
    }
}
