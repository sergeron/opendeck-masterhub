//! Porównuje standardowy i zoptymalizowany JPEG na tej samej klatce LCD.
#[path = "../src/hub.rs"]
#[allow(dead_code)]
mod hub;
#[path = "../src/images.rs"]
#[allow(dead_code)]
mod images;
#[path = "../src/render.rs"]
#[allow(dead_code)]
mod render;

use base64::Engine;
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

fn main() {
    let mut args = std::env::args().skip(1);
    let bytes = std::fs::read(args.next().expect("obraz feedbacku")).unwrap();
    let output = std::path::PathBuf::from(args.next().expect("katalog wyjściowy"));
    std::fs::create_dir_all(&output).unwrap();
    let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    let image = images::dial_feedback(&url, hub::DIAL_IMAGE_SIZE).unwrap().render(hub::DIAL_IMAGE_SIZE);
    for quality in [80, 85, 90, 95] {
        for (name, sampling) in [("444", SamplingFactor::F_1_1), ("420", SamplingFactor::F_2_2)] {
            let started = std::time::Instant::now();
            let mut jpeg = Vec::new();
            let mut encoder = Encoder::new(&mut jpeg, quality);
            encoder.set_sampling_factor(sampling);
            encoder.set_optimized_huffman_tables(true);
            encoder.encode(image.as_raw(), image.width() as u16, image.height() as u16, ColorType::Rgb).unwrap();
            let encoded_for = started.elapsed();
            let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
            let error: u64 = image.as_raw().iter().zip(decoded.as_raw()).map(|(a, b)| a.abs_diff(*b) as u64).sum();
            let error = error as f64 / image.as_raw().len() as f64;
            println!(
                "Q{quality} {name}: {} B / {} raportów HID; {:.2} ms; MAE {:.2}",
                jpeg.len(),
                jpeg.len().div_ceil(312),
                encoded_for.as_secs_f64() * 1000.0,
                error
            );
            std::fs::write(output.join(format!("optimized-{quality}-{name}.jpg")), jpeg).unwrap();
        }
    }
}
