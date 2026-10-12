//! Akcje pluginu widoczne w OpenDecku: podświetlenie modułów i głośność.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{LazyLock, Mutex},
};

use crate::{FADER_VALUES, MODULES, audio, hub, hub::Kind, with_writer};

pub const LIGHTING: &str = "io.github.opendeck-masterhub.lighting";
pub const VOLUME: &str = "io.github.opendeck-masterhub.volume";
pub const GIF: &str = "io.github.opendeck-masterhub.gif";
pub const SENSOR: &str = "io.github.opendeck-masterhub.sensor";
pub const CLOCK: &str = "io.github.opendeck-masterhub.clock";
pub const CS2: &str = "io.github.opendeck-masterhub.cs2";

// --- Zegar -------------------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct ClockSettings {
    /// Data pod godziną ("czw, 9 paź").
    date: bool,
    /// Sekundy w godzinie.
    seconds: bool,
    /// Wciśnięcie: "none", "open" (plik/program/URL), "key" (skrót klawiszowy)
    /// albo "mute" (wycisz / włącz dźwięk jak akcja Głośność).
    press: String,
    /// Co otworzyć albo jaki skrót wysłać.
    target: String,
    /// Co wyciszać przy press = "mute": "master", "mic" albo "focused".
    #[serde(rename = "muteTarget")]
    mute_target: String,
}

impl Default for ClockSettings {
    fn default() -> Self {
        ClockSettings {
            date: true,
            seconds: false,
            press: "none".into(),
            target: String::new(),
            mute_target: "master".into(),
        }
    }
}

pub fn clock_press(settings: &serde_json::Value) {
    let s: ClockSettings = parse(settings);
    if s.press == "mute" {
        // Jak wciśnięcie akcji Głośność: przełącza wyciszenie i pokazuje poziom na pokrętle.
        volume_toggle_mute(&serde_json::json!({ "target": s.mute_target }));
        return;
    }
    run_press(&s.press, &s.target);
}

struct ClockInstance {
    settings: ClockSettings,
    /// Ostatnio narysowany tekst (rysujemy tylko po zmianie).
    last: String,
    /// Urządzenie, jeśli zegar stoi na pokrętle – wtedy obraz idzie prosto na jego ekran.
    dial: Option<String>,
}

/// Widoczne zegary: kontekst -> instancja.
static CLOCK_INSTANCES: LazyLock<Mutex<HashMap<String, ClockInstance>>> = LazyLock::new(Default::default);

pub fn clock_register(context: &str, device: &str, settings: &serde_json::Value) {
    let s: ClockSettings = parse(settings);
    let is_dial = MODULES.lock().unwrap().get(device).is_some_and(|m| m.kind == Kind::Dial);
    let dial = is_dial.then(|| device.to_string());
    CLOCK_INSTANCES.lock().unwrap().insert(context.to_string(), ClockInstance { settings: s, last: String::new(), dial });
}

/// Zwraca urządzenie pokrętła, jeśli zegar był na nim (trzeba wtedy przywrócić obraz OpenDecka).
pub fn clock_unregister(context: &str) -> Option<String> {
    let dial = CLOCK_INSTANCES.lock().unwrap().remove(context).and_then(|c| c.dial);
    dial.filter(|d| crate::overlay::clear_direct(d))
}

/// Bieżąca godzina ("14:05" albo "14:05:09") i data ("czw, 9 paź").
pub fn clock_text(seconds: bool) -> (String, String) {
    use chrono::{Datelike, Local, Timelike};
    const DAYS: [&str; 7] = ["pon", "wt", "śr", "czw", "pt", "sob", "niedz"];
    const MONTHS: [&str; 12] = ["sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru"];
    let now = Local::now();
    let date = format!("{}, {} {}", DAYS[now.weekday().num_days_from_monday() as usize], now.day(), MONTHS[now.month0() as usize]);
    let time = if seconds {
        format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
    } else {
        format!("{:02}:{:02}", now.hour(), now.minute())
    };
    (time, date)
}

