//! Rysowanie obrazów generowanych przez plugin: wskaźnik poziomu (nakładka na pokrętle) i zegar.
//!
//! Tekst rysujemy czcionką systemową (Windows: Segoe UI, Linux: DejaVu/Noto/Liberation,
//! macOS: Arial), żeby nie dołączać czcionek do pluginu.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use std::sync::LazyLock;

const BG: Rgb<u8> = Rgb([16, 16, 22]);
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
    vec![
        format!("/System/Library/Fonts/Supplemental/{file}"),
        format!("/Library/Fonts/{file}"),
    ]
}

static BOLD: LazyLock<Option<FontArc>> = LazyLock::new(|| load_font(&font_paths(true)));
static REGULAR: LazyLock<Option<FontArc>> = LazyLock::new(|| load_font(&font_paths(false)));

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
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let r = (dx * dx + dy * dy).sqrt();
            // Wygładzone krawędzie pierścienia.
            let coverage = (r - inner + 0.5).clamp(0.0, 1.0) * (outer - r + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let angle = (dx.atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0);
            let ring = if angle <= level && level > 0.0 { color } else { track };
            img.put_pixel(x, y, blend(BG, ring, coverage));
        }
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

/// Kafelek na klawisz: pasek w kolorze akcentu u góry, podpis, duża wartość, opcjonalny dopisek
/// i pasek postępu na dole. `tint` zabarwia tło (np. zielone / czerwone przy zakupach).
pub fn tile(
    size: u32,
    label: &str,
    value: &str,
    sub: Option<&str>,
    accent: Rgb<u8>,
    tint: Option<Rgb<u8>>,
    progress: Option<f32>,
) -> RgbImage {
    let bg = tint.map(|t| blend(BG, t, 0.45)).unwrap_or(BG);
    let mut img = ImageBuffer::from_pixel(size, size, bg);
    let s = size as f32;
    let bar = (s * 0.05).round() as u32;
    for y in 0..bar {
        for x in 0..size {
            img.put_pixel(x, y, accent);
        }
    }
    if let Some(p) = progress {
        let h = (s * 0.07).round() as u32;
        let filled = (s * p.clamp(0.0, 1.0)).round() as u32;
        for y in size - h..size {
            for x in 0..size {
                img.put_pixel(x, y, if x < filled { accent } else { TRACK });
            }
        }
    }
    let max_width = s * 0.9;
    if let Some(font) = REGULAR.as_ref() {
        centered(&mut img, font, s * 0.16, max_width, s * 0.2, SOFT_TEXT, label);
    }
    let value_y = if sub.is_some() { s * 0.52 } else { s * 0.58 };
    if let Some(font) = BOLD.as_ref() {
        centered(&mut img, font, s * 0.36, max_width, value_y, TEXT, value);
    }
    if let (Some(sub), Some(font)) = (sub, REGULAR.as_ref()) {
        centered(&mut img, font, s * 0.14, max_width, s * 0.79, DIM_TEXT, sub);
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
    let mut out = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95).encode_image(img);
    out
}
