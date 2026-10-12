//! Odczyty czujników sprzętowych, zależnie od systemu:
//!
//! - Windows: LibreHardwareMonitor (wbudowany serwer WWW, data.json); id czujników jak w LHM,
//!   np. "/intelcpu/0/temperature/16",
//! - Linux: /sys/class/hwmon (temperatury, wentylatory, moc, napięcia) oraz obciążenie CPU
//!   i zajęta pamięć z /proc; id np. "/hwmon/coretemp/temp1",
//! - macOS: brak (system nie udostępnia czujników bez dodatkowych programów).
//!
//! Na Windows dochodzi FPS aktywnego okna z RivaTuner Statistics Server (id "/rtss/fps"
//! i "/rtss/frametime") – działa także bez LibreHardwareMonitor.
//!
//! Plugin odczytuje wszystko raz na odświeżenie, niezależnie od liczby akcji.

use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// Pojedynczy czujnik.
#[derive(Clone, Debug, Serialize)]
pub struct Sensor {
    pub id: String,
    /// Urządzenie, np. "Intel Core i5-14600K".
    pub hardware: String,
    pub name: String,
    pub kind: String,
    pub value: Option<f64>,
    pub unit: String,
}

/// Ostatni odczyt (ważny przez chwilę, żeby wiele akcji nie odpytywało źródła osobno).
static CACHE: LazyLock<Mutex<Option<(Instant, Result<HashMap<String, Sensor>, String>)>>> =
    LazyLock::new(Default::default);

#[cfg(windows)]
use lhm::fetch;

#[cfg(windows)]
mod lhm {
    use super::Sensor;
    use serde::Deserialize;
    use std::{collections::HashMap, time::Duration};

    pub const URL: &str = "http://127.0.0.1:8085/data.json";

    #[derive(Deserialize)]
    struct Node {
        #[serde(rename = "Text", default)]
        text: String,
        #[serde(rename = "Value", default)]
        value: String,
        #[serde(rename = "SensorId", default)]
        sensor_id: Option<String>,
        #[serde(rename = "Type", default)]
        kind: Option<String>,
        #[serde(rename = "Children", default)]
        children: Vec<Node>,
    }

    /// "60,0 °C" -> (60.0, "°C"); "1149 RPM" -> (1149.0, "RPM").
    fn parse_value(text: &str) -> (Option<f64>, String) {
        let text = text.trim();
        let split = text.find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.' || c == '-')).unwrap_or(text.len());
        let (num, unit) = text.split_at(split);
        (num.replace(',', ".").parse().ok(), unit.trim().to_string())
    }

    fn collect(node: &Node, parents: &mut Vec<String>, out: &mut HashMap<String, Sensor>) {
        if node.children.is_empty() {
            if let Some(id) = &node.sensor_id {
                let (value, unit) = parse_value(&node.value);
                // parents: [komputer, urządzenie, (podurządzenie...), grupa typu]
                let hardware = parents.iter().rev().nth(1).cloned().unwrap_or_default();
                out.insert(
                    id.clone(),
                    Sensor {
                        id: id.clone(),
                        hardware,
                        name: node.text.clone(),
                        kind: node.kind.clone().unwrap_or_default(),
                        value,
                        unit,
                    },
                );
            }
            return;
        }
        parents.push(node.text.clone());
        for child in &node.children {
            collect(child, parents, out);
        }
        parents.pop();
    }

    pub fn fetch() -> Result<HashMap<String, Sensor>, String> {
        let body = ureq::get(URL)
            .timeout(Duration::from_secs(2))
            .call()
            .map_err(|e| format!("{} ({e})", crate::i18n::tr("LibreHardwareMonitor nie odpowiada", "LibreHardwareMonitor is not responding")))?
            .into_string()
            .map_err(|e| e.to_string())?;
        let root: Node = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        collect(&root, &mut vec![], &mut out);
        Ok(out)
    }
}

/// FPS z RivaTuner Statistics Server: czyta jego pamięć współdzieloną ("RTSSSharedMemoryV2",
/// format z RTSSSharedMemory.h z SDK RTSS) – bez uprawnień administratora.
#[cfg(windows)]
pub mod rtss {
    use super::Sensor;
    use std::collections::HashMap;
    use windows::{
        Win32::{
            Foundation::CloseHandle,
            System::{Memory::*, SystemInformation::GetTickCount},
            UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        },
        core::w,
    };

