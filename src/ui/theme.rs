//! 界面主题：用 gpuikit 的 `Theme` 结构，配 Catppuccin 的色板。
//!
//! 这里**不自己发明组件**。gpuikit 的按钮、输入框、下拉框、对话框、卡片、
//! 徽标全部从 [`Themeable`] 的语义色取色，圆角/内边距/高度由库的
//! `ControlScale` 决定，我们一个都不覆盖——「用 gpuikit 的风格」就是让它
//! 的组件按自己的规则画，只把颜色换成本应用要的那一套。
//!
//! # 为什么不是 gpuikit 的默认主题
//!
//! gpuikit 的默认是 Gruvbox Dark：暖褐底（`#282828` / `#3c3836`）+ 亮绿
//! accent（`#8ec07c`）。那套色板适合终端和编辑器，用在满屏中文小字的设置
//! 界面上有两个问题：暖褐底色让文字发黄显脏，而绿色同时占了侧栏选中、主
//! 按钮填充和图表三条线，主按钮一大块亮绿尤其抢眼。
//!
//! 换成 Catppuccin：中性冷灰底，层次靠亮度差而不是色相差，中文小字干净；
//! accent 用官方的 blue 而不是招牌的 mauve——蓝在工具软件里更稳重，也不会
//! 和「运行中」的绿、图表的绿撞色。
//!
//! # 换肤
//!
//! ```text
//! set TUN2PROXY_GUI_THEME=light    # Catppuccin Latte（浅色）
//! set TUN2PROXY_GUI_THEME=frappe   # Catppuccin Frappe
//! set TUN2PROXY_GUI_THEME=gruvbox  # 换回 gpuikit 的 Gruvbox 深色
//! ```
//!
//! 认不出来的名字回落到默认主题，而不是报错——环境变量写错不该让程序起不来。

use std::sync::Arc;

use gpui::{App, FontWeight, Hsla};
use gpuikit::theme::{GlobalTheme, Theme, ThemeVariant};

/// 正文字号（逻辑像素）。
///
/// 下面这组数字对齐 gpuikit 的类型尺度：卡片标题 `text_sm`(14)、说明文字
/// `text_xs`(12)、标题按 minor third 递进。自己画的文字跟组件放在一起时，
/// 用同一套档位才不会一大一小。
pub const TEXT_BODY: f32 = 14.0;
/// 说明文字：gpuikit 卡片描述用的就是 12。
pub const TEXT_SMALL: f32 = 12.0;
/// 分组标题。
pub const TEXT_GROUP: f32 = 14.0;
/// 页面标题。
pub const TEXT_TITLE: f32 = 20.0;
/// 首屏大标题。
pub const TEXT_HERO: f32 = 24.0;
/// 等宽日志。
pub const TEXT_MONO: f32 = 12.5;

/// 日志区用的等宽字体。
///
/// 只写字体名，不判断平台：找不到时 cosmic-text 会回落到系统默认字体，
/// 布局不会因此错位（只是不再等宽对齐）。
pub const MONO_FAMILY: &str = "Consolas";

/// 标题字重。
pub const WEIGHT_SEMIBOLD: FontWeight = FontWeight(600.0);

/// 安装主题（在 `gpuikit::init` 之后、开窗之前调用）。
///
/// gpuikit 的 `init` 已经装了一套默认主题，这里再装一次是为了按环境变量
/// 换肤并应用 [`tune`] 的修正；不设环境变量时得到的就是 Catppuccin Mocha。
pub fn install(cx: &mut App) {
    cx.set_global(GlobalTheme(Arc::new(resolve())));
}

/// 按 `TUN2PROXY_GUI_THEME` 选主题，认不出来就用默认的 Catppuccin Mocha。
fn resolve() -> Theme {
    let mut theme = match std::env::var("TUN2PROXY_GUI_THEME")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "light" | "latte" | "catppuccin-latte" | "catppuccin_latte" => {
            Theme::catppuccin_latte()
        }
        "frappe" | "catppuccin-frappe" | "catppuccin_frappe" => Theme::catppuccin_frappe(),
        "macchiato" | "catppuccin-macchiato" | "catppuccin_macchiato" => {
            Theme::catppuccin_macchiato()
        }
        "gruvbox" | "gruvbox-dark" | "gruvbox_dark" => Theme::gruvbox_dark(),
        "gruvbox-light" | "gruvbox_light" => Theme::gruvbox_light(),
        _ => Theme::catppuccin_mocha(),
    };
    tune(&mut theme);
    theme
}

