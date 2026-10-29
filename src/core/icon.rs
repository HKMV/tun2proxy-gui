//! 程序图标：唯一的真源是 `assets/icon.ico`。
//!
//! 图标在四个地方出现，之前各自用不同的取法，于是同一台机器上能同时看到
//! 三个版本（exe 资源里是新的、标题栏读数据目录里的旧副本、托盘按 exe 路径
//! 反查又拿到系统缓存的那一份）。现在全部走这里：
//!
//! | 位置 | 取法 |
//! |------|------|
//! | 资源管理器 / 任务栏 | `build.rs` 把 `app.rc` 里的 `ICON` 资源编进 exe |
//! | 标题栏（zsui） | [`install_to_app_dir`] 把内嵌字节落到数据目录，交给 `icon_path` |
//! | 系统托盘 | [`hicon`] 直接从内嵌字节造 `HICON`，不读文件、不碰 exe 资源 |
//!
//! 托盘这一步用 `CreateIconFromResourceEx` 而不是 `ExtractIconExW`：
//! 后者按 exe 路径去查图标，会被资源管理器缓存和 exe 位置影响；前者只认
//! 我们手里这份字节，编译进去是什么就画什么。

use std::path::PathBuf;

/// 内嵌的 ico 字节（9 个尺寸：16/20/24/32/40/48/64/128/256）。
const ICON: &[u8] = include_bytes!("../../assets/icon.ico");

/// ico 里的一帧。
struct Frame {
    /// 边长（ico 目录项里 0 表示 256）。
    size: u32,
    offset: usize,
    length: usize,
}

/// 读出 ico 目录。
///
/// 只认 PNG / BMP 两种帧，两种 Windows 都能吃；目录项里出现别的编码时
/// 直接跳过，不影响其它帧。
fn frames() -> Vec<Frame> {
    let mut out = Vec::new();
    if ICON.len() < 6 || &ICON[0..4] != b"\x00\x00\x01\x00" {
        return out;
    }
    let count = u16::from_le_bytes([ICON[4], ICON[5]]) as usize;
    for index in 0..count {
        let base = 6 + index * 16;
        let Some(entry) = ICON.get(base..base + 16) else {
            break;
        };
        let width = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let length = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if offset + length > ICON.len() || length == 0 {
            continue;
        }
        out.push(Frame {
            size: width,
            offset,
            length,
        });
    }
    out
}

/// 把内嵌图标写到数据目录，返回路径。
///
/// 每次都跟磁盘上的比字节：换了图标之后旧副本必须被覆盖，否则标题栏会一直
/// 显示上一版的图（`zsui` 只认路径，不会去猜文件变没变）。
pub fn install_to_app_dir() -> Option<PathBuf> {
    let path = crate::core::paths::app_dir().join("icon.ico");
    if std::fs::read(&path).is_ok_and(|existing| existing == ICON) {
        return Some(path);
    }
    std::fs::create_dir_all(crate::core::paths::app_dir()).ok()?;
    std::fs::write(&path, ICON).ok()?;
    Some(path)
}

#[cfg(windows)]
pub use windows_icon::hicon;

#[cfg(windows)]
mod windows_icon {
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON};

    /// 造一个边长为 `size` 的 `HICON`（取 ico 里最接近的一帧）。
    ///
    /// 调用方负责在销毁时 `DestroyIcon`。
    pub fn hicon(size: u32) -> Option<HICON> {
        let frames = super::frames();
        // 挑「不小于目标」的最小一帧，没有就退到最大一帧：
        // 缩小比放大好看，所以优先往上取。
        let frame = frames
            .iter()
            .filter(|frame| frame.size >= size)
            .min_by_key(|frame| frame.size)
            .or_else(|| frames.iter().max_by_key(|frame| frame.size))?;
        let bytes = &super::ICON[frame.offset..frame.offset + frame.length];
        // SAFETY: `bytes` 指向内嵌的静态数据，长度取自 ico 目录且已校验在界内；
        // `dwVer` 是图标的资源版本号（0x00030000 为 Windows 图标），
        // 返回的句柄归调用方所有。
        let icon = unsafe {
            CreateIconFromResourceEx(
                bytes.as_ptr(),
                bytes.len() as u32,
                BOOL::from(true),
                0x0003_0000,
                size as i32,
                size as i32,
                0,
            )
        };
        if icon.is_null() {
            None
        } else {
            Some(icon)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_stays_inside_the_embedded_bytes() {
        let frames = frames();
        assert!(frames.len() >= 5, "内嵌图标应有多个尺寸");
        for frame in &frames {
            assert!(frame.offset + frame.length <= ICON.len());
            assert!(matches!(
                frame.size,
                16 | 20 | 24 | 32 | 40 | 48 | 64 | 128 | 256
            ));
        }
    }
}
