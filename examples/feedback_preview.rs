//! Podgląd i pomiar lokalnej konwersji feedbacku (bez HA, OpenDecka i USB).
//! cargo run --release --example feedback_preview -- tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png

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
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let source = std::fs::read(args.next().expect("obraz feedbacku")).unwrap();
    let original = image::load_from_memory(&source).unwrap();
    let output = args.next().expect("wynik PNG");
    let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(source));
    let feedback = images::dial_feedback(&url, hub::DIAL_IMAGE_SIZE).unwrap();
    if matches!(feedback, images::DialFeedback::Incomplete) {
        eprintln!("Niepełny $B1: układ nie zawiera jeszcze potwierdzonej wartości.");
        std::process::exit(1);
    }
    let img = feedback.render(hub::DIAL_IMAGE_SIZE);
    img.save(&output).unwrap();
    let old_size = images::fit_jpeg(&original, 240, false, 95).unwrap().len();
    let jpeg = render::jpeg_with_quality(&img, 80);
    std::fs::write(std::path::Path::new(&output).with_extension("jpg"), &jpeg).unwrap();
    let started = Instant::now();
    for _ in 0..100 {
        let img = images::dial_feedback(&url, hub::DIAL_IMAGE_SIZE).unwrap().render(hub::DIAL_IMAGE_SIZE);
        std::hint::black_box(render::jpeg_with_quality(&img, 80));
    }
    println!(
        "Konwersja + JPEG: {:.2} ms/klatkę (średnia ze 100); JPEG: {} B, poprzedni pasek: {} B",
        started.elapsed().as_secs_f64() * 10.0,
        jpeg.len(),
        old_size
    );
}
