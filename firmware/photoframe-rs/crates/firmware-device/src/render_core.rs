#![cfg_attr(not(target_os = "espidf"), allow(dead_code))]

const PANEL_WIDTH: usize = 800;
const PANEL_HEIGHT: usize = 480;
const DISPLAY_LEN: usize = (PANEL_WIDTH * PANEL_HEIGHT) / 2;
const RENDER_SERVICE_ROW_INTERVAL: usize = 64;
const RENDER_PROGRESS_ROW_INTERVAL: usize = 96;

const COLOR_BLACK: u8 = 0;
const COLOR_WHITE: u8 = 1;
const COLOR_YELLOW: u8 = 2;
const COLOR_RED: u8 = 3;
const COLOR_BLUE: u8 = 5;
const COLOR_GREEN: u8 = 6;

#[derive(Clone, Copy)]
struct PaletteColor {
    code: u8,
    r: u8,
    g: u8,
    b: u8,
}

const PALETTE: [PaletteColor; 6] = [
    PaletteColor {
        code: COLOR_BLACK,
        r: 0,
        g: 0,
        b: 0,
    },
    PaletteColor {
        code: COLOR_WHITE,
        r: 255,
        g: 255,
        b: 255,
    },
    PaletteColor {
        code: COLOR_YELLOW,
        r: 255,
        g: 255,
        b: 0,
    },
    PaletteColor {
        code: COLOR_RED,
        r: 255,
        g: 0,
        b: 0,
    },
    PaletteColor {
        code: COLOR_BLUE,
        r: 0,
        g: 0,
        b: 255,
    },
    PaletteColor {
        code: COLOR_GREEN,
        r: 0,
        g: 255,
        b: 0,
    },
];

