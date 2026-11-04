//! Album-art palette (`ColorExtractor.extractPalette` in
//! Sources/UI/ColorExtractor.swift): a 3×3 grid of cell averages, each
//! "vivified" (saturation pushed, brightness spread with a per-cell offset)
//! so averaged colours do not come out muddy.

use std::io::Cursor;
use std::sync::LazyLock;

use image::{DynamicImage, ImageReader, Limits};

use crate::contract::Palette;

const COLUMNS: usize = 3;
const ROWS: usize = 3;
/// Brightness offsets per cell, row-major (same table as macOS).
const OFFSETS: [f64; 9] = [-0.12, -0.04, 0.06, 0.03, 0.12, -0.08, -0.04, 0.08, 0.02];
/// Cells are averaged from at most about this many samples per axis.
const SAMPLES_PER_AXIS: u32 = 192;

/// The macOS `fallbackPalette` (sRGB), used before artwork resolves and when
/// a cover cannot be decoded.
const FALLBACK_RGB: [[f64; 3]; 9] = [
    [0.23, 0.16, 0.58],
    [0.12, 0.40, 0.44],
    [0.65, 0.28, 0.43],
    [0.15, 0.30, 0.56],
    [0.54, 0.25, 0.16],
    [0.43, 0.19, 0.56],
    [0.10, 0.46, 0.40],
    [0.55, 0.34, 0.16],
    [0.30, 0.19, 0.52],
];

static TO_LINEAR: LazyLock<[f64; 256]> = LazyLock::new(|| {
    let mut table = [0.0; 256];
    for (i, value) in table.iter_mut().enumerate() {
        *value = srgb_to_linear(i as f64 / 255.0);
    }
    table
});

fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB (0..=1) to hue/saturation/brightness (0..=1), as
/// `NSColor.getHue(_:saturation:brightness:alpha:)`.
pub fn rgb_to_hsb(r: f64, g: f64, b: f64) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let saturation = if max > 0.0 { delta / max } else { 0.0 };
    let sector = if delta <= 0.0 {
        0.0
    } else if max == r {
        (g - b) / delta
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let mut hue = sector / 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    [hue, saturation, max]
}

/// The palette shown when there is no usable artwork.
pub fn fallback_palette() -> Palette {
    Palette { hsb: FALLBACK_RGB.iter().map(|c| rgb_to_hsb(c[0], c[1], c[2])).collect(), from_artwork: false }
}

/// Palette from encoded artwork (JPEG, PNG, BMP, GIF or WebP); the fallback
/// palette when it cannot be decoded or is 1 px wide or high.
pub fn extract_palette(bytes: &[u8]) -> Palette {
    try_extract_palette(bytes).unwrap_or_else(fallback_palette)
}

/// Like [`extract_palette`] but None instead of the fallback.
pub fn try_extract_palette(bytes: &[u8]) -> Option<Palette> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    palette_from_image(&image)
}

/// Palette from decoded pixels.
pub fn palette_from_image(image: &DynamicImage) -> Option<Palette> {
    match image {
        DynamicImage::ImageRgb8(rgb) => palette_from_pixels(rgb.width(), rgb.height(), |x, y| {
            let p = rgb.get_pixel(x, y).0;
            [p[0], p[1], p[2], 255]
        }),
        DynamicImage::ImageRgba8(rgba) => {
            palette_from_pixels(rgba.width(), rgba.height(), |x, y| rgba.get_pixel(x, y).0)
        }
        other => {
            let rgba = other.to_rgba8();
            palette_from_pixels(rgba.width(), rgba.height(), |x, y| rgba.get_pixel(x, y).0)
        }
    }
}

/// Palette from a tightly packed RGBA8 buffer (row-major, top row first).
pub fn palette_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Palette> {
    if rgba.len() < width as usize * height as usize * 4 {
        return None;
    }
    palette_from_pixels(width, height, |x, y| {
        let i = (y as usize * width as usize + x as usize) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    })
}

