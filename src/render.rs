//! Rysowanie obrazów generowanych przez plugin: wskaźnik poziomu (nakładka na pokrętle) i zegar.
//!
//! Tekst rysujemy czcionką systemową (Windows: Segoe UI, Linux: DejaVu/Noto/Liberation,
//! macOS: Arial), żeby nie dołączać czcionek do pluginu.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

pub const BG: Rgb<u8> = Rgb([16, 16, 22]);
pub const TRACK: Rgb<u8> = Rgb([48, 48, 60]);
const ACCENT: Rgb<u8> = Rgb([124, 58, 237]);
const MUTED: Rgb<u8> = Rgb([200, 40, 40]);
const TEXT: Rgb<u8> = Rgb([240, 240, 245]);
const DIM_TEXT: Rgb<u8> = Rgb([160, 160, 175]);
/// Jaśniejszy odcień dla drobnego tekstu (data), żeby był czytelny na małym ekranie.
const SOFT_TEXT: Rgb<u8> = Rgb([205, 205, 220]);

/// Pierwsza czcionka z listy ścieżek, która istnieje i da się wczytać.
fn load_font(paths: &[String]) -> Option<FontArc> {
    let font = paths.iter().find_map(|p| FontArc::try_from_vec(std::fs::read(p).ok()?).ok());
    if font.is_none() {
        log::warn!("Nie znaleziono czcionki (szukano: {paths:?}) – tekst nie będzie rysowany");
    }
    font
}

#[cfg(windows)]
fn font_paths(bold: bool) -> Vec<String> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let files: &[&str] = if bold { &["segoeuib.ttf", "arialbd.ttf"] } else { &["segoeui.ttf", "arial.ttf"] };
    files.iter().map(|f| format!(r"{dir}\Fonts\{f}")).collect()
}

#[cfg(target_os = "linux")]
fn font_paths(bold: bool) -> Vec<String> {
    let style = if bold { "Bold" } else { "Regular" };
    let dejavu = if bold { "DejaVuSans-Bold" } else { "DejaVuSans" };
    [
        format!("/usr/share/fonts/truetype/dejavu/{dejavu}.ttf"),
        format!("/usr/share/fonts/TTF/{dejavu}.ttf"),
        format!("/usr/share/fonts/dejavu-sans-fonts/{dejavu}.ttf"),
        format!("/usr/share/fonts/truetype/noto/NotoSans-{style}.ttf"),
        format!("/usr/share/fonts/noto/NotoSans-{style}.ttf"),
        format!("/usr/share/fonts/google-noto/NotoSans-{style}.ttf"),
        format!("/usr/share/fonts/truetype/liberation/LiberationSans-{style}.ttf"),
        format!("/usr/share/fonts/liberation-sans/LiberationSans-{style}.ttf"),
    ]
    .into()
}

#[cfg(target_os = "macos")]
fn font_paths(bold: bool) -> Vec<String> {
    let file = if bold { "Arial Bold.ttf" } else { "Arial.ttf" };
    vec![format!("/System/Library/Fonts/Supplemental/{file}"), format!("/Library/Fonts/{file}")]
}

static BOLD: LazyLock<Option<FontArc>> = LazyLock::new(|| load_font(&font_paths(true)));
static REGULAR: LazyLock<Option<FontArc>> = LazyLock::new(|| load_font(&font_paths(false)));

pub fn has_text_font() -> bool {
    BOLD.is_some()
}

struct RingPixel {
    x: u32,
    y: u32,
    angle: f32,
    coverage: f32,
}

static RING_PIXELS: LazyLock<Mutex<HashMap<u32, Arc<[RingPixel]>>>> = LazyLock::new(Default::default);

fn ring_pixels(size: u32) -> Arc<[RingPixel]> {
    let mut cache = RING_PIXELS.lock().unwrap();
    cache
        .entry(size)
        .or_insert_with(|| {
            let c = size as f32 / 2.0;
            let outer = c - size as f32 * 0.05;
            let inner = outer - size as f32 * 0.08;
            let mut pixels = Vec::new();
            for y in 0..size {
                for x in 0..size {
                    let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
                    let r = (dx * dx + dy * dy).sqrt();
                    let coverage = (r - inner + 0.5).clamp(0.0, 1.0) * (outer - r + 0.5).clamp(0.0, 1.0);
                    if coverage > 0.0 {
                        pixels.push(RingPixel { x, y, angle: (dx.atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0), coverage });
                    }
                }
            }
            pixels.into()
        })
        .clone()
}

fn text_width(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum()
}

/// Tekst wyśrodkowany poziomo; `cy` to środek wysokości tekstu. Zbyt szeroki tekst jest zmniejszany.
fn centered(img: &mut RgbImage, font: &FontArc, size: f32, max_width: f32, cy: f32, color: Rgb<u8>, text: &str) {
    let mut size = size;
    while size > 8.0 && text_width(font, size, text) > max_width {
        size -= 1.0;
    }
    let w = text_width(font, size, text);
    let scaled = font.as_scaled(PxScale::from(size));
    let h = scaled.ascent() - scaled.descent();
    let x = (img.width() as f32 - w) / 2.0;
    let y = cy - h / 2.0;
    draw_text_mut(img, color, x.round() as i32, y.round() as i32, PxScale::from(size), font, text);
}