const BAYER_4X4: [[i8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
const DITHER_STRENGTH: i32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedImage {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub panel_rotation: u8,
    pub color_process_mode: u8,
    pub dithering_mode: u8,
    pub six_color_tolerance: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct BmpFileHeader {
    kind: [u8; 2],
    size: u32,
    reserved1: u16,
    reserved2: u16,
    offset: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct BmpInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    image_size: u32,
    xppm: i32,
    yppm: i32,
    clr_used: u32,
    clr_important: u32,
}

fn clamp_byte(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn match_palette_color(r: u8, g: u8, b: u8, tolerance: u8) -> Option<u8> {
    PALETTE.iter().find_map(|p| {
        let dr = (r as i32 - p.r as i32).abs();
        let dg = (g as i32 - p.g as i32).abs();
        let db = (b as i32 - p.b as i32).abs();
        (dr <= tolerance as i32 && dg <= tolerance as i32 && db <= tolerance as i32)
            .then_some(p.code)
    })
}

fn apply_ordered_dither(x: usize, y: usize, r: &mut u8, g: &mut u8, b: &mut u8) {
    let threshold = BAYER_4X4[y & 0x3][x & 0x3] as i32 - 8;
    let delta = threshold * DITHER_STRENGTH;
    *r = clamp_byte(*r as i32 + delta);
    *g = clamp_byte(*g as i32 + delta);
    *b = clamp_byte(*b as i32 + delta);
}

fn quantize_color(r: u8, g: u8, b: u8) -> u8 {
    let mut best_code = COLOR_WHITE;
    let mut best_dist = i32::MAX;
    for p in PALETTE {
        let dr = r as i32 - p.r as i32;
        let dg = g as i32 - p.g as i32;
        let db = b as i32 - p.b as i32;
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best_code = p.code;
        }
    }
    best_code
}

fn set_packed_pixel(buf: &mut [u8], width: usize, x: usize, y: usize, px: u8) {
    let index = (y * width + x) >> 1;
    if (x & 1) == 0 {
        buf[index] = (buf[index] & 0x0F) | ((px & 0x0F) << 4);
    } else {
        buf[index] = (buf[index] & 0xF0) | (px & 0x0F);
    }
}

fn get_packed_pixel(buf: &[u8], width: usize, x: usize, y: usize) -> u8 {
    let index = (y * width + x) >> 1;
    let value = buf[index];
    if (x & 1) == 0 {
        (value >> 4) & 0x0F
    } else {
        value & 0x0F
    }
}

#[cfg(target_os = "espidf")]
fn service_render_loop(stage: Option<&str>, phase: &str, row: usize, total_rows: usize) {
    if row > 0 && row % RENDER_SERVICE_ROW_INTERVAL == 0 {
        unsafe {
            let _ = esp_idf_sys::esp_task_wdt_reset();
            esp_idf_sys::vTaskDelay(1);
        }
    }

    if let Some(stage) = stage
        && row > 0
        && row % RENDER_PROGRESS_ROW_INTERVAL == 0
    {
        let percent = row * 100 / total_rows.max(1);
        crate::device_log!(
            "INFO",
            "photoframe-rs/render: {stage} {phase} row={row}/{total_rows} ({percent}%)"
        );
    }
}

#[cfg(not(target_os = "espidf"))]
fn service_render_loop(_stage: Option<&str>, _phase: &str, _row: usize, _total_rows: usize) {}

#[cfg(target_os = "espidf")]
fn finish_render_loop(stage: Option<&str>, phase: &str, total_rows: usize) {
    if let Some(stage) = stage {
        crate::device_log!(
            "INFO",
            "photoframe-rs/render: {stage} {phase} row={total_rows}/{total_rows} (100%)"
        );
    }
}

#[cfg(not(target_os = "espidf"))]
fn finish_render_loop(_stage: Option<&str>, _phase: &str, _total_rows: usize) {}

fn rotate_buffer(display: &[u8], rotation: u8, stage: Option<&str>) -> Vec<u8> {
    let rotation = rotation % 4;
    if rotation == 0 {
        return display.to_vec();
    }
    let mut out = vec![0u8; DISPLAY_LEN];
    for y in 0..PANEL_HEIGHT {
        service_render_loop(stage, "rotate", y, PANEL_HEIGHT);
        for x in 0..PANEL_WIDTH {
            let px = get_packed_pixel(display, PANEL_WIDTH, x, y);
            let nx = PANEL_WIDTH - 1 - x;
            let ny = PANEL_HEIGHT - 1 - y;
            set_packed_pixel(&mut out, PANEL_WIDTH, nx, ny, px);
        }
    }
    finish_render_loop(stage, "rotate", PANEL_HEIGHT);
    out
}

fn render_pixels<F>(options: RenderOptions, stage: Option<&str>, mut get_rgb: F) -> PackedImage
where
    F: FnMut(usize, usize) -> (u8, u8, u8),
{
    let color_mode = options.color_process_mode.min(2);
    let dithering_mode = options.dithering_mode.min(1);
    let tolerance = options.six_color_tolerance.min(64);
    let treat_as_six_color = color_mode == 2;
    let use_dither = !treat_as_six_color && dithering_mode == 1;
    let mut display = vec![((COLOR_WHITE << 4) | COLOR_WHITE) as u8; DISPLAY_LEN];
    for y in 0..PANEL_HEIGHT {
        service_render_loop(stage, "quantize", y, PANEL_HEIGHT);
        for x in 0..PANEL_WIDTH {
            let (mut r, mut g, mut b) = get_rgb(x, y);
            if use_dither {
                apply_ordered_dither(x, y, &mut r, &mut g, &mut b);
            }
            let code =
                match_palette_color(r, g, b, tolerance).unwrap_or_else(|| quantize_color(r, g, b));
            set_packed_pixel(&mut display, PANEL_WIDTH, x, y, code);
        }
    }
    finish_render_loop(stage, "quantize", PANEL_HEIGHT);

    PackedImage {
        width: PANEL_WIDTH,
        height: PANEL_HEIGHT,
        bytes: rotate_buffer(&display, options.panel_rotation, stage),
    }
}

pub fn render_rgb888_to_packed(
    rgb: &[u8],
    width: usize,
    height: usize,
    options: RenderOptions,
) -> Result<PackedImage, String> {
    if !((width == PANEL_WIDTH && height == PANEL_HEIGHT)
        || (width == PANEL_HEIGHT && height == PANEL_WIDTH))
    {
        return Err(format!("unsupported rgb dimension: {width}x{height}"));
    }
    let row_stride = width * 3;
    Ok(render_pixels(options, Some("rgb_pack"), |x, y| {
        let (sx, sy) = if width == PANEL_WIDTH && height == PANEL_HEIGHT {
            (x, y)
        } else {
            (y, height - 1 - x)
        };
        let offset = sy * row_stride + sx * 3;
        (rgb[offset], rgb[offset + 1], rgb[offset + 2])
    }))
}

pub fn render_bmp24_to_packed(bmp: &[u8], options: RenderOptions) -> Result<PackedImage, String> {
    if bmp.len() < core::mem::size_of::<BmpFileHeader>() + core::mem::size_of::<BmpInfoHeader>() {
        return Err("bmp too short".into());
    }
    let file = unsafe { &*(bmp.as_ptr() as *const BmpFileHeader) };
    if file.kind != *b"BM" {
        return Err("invalid bmp magic".into());
    }
    let info = unsafe {
        &*(bmp[core::mem::size_of::<BmpFileHeader>()..].as_ptr() as *const BmpInfoHeader)
    };
    let width = info.width;
    let height_abs = info.height.abs();
    let bottom_up = info.height > 0;
    if info.size < core::mem::size_of::<BmpInfoHeader>() as u32
        || info.planes != 1
        || info.bit_count != 24
        || info.compression != 0
    {
        return Err("unsupported bmp format".into());
    }
    if !((width == PANEL_WIDTH as i32 && height_abs == PANEL_HEIGHT as i32)
        || (width == PANEL_HEIGHT as i32 && height_abs == PANEL_WIDTH as i32))
    {
        return Err(format!("unsupported bmp dimension: {width}x{height_abs}"));
    }
    let row_stride = ((width as usize * 3) + 3) & !3;
    let need = file.offset as usize + row_stride * height_abs as usize;
    if need > bmp.len() {
        return Err("bmp size mismatch".into());
    }
    let pixels = &bmp[file.offset as usize..];
    Ok(render_pixels(options, Some("bmp_pack"), |x, y| {
        let (sx, sy) = if width as usize == PANEL_WIDTH && height_abs as usize == PANEL_HEIGHT {
            (x, y)
        } else {
            (y, height_abs as usize - 1 - x)
        };
        let src_row = if bottom_up {
            height_abs as usize - 1 - sy
        } else {
            sy
        };
        let offset = src_row * row_stride + sx * 3;
        let b = pixels[offset];
        let g = pixels[offset + 1];
        let r = pixels[offset + 2];
        (r, g, b)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_pixel_roundtrip() {
        let mut buf = vec![0u8; DISPLAY_LEN];
        set_packed_pixel(&mut buf, PANEL_WIDTH, 0, 0, COLOR_RED);
        set_packed_pixel(&mut buf, PANEL_WIDTH, 1, 0, COLOR_BLUE);
        assert_eq!(get_packed_pixel(&buf, PANEL_WIDTH, 0, 0), COLOR_RED);
        assert_eq!(get_packed_pixel(&buf, PANEL_WIDTH, 1, 0), COLOR_BLUE);
    }

    #[test]
    fn rgb_primary_red_maps_to_red_palette() {
        let rgb = vec![255u8, 0, 0]
            .into_iter()
            .cycle()
            .take(PANEL_WIDTH * PANEL_HEIGHT * 3)
            .collect::<Vec<_>>();
        let image = render_rgb888_to_packed(
            &rgb,
            PANEL_WIDTH,
            PANEL_HEIGHT,
            RenderOptions {
                panel_rotation: 0,
                color_process_mode: 2,
                dithering_mode: 0,
                six_color_tolerance: 0,
            },
        )
        .unwrap();
        assert_eq!(get_packed_pixel(&image.bytes, PANEL_WIDTH, 0, 0), COLOR_RED);
    }

    #[test]
    fn rotation_180_moves_first_pixel_to_last() {
        let mut rgb = vec![255u8, 255, 255]
            .into_iter()
            .cycle()
            .take(PANEL_WIDTH * PANEL_HEIGHT * 3)
            .collect::<Vec<_>>();
        rgb[0] = 255;
        rgb[1] = 0;
        rgb[2] = 0;
        let image = render_rgb888_to_packed(
            &rgb,
            PANEL_WIDTH,
            PANEL_HEIGHT,
            RenderOptions {
                panel_rotation: 2,
                color_process_mode: 2,
                dithering_mode: 0,
                six_color_tolerance: 0,
            },
        )
        .unwrap();
        assert_eq!(
            get_packed_pixel(&image.bytes, PANEL_WIDTH, PANEL_WIDTH - 1, PANEL_HEIGHT - 1),
            COLOR_RED
        );
    }
}
