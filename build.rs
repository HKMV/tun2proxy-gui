//! 构建脚本：把图标与版本信息编进 Windows 可执行文件的资源段。
//!
//! 为什么需要它：`#![windows_subsystem = "windows"]` 只是「不建控制台」，
//! 资源管理器 / 任务栏显示的图标来自 exe 里的 `ICON` 资源，而不是运行时的
//! 任何代码。没有这一步，程序在任务栏和资源管理器里就是系统默认的空白图标。
//!
//! 运行时那一路（窗口标题栏）由 `src/core/paths.rs::install_icon` +
//! `native_window(..).icon_path(..)` 负责：把内嵌的 `assets/icon.ico`
//! 落到数据目录，再把绝对路径交给 zsui。两条路互不依赖，各自在不同平台生效。

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(windows)]
    {
        // 第二个参数是「要不要顺带嵌一个 manifest」：这里不嵌（DPI 由
        // `platform::init` 在运行时声明）。返回的是编译结果，失败会在里面 panic，
        // 这里显式忽略以避开 `must_use` 警告。
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    }
}
