//! 界面层要用的内核句柄：配置与引擎。
//!
//! nsh 的根组件是「跑一次、绑定按信号重跑」的模型，没有 GPUI 那种
//! `Entity<App>` 可以随手塞字段；启动流程又在窗口之前（提权、单实例都在
//! 那里），所以配置与引擎由 [`install`] 在 `main` 里装进来，界面从这里取。
//!
//! 配置是**可变**的：配色和明暗都能在界面上换，换完要写回配置文件。因此这里
//! 存的是 `Mutex<Config>`，改一项就改在同一份上——各写各的副本会把别的字段
//! 悄悄覆盖回旧值（换配色时把刚选的明暗写回默认）。
//! `Engine` 内部自己用 `Arc` + 锁维护状态，这里不用再包一层。

use std::sync::{Mutex, OnceLock};

use crate::core::config::{Palette, ThemeSetting};
use crate::core::{Config, Engine};

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
static ENGINE: OnceLock<Engine> = OnceLock::new();

/// 记下本次运行要用的配置与引擎（只在 `main` 里调一次）。
pub fn install(config: Config, engine: Engine) {
    let _ = CONFIG.set(Mutex::new(config));
    let _ = ENGINE.set(engine);
}

/// 本次运行配置的一份副本；界面在 `install` 之前渲染时返回 `None`。
///
/// 返回副本而不是引用：调用方多半只取一两个字段（配色、明暗），拿着副本既
/// 不会把锁带到界面渲染里，也不用关心别处正在改哪一项。
pub fn config() -> Option<Config> {
    CONFIG.get().map(|config| lock(config).clone())
}

/// 本次运行的引擎；界面在 `install` 之前渲染时返回 `None`。
pub fn engine() -> Option<&'static Engine> {
    ENGINE.get()
}

/// 记住用户这次选的配色，并写回配置文件。
pub fn remember_palette(palette: Palette) {
    let Some(config) = CONFIG.get() else {
        return;
    };
    let mut config = lock(config);
    if config.palette == palette {
        return;
    }
    config.palette = palette;
    if let Err(error) = config.save() {
        log::warn!("保存配色失败：{error}");
    }
}

/// 记住用户这次选的明暗，并写回配置文件。
pub fn remember_theme(setting: ThemeSetting) {
    let Some(config) = CONFIG.get() else {
        return;
    };
    let mut config = lock(config);
    if config.theme == setting {
        return;
    }
    config.theme = setting;
    if let Err(error) = config.save() {
        log::warn!("保存明暗设置失败：{error}");
    }
}

/// 取锁；上一次持锁的线程 panic 了也照常用里面的值（配置不该因此失效）。
fn lock(config: &Mutex<Config>) -> std::sync::MutexGuard<'_, Config> {
    config.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