/// Nowe obrazy zegarów (kontekst, data URL) – tylko gdy zmienił się wyświetlany tekst.
pub fn clock_updates() -> Vec<(String, String)> {
    let mut out = vec![];
    for (context, c) in CLOCK_INSTANCES.lock().unwrap().iter_mut() {
        let (s, last) = (&c.settings, &mut c.last);
        let (time, date) = clock_text(s.seconds);
        let key = format!("{time}|{}", if s.date { date.as_str() } else { "" });
        if *last != key {
            *last = key;
            let img = crate::render::clock(hub::DIAL_IMAGE_SIZE, &time, s.date.then_some(date.as_str()));
            // Na pokrętle wysyłamy obraz 240×240 bezpośrednio – OpenDeck przeskalowałby go
            // do 144×144 i z powrotem, co rozmywa drobny tekst.
            if let Some(device) = &c.dial {
                crate::overlay::set_direct(device, img.clone());
            }
            out.push((context.clone(), crate::render::png_data_url(&img)));
        }
    }
    out
}

// --- Czujnik (LibreHardwareMonitor) ------------------------------------------------------------

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct SensorSettings {
    /// SensorId z LibreHardwareMonitor, np. "/intelcpu/0/temperature/16".
    sensor: String,
    /// Etykieta nad wartością (pusta = brak).
    label: String,
    /// Miejsca po przecinku; -1 = automatycznie.
    decimals: i8,
    /// Próg ostrzeżenia (czerwone tło po przekroczeniu); brak = wyłączony.
    warn: Option<f64>,
}

impl Default for SensorSettings {
    fn default() -> Self {
        SensorSettings { sensor: String::new(), label: String::new(), decimals: -1, warn: None }
    }
}

/// Widoczne akcje czujników: kontekst -> (ustawienia, ostatni tytuł, ostatni obraz).
type SensorInstances = HashMap<String, (SensorSettings, Option<String>, Option<&'static str>)>;
static SENSOR_INSTANCES: LazyLock<Mutex<SensorInstances>> = LazyLock::new(Default::default);

pub fn sensor_register(context: &str, settings: &serde_json::Value) {
    let s: SensorSettings = parse(settings);
    SENSOR_INSTANCES.lock().unwrap().insert(context.to_string(), (s, None, None));
}

pub fn sensor_unregister(context: &str) {
    SENSOR_INSTANCES.lock().unwrap().remove(context);
}

pub fn sensors_active() -> bool {
    !SENSOR_INSTANCES.lock().unwrap().is_empty()
}

/// Wartość do wyświetlenia: "60°C", "12%", "5.3GHz", "18.2GB".
fn format_value(value: f64, unit: &str, decimals: i8) -> String {
    let (value, unit) = match unit {
        "MHz" if value >= 1000.0 => (value / 1000.0, "GHz"),
        _ => (value, unit),
    };
    let decimals = if decimals >= 0 {
        decimals as usize
    } else if unit == "GHz" || unit == "GB" || unit == "V" || value.abs() < 10.0 {
        1
    } else {
        0
    };
    format!("{value:.decimals$}{unit}")
}

/// Ikona tła zależna od typu czujnika (i stanu ostrzeżenia).
fn sensor_icon(kind: &str, warn: bool) -> &'static str {
    match (kind, warn) {
        ("Temperature", false) => "temperature",
        ("Temperature", true) => "temperature-warn",
        ("Load", false) | ("Control", false) => "load",
        ("Load", true) | ("Control", true) => "load-warn",
        ("Power", false) => "power",
        ("Power", true) => "power-warn",
        ("Fan", false) => "fan",
        ("Fan", true) => "fan-warn",
        ("Clock", false) => "clock",
        ("Clock", true) => "clock-warn",
        (_, false) => "generic",
        (_, true) => "generic-warn",
    }
}

