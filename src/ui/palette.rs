//! 配色方案：把 `themes/<id>.json` 解析成 nsh 的设计 token，并支持运行中切换。
//!
//! nsh 的外观由两层决定：`appearance`（浅色 / 深色，见 [`appearance`]）和
//! **token**（每个语义槽位的具体颜色）。这里两个都能改：每套配色都同时给出
//! light / dark 两份值，所以「赛博青 + 深色」「矩阵绿 + 浅色」这种组合都成立。
//!
//! 色值一律放在 `themes/*.json` 而不是写进 Rust：调色的时候只改 JSON，重编
//! 一次就能看到，也不用在代码里做十六进制运算。

use nsh_base::Theme;
use nsh_core::element::{AdaptiveColor, Tokens};
use nsh_core::platform::Appearance;
use nsh_core::prelude::{set_color_scheme, set_tokens};

use crate::api::theme::source;
use crate::core::config::{Palette, ThemeSetting};

use super::state;

/// 启动时用哪套配色：环境变量 > 配置文件 > 默认。
///
/// `TUN2PROXY_GUI_PALETTE` 是给截图和排障用的（能一次定死配色，不用点界面），
/// 优先级和 `TUN2PROXY_GUI_THEME` 一致：环境变量说了算。
pub fn initial(configured: Palette) -> Palette {
    match std::env::var("TUN2PROXY_GUI_PALETTE") {
        Ok(name) if !name.trim().is_empty() => Palette::from_id(&name),
        _ => configured,
    }
}

/// 启动时的明暗：`TUN2PROXY_GUI_THEME` > 配置。
///
/// 环境变量原本用于截图与排障（能一次定死明暗），名字里带 `light`（含
/// `latte`）就是浅色，其它非空名字一律深色——和 GPUI 时代「认不出来就回落到
/// Mocha」的行为一致。它会把配置里的「跟随系统」压成具体的浅 / 深：界面上的
/// 明暗切换按钮因此也从一个确定的态开始。
pub fn initial_setting(configured: ThemeSetting) -> ThemeSetting {
    match std::env::var("TUN2PROXY_GUI_THEME") {
        Ok(name) if !name.trim().is_empty() => {
            let name = name.trim().to_ascii_lowercase();
            if name.contains("light") || name == "latte" {
                ThemeSetting::Light
            } else {
                ThemeSetting::Dark
            }
        }
        _ => configured,
    }
}

/// 配置里的明暗偏好 → nsh 的外观。
pub fn appearance(setting: ThemeSetting) -> Appearance {
    match setting {
        ThemeSetting::Auto => Appearance::System,
        ThemeSetting::Light => Appearance::Light,
        ThemeSetting::Dark => Appearance::Dark,
    }
}

/// 换明暗：立刻套用外观，并把选择写回配置。
pub fn apply_appearance(setting: ThemeSetting) {
    set_color_scheme(appearance(setting));
    state::remember_theme(setting);
}

/// 取一套配色的设计 token。
///
/// 解析失败不让程序起不来：记一行日志、回落到 nsh 默认主题，用户至少看得见
/// 界面，日志里也说得出为什么。
pub fn tokens(palette: Palette) -> Tokens {
    match Theme::from_json_lenient(source(palette)) {
        Ok((theme, ignored)) => {
            for line in ignored {
                log::warn!("themes/{}.json: {line}", palette.id());
            }
            theme.into_tokens()
        }
        Err(error) => {
            log::error!(
                "themes/{}.json 解析失败，改用默认主题：{error}",
                palette.id()
            );
            Theme::default().into_tokens()
        }
    }
}

/// 换一套配色：立刻换掉全窗 token，并把选择写回配置。
///
/// 顺序是先换 token 再存盘：界面要马上有反应，写文件失败不该拦住切换
/// （下次启动回到配置文件里的旧配色而已）。
pub fn apply(palette: Palette) {
    set_tokens(tokens(palette));
    state::remember_palette(palette);
}

/// 配色选择器上的小色块：各套配色自己的主色（浅色 / 深色各一档）。
///
/// 这里必须写死色值而不是从 `theme()` 取：色块要同时展示**别的**配色长什么
/// 样，取当前主题的主色会让四个方块变成一个颜色。
pub fn swatch(palette: Palette) -> AdaptiveColor {
    match palette {
        Palette::Cyber => AdaptiveColor::rgb(0x0891b2, 0x22d3ee),
        Palette::Matrix => AdaptiveColor::rgb(0x059669, 0x22e06a),
        Palette::Ember => AdaptiveColor::rgb(0xea580c, 0xfb923c),
        Palette::Aurora => AdaptiveColor::rgb(0x7c3aed, 0xa78bfa),
    }
}