    const SIGNATURE: u32 = u32::from_be_bytes(*b"RTSS");
    // Pola wpisu aplikacji: pid, nazwa (260 B), flagi, czas0, czas1, klatki, czas klatki (µs).
    const PID: usize = 0;
    const TIME0: usize = 268;
    const TIME1: usize = 272;
    const FRAMES: usize = 276;
    const FRAME_TIME: usize = 280;
    const ENTRY_MIN: usize = 284;

    /// (FPS, czas klatki w ms) procesu aktywnego okna, jeśli RTSS go śledzi.
    fn foreground() -> Option<(f64, f64)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let map = OpenFileMappingW(FILE_MAP_READ.0, false, w!("RTSSSharedMemoryV2")).ok()?;
            let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0);
            let result = if view.Value.is_null() {
                None
            } else {
                let mut info = MEMORY_BASIC_INFORMATION::default();
                let n = VirtualQuery(Some(view.Value), &mut info, size_of::<MEMORY_BASIC_INFORMATION>());
                let len = if n == 0 { 0 } else { info.RegionSize };
                let mem = std::slice::from_raw_parts(view.Value as *const u8, len);
                read(mem, pid, GetTickCount())
            };
            let _ = UnmapViewOfFile(view);
            let _ = CloseHandle(map);
            result
        }
    }

    fn read(mem: &[u8], pid: u32, now: u32) -> Option<(f64, f64)> {
        let u32_at = |off: usize| -> Option<u32> { Some(u32::from_le_bytes(mem.get(off..off + 4)?.try_into().ok()?)) };
        if u32_at(0)? != SIGNATURE || u32_at(4)? < 0x0002_0000 {
            return None;
        }
        let (entry_size, offset, count) = (u32_at(8)? as usize, u32_at(12)? as usize, u32_at(16)? as usize);
        if entry_size < ENTRY_MIN {
            return None;
        }
        for i in 0..count.min(1024) {
            let base = offset + i * entry_size;
            if u32_at(base + PID)? != pid {
                continue;
            }
            let (t0, t1, frames) = (u32_at(base + TIME0)?, u32_at(base + TIME1)?, u32_at(base + FRAMES)?);
            // RTSS przelicza średnią mniej więcej co sekundę; stary odczyt = gra nie renderuje.
            if t1 <= t0 || now.wrapping_sub(t1) > 3000 {
                return None;
            }
            let fps = 1000.0 * frames as f64 / (t1 - t0) as f64;
            let frame_ms = u32_at(base + FRAME_TIME)? as f64 / 1000.0;
            return Some((fps, frame_ms));
        }
        None
    }

    /// FPS aktywnego okna. Wpisy są zawsze (wartość `None`, gdy brak RTSS lub okno nie jest grą).
    pub fn add(out: &mut HashMap<String, Sensor>) {
        let reading = foreground();
        let mut sensor = |id: &str, name: &str, kind: &str, value: Option<f64>, unit: &str| {
            out.insert(
                id.into(),
                Sensor {
                    id: id.into(),
                    hardware: crate::i18n::tr("RivaTuner (aktywne okno)", "RivaTuner (focused window)").into(),
                    name: name.into(),
                    kind: kind.into(),
                    value,
                    unit: unit.into(),
                },
            );
        };
        sensor("/rtss/fps", "FPS", "Framerate", reading.map(|r| r.0), "FPS");
        sensor("/rtss/frametime", crate::i18n::tr("Czas klatki", "Frame time"), "Frametime", reading.map(|r| r.1), "ms");
    }
}

#[cfg(target_os = "linux")]
use hwmon::fetch;

#[cfg(target_os = "linux")]
mod hwmon {
    use super::Sensor;
    use std::{collections::HashMap, fs, path::Path, sync::Mutex};