/// Nowe tytuły i ikony do wysłania: (kontekst, tytuł jeśli zmieniony, ikona jeśli zmieniona).
pub fn sensor_updates(force: bool) -> Vec<(String, Option<String>, Option<&'static str>)> {
    let all = crate::sensors::all();
    let mut instances = SENSOR_INSTANCES.lock().unwrap();
    let mut out = vec![];
    for (context, (s, last_title, last_icon)) in instances.iter_mut() {
        let (value_text, kind, warn) = match &all {
            Err(_) => ("LHM?".to_string(), "", false),
            Ok(_) if s.sensor.is_empty() => ("wybierz".to_string(), "", false),
            Ok(_) if crate::sensors::source_error(&s.sensor).is_some() => ("LHM?".to_string(), "", false),
            Ok(map) => match map.get(&s.sensor) {
                Some(sensor) => match sensor.value {
                    Some(v) => (
                        format_value(v, &sensor.unit, s.decimals),
                        sensor.kind.as_str(),
                        s.warn.is_some_and(|w| v >= w),
                    ),
                    None => ("—".to_string(), sensor.kind.as_str(), false),
                },
                None => ("brak".to_string(), "", false),
            },
        };
        let title = if s.label.trim().is_empty() { value_text } else { format!("{}\n{value_text}", s.label.trim()) };
        let icon = sensor_icon(kind, warn);
        let new_title = (force || last_title.as_deref() != Some(title.as_str())).then(|| title.clone());
        let new_icon = (force || *last_icon != Some(icon)).then_some(icon);
        if new_title.is_some() || new_icon.is_some() {
            *last_title = Some(title);
            *last_icon = Some(icon);
            out.push((context.clone(), new_title, new_icon));
        }
    }
    out
}

/// Lista czujników dla panelu ustawień: `{ sensors: [...] }` albo `{ error: "..." }`.
pub fn sensor_list_json() -> serde_json::Value {
    match crate::sensors::all() {
        Ok(map) => {
            let mut list: Vec<_> = map.into_values().collect();
            if let Some(e) = crate::sensors::source_error("") {
                // RivaTuner działa, ale LibreHardwareMonitor nie – panel pokazuje błąd i listę z FPS.
                let mut list = list;
                list.sort_by(|a, b| (&a.hardware, &a.kind, &a.id).cmp(&(&b.hardware, &b.kind, &b.id)));
                return serde_json::json!({ "sensors": list, "error": e });
            }
            list.sort_by(|a, b| (&a.hardware, &a.kind, &a.id).cmp(&(&b.hardware, &b.kind, &b.id)));
            serde_json::json!({ "sensors": list })
        }
        Err(e) => serde_json::json!({ "error": e }),
    }
}

// --- Animacja GIF ----------------------------------------------------------------------------

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct GifSettings {
    /// Ścieżka do zapisanego GIF-a (w katalogu pluginu).
    file: String,
    /// Oryginalna nazwa pliku – tylko do wyświetlenia w panelu.
    name: String,
    /// "cover" (przytnij, wypełnij) albo "contain" (zmieść całość).
    fit: String,
    /// "key" (ten klawisz), "module" (cały moduł 15xKey, pocięty na kafelki)
    /// albo "copies" (ta sama animacja na każdym klawiszu modułu).
    span: String,
    /// Odstęp między klawiszami przy "module", w % szerokości klawisza.
    gap: u8,
    /// "none", "open" (plik/program/URL) albo "key" (skrót klawiszowy).
    press: String,
    /// Co otworzyć albo jaki skrót wysłać.
    target: String,
    /// Plik przesłany z panelu (data URL) – zapisywany i usuwany z ustawień.
    #[serde(skip_serializing_if = "Option::is_none")]
    upload: Option<String>,
}

impl Default for GifSettings {
    fn default() -> Self {
        GifSettings {
            file: String::new(),
            name: String::new(),
            fit: "cover".into(),
            span: "key".into(),
            gap: 30,
            press: "none".into(),
            target: String::new(),
            upload: None,
        }
    }
}

/// Numer klawisza z współrzędnych akcji; `None`, jeśli moduł nie ma ekranu.
pub fn screen_position(device: &str, row: u8, column: u8) -> Option<(Kind, u8)> {
    let kind = MODULES.lock().unwrap().get(device).map(|m| m.kind)?;
    let (_, columns, _) = kind.layout();
    kind.has_screen().then_some((kind, row * columns + column))
}

