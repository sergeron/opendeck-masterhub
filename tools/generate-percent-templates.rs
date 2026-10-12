//! Uruchamiane jako przykład w streamdeck-strip-render, nie w pluginie.
//! cargo run --release --example masterhub_percent_templates -- B1.json percent-templates.png
use image::{
    ImageEncoder, RgbImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
    imageops,
};
use serde_json::Value;
use std::{fs, path::PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let layout: Value = serde_json::from_str(&fs::read_to_string(args.next().expect("B1.json")).unwrap()).unwrap();
    let output = PathBuf::from(args.next().expect("atlas PNG"));
    let mut atlas = RgbImage::new(108 * 10, 32 * ((1211 + 9) / 10));
    for index in 0..1211 {
        let text = if index < 1000 {
            format!("{index}%")
        } else if index < 1010 {
            format!("{:02}%", index - 1000)
        } else if index < 1110 {
            format!("{:03}%", index - 1010)
        } else {
            format!("-{}%", index - 1110)
        };
        let mut layout = layout.clone();
        for item in layout["items"].as_array_mut().unwrap() {
            if item["key"] == "value" {
                item["value"] = text.clone().into();
            }
        }
        let image = streamdeck_strip_render::render_to_image(layout, None).unwrap().to_rgb8();
        let value = imageops::crop_imm(&image, 76, 40, 108, 32).to_image();
        imageops::overlay(&mut atlas, &value, (index % 10) * 108, (index / 10) * 32);
    }
    let atlas = image::DynamicImage::ImageRgb8(atlas).to_luma8();
    PngEncoder::new_with_quality(fs::File::create(output).unwrap(), CompressionType::Best, FilterType::Adaptive)
        .write_image(atlas.as_raw(), atlas.width(), atlas.height(), image::ColorType::L8.into())
        .unwrap();
}
