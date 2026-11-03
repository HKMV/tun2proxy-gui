//! 界面主题：用 gpuikit 的 `Theme` 结构，配 Catppuccin 的色板。
//!
//! 这里**不自己发明组件**。gpuikit 的按钮、输入框、下拉框、对话框、卡片、
//! 徽标全部从 [`Themeable`] 的语义色取色，圆角/内边距/高度由库的
//! `ControlScale` 决定——「用 gpuikit 的风格」就是让它的组件按自己的规则
//! 画，只把颜色换成本应用要的那一套。唯一一处例外是 [`tune`] 末尾把
//! `ControlScale` 的 Large 档抬到 32dp：那是尺寸对不上正文，不是风格问题。
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
//! 明暗由三处决定，优先级从高到低：
//!
//! 1. 环境变量 `TUN2PROXY_GUI_THEME`（排障与截图用，能一次定死明暗）；
//! 2. 设置页里的「界面主题」（跟随系统 / 浅色 / 深色）；
//! 3. 跟随系统时读 Windows 的 `AppsUseLightTheme`。
//!
//! ```text
//! set TUN2PROXY_GUI_THEME=light    # Catppuccin Latte（浅色）
//! set TUN2PROXY_GUI_THEME=frappe   # Catppuccin Frappe
//! set TUN2PROXY_GUI_THEME=gruvbox  # 换回 gpuikit 的 Gruvbox 深色
//! ```
//!
//! 认不出来的名字回落到默认主题，而不是报错——环境变量写错不该让程序起不来。

use std::sync::Arc;

use gpui::{App, FontWeight, Hsla, rems};
use gpuikit::theme::{ControlMetrics, GlobalTheme, Theme, ThemeVariant};

use crate::core::config::ThemeSetting;

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

/// `ControlSize::Large` 档的控件高度（逻辑像素）。
///
/// 输入框、下拉框、按钮、勾选框的外框、行内的图标按钮全部落在这一档；
/// [`tune`] 用它设 `ControlScale`，折线图高度按 3 倍算——同一个数字只写
/// 一处，改档位时不会有人漏改。
pub const CONTROL_HEIGHT_LARGE: f32 = 32.0;

/// 日志区用的等宽字体。
///
/// 只写字体名，不判断平台：找不到时 cosmic-text 会回落到系统默认字体，
/// 布局不会因此错位（只是不再等宽对齐）。
pub const MONO_FAMILY: &str = "Consolas";

/// 标题字重。
pub const WEIGHT_SEMIBOLD: FontWeight = FontWeight(600.0);

/// 安装主题（在 `gpuikit::init` 之后、开窗之前调用）。
///
/// gpuikit 的 `init` 已经装了一套默认主题，这里再装一次是为了按配置和环境
/// 变量选明暗，并应用 [`tune`] 的修正。
pub fn install(cx: &mut App, setting: ThemeSetting) {
    cx.set_global(GlobalTheme(Arc::new(resolve(setting))));
}

/// 运行时换主题：换掉全局主题、同步原生标题栏、让所有窗口重画。
///
/// 标题栏要单独同步一次：gpui 只在**系统**配色变化时调 DWM，用户在这里
/// 手动切成深色时系统还是浅色，标题栏会留在浅色上。
pub fn reinstall(cx: &mut App, setting: ThemeSetting) {
    let theme = resolve(setting);
    let dark = theme.variant == ThemeVariant::Dark;
    cx.set_global(GlobalTheme(Arc::new(theme)));
    crate::core::platform::set_dark_titlebar(dark);
    cx.refresh_windows();
}

/// 按「环境变量 > 设置项 > 系统」选主题。
fn resolve(setting: ThemeSetting) -> Theme {
    // 环境变量最优先：截图脚本、排障都要能把明暗一次定死，
    // 不能受用户当前配置或系统设置影响。
    if let Some(theme) = from_env() {
        return theme;
    }

    let light = match setting {
        ThemeSetting::Light => true,
        ThemeSetting::Dark => false,
        ThemeSetting::Auto => system_prefers_light(),
    };
    let mut theme = if light {
        Theme::catppuccin_latte()
    } else {
        Theme::catppuccin_mocha()
    };
    tune(&mut theme);
    theme
}