/// Uruchamia animację według ustawień. Zwraca nowe ustawienia, jeśli trzeba je zapisać
/// (po przesłaniu pliku z panelu).
pub fn gif_apply(device: &str, row: u8, column: u8, settings: &serde_json::Value) -> Option<serde_json::Value> {
    let mut s: GifSettings = parse(settings);
    let mut changed = None;
    if let Some(upload) = s.upload.take() {
        match crate::gif::save_upload(&upload) {
            Ok(path) => {
                s.file = path.to_string_lossy().into_owned();
                changed = serde_json::to_value(&s).ok();
            }
            Err(e) => log::error!("Nie udało się zapisać GIF-a: {e}"),
        }
    }
    if let Some((kind, position)) = screen_position(device, row, column) {
        // Przełączenie między "ten klawisz" a "cały moduł" – zatrzymujemy oba warianty.
        crate::gif::stop(device, position, false);
        crate::gif::stop(device, position, true);
        if !s.file.is_empty() {
            let cover = s.fit != "contain";
            let key = hub::KEY_IMAGE_SIZE;
            let layout = if kind == Kind::Keys15 && s.span == "module" {
                crate::gif::Layout::Grid { cols: 5, rows: 3, key, gap: key * s.gap.min(100) as u32 / 100, cover }
            } else if kind == Kind::Keys15 && s.span == "copies" {
                crate::gif::Layout::Copies { count: 15, size: key, cover }
            } else {
                let size = if kind == Kind::Dial { hub::DIAL_IMAGE_SIZE } else { hub::KEY_IMAGE_SIZE };
                crate::gif::Layout::Single { size, cover }
            };
            crate::gif::play(device, kind, position, s.file.clone().into(), layout);
        }
    }
    changed
}

/// Zatrzymuje animację; zwraca true, jeśli trzeba poprosić OpenDeck o odświeżenie obrazów.
pub fn gif_stop(device: &str, row: u8, column: u8) -> bool {
    match screen_position(device, row, column) {
        Some((_, position)) => {
            let single = crate::gif::stop(device, position, false);
            let whole = crate::gif::stop(device, position, true);
            single || whole
        }
        None => false,
    }
}

/// Otwiera program, plik albo adres URL domyślną aplikacją systemu.
fn open_target(target: &str) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    // "start" działa jak okno Uruchom: programy, pliki i adresy URL.
    return std::process::Command::new("cmd").args(["/C", "start", "", target]).spawn();
    #[cfg(target_os = "macos")]
    return std::process::Command::new("open").arg(target).spawn();
    #[cfg(target_os = "linux")]
    return crate::host::command("xdg-open").arg(target).spawn();
}

pub fn gif_press(settings: &serde_json::Value) {
    let s: GifSettings = parse(settings);
    run_press(&s.press, &s.target);
}

/// Wciśnięcie akcji z opcją "Wciśnięcie": otwiera cel ("open") albo wysyła skrót ("key").
fn run_press(press: &str, target: &str) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    match press {
        "open" => {
            if let Err(e) = open_target(target) {
                log::error!("Nie udało się otworzyć {target}: {e}");
            }
        }
        "key" => {
            if let Err(e) = crate::input::send_combo(target) {
                log::error!("Skrót {target}: {e}");
            }
        }
        _ => {}
    }
}

// --- Podświetlenie ---------------------------------------------------------------------------

/// Ustawienia jednej pozycji (cały moduł albo pojedyncza dioda).
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
struct Entry {
    mode: u8,
    color: String,
    brightness: u8,
    speed: u8,
}

impl Default for Entry {
    fn default() -> Self {
        Entry { mode: 1, color: "#743adf".into(), brightness: 100, speed: 50 }
    }
}

impl Entry {
    fn rgb(&self) -> [u8; 3] {
        let hex = self.color.trim_start_matches('#');
        let c = |i: usize| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0);
        [c(0), c(2), c(4)]
    }
}

/// Ustawienia akcji: `{ press, entries: { "<moduł>:<dioda>": Entry } }`,
/// moduł = all | base | knob | fader, dioda 0 = cały moduł.
#[derive(Clone, Debug, Default)]
pub struct Lighting {
    /// Co robi wciśnięcie: "toggle" (włącz/wyłącz) albo "apply".
    press: String,
    /// (moduł, dioda, ustawienia) od najogólniejszego do najbardziej szczegółowego.
    entries: Vec<(String, u8, Entry)>,
}

