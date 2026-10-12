//! Sprawdza części pluginu zależne od systemu (bez sprzętu MasterHUB):
//! czcionki i rysowanie, czujniki oraz odczyt głośności.
//! Użycie: platform_check <katalog na podgląd PNG>

#[cfg(target_os = "linux")]
#[path = "../src/host.rs"]
mod host;
#[path = "../src/audio.rs"]
#[allow(dead_code)]
mod audio;
#[path = "../src/render.rs"]
#[allow(dead_code)]
mod render;
#[path = "../src/i18n.rs"]
#[allow(dead_code)]
mod i18n;
#[path = "../src/sensors.rs"]
#[allow(dead_code)]
mod sensors;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());

    let img = render::clock(240, "12:34", Some("pt, 10 paź"));
    let lit = img.pixels().filter(|p| p.0[0] > 200).count();
    img.save(format!("{out}/clock-check.png")).unwrap();
    println!("czcionka/zegar: {} jasnych pikseli (0 = brak czcionki)", lit);

    match sensors::all() {
        Ok(map) => {
            println!("czujniki: {}", map.len());
            let mut list: Vec<_> = map.values().collect();
            list.sort_by(|a, b| a.id.cmp(&b.id));
            for s in list.iter().take(8) {
                println!("  {:32} {:12} {:20} {:?} {}", s.id, s.hardware, s.name, s.value, s.unit);
            }
        }
        Err(e) => println!("czujniki: błąd {e}"),
    }

    println!("głośność master: {:?}", audio::run("master", audio::Op::Step(0.0)));
    println!("aktywne okno: {:?}", audio::focused_app());
}
