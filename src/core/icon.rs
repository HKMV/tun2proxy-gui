//! 程序图标：唯一的真源是 `assets/icon.ico`。
//!
//! 图标在两个地方出现，取法必须一致：
//!
//! | 位置 | 取法 |
//! |------|------|
//! | 资源管理器 / 任务栏 / 标题栏 | `build.rs` 把 `app.rc` 里的 `ICON` 资源编进 exe，系统自己从 exe 里取 |
//! | 系统托盘 | [`hicon_tinted`] 直接从内嵌字节造 `HICON`，不读文件、不碰 exe 资源 |
//!
//! 换到 GPUI 之后不再需要「把图标写到数据目录再让窗口按路径加载」那一步：
//! GPUI 的 Windows 后端不设窗口类图标，系统于是回落到 exe 的主图标资源——
//! 那正是 `build.rs` 编进去的这一份。
//!
//! 托盘这一步用 `CreateIconFromResourceEx` 而不是 `ExtractIconExW`：
//! 后者按 exe 路径去查图标，会被资源管理器缓存和 exe 位置影响；前者只认
//! 我们手里这份字节，编译进去是什么就画什么。
//!
//! # 托盘的两档颜色
//!
//! 托盘区域只有 16~32px，靠形状分不出「在跑」和「没跑」，所以托盘图标按服务
//! 状态换色：未运行是灰的（熄灯），运行中保持原色。做法见 [`Tint`]——熄灯那
//! 一档在内存里改像素，运行态直接用内嵌的帧，不额外维护两份图片。

/// 内嵌的 ico 字节（9 个尺寸：16/20/24/32/40/48/64/128/256）。
const ICON: &[u8] = include_bytes!("../../assets/icon.ico");

/// ico 里的一帧。
#[derive(Clone, Copy)]
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

#[cfg(windows)]
pub use windows_icon::hicon_tinted;

/// 托盘图标的两种状态色。
///
/// 图标本身是「白色拱形 + 蓝色圆角方块」。托盘要回答的是「现在到底跑没跑」，
/// 而托盘区域只有 16~32px，靠形状区分不开，所以靠颜色：
///
/// | 档位 | 读作 |
/// |------|------|
/// | [`Tint::Dimmed`] | 熄灯：整体去饱和并压暗 |
/// | [`Tint::Original`] | 在跑：保持原色，和任务栏 / 标题栏里那份一致 |
///
/// 只有「熄灯」那一档需要算，另一档就是原图——不额外维护两份 ico。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tint {
    /// 未运行：整体去饱和并压暗。
    Dimmed,
    /// 运行中：原色，不做任何处理。
    Original,
}

/// 按档位重排一个像素的颜色。
///
/// `Dimmed` 下只保留亮度：有彩色像素（蓝色底）压暗成灰，无彩色像素（白色
/// 拱形、深色描边）压得轻一点——托盘在深色任务栏上是 16px 的小图，图案再暗
/// 就和灰底糊在一起，反而认不出这是哪个程序。
pub fn recolor_pixel(r: u8, g: u8, b: u8, tint: Tint) -> (u8, u8, u8) {
    if tint == Tint::Original {
        return (r, g, b);
    }
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let saturation = (max - min) as f32 / max.max(1) as f32;
    let factor = if saturation < 0.12 { 0.92 } else { 0.55 };
    let v = (max as f32 * factor) as u8;
    (v, v, v)
}

/// 取 ico 里最接近 `size` 的那一帧。
fn pick_frame(size: u32) -> Option<Frame> {
    let frames = frames();
    // 挑「不小于目标」的最小一帧，没有就退到最大一帧：
    // 缩小比放大好看，所以优先往上取。
    frames
        .iter()
        .filter(|frame| frame.size >= size)
        .min_by_key(|frame| frame.size)
        .or_else(|| frames.iter().max_by_key(|frame| frame.size))
        .copied()
}