/// 修正 Catppuccin 在「卡片式布局」下的层次，并把 accent 换成蓝色。
///
/// gpuikit 把 Catppuccin 的 `surface` 映射成 **mantle**（比 base 更暗的那一
/// 档）。那是给「凹陷区域」用的语义，而本应用的卡片必须浮在页面之上——
/// 照原样画，卡片会沉进背景里，整页看起来像一块布。
///
/// 所以按 Catppuccin 官方色板重排成三级亮度：背景 base < 卡片 surface0 <
/// 按钮/悬停 surface1。浅色下同理，只是往白的一端推：base < 纯白卡片 <
/// mantle 按钮底。
///
/// 只认名字里带 Catppuccin 的主题；Gruvbox 等其它主题原样放过，因为它们的
/// `surface` 本来就是「比背景亮」的语义。
fn tune(theme: &mut Theme) {
    if theme.name.contains("Catppuccin") {
        let dark = theme.variant == ThemeVariant::Dark;
        let (surface, secondary, tertiary, border, subtle, btn, btn_hover, btn_active) = if dark {
            (
                0x31_3244, // surface0：卡片
                0x45_475a, // surface1：输入框 / 悬停
                0x58_5b70, // surface2：按下
                0x45_475a, // 描边
                0x31_3244, // 卡片内的分隔线（与卡片同色，靠间距分隔）
                0x45_475a, // 次要按钮底
                0x58_5b70,
                0x6c_7086, // overlay0
            )
        } else {
            (
                0xff_ffff, // 纯白卡片
                0xe6_e9ef, // mantle
                0xdc_e0e8, // crust
                0xcc_d0da, // surface0
                0xdc_e0e8,
                0xe6_e9ef,
                0xdc_e0e8,
                0xcc_d0da,
            )
        };
        theme.surface_color = hex(surface);
        theme.surface_secondary_color = Some(hex(secondary));
        theme.surface_tertiary_color = Some(hex(tertiary));
        theme.border_color = hex(border);
        theme.border_subtle_color = Some(hex(subtle));
        theme.border_secondary_color = Some(hex(tertiary));
        theme.button_bg_color = Some(hex(btn));
        theme.button_bg_hover_color = Some(hex(btn_hover));
        theme.button_bg_active_color = Some(hex(btn_active));
        // 蓝比 mauve 稳重，也不会和状态色里的绿（运行中）撞车。
        let accent = hex(if dark { 0x89_b4fa } else { 0x1e_66f5 });
        theme.accent_color = accent;

        // 输入框：gpuikit 默认让输入框底 = `surface()`，也就是和卡片同色，
        // 只靠一圈 1dp 描边区分。放在满屏卡片的设置页里，那个方框几乎看不
        // 见——用户不知道哪里能点。这里改成「比卡片暗一档 + 更亮的描边」，
        // 输入框凹进卡片，边界立刻成立，也是 Catppuccin 官方表单的做法。
        let (input_bg, input_border, input_hover) = if dark {
            (0x18_1825, 0x58_5b70, 0x6c_7086) // mantle / surface2 / overlay0
        } else {
            (0xe6_e9ef, 0xbc_c0cc, 0xac_b0_be) // mantle / surface1 / surface2
        };
        theme.input_bg_color = Some(hex(input_bg));
        theme.input_border_color = Some(hex(input_border));
        theme.input_border_hover_color = Some(hex(input_hover));
        theme.input_border_focused_color = Some(accent);
    }

    // 描边提亮：深色下 gpuikit 的 `border()` 只比 `surface()` 亮一点点，
    // 用在 16dp 的勾选框、输入框上会淡到看不见。浅色主题的描边本来就有
    // 对比，不碰。
    if theme.variant == ThemeVariant::Dark {
        let mut border = theme.border_color;
        border.l = (border.l + 0.04).min(1.0);
        theme.border_color = border;
    }
}

/// `0xRRGGBB` → `Hsla`。
fn hex(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}
