//! Sprawdza odczyt czujników z LibreHardwareMonitor tak, jak robi to plugin.
#[path = "../src/i18n.rs"]
#[allow(dead_code)]
mod i18n;
#[allow(dead_code)]
#[path = "../src/sensors.rs"]
mod sensors;

fn main() {
    let all = sensors::all().expect("brak odczytu");
    println!("czujników: {}", all.len());
    for id in ["/gpu-nvidia/0/power/0", "/gpu-nvidia/0/temperature/0", "/gpu-nvidia/0/load/0", "/intelcpu/0/power/0", "/intelcpu/0/temperature/16", "/intelcpu/0/load/0", "/ram/data/0"] {
        let s = &all[id];
        println!("{id:30} {:28} {:?} {}", s.hardware, s.value, s.unit);
    }
}
