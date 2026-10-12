//! Konwersja obrazków z OpenDecka (data URL) na JPG dla klawiszy MasterHUB.

use base64::Engine;
use image::{DynamicImage, ImageBuffer, Rgb, RgbImage, codecs::jpeg::JpegEncoder, imageops::FilterType};

fn decode(data_url: &str) -> Result<DynamicImage, String> {
    let (header, data) = data_url.split_once(',').ok_or("niepoprawny data URL")?;
    if !header.ends_with(";base64") {
        return Err(format!("nieobsługiwany format obrazka: {header}"));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| e.to_string())?;
    image::load_from_memory(&bytes).map_err(|e| format!("{header}: {e}"))
}

pub enum DialFeedback {
    Image(RgbImage),
    Ring(RingFeedback),
    /// setFeedbackLayout odrysowuje $B1 przed setFeedback, jeszcze bez wartości.
    Incomplete,
}

#[derive(PartialEq)]
pub struct RingFeedback {
    pub level: f32,
    value: image::GrayImage,
    title: Option<image::GrayImage>,
}

impl RingFeedback {
    /// Wartość tekstowa jest potwierdzona przez HA; animujemy tylko pierścień.
    pub fn render(&self, size: u32, level: f32) -> RgbImage {
        let mut dial = crate::render::gauge(size, level, false, "", "");
        paste_text(&mut dial, &self.value, size * 64 / 100, size * 26 / 100, size * 47 / 100);
        if let Some(title) = &self.title {
            paste_text(&mut dial, title, size * 52 / 100, size * 9 / 100, size * 71 / 100);
        }
        dial
    }
}

impl DialFeedback {
    pub fn render(&self, size: u32) -> RgbImage {
        match self {
            Self::Image(img) => img.clone(),
            Self::Ring(ring) => ring.render(size, ring.level),
            Self::Incomplete => RgbImage::new(size, size),
        }
    }
}

/// Odczytuje szablon pierścienia `$B1` albo dopasowuje obraz pozostałych układów.
pub fn dial_feedback(data_url: &str, size: u32) -> Result<DialFeedback, String> {
    dial_feedback_region(data_url, size, None)
}

/// Obraz całego rzędu dzielimy przed rozpoznaniem $B1, aby pokazać właściwy enkoder.
pub fn dial_feedback_region(data_url: &str, size: u32, row: Option<(u8, u8)>) -> Result<DialFeedback, String> {
    let img = decode(data_url)?;
    let img = if let Some((position, count)) = row {
        if count == 0 || position >= count || img.width() % count as u32 != 0 {
            return Err("obraz rzędu nie pasuje do liczby enkoderów".into());
        }
        let width = img.width() / count as u32;
        img.crop_imm(position as u32 * width, 0, width, img.height())
    } else {
        img
    };
    let rgb = flatten(img.clone());
    let Some(level) = b1_level(&rgb) else { return Ok(DialFeedback::Image(fit_rgb(&img, size, false))) };
    let Some(value) = text_crop(&rgb, (76, 40, 108, 32)) else { return Ok(DialFeedback::Incomplete) };
    // Procent pochodzi z obrazu HA, nie z zaokrąglenia szerokości paska. Dzięki temu
    // zachowujemy dokładną wartość i nie próbujemy rozpoznawać cyfr przez OCR.
    Ok(DialFeedback::Ring(RingFeedback { level, value, title: text_crop(&rgb, (16, 10, 136, 24)) }))
}

/// `$B1` OpenDecka: płótno 200×100, biały pasek na ciemnoszarym tle w [76,74,108,12].
/// Sprawdzamy geometrię i kolory, żeby nie przerabiać innych układów na błędny wskaźnik.
fn b1_level(img: &RgbImage) -> Option<f32> {
    if img.dimensions() != (200, 100) {
        return None;
    }
    let brightness = |x, y| img.get_pixel(x, y).0.iter().map(|&c| c as u32).sum::<u32>() / 3;
    for y in 79..=80 {
        if brightness(73, y) > 60 || brightness(186, y) > 60 {
            return None;
        }
        for x in 76..184 {
            let p = img.get_pixel(x, y).0;
            if p.iter().max()? - p.iter().min()? > 35 || !(125..=255).contains(&brightness(x, y)) {
                return None;
            }
        }
    }
    let mut fill = 0;
    let mut track_started = false;
    for x in 76..184 {
        let filled = (brightness(x, 79) + brightness(x, 80)) / 2 > 212;
        if filled {
            if track_started {
                return None;
            }
            fill += 1;
        } else {
            track_started = true;
        }
    }
    Some(fill as f32 / 108.0)
}