impl Lighting {
    fn parse(settings: &serde_json::Value) -> Self {
        let press = settings["press"].as_str().unwrap_or("toggle").to_string();
        let mut entries: Vec<(String, u8, Entry)> = match settings["entries"].as_object() {
            Some(map) => map
                .iter()
                .filter_map(|(key, v)| {
                    let (target, led) = key.split_once(':')?;
                    Some((target.to_string(), led.parse().ok()?, serde_json::from_value(v.clone()).ok()?))
                })
                .collect(),
            // Stary format: jeden zestaw ustawień. Akcja bez żadnych ustawień (świeżo dodana)
            // niczego nie zmienia – wcześniej dostawała domyślny fiolet na wszystkich diodach.
            None if settings.get("target").is_none() => vec![],
            None => {
                let target = settings["target"].as_str().unwrap_or("all").to_string();
                let led = settings["led"].as_u64().unwrap_or(0) as u8;
                vec![(target, led, serde_json::from_value(settings.clone()).unwrap_or_default())]
            }
        };
        entries.sort_by_key(|(target, led, _)| (target != "all") as u8 + (*led > 0) as u8);
        Lighting { press, entries }
    }

    /// Nazwa modułu w kluczach ustawień ("base", "knob", "fader").
    fn module_name(kind: Kind) -> &'static str {
        match kind {
            Kind::Base => "base",
            Kind::Knob3 => "knob",
            _ => "fader",
        }
    }

    fn matches(target: &str, kind: Kind) -> bool {
        match target {
            "base" => kind == Kind::Base,
            "knob" => kind == Kind::Knob3,
            "fader" => kind == Kind::Fader,
            _ => kind.led_module().is_some(),
        }
    }

    /// Wysyła ustawienia do pasujących modułów (lub tylko do `only`).
    fn apply(&self, on: bool, only: Option<&str>) {
        let modules: Vec<(String, Kind)> = MODULES
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, m)| m.kind.led_module().is_some() && only.is_none_or(|o| o == id.as_str()))
            .map(|(id, m)| (id.clone(), m.kind))
            .collect();
        for (id, kind) in modules {
            let entries: Vec<_> = self.entries.iter().filter(|(t, _, _)| Self::matches(t, kind)).collect();
            if entries.is_empty() {
                continue;
            }
            let name = Self::module_name(kind);
            let all_leds = 1..=kind.led_count();
            with_writer(&id, |dev| {
                if !on {
                    for led in all_leds.clone() {
                        led_state_update(name, led, |s| s.mode = 0);
                    }
                    return hub::set_led_mode(dev, kind, None, 0);
                }
                for (_, led, e) in &entries {
                    let (index, leds) = match *led {
                        0 => (None, all_leds.clone()),
                        n if n <= kind.led_count() => (Some(n - 1), n..=n),
                        _ => continue,
                    };
                    hub::set_led_color(dev, kind, index, e.rgb())?;
                    hub::set_led_brightness(dev, kind, index, e.brightness)?;
                    hub::set_led_speed(dev, kind, e.speed)?;
                    hub::set_led_mode(dev, kind, index, e.mode)?;
                    for l in leds {
                        led_state_update(name, l, |s| *s = e.clone());
                    }
                }
                Ok(())
            });
        }
        save_led_state();
    }
}

// --- Zapamiętany stan diod -------------------------------------------------------------------
// Moduły pozwalają odczytać tylko tryb i jasność, więc kolor pamiętamy sami ("knob:2" -> Entry).
// Stan jest zapisywany obok pluginu, żeby panel ustawień pokazywał go także po restarcie.

static LED_STATE: LazyLock<Mutex<BTreeMap<String, Entry>>> = LazyLock::new(|| {
    let state = std::fs::read_to_string(led_state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Mutex::new(state)
});

fn led_state_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("led_state.json")))
        .unwrap_or_else(|| "led_state.json".into())
}

fn led_state_update(module: &str, led: u8, f: impl FnOnce(&mut Entry)) {
    f(LED_STATE.lock().unwrap().entry(format!("{module}:{led}")).or_default());
}

fn save_led_state() {
    let json = serde_json::to_string_pretty(&*LED_STATE.lock().unwrap()).unwrap_or_default();
    if let Err(e) = std::fs::write(led_state_path(), json) {
        log::warn!("Nie udało się zapisać stanu diod: {e}");
    }
}

/// Aktualny stan diod dla panelu ustawień: `{ "state": { "knob:1": {...}, ... } }`.
pub fn led_state_json() -> serde_json::Value {
    serde_json::json!({ "state": &*LED_STATE.lock().unwrap() })
}