pub fn blend(a: Rgb<u8>, b: Rgb<u8>, t: f32) -> Rgb<u8> {
    let t = t.clamp(0.0, 1.0);
    Rgb([0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8))
}

/// Okrągły wskaźnik poziomu 0–1 z dużą wartością pośrodku i podpisem pod nią.
pub fn gauge(size: u32, level: f32, muted: bool, value_text: &str, label: &str) -> RgbImage {
    let (color, track) = if muted { (MUTED, blend(TRACK, MUTED, 0.35)) } else { (ACCENT, TRACK) };
    ring(size, if muted { 0.0 } else { level }, color, track, None, value_text, label)
}

/// Pierścień wypełniony w `level` (0–1, od góry zgodnie z ruchem wskazówek zegara), duża wartość
/// pośrodku, opcjonalny mały napis nad nią i podpis pod nią.
pub fn ring(size: u32, level: f32, color: Rgb<u8>, track: Rgb<u8>, top: Option<&str>, value_text: &str, label: &str) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(size, size, BG);
    let c = size as f32 / 2.0;
    let (outer, width) = (c - size as f32 * 0.05, size as f32 * 0.08);
    let inner = outer - width;
    let level = level.clamp(0.0, 1.0);
    for pixel in ring_pixels(size).iter() {
        let ring = if pixel.angle <= level && level > 0.0 { color } else { track };
        img.put_pixel(pixel.x, pixel.y, blend(BG, ring, pixel.coverage));
    }
    if let Some(font) = BOLD.as_ref() {
        centered(&mut img, font, size as f32 * 0.27, inner * 1.6, c - size as f32 * 0.03, TEXT, value_text);
    }
    if let Some(font) = REGULAR.as_ref() {
        centered(&mut img, font, size as f32 * 0.09, inner * 1.3, c + size as f32 * 0.19, DIM_TEXT, label);
    }
    if let (Some(top), Some(font)) = (top, BOLD.as_ref()) {
        centered(&mut img, font, size as f32 * 0.085, inner * 1.2, c - size as f32 * 0.24, SOFT_TEXT, top);
    }
    img
}

/// Zegar: godzina dużą czcionką, opcjonalnie data pod spodem.
pub fn clock(size: u32, time: &str, date: Option<&str>) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(size, size, BG);
    let c = size as f32 / 2.0;
    // Na okrągłym ekranie pokrętła tekst musi zmieścić się w kole – stąd ~70% szerokości.
    let max_width = size as f32 * 0.72;
    let time_y = if date.is_some() { c - size as f32 * 0.06 } else { c };
    if let Some(font) = BOLD.as_ref() {
        centered(&mut img, font, size as f32 * 0.30, max_width, time_y, TEXT, time);
    }
    // Data pogrubioną czcionką i jaśniej – cienka szara była na ekranie nieczytelna.
    if let (Some(date), Some(font)) = (date, BOLD.as_ref()) {
        centered(&mut img, font, size as f32 * 0.115, max_width, c + size as f32 * 0.21, SOFT_TEXT, date);
    }
    img
}

/// PNG jako data URL (dla obrazów ustawianych przez akcje w OpenDecku).
pub fn png_data_url(img: &RgbImage) -> String {
    use base64::Engine;
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut buf, image::ImageFormat::Png);
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// JPG gotowy do wysłania na moduł.
pub fn jpeg(img: &RgbImage) -> Vec<u8> {
    jpeg_with_quality(img, 95)
}

pub fn jpeg_with_quality(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(img);
    out
}

/// Standardowy baseline JPEG 4:4:4; zoptymalizowane tablice Huffmana oszczędzają
/// raporty HID przy tej samej jakości 80. Nie używamy progresywnego JPEG.
pub fn feedback_jpeg(img: &RgbImage) -> Result<Vec<u8>, String> {
    let width = u16::try_from(img.width()).map_err(|error| error.to_string())?;
    let height = u16::try_from(img.height()).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, 80);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    encoder.set_progressive(false);
    encoder.set_optimized_huffman_tables(true);
    encoder.encode(img.as_raw(), width, height, jpeg_encoder::ColorType::Rgb).map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_jpeg_is_baseline_smaller_and_decodes_to_the_correct_colours() {
        let img = gauge(240, 0.55, false, "55%", "Home Assistant");
        let jpeg = feedback_jpeg(&img).unwrap();
        assert!(jpeg.windows(2).any(|marker| marker == [0xff, 0xc0]));
        let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), img.dimensions());
        let error: u64 = decoded.as_raw().iter().zip(img.as_raw()).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        assert!(error as f64 / (img.as_raw().len() as f64) < 3.0, "barwy i tekst muszą przejść bez uszkodzeń");
        assert!(jpeg.len() < jpeg_with_quality(&img, 80).len(), "optymalizacja musi oszczędzać transfer");
    }
}