#[cfg(windows)]
mod windows_icon {
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON};

    use super::{pick_frame, ICON};

    /// 造一个边长为 `size` 的 `HICON`，颜色换成 `tint` 那一档。
    ///
    /// `Dimmed` 需要解码 PNG → 改像素 → 编码回去；`Original` 直接用内嵌的
    /// 那一帧，不解码。都只在状态变化时做一次（调用方缓存句柄），开销可以
    /// 忽略。
    ///
    /// 着色失败（PNG 解不开、编码器报错）时退回未着色的原图：颜色不对是小事，
    /// 托盘图标整个消失是大事。
    ///
    /// 调用方负责在销毁时 `DestroyIcon`。
    pub fn hicon_tinted(size: u32, tint: super::Tint) -> Option<HICON> {
        let frame = pick_frame(size)?;
        let bytes = &ICON[frame.offset..frame.offset + frame.length];
        if tint == super::Tint::Original {
            return create(bytes, size);
        }
        match recolor(bytes, tint) {
            Some(recolored) => create(&recolored, size),
            None => create(bytes, size),
        }
    }

    /// 解码 PNG、按档位重排每个像素、再编码回 PNG。
    pub(super) fn recolor(png_bytes: &[u8], tint: super::Tint) -> Option<Vec<u8>> {
        let mut decoder = png::Decoder::new(png_bytes);
        // EXPAND 把索引色/灰度展开，ALPHA 保证输出一定带 alpha 通道。
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        let rgba = &mut buf[..info.buffer_size()];
        for pixel in rgba.chunks_exact_mut(4) {
            let (r, g, b) = super::recolor_pixel(pixel[0], pixel[1], pixel[2], tint);
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        }

        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, info.width, info.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(rgba).ok()?;
        }
        Some(out)
    }

    /// PNG 字节 → `HICON`。
    fn create(bytes: &[u8], size: u32) -> Option<HICON> {
        // SAFETY: `bytes` 是内嵌静态数据或本函数栈上的 `Vec`，长度取自
        // ico 目录 / PNG 编码器；`dwVer` 是图标的资源版本号（0x00030000
        // 为 Windows 图标），返回的句柄归调用方所有。
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

    #[test]
    fn original_tint_keeps_the_colour() {
        // 运行中用原色：和任务栏、标题栏里那份一致。
        assert_eq!(recolor_pixel(0x25, 0x63, 0xeb, Tint::Original), (0x25, 0x63, 0xeb));
    }

    #[test]
    fn dimmed_tint_flattens_colour_to_grey() {
        let (r, g, b) = recolor_pixel(0x25, 0x63, 0xeb, Tint::Dimmed);
        assert_eq!((r, g, b), (r, r, r), "未运行应当是灰的");
    }

    #[test]
    fn the_white_motif_stays_brighter_than_the_grey_background() {
        // 熄灯态下图案不能和底色糊在一起，否则认不出是哪个程序。
        let (br, ..) = recolor_pixel(0xff, 0xff, 0xff, Tint::Dimmed);
        let (gr, ..) = recolor_pixel(0x25, 0x63, 0xeb, Tint::Dimmed);
        assert!(br > gr + 40, "图案应当明显亮于底色：{br} vs {gr}");
    }

    /// 把两档托盘图标导成 PNG，肉眼看一眼颜色对不对。
    ///
    /// 默认跳过（会写文件），需要时手动跑：
    /// `cargo test dump_tray_previews -- --ignored`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn dump_tray_previews() {
        let frame = pick_frame(256).expect("ico 里应有 256 帧");
        let bytes = &ICON[frame.offset..frame.offset + frame.length];
        for (tint, name) in [(Tint::Dimmed, "dimmed"), (Tint::Original, "original")] {
            let png = windows_icon::recolor(bytes, tint).expect("PNG 应当能解码");
            std::fs::write(format!(".tmp/tray-{name}.png"), png).expect("写预览图");
        }
    }
}