/// Przywraca na wszystkich modułach zapamiętany stan diod (np. po efektach z gry).
pub fn restore_led_state() {
    let modules: Vec<(String, Kind)> = MODULES
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, m)| m.kind.led_module().is_some())
        .map(|(id, m)| (id.clone(), m.kind))
        .collect();
    let state = LED_STATE.lock().unwrap().clone();
    for (id, kind) in modules {
        let name = Lighting::module_name(kind);
        let entry = |led: u8| state.get(&format!("{name}:{led}")).cloned().unwrap_or_default();
        with_writer(&id, |dev| {
            hub::set_led_speed(dev, kind, entry(1).speed)?;
            for led in 1..=kind.led_count() {
                let e = entry(led);
                let index = Some(led - 1);
                hub::set_led_color(dev, kind, index, e.rgb())?;
                hub::set_led_brightness(dev, kind, index, e.brightness)?;
                hub::set_led_mode(dev, kind, index, e.mode)?;
            }
            Ok(())
        });
    }
}

/// Aktywne akcje podświetlenia: kontekst -> (ustawienia, włączone?).
static ACTIVE: LazyLock<Mutex<HashMap<String, (Lighting, bool)>>> = LazyLock::new(Default::default);

fn parse<T: for<'de> Deserialize<'de> + Default>(settings: &serde_json::Value) -> T {
    serde_json::from_value(settings.clone()).unwrap_or_default()
}

pub fn lighting_appear(context: &str, settings: &serde_json::Value) {
    let l = Lighting::parse(settings);
    l.apply(true, None);
    ACTIVE.lock().unwrap().insert(context.to_string(), (l, true));
}

pub fn lighting_settings(context: &str, settings: &serde_json::Value) {
    let l = Lighting::parse(settings);
    let mut active = ACTIVE.lock().unwrap();
    let on = active.get(context).map(|(_, on)| *on).unwrap_or(true);
    l.apply(on, None);
    active.insert(context.to_string(), (l, on));
}

pub fn lighting_press(context: &str) {
    let mut active = ACTIVE.lock().unwrap();
    if let Some((l, on)) = active.get_mut(context) {
        if l.press == "toggle" {
            *on = !*on;
        }
        l.apply(*on, None);
    }
}

pub fn lighting_disappear(context: &str) {
    ACTIVE.lock().unwrap().remove(context);
}

/// Po podłączeniu modułu przywraca jego podświetlenie z aktywnych akcji.
pub fn restore_lighting(id: &str) {
    for (l, on) in ACTIVE.lock().unwrap().values() {
        l.apply(*on, Some(id));
    }
}

// --- Głośność --------------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
struct Volume {
    /// "master", "mic", "apps" albo "focused" (aplikacja aktywnego okna).
    target: String,
    /// Nazwy procesów po przecinku, gdy target = "apps".
    apps: String,
    /// Krok w % na jeden ząbek (gałki, rolki, pokrętło).
    step: u8,
    /// Dioda suwaka pokazuje poziom głośności.
    #[serde(rename = "ledLevel")]
    led_level: bool,
    /// Przy zmianie na chwilę pokazuje poziom na ekranie pokrętła.
    #[serde(rename = "dialOverlay")]
    dial_overlay: bool,
}

impl Default for Volume {
    fn default() -> Self {
        Volume { target: "master".into(), apps: String::new(), step: 2, led_level: true, dial_overlay: true }
    }
}

impl Volume {
    fn targets(&self) -> &str {
        if self.target == "apps" { &self.apps } else { &self.target }
    }

    /// Podpis na nakładce: "System", "Mikrofon" albo nazwa pierwszej aplikacji ("Spotify").
    fn label(&self) -> String {
        let capitalize = |s: &str| -> String {
            s.chars().take(1).flat_map(char::to_uppercase).chain(s.chars().skip(1)).collect()
        };
        match self.target.as_str() {
            "mic" => "Mikrofon".into(),
            "focused" => audio::focused_app()
                .map(|a| capitalize(a.trim_end_matches(".exe")))
                .unwrap_or_else(|| "Aktywne okno".into()),
            "apps" => {
                let mut apps = self.apps.split(',').map(|a| a.trim().trim_end_matches(".exe")).filter(|a| !a.is_empty());
                let mut name = capitalize(apps.next().unwrap_or("Aplikacje"));
                if apps.next().is_some() {
                    name.push_str(" +");
                }
                name
            }
            _ => "System".into(),
        }
    }

