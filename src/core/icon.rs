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
//! 状态换色：未运行是灰的，运行中是绿的。做法见 [`Tint`]——在内存里改像素，
//! 不额外维护两份图片。

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

/// 托盘图标的两档配色。
///
/// 图标本身是「白色拱形 + 蓝色圆角方块」。托盘要回答的是「现在到底跑没跑」，
/// 而托盘区域只有 16~32px，靠形状区分不开，所以靠颜色：
///
/// | 档位 | 蓝色底 | 白色图案 | 读作 |
/// |------|--------|----------|------|
/// | [`Tint::Idle`] | 灰、压暗 | 灰 | 熄灯 |
/// | [`Tint::Running`] | 绿 | 白 | 在跑 |
///
/// 不需要维护两份 ico：颜色在运行时从内嵌的那一份算出来。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tint {
    /// 未运行：整体去饱和并压暗。
    Idle,
    /// 运行中：底色换成绿色。
    Running,
}

/// 按档位重排一个像素的颜色。
///
/// 只改**有彩色**的像素（蓝色底）；无彩色的像素（白色图案、深色描边）在
/// `Running` 下原样保留——否则整个图标会被染色，图案和底色糊成一片。
/// `Idle` 下连图案一起压暗，整体读作「熄灯」。
pub fn recolor_pixel(r: u8, g: u8, b: u8, tint: Tint) -> (u8, u8, u8) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let saturation = (max - min) as f32 / max.max(1) as f32;

    // 无彩色：白色拱形、抗锯齿边缘、深色描边都走这一支。
    if saturation < 0.12 {
        return match tint {
            Tint::Idle => {
                // 只压一点点：托盘在深色任务栏上是 16px 的小图，图案再暗就
                // 和灰底糊在一起，反而认不出这是哪个程序。
                let v = (max as f32 * 0.92) as u8;
                (v, v, v)
            }
            Tint::Running => (r, g, b),
        };
    }

    match tint {
        Tint::Idle => {
            let v = (max as f32 * 0.55) as u8;
            (v, v, v)
        }
        // 145° 是绿；亮度取原像素的 max，蓝色渐变的高光/暗部都会保留下来。
        Tint::Running => hsv_to_rgb(145.0, 0.60, max as f32 / 255.0),
    }
}

/// HSV → RGB，输入输出都归一化到 0..1（除返回值是 0..255）。
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> (u8, u8, u8) {
    let h = (hue / 60.0).rem_euclid(6.0);
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
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
    /// ico 里的帧是 PNG，所以要解码 → 改像素 → 编码回去。只在状态变化时
    /// 做一次（调用方缓存句柄），开销可以忽略。
    ///
    /// 着色失败（PNG 解不开、编码器报错）时退回未着色的原图：颜色不对是小事，
    /// 托盘图标整个消失是大事。
    ///
    /// 调用方负责在销毁时 `DestroyIcon`。
    pub fn hicon_tinted(size: u32, tint: super::Tint) -> Option<HICON> {
        let frame = pick_frame(size)?;
        let bytes = &ICON[frame.offset..frame.offset + frame.length];
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
    fn running_tint_turns_the_blue_background_green() {
        // 图标底色的蓝：色相 ~220°，tint 到 145° 之后绿分量应当最大。
        let (r, g, b) = recolor_pixel(0x25, 0x63, 0xeb, Tint::Running);
        assert!(g > r && g > b, "运行中应当是绿的，实际 {r},{g},{b}");
    }

    #[test]
    fn idle_tint_flattens_colour_to_grey() {
        let (r, g, b) = recolor_pixel(0x25, 0x63, 0xeb, Tint::Idle);
        assert_eq!((r, g, b), (r, r, r), "未运行应当是灰的");
    }

    #[test]
    fn the_white_motif_is_left_alone_while_running() {
        // 白色拱形不能跟着染色，否则图案和底色糊成一片。
        assert_eq!(recolor_pixel(0xff, 0xff, 0xff, Tint::Running), (255, 255, 255));
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
        for (tint, name) in [(Tint::Idle, "idle"), (Tint::Running, "running")] {
            let png = windows_icon::recolor(bytes, tint).expect("PNG 应当能解码");
            std::fs::write(format!(".tmp/tray-{name}.png"), png).expect("写预览图");
        }
    }
}