/// Averages each cell (row 0 = top of the image), then vivifies.
///
/// Averaging happens in linear light on premultiplied colour, as Core
/// Image's `CIAreaAverage` does in its linear working space; the average
/// is re-encoded to sRGB and quantised to 8 bits like the macOS 1×1
/// bitmap render. Large images are subsampled on a regular grid.
fn palette_from_pixels(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Option<Palette> {
    if width <= 1 || height <= 1 {
        return None;
    }
    let step = width.max(height).div_ceil(SAMPLES_PER_AXIS).max(1);
    let linear = &*TO_LINEAR;
    let mut sums = [[0.0f64; 3]; COLUMNS * ROWS];
    let mut counts = [0u32; COLUMNS * ROWS];
    let mut y = step / 2;
    while y < height {
        let row = (y as u64 * ROWS as u64 / height as u64) as usize;
        let mut x = step / 2;
        while x < width {
            let column = (x as u64 * COLUMNS as u64 / width as u64) as usize;
            let [r, g, b, a] = pixel(x, y);
            let alpha = a as f64 / 255.0;
            let cell = row * COLUMNS + column;
            sums[cell][0] += linear[r as usize] * alpha;
            sums[cell][1] += linear[g as usize] * alpha;
            sums[cell][2] += linear[b as usize] * alpha;
            counts[cell] += 1;
            x += step;
        }
        y += step;
    }
    let samples: Vec<[f64; 3]> = sums
        .iter()
        .zip(counts)
        .map(|(sum, count)| {
            let n = count.max(1) as f64;
            let channel = |v: f64| (linear_to_srgb(v / n) * 255.0).round() / 255.0;
            rgb_to_hsb(channel(sum[0]), channel(sum[1]), channel(sum[2]))
        })
        .collect();
    Some(Palette { hsb: vivify(&samples), from_artwork: true })
}

/// `ColorExtractor.vivify`: saturation × 1.22 (≤ 1); brightness
/// `b × 0.84 + 0.12 + offset`, clamped to 0.08..=0.96.
fn vivify(samples: &[[f64; 3]]) -> Vec<[f64; 3]> {
    samples
        .iter()
        .enumerate()
        .map(|(index, [h, s, b])| {
            let saturation = (s * 1.22).min(1.0);
            let brightness = (b * 0.84 + 0.12 + OFFSETS[index % OFFSETS.len()]).clamp(0.08, 0.96);
            [*h, saturation, brightness]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgb, RgbImage, RgbaImage};

    fn encode(image: &RgbImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn hsb_conversion_matches_nscolor() {
        assert_eq!(rgb_to_hsb(1.0, 0.0, 0.0), [0.0, 1.0, 1.0]);
        let green = rgb_to_hsb(0.0, 1.0, 0.0);
        assert!((green[0] - 1.0 / 3.0).abs() < 1e-12);
        let blue = rgb_to_hsb(0.0, 0.0, 0.5);
        assert!((blue[0] - 2.0 / 3.0).abs() < 1e-12 && blue[2] == 0.5);
        assert_eq!(rgb_to_hsb(0.5, 0.5, 0.5), [0.0, 0.0, 0.5]);
        assert_eq!(rgb_to_hsb(0.0, 0.0, 0.0), [0.0, 0.0, 0.0]);
        let magenta_ish = rgb_to_hsb(1.0, 0.0, 0.5);
        assert!((magenta_ish[0] - (1.0 - 0.5 / 6.0)).abs() < 1e-12, "negative hue wraps");
    }

    #[test]
    fn fallback_palette_is_the_macos_table_in_hsb() {
        let palette = fallback_palette();
        assert!(!palette.from_artwork);
        assert_eq!(palette.hsb.len(), 9);
        // Color(red: 0.23, green: 0.16, blue: 0.58): blue max.
        let [h, s, b] = palette.hsb[0];
        assert!((b - 0.58).abs() < 1e-12);
        assert!((s - (0.58 - 0.16) / 0.58).abs() < 1e-12);
        assert!((h - ((0.23 - 0.16) / 0.42 + 4.0) / 6.0).abs() < 1e-12);
        assert_eq!(extract_palette(b"not an image"), palette);
        assert!(try_extract_palette(&[]).is_none());
    }

    #[test]
    fn synthetic_grid_maps_cells_row_major_from_the_top() {
        // 3x3 blocks of pure colours, 30x30 pixels each.
        let colours: [[u8; 3]; 9] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [0, 255, 255],
            [255, 0, 255],
            [128, 128, 128],
            [255, 255, 255],
            [0, 0, 0],
        ];
        let image = RgbImage::from_fn(90, 90, |x, y| Rgb(colours[(y / 30 * 3 + x / 30) as usize]));
        for format in [ImageFormat::Png, ImageFormat::Bmp] {
            let palette = extract_palette(&encode(&image, format));
            assert!(palette.from_artwork);
            assert_eq!(palette.hsb.len(), 9);
            let expected_hues = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0 / 6.0, 0.5, 5.0 / 6.0, 0.0, 0.0, 0.0];
            for (i, [h, s, b]) in palette.hsb.iter().enumerate() {
                assert!((h - expected_hues[i]).abs() < 1e-9, "cell {i} hue {h}");
                let source = rgb_to_hsb(
                    colours[i][0] as f64 / 255.0,
                    colours[i][1] as f64 / 255.0,
                    colours[i][2] as f64 / 255.0,
                );
                assert!((s - (source[1] * 1.22).min(1.0)).abs() < 1e-9);
                let want = (source[2] * 0.84 + 0.12 + OFFSETS[i]).clamp(0.08, 0.96);
                assert!((b - want).abs() < 1e-9, "cell {i} brightness {b} vs {want}");
            }
        }
    }

    #[test]
    fn averaging_is_in_linear_light() {
        // Half black, half white columns inside every cell.
        let image = RgbImage::from_fn(60, 60, |x, _| if x % 2 == 0 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) });
        let palette = palette_from_image(&DynamicImage::ImageRgb8(image)).unwrap();
        // linear mean 0.5 -> sRGB 0.735 -> 188/255
        let average = 188.0 / 255.0;
        let want = (average * 0.84 + 0.12 + OFFSETS[4]).clamp(0.08, 0.96);
        assert!((palette.hsb[4][2] - want).abs() < 1e-9);
        assert_eq!(palette.hsb[4][1], 0.0);
    }

    #[test]
    fn transparency_counts_as_black_and_tiny_images_fall_back() {
        let clear = RgbaImage::from_pixel(9, 9, image::Rgba([255, 255, 255, 0]));
        let palette = palette_from_image(&DynamicImage::ImageRgba8(clear)).unwrap();
        assert_eq!(palette.hsb[0][2], 0.08, "black, clamped to the minimum brightness");
        assert!(palette_from_rgba(1, 50, &[0; 200]).is_none());
        assert!(palette_from_rgba(10, 10, &[0; 10]).is_none());
        let thin = encode(&RgbImage::from_pixel(1, 40, Rgb([10, 20, 30])), ImageFormat::Png);
        assert!(!extract_palette(&thin).from_artwork);
    }

    #[test]
    fn large_images_are_subsampled_consistently() {
        let image = RgbImage::from_fn(1500, 900, |x, y| Rgb([(x * 255 / 1500) as u8, (y * 255 / 900) as u8, 90]));
        let jpeg = encode(&image, ImageFormat::Jpeg);
        let palette = extract_palette(&jpeg);
        assert!(palette.from_artwork);
        // Left column is less red than the right column.
        let red = |i: usize| palette.hsb[i];
        assert!(red(0)[0] != red(2)[0]);
        for [h, s, b] in &palette.hsb {
            assert!((0.0..=1.0).contains(h) && (0.0..=1.0).contains(s) && (0.08..=0.96).contains(b));
        }
    }
}