    /// Pokazuje poziom na ekranie pokrętła (jeśli włączone w ustawieniach).
    fn show_overlay(&self, state: Option<audio::State>) {
        if !self.dial_overlay {
            return;
        }
        let img = match state {
            Some(s) => crate::render::gauge(hub::DIAL_IMAGE_SIZE, s.level, s.muted, &volume_title(s), &self.label()),
            // Aktywne okno bez dźwięku – pokazujemy, że nie ma czym sterować.
            None if self.target == "focused" => crate::render::gauge(hub::DIAL_IMAGE_SIZE, 0.0, false, "—", &self.label()),
            None => return,
        };
        crate::overlay::show(img);
    }
}

/// Tytuł akcji po zmianie głośności, np. "45%" albo "Wycisz.".
fn volume_title(state: audio::State) -> String {
    if state.muted { "Wycisz.".into() } else { format!("{:.0}%", state.level * 100.0) }
}

/// Widoczne akcje głośności: kontekst -> (cel, ostatnio wysłany tytuł).
static VOLUME_INSTANCES: LazyLock<Mutex<HashMap<String, (String, Option<String>)>>> = LazyLock::new(Default::default);

pub fn volume_register(context: &str, settings: &serde_json::Value) {
    let v: Volume = parse(settings);
    VOLUME_INSTANCES.lock().unwrap().insert(context.to_string(), (v.targets().to_string(), None));
}

pub fn volume_unregister(context: &str) {
    VOLUME_INSTANCES.lock().unwrap().remove(context);
}

/// Odczytuje głośność wszystkich celów i zwraca tytuły do zmiany (kontekst, tytuł).
/// `force` wysyła tytuł także wtedy, gdy się nie zmienił.
pub fn volume_titles(force: bool) -> Vec<(String, String)> {
    let instances: Vec<(String, String)> =
        VOLUME_INSTANCES.lock().unwrap().iter().map(|(c, (t, _))| (c.clone(), t.clone())).collect();
    let mut states: HashMap<String, Option<audio::State>> = HashMap::new();
    let mut changed = vec![];
    for (context, targets) in instances {
        let state = *states
            .entry(targets.clone())
            .or_insert_with(|| audio::run(&targets, audio::Op::Step(0.0)));
        let Some(state) = state else { continue };
        let title = volume_title(state);
        let mut map = VOLUME_INSTANCES.lock().unwrap();
        if let Some((_, last)) = map.get_mut(&context) {
            if force || last.as_deref() != Some(title.as_str()) {
                *last = Some(title.clone());
                changed.push((context, title));
            }
        }
    }
    changed
}

/// Obrót enkodera. Na faderze ustawia poziom bezwzględnie (pozycja suwaka), gdzie indziej krokowo.
pub fn volume_rotate(device: &str, position: u8, ticks: i16, settings: &serde_json::Value) -> Option<audio::State> {
    let v: Volume = parse(settings);
    let kind = MODULES.lock().unwrap().get(device).map(|m| m.kind);
    let absolute = (kind == Some(Kind::Fader))
        .then(|| FADER_VALUES.lock().unwrap().get(&(device.to_string(), position)).copied())
        .flatten();
    let op = match absolute {
        Some(value) => audio::Op::Set(value as f32 / 100.0),
        None => audio::Op::Step(ticks as f32 * v.step.max(1) as f32 / 100.0),
    };
    let state = audio::run(v.targets(), op);
    v.show_overlay(state);
    if kind == Some(Kind::Fader) && v.led_level {
        if let Some(s) = state {
            let level = if s.muted { 0 } else { (s.level * 100.0).round() as u8 };
            with_writer(device, |dev| hub::set_led_brightness(dev, Kind::Fader, Some(position), level));
        }
    }
    state
}

pub fn volume_toggle_mute(settings: &serde_json::Value) -> Option<audio::State> {
    let v: Volume = parse(settings);
    let state = audio::run(v.targets(), audio::Op::ToggleMute);
    v.show_overlay(state);
    state
}
