//! 界面语言（i18n）：中文 / 英文。
//!
//! 只有两种语言，所以不做「key → 翻译表」的间接层：调用点直接写
//! `t("启动", "Start")`，两种文案并排放在一起。改文案时不用再翻另一个文件，
//! 也不会出现「key 拼错导致界面空白」这种只能靠眼睛发现的错。
//!
//! 语言是**进程级**的：一个窗口只有一种语言，而视图每次状态变化都整棵重建，
//! 所以用全局原子量，而不是把它塞进每个函数的参数列表——那会让几十个纯函数
//! 都多一个参数，收益为零。

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// 配置里存的语言偏好：跟随系统 / 强制中文 / 强制英文。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LangSetting {
    #[default]
    Auto,
    Zh,
    En,
}

/// 实际使用的语言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl LangSetting {
    pub const ALL: [LangSetting; 3] = [Self::Auto, Self::Zh, Self::En];

    /// 下拉框里的文字。语言名用各自的语言写（中文写「中文」，英文写
    /// 「English」），这样认不出当前界面语言的人也能找到自己那一项。
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => t("跟随系统", "System"),
            Self::Zh => "中文",
            Self::En => "English",
        }
    }

    /// 解析成实际语言。
    pub fn resolve(self) -> Lang {
        match self {
            Self::Auto => detect(),
            Self::Zh => Lang::Zh,
            Self::En => Lang::En,
        }
    }
}

impl Lang {
    pub fn pick<'a>(self, zh: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Zh => zh,
            Self::En => en,
        }
    }
}

/// 当前语言。`0` = 中文，`1` = 英文。
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// 当前界面语言。
pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::Zh,
    }
}

/// 切换全局语言。
pub fn set_lang(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Zh => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

/// 按配置里的偏好切换语言，返回解析结果。
pub fn apply(setting: LangSetting) -> Lang {
    let lang = setting.resolve();
    set_lang(lang);
    lang
}

/// 取当前语言的文案：`t("中文", "English")`。
///
/// 界面上每一处用户可见的文字都要走它——包括按钮、说明、弹窗、以及会显示在
/// 日志面板里的程序自身消息（内核自己打印的日志是 tun2proxy 的输出，不翻译）。
pub fn t<'a>(zh: &'a str, en: &'a str) -> &'a str {
    lang().pick(zh, en)
}

/// 带占位符的文案，两种语言各写一份 `format!`。
///
/// `format!` 只认字面量，所以「翻译过的模板 + 运行时参数」不能直接交给它；
/// 又不想为了 i18n 把几十处 `format!` 拆成「先拼再填」。这个宏让两种语言的
/// 模板都是**字面量**（拼写错误编译期就报），参数只求值一次（只走一个分支）。
///
/// ```ignore
/// tr_format!("启动：{} {}", "Starting: {} {}", exe, args)
/// ```
#[macro_export]
macro_rules! tr_format {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::core::i18n::lang() {
            $crate::core::i18n::Lang::Zh => format!($zh $(, $arg)*),
            $crate::core::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}

/// 带占位符的文案。
///
/// `format!` 只认字符串字面量，所以「翻译过的模板 + 运行时参数」不能直接
/// 交给它。这里按 `{key}` 做纯文本替换：模板里没出现的 key 会被忽略，
/// 参数里的花括号也不会被当成格式化指令。
///
/// 位置占位符 `{}` 用空 key 表示：`tf(t("配置 · {} 套", "Profiles · {}"), &[("", &n)])`。
pub fn tf(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in pairs {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// 按系统语言猜一个默认值。
///
/// Windows 上问系统 UI 语言（`GetUserDefaultUILanguage` 的主语言 id，
/// `0x04` 是中文）；其它平台看 `LC_ALL` / `LC_MESSAGES` / `LANG`。
/// 认不出来就当英文——中文用户至少有「跟随系统」之外的选项可选。
fn detect() -> Lang {
    #[cfg(windows)]
    {
        // 主语言 id 在低 10 位；0x04 = LANG_CHINESE。
        const LANG_CHINESE: u16 = 0x04;
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        if (id & 0x3ff) == LANG_CHINESE {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    #[cfg(not(windows))]
    {
        let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty());
        match value {
            Some(value) if value.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
            _ => Lang::En,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_settings_win_over_the_system_locale() {
        assert_eq!(LangSetting::Zh.resolve(), Lang::Zh);
        assert_eq!(LangSetting::En.resolve(), Lang::En);
    }

    #[test]
    fn pick_returns_the_selected_language() {
        assert_eq!(Lang::Zh.pick("启动", "Start"), "启动");
        assert_eq!(Lang::En.pick("启动", "Start"), "Start");
    }

    #[test]
    fn global_language_round_trips() {
        // 这个测试会动全局状态，所以自己还原，免得影响别的测试。
        let before = lang();
        set_lang(Lang::En);
        assert_eq!(t("启动", "Start"), "Start");
        set_lang(Lang::Zh);
        assert_eq!(t("启动", "Start"), "启动");
        set_lang(before);
    }
}