    fn read(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    /// (typ czujnika, prefiks pliku, końcówka pliku, dzielnik do jednostki, jednostka)
    const KINDS: [(&str, &str, &str, f64, &str); 4] = [
        ("Temperature", "temp", "input", 1000.0, "°C"),
        ("Fan", "fan", "input", 1.0, "RPM"),
        ("Power", "power", "average", 1_000_000.0, "W"),
        ("Voltage", "in", "input", 1000.0, "V"),
    ];

    fn hwmon(out: &mut HashMap<String, Sensor>) {
        let Ok(dirs) = fs::read_dir("/sys/class/hwmon") else { return };
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut dirs: Vec<_> = dirs.flatten().map(|d| d.path()).collect();
        dirs.sort();
        for dir in dirs {
            let name = read(&dir.join("name")).unwrap_or_else(|| "hwmon".into());
            // Kilka urządzeń o tej samej nazwie (np. dwa dyski nvme) – dopisujemy numer.
            let n = *seen.entry(name.clone()).and_modify(|n| *n += 1).or_insert(0);
            let hw = if n == 0 { name.clone() } else { format!("{name}{n}") };
            let Ok(files) = fs::read_dir(&dir) else { continue };
            for file in files.flatten() {
                let fname = file.file_name().to_string_lossy().into_owned();
                for (kind, prefix, suffix, div, unit) in KINDS {
                    let Some(stem) = fname.strip_suffix(&format!("_{suffix}")) else { continue };
                    if !stem.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
                        continue;
                    }
                    let label = read(&dir.join(format!("{stem}_label"))).unwrap_or_else(|| stem.to_string());
                    let value = read(&file.path()).and_then(|v| v.parse::<f64>().ok()).map(|v| v / div);
                    let id = format!("/hwmon/{hw}/{stem}");
                    out.insert(
                        id.clone(),
                        Sensor { id, hardware: hw.clone(), name: label, kind: kind.into(), value, unit: unit.into() },
                    );
                }
            }
        }
    }

    /// Poprzedni odczyt /proc/stat: (czas całkowity, czas bezczynności).
    static LAST_CPU: Mutex<Option<(u64, u64)>> = Mutex::new(None);

    fn cpu_load(out: &mut HashMap<String, Sensor>) {
        let Some(stat) = read(Path::new("/proc/stat")) else { return };
        let Some(line) = stat.lines().next() else { return };
        let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        if v.len() < 5 {
            return;
        }
        let (total, idle) = (v.iter().sum::<u64>(), v[3] + v[4]);
        let mut last = LAST_CPU.lock().unwrap();
        let value = last.and_then(|(t, i)| {
            let (dt, di) = (total.saturating_sub(t), idle.saturating_sub(i));
            (dt > 0).then(|| 100.0 * (1.0 - di as f64 / dt as f64))
        });
        *last = Some((total, idle));
        let id = "/cpu/load".to_string();
        out.insert(
            id.clone(),
            Sensor { id, hardware: "CPU".into(), name: "CPU Total".into(), kind: "Load".into(), value, unit: "%".into() },
        );
    }

    fn memory(out: &mut HashMap<String, Sensor>) {
        let Some(info) = read(Path::new("/proc/meminfo")) else { return };
        let kb = |key: &str| -> Option<f64> { info.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse().ok() };
        let (Some(total), Some(avail)) = (kb("MemTotal:"), kb("MemAvailable:")) else { return };
        let id = "/ram/used".to_string();
        let used = (total - avail) / 1024.0 / 1024.0;
        out.insert(
            id.clone(),
            Sensor { id, hardware: "RAM".into(), name: "Memory Used".into(), kind: "Data".into(), value: Some(used), unit: "GB".into() },
        );
    }

    pub fn fetch() -> Result<HashMap<String, Sensor>, String> {
        let mut out = HashMap::new();
        hwmon(&mut out);
        cpu_load(&mut out);
        memory(&mut out);
        Ok(out)
    }
}

#[cfg(target_os = "macos")]
fn fetch() -> Result<HashMap<String, Sensor>, String> {
    Err(crate::i18n::tr("Czujniki nie są obsługiwane na macOS", "Sensors are not supported on macOS").into())
}

/// Błąd LibreHardwareMonitor z ostatniego odczytu (czujniki LHM pokazują wtedy "LHM?").
#[cfg(windows)]
static LHM_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Błąd źródła danego czujnika, jeśli jest niedostępne.
pub fn source_error(id: &str) -> Option<String> {
    #[cfg(windows)]
    if !id.starts_with("/rtss/") {
        return LHM_ERROR.lock().unwrap().clone();
    }
    let _ = id;
    None
}

/// Wszystkie czujniki (z cache do ~0,8 s).
pub fn all() -> Result<HashMap<String, Sensor>, String> {
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, result)) = cache.as_ref() {
        if at.elapsed() < Duration::from_millis(800) {
            return result.clone();
        }
    }
    let result = fetch();
    // FPS z RivaTunera działa niezależnie od LibreHardwareMonitor.
    #[cfg(windows)]
    let result = {
        let mut map = result.clone().unwrap_or_default();
        rtss::add(&mut map);
        *LHM_ERROR.lock().unwrap() = result.err();
        Ok(map)
    };
    *cache = Some((Instant::now(), result.clone()));
    result
}