/// 环境变量指定的主题；没设或认不出来时返回 `None` 交给设置项决定。
///
/// 认不出来的名字回落到 Catppuccin Mocha 而不是「没设」：用户明确写了
/// 一个名字，就该看到某个主题，而不是悄悄变回「跟随系统」。
fn from_env() -> Option<Theme> {
    let raw = std::env::var("TUN2PROXY_GUI_THEME").ok()?;
    let name = raw.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let mut theme = match name.as_str() {
        "light" | "latte" | "catppuccin-latte" | "catppuccin_latte" => Theme::catppuccin_latte(),
        "frappe" | "catppuccin-frappe" | "catppuccin_frappe" => Theme::catppuccin_frappe(),
        "macchiato" | "catppuccin-macchiato" | "catppuccin_macchiato" => {
            Theme::catppuccin_macchiato()
        }
        "gruvbox" | "gruvbox-dark" | "gruvbox_dark" => Theme::gruvbox_dark(),
        "gruvbox-light" | "gruvbox_light" => Theme::gruvbox_light(),
        _ => Theme::catppuccin_mocha(),
    };
    tune(&mut theme);
    Some(theme)
}

/// 系统当前是不是浅色外观。
///
/// Windows 把这个偏好放在用户注册表的 `AppsUseLightTheme`（DWORD，1 = 浅色）。
/// 读不到就当深色：这是本应用的默认外观，也是 Windows 默认值。
///
/// 其它平台一律返回深色。真正的「跟随系统」需要各平台的 API（macOS 的
/// `NSAppearance`、Linux 的桌面环境设置），本项目目前只发布 Windows 版，
/// 与其写半套不如明说没有。
#[cfg(windows)]
fn system_prefers_light() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

    /// Rust 字符串 → 以 NUL 结尾的 UTF-16，注册表 API 要的格式。
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: 两个字符串都以 NUL 结尾且活到调用结束；`data` 与 `size`
    // 描述的缓冲区大小一致，正是 `RegGetValueW` 要求的「DWORD 输出」。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    status == 0 && data == 1
}

#[cfg(not(windows))]
fn system_prefers_light() -> bool {
    false
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
                // surface1 而不是 surface0：1dp 的 #ccd0da 压在白卡上只有
                // 约 1.3:1，勾选框那圈描边、行分隔线和卡片轮廓全都糊成一片。
                // 深一档之后三样东西同时立起来，卡片也不至于显得厚重。
                0xbc_c0cc,
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

    // 控件档位：gpuikit 的 Large 是 24dp 高 / 13px 字，而本应用的正文是
    // 14px——按钮和输入框里的字比旁边那行说明还小半号，24dp 的框装 14px 的
    // 字也只剩上下各 3dp 的余量，看着发挤。抬到 32dp / 14px（Windows 11
    // 设置页的尺寸）：按钮、输入框、下拉框、下拉弹层的行全部落在同一档上，
    // 弹层行高也跟着从 24 变 32，点起来不再是一条细缝。
    //
    // 只动 Large 一档：Small（16dp）留给徽标，Medium（20dp）留给侧栏的图标
    // 按钮，它们本来就不该和输入框一样高。`ink` 保持 16dp，勾选框和开关的
    // 图形尺寸不跟着变。
    theme.controls.large = ControlMetrics {
        height: rems(CONTROL_HEIGHT_LARGE / 16.0),
        padding_x: rems(0.75),
        gap: rems(0.375),
        radius: rems(0.375),
        text_size: rems(0.875),
        line_height: rems(1.25),
        ink: rems(1.0),
    };
}

/// `0xRRGGBB` → `Hsla`。
fn hex(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}