/// Wycina jasny tekst z czarnego tła, pomijając szum kompresji JPEG.
fn text_crop(img: &RgbImage, rect: (u32, u32, u32, u32)) -> Option<image::GrayImage> {
    let (x, y, w, h) = rect;
    let mask = image::GrayImage::from_fn(w, h, |cx, cy| {
        let p = img.get_pixel(x + cx, y + cy).0;
        let luma = (p[0] as u16 + p[1] as u16 + p[2] as u16) / 3;
        image::Luma([((luma.saturating_sub(32) * 255) / 223) as u8])
    });
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for (x, y, p) in mask.enumerate_pixels() {
        if p[0] > 64 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    (left <= right && top <= bottom).then(|| image::imageops::crop_imm(&mask, left, top, right - left + 1, bottom - top + 1).to_image())
}

fn paste_text(dial: &mut RgbImage, text: &image::GrayImage, max_width: u32, max_height: u32, cy: u32) {
    let text = DynamicImage::ImageLuma8(text.clone()).resize(max_width, max_height, FilterType::Triangle).to_luma8();
    let (left, top) = ((dial.width() - text.width()) / 2, cy.saturating_sub(text.height() / 2));
    for (x, y, p) in text.enumerate_pixels() {
        dial.put_pixel(left + x, top + y, crate::render::blend(crate::render::BG, Rgb([240, 240, 245]), p[0] as f32 / 255.0));
    }
}

/// Zwraca kwadratowy JPG `size`×`size` gotowy do wysłania; `None` daje czarny obraz.
pub fn to_jpeg(data_url: Option<&str>, size: u32) -> Result<Vec<u8>, String> {
    let rgb = match data_url {
        None => ImageBuffer::from_pixel(size, size, Rgb([0, 0, 0])),
        Some(url) => flatten(decode(url)?.resize_exact(size, size, FilterType::Triangle)),
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90).encode_image(&rgb).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Skaluje obraz do kwadratu `size`×`size`: `cover` przycina do wypełnienia, inaczej mieści
/// całość na czarnym tle. Zwraca JPG.
pub fn fit_jpeg(img: &DynamicImage, size: u32, cover: bool, quality: u8) -> Result<Vec<u8>, String> {
    let canvas = fit_rgb(img, size, cover);
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality).encode_image(&canvas).map_err(|e| e.to_string())?;
    Ok(out)
}

fn fit_rgb(img: &DynamicImage, size: u32, cover: bool) -> RgbImage {
    let scaled = if cover { img.resize_to_fill(size, size, FilterType::Triangle) } else { img.resize(size, size, FilterType::Triangle) };
    let mut canvas = ImageBuffer::from_pixel(size, size, Rgb([0, 0, 0]));
    let flat = flatten(scaled);
    let (x, y) = ((size - flat.width()) / 2, (size - flat.height()) / 2);
    image::imageops::overlay(&mut canvas, &flat, x as i64, y as i64);
    canvas
}

/// Nakłada przezroczystość na czarne tło.
fn flatten(img: DynamicImage) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let rgba = img.to_rgba8();
    ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u16;
        Rgb([0, 1, 2].map(|c| (p[c] as u16 * a / 255) as u8))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn png_url(img: DynamicImage) -> String {
        let mut bytes = Cursor::new(Vec::new());
        img.write_to(&mut bytes, ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()))
    }

    #[test]
    fn encoder_strip_keeps_proportions_and_both_edges() {
        // Format paska feedbacku: szeroki obraz z treścią przy obu krawędziach.
        let source = RgbImage::from_fn(200, 100, |x, _| {
            if x < 20 {
                Rgb([255, 0, 0])
            } else if x >= 180 {
                Rgb([0, 0, 255])
            } else {
                Rgb([255, 255, 255])
            }
        });
        let img = dial_feedback(&png_url(source.into()), crate::hub::DIAL_IMAGE_SIZE).unwrap().render(crate::hub::DIAL_IMAGE_SIZE);

        assert_eq!(img.dimensions(), (240, 240));
        assert_eq!(*img.get_pixel(120, 59), Rgb([0, 0, 0]));
        assert_eq!(*img.get_pixel(120, 60), Rgb([255, 255, 255]));
        assert_eq!(*img.get_pixel(120, 179), Rgb([255, 255, 255]));
        assert_eq!(*img.get_pixel(120, 180), Rgb([0, 0, 0]));
        assert_eq!(*img.get_pixel(0, 120), Rgb([255, 0, 0]));
        assert_eq!(*img.get_pixel(239, 120), Rgb([0, 0, 255]));
    }

    #[test]
    fn transparent_feedback_is_composited_on_black() {
        let source = RgbaImage::from_pixel(100, 200, Rgba([200, 100, 50, 128]));
        let img = dial_feedback(&png_url(source.into()), crate::hub::DIAL_IMAGE_SIZE).unwrap().render(crate::hub::DIAL_IMAGE_SIZE);

        assert_eq!(img.dimensions(), (240, 240));
        assert_eq!(*img.get_pixel(59, 120), Rgb([0, 0, 0]));
        assert_eq!(*img.get_pixel(120, 120), Rgb([100, 50, 25]));
        assert_eq!(*img.get_pixel(180, 120), Rgb([0, 0, 0]));
    }

    #[test]
    fn invalid_feedback_returns_error() {
        for url in ["invalid", "data:image/png,raw", "data:image/png;base64,%%%", "data:image/png;base64,aGVsbG8="] {
            assert!(dial_feedback(url, crate::hub::DIAL_IMAGE_SIZE).is_err(), "{url}");
        }
    }

    #[test]
    fn keypad_images_still_fill_the_square() {
        let source = RgbImage::from_pixel(200, 100, Rgb([255, 255, 255]));
        let url = png_url(source.into());
        let img = image::load_from_memory(&to_jpeg(Some(&url), crate::hub::KEY_IMAGE_SIZE).unwrap()).unwrap().to_rgb8();

        assert_eq!(img.dimensions(), (72, 72));
        assert_eq!(*img.get_pixel(36, 0), Rgb([255, 255, 255]));
        assert_eq!(*img.get_pixel(36, 71), Rgb([255, 255, 255]));
    }

    #[test]
    fn actual_b1_jpegs_have_correct_ring_levels_including_endpoints() {
        let fixtures: &[(u8, &[u8])] = &[
            (0, include_bytes!("../tests/fixtures/encoder-b1/b1-0.jpg")),
            (1, include_bytes!("../tests/fixtures/encoder-b1/b1-1.jpg")),
            (5, include_bytes!("../tests/fixtures/encoder-b1/b1-5.jpg")),
            (19, include_bytes!("../tests/fixtures/encoder-b1/b1-19.jpg")),
            (50, include_bytes!("../tests/fixtures/encoder-b1/b1-50.jpg")),
            (55, include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")),
            (99, include_bytes!("../tests/fixtures/encoder-b1/b1-99.jpg")),
            (100, include_bytes!("../tests/fixtures/encoder-b1/b1-100.jpg")),
        ];
        for &(percent, jpeg) in fixtures {
            let strip = image::load_from_memory(jpeg).unwrap().to_rgb8();
            let level = b1_level(&strip).unwrap_or_else(|| panic!("nierozpoznany $B1: {percent}%"));
            assert!((level * 100.0 - percent as f32).abs() <= 1.0, "{percent}%: {level}");
            let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg));
            let dial = dial_feedback(&url, 240).unwrap().render(240);
            assert_eq!(dial.dimensions(), (240, 240));
            // Lewy dolny punkt należy do ścieżki pierścienia, poza wyciętym tekstem.
            let expected = if percent > 63 { Rgb([124, 58, 237]) } else { crate::render::TRACK };
            assert_eq!(*dial.get_pixel(50, 190), expected, "{percent}%");
            assert!(dial.enumerate_pixels().any(|(x, y, p)| x > 65 && x < 175 && y > 80 && y < 140 && p[0] > 200));
        }
    }

    #[test]
    fn missing_or_custom_bar_preserves_original_feedback() {
        let mut img = image::load_from_memory(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")).unwrap().to_rgb8();
        for x in 76..184 {
            for y in 74..86 {
                img.put_pixel(x, y, Rgb([40, 120, 220]));
            }
        }
        assert_eq!(b1_level(&img), None);
        let expected = fit_rgb(&DynamicImage::ImageRgb8(img.clone()), 240, false);
        assert_eq!(dial_feedback(&png_url(img.into()), 240).unwrap().render(240), expected);
    }

    #[test]
    fn encoder_row_selects_the_correct_knob_or_roller_before_reading_b1() {
        let sources: &[(f32, &[u8])] = &[
            (0.0, include_bytes!("../tests/fixtures/encoder-b1/b1-0.jpg")),
            (0.55, include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")),
            (1.0, include_bytes!("../tests/fixtures/encoder-b1/b1-100.jpg")),
        ];
        for count in [2, 3] {
            let mut row = RgbImage::new(200 * count, 100);
            for (position, (_, bytes)) in sources.iter().take(count as usize).enumerate() {
                let strip = image::load_from_memory(bytes).unwrap().to_rgb8();
                image::imageops::overlay(&mut row, &strip, position as i64 * 200, 0);
            }
            let url = png_url(row.into());
            for (position, &(expected, _)) in sources.iter().take(count as usize).enumerate() {
                let DialFeedback::Ring(ring) = dial_feedback_region(&url, 240, Some((position as u8, count as u8))).unwrap() else {
                    panic!("powinien zostać rozpoznany pojedynczy $B1")
                };
                assert!((ring.level - expected).abs() <= 0.01, "{position}/{count}: {}", ring.level);
            }
            assert!(dial_feedback_region(&url, 240, Some((count as u8, count as u8))).is_err());
            assert!(dial_feedback_region(&url, 240, Some((0, 0))).is_err());
        }
    }

    #[test]
    fn fully_transparent_b1_does_not_create_a_spurious_ring() {
        let strip = image::load_from_memory(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")).unwrap().to_rgb8();
        let transparent = RgbaImage::from_fn(200, 100, |x, y| {
            let p = strip.get_pixel(x, y).0;
            Rgba([p[0], p[1], p[2], 0])
        });
        let DialFeedback::Image(img) = dial_feedback(&png_url(transparent.into()), 240).unwrap() else {
            panic!("niewidoczny pasek nie jest feedbackiem")
        };
        assert!(img.pixels().all(|p| *p == Rgb([0, 0, 0])));
    }

    #[test]
    fn b1_layout_reset_without_a_value_is_incomplete_feedback() {
        let mut strip = image::load_from_memory(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")).unwrap().to_rgb8();
        for y in 40..72 {
            for x in 76..184 {
                strip.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        let bytes = crate::render::jpeg_with_quality(&strip, 75);
        let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
        assert!(matches!(dial_feedback(&url, 240).unwrap(), DialFeedback::Incomplete));
    }

    #[test]
    fn value_is_taken_from_text_even_when_indicator_differs() {
        let mut img = image::load_from_memory(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")).unwrap().to_rgb8();
        let value = text_crop(&img, (76, 40, 108, 32)).unwrap();
        // Celowo inny wskaźnik: liczba na LCD nadal musi pochodzić z tekstu HA.
        for x in 76..184 {
            for y in 79..=80 {
                img.put_pixel(x, y, Rgb([169, 169, 169]));
            }
        }
        let dial = dial_feedback(&png_url(img.into()), 240).unwrap().render(240);
        let mut expected = crate::render::gauge(240, 0.0, false, "", "");
        paste_text(&mut expected, &value, 240 * 64 / 100, 240 * 26 / 100, 240 * 47 / 100);
        assert!(
            image::imageops::crop_imm(&dial, 40, 76, 160, 70).to_image()
                == image::imageops::crop_imm(&expected, 40, 76, 160, 70).to_image()
        );
    }
}
