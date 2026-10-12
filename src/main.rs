//! Plugin OpenDecka dla modułów Cooler Master MasterHUB.
//!
//! Każdy moduł jest zgłaszany jako osobne urządzenie (`mh-<numer seryjny>`), więc ma własny
//! profil i przepinanie modułów nie rusza przypisań. Obsługa sprzętu działa na zwykłych
//! wątkach (hidapi jest blokujące), a zdarzenia trafiają do OpenDecka przez kanał.

mod actions;
mod audio;
mod gif;
#[cfg(target_os = "linux")]
mod host;
mod hub;
mod images;
mod input;
mod overlay;
mod render;
mod sensors;

use hidapi::{HidApi, HidDevice};
use hub::{Input, Kind};
use openaction::*;
use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

enum Msg {
    Register { id: String, kind: Kind },
    Deregister(String),
    Input(String, Input),
}

struct Module {
    kind: Kind,
    /// Osobny uchwyt do zapisu, żeby nie blokować wątku czytającego.
    writer: Arc<Mutex<HidDevice>>,
}

static MODULES: LazyLock<Mutex<HashMap<String, Module>>> = LazyLock::new(Default::default);
/// Ostatnie pozycje suwaków: (urządzenie, suwak) -> 0–100.
static FADER_VALUES: LazyLock<Mutex<HashMap<(String, u8), u8>>> = LazyLock::new(Default::default);
static BRIGHTNESS: AtomicU8 = AtomicU8::new(100);
static STARTED: OnceLock<()> = OnceLock::new();

/// Wyszukuje nowe moduły co 2 s i uruchamia dla nich wątki czytające.
fn watcher(tx: UnboundedSender<Msg>) {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            log::error!("Nie udało się zainicjować HID: {e}");
            return;
        }
    };
    let open_paths: Arc<Mutex<HashSet<CString>>> = Default::default();
    loop {
        let _ = api.refresh_devices();
        for info in api.device_list().filter(|d| d.vendor_id() == hub::VID) {
            let Some(kind) = Kind::from_pid(info.product_id()) else { continue };
            let path = info.path().to_owned();
            if open_paths.lock().unwrap().contains(&path) {
                continue;
            }
            let (Ok(reader), Ok(writer)) = (info.open_device(&api), info.open_device(&api)) else {
                continue;
            };
            if let Err(e) = hub::set_sw_mode(&writer, kind, true) {
                log::warn!("{}: nie udało się włączyć trybu software: {e}", kind.name());
            }
            let serial = hub::read_serial(&writer, kind)
                .or_else(|| info.serial_number().map(str::to_owned))
                .unwrap_or_else(|| format!("{:04x}", info.product_id()));
            let id = format!("mh-{serial}");
            log::info!("Podłączono {} ({id})", kind.name());

            open_paths.lock().unwrap().insert(path.clone());
            MODULES.lock().unwrap().insert(id.clone(), Module { kind, writer: Arc::new(Mutex::new(writer)) });
            let _ = tx.send(Msg::Register { id: id.clone(), kind });

            let (tx, open_paths) = (tx.clone(), open_paths.clone());
            thread::spawn(move || {
                read_loop(reader, kind, &id, &tx);
                log::info!("Odłączono {} ({id})", kind.name());
                MODULES.lock().unwrap().remove(&id);
                overlay::clear_feedback(&id, None);
                open_paths.lock().unwrap().remove(&path);
                let _ = tx.send(Msg::Deregister(id));
            });
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn read_loop(dev: HidDevice, kind: Kind, id: &str, tx: &UnboundedSender<Msg>) {
    let mut decoder = hub::Decoder::new(kind);
    let mut buf = [0u8; 512];
    loop {
        let n = match dev.read_timeout(&mut buf, 1000) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(_) => return,
        };
        for input in decoder.decode(&buf[..n]) {
            if let Input::EncoderTwist(position, _) = input {
                overlay::encoder_twist(id, position);
            }
            if let Input::FaderValue(i, v) = input {
                FADER_VALUES.lock().unwrap().insert((id.to_string(), i), v);
                continue;
            }
            if tx.send(Msg::Input(id.to_string(), input)).is_err() {
                return;
            }
        }
    }
}

/// Przekazuje zdarzenia sprzętowe do OpenDecka.
async fn dispatcher(mut rx: UnboundedReceiver<Msg>) {
    while let Some(msg) = rx.recv().await {
        let mut lock = OUTBOUND_EVENT_MANAGER.lock().await;
        let Some(out) = lock.as_mut() else { continue };
        let result = match msg {
            // Baza nie ma żadnych elementów do przypisania – obsługujemy tylko jej diody.
            Msg::Register { id, kind: Kind::Base } => {
                background(move || actions::restore_lighting(&id));
                Ok(())
            }
            Msg::Deregister(id) if id.starts_with("mh-MHBS") => Ok(()),
            Msg::Register { id, kind } => {
                let (rows, columns, encoders) = kind.layout();
                let r = out.register_device(id.clone(), kind.name().to_string(), rows, columns, encoders, 0).await;
                // Komendy dla modułu idą w tle: zawieszony moduł nie może blokować zgłaszania
                // kolejnych urządzeń ani zdarzeń.
                background(move || {
                    if kind.has_screen() {
                        with_writer(&id, |dev| hub::set_backlight(dev, kind, BRIGHTNESS.load(Ordering::Relaxed)));
                    }
                    if kind == Kind::Dial {
                        overlay::refresh_direct(&id);
                    }
                    actions::restore_lighting(&id);
                });
                r
            }
            Msg::Deregister(id) => out.deregister_device(id).await,
            Msg::Input(id, input) => match input {
                Input::KeyDown(p) => out.key_down(id, p).await,
                Input::KeyUp(p) => out.key_up(id, p).await,
                Input::EncoderDown(p) => out.encoder_down(id, p).await,
                Input::EncoderUp(p) => out.encoder_up(id, p).await,
                Input::EncoderTwist(p, t) => out.encoder_change(id, p, t).await,
                Input::FaderValue(..) => Ok(()),
            },
        };
        if let Err(e) = result {
            log::error!("Błąd wysyłania do OpenDecka: {e}");
        }
    }
}

/// Obrazy czekające na wysłanie: urządzenie -> (klawisz -> data URL, czy działa wątek wysyłający).
type PendingImages = HashMap<String, (HashMap<u8, Option<String>>, bool)>;
static PENDING_IMAGES: LazyLock<Mutex<PendingImages>> = LazyLock::new(Default::default);

/// Wysyła oczekujące obrazy urządzenia, aż kolejka będzie pusta.
fn image_worker(device: String) {
    loop {
        let next = {
            let mut pending = PENDING_IMAGES.lock().unwrap();
            let Some((images, running)) = pending.get_mut(&device) else { return };
            match images.keys().next().copied() {
                Some(p) => images.remove(&p).map(|img| (p, img)),
                None => {
                    *running = false;
                    return;
                }
            }
        };
        let Some((position, image)) = next else { continue };
        match images::to_jpeg(image.as_deref(), hub::KEY_IMAGE_SIZE) {
            Ok(jpeg) => {
                with_writer(&device, |dev| hub::upload_key_image(dev, position, &jpeg));
            }
            Err(e) => log::error!("Nie udało się przygotować obrazka: {e}"),
        }
    }
}

/// Wykonuje zapis do modułu pod jego blokadą; zwraca, czy się udał.
fn with_writer(id: &str, f: impl FnOnce(&HidDevice) -> Result<(), hidapi::HidError>) -> bool {
    let writer = MODULES.lock().unwrap().get(id).map(|m| m.writer.clone());
    let Some(w) = writer else { return false };
    match f(&w.lock().unwrap()) {
        Ok(()) => true,
        Err(e) => {
            log::error!("{id}: błąd zapisu do modułu: {e}");
            false
        }
    }
}

struct GlobalEventHandler;

impl openaction::GlobalEventHandler for GlobalEventHandler {
    async fn plugin_ready(&self, _outbound: &mut OutboundEventManager) -> EventHandlerResult {
        if STARTED.set(()).is_ok() {
            let (tx, rx) = unbounded_channel();
            tokio::spawn(dispatcher(rx));
            tokio::spawn(volume_poller());
            thread::spawn(move || watcher(tx));
            log::info!("Plugin MasterHUB {} gotowy", env!("CARGO_PKG_VERSION"));
        }
        Ok(())
    }

    async fn set_image(&self, event: SetImageEvent, _outbound: &mut OutboundEventManager) -> EventHandlerResult {
        let Some(kind) = MODULES.lock().unwrap().get(&event.device).map(|m| m.kind) else { return Ok(()) };
        if event.controller.is_none() && event.position.is_none() && event.image.is_none() {
            overlay::clear_feedback(&event.device, None);
        }
        if event.controller.as_deref() == Some("Encoder") {
            let encoders = kind.layout().2;
            if encoders == 0 || event.position.is_some_and(|p| p >= encoders) {
                return Ok(());
            }
            let Some(image) = event.image.filter(|image| !image.trim().is_empty()) else {
                overlay::clear_feedback(&event.device, event.position);
                return Ok(());
            };
            // Źródłem może być dowolny enkoder; knoby i rollery korzystają z LCD
            // podłączonych pokręteł. Stan jest przypisany do urządzenia ORAZ pozycji.
            overlay::show_feedback(&event.device, event.position, encoders, image);
            return Ok(());
        }
        let key_count = match kind {
            Kind::Keys15 => 15,
            Kind::Dial => 1,
            _ => return Ok(()),
        };
        let positions: Vec<u8> = match event.position {
            Some(p) if p < key_count => vec![p],
            Some(_) => return Ok(()),
            None => (0..key_count).collect(),
        };
        // Klawisze z animacją GIF rysuje odtwarzacz, nie OpenDeck.
        let positions: Vec<u8> = positions.into_iter().filter(|p| !gif::is_playing(&event.device, *p)).collect();
        let image = if event.position.is_some() { event.image } else { None };
        if kind == Kind::Dial {
            if !positions.is_empty() {
                // Wszystkie obrazy LCD obsługuje jeden worker, także zegar i GIF.
                overlay::remember(&event.device, image);
            }
            return Ok(());
        }

        // Zapamiętujemy tylko najnowszy obraz dla każdego klawisza; przy szybkich zmianach
        // (np. procent głośności podczas kręcenia) pośrednie klatki są pomijane.
        let start_worker = {
            let mut pending = PENDING_IMAGES.lock().unwrap();
            let (images, running) = pending.entry(event.device.clone()).or_default();
            for p in positions {
                images.insert(p, image.clone());
            }
            !std::mem::replace(running, true)
        };
        if start_worker {
            tokio::task::spawn_blocking(move || image_worker(event.device));
        }
        Ok(())
    }

    async fn set_brightness(&self, event: SetBrightnessEvent, _outbound: &mut OutboundEventManager) -> EventHandlerResult {
        BRIGHTNESS.store(event.brightness, Ordering::Relaxed);
        let kind = MODULES.lock().unwrap().get(&event.device).map(|m| m.kind);
        if let Some(kind) = kind.filter(|k| k.has_screen()) {
            background(move || {
                with_writer(&event.device, |dev| hub::set_backlight(dev, kind, event.brightness));
            });
        }
        Ok(())
    }
}

struct ActionEventHandler;

/// Uruchamia blokującą operację (HID, Core Audio) poza wątkami tokio.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, anyhow::Error> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// Jak `blocking`, ale czeka najwyżej sekundę – dłużej trwa tylko przy zawieszonym module
/// (komendy HID mają 5 s limitu), a wtedy operacja kończy się w tle.
async fn blocking_brief(f: impl FnOnce() + Send + 'static) {
    let _ = tokio::time::timeout(Duration::from_secs(1), tokio::task::spawn_blocking(f)).await;
}

/// Uruchamia blokującą operację w tle, bez czekania na wynik.
fn background(f: impl FnOnce() + Send + 'static) {
    tokio::task::spawn_blocking(f);
}

/// Odświeża tytuły wszystkich akcji głośności (np. "45%"), które się zmieniły.
/// Na klawiszu pokrętła tytuł trafia razem z ikoną na jego ekran.
async fn sync_volume(out: &mut OutboundEventManager, force: bool) -> EventHandlerResult {
    for (context, title) in blocking(move || actions::volume_titles(force)).await? {
        out.set_title(context, Some(title), None).await?;
    }
    Ok(())
}

/// Ikony tła akcji "Czujnik" (wbudowane w plugin).
fn sensor_icon_url(name: &str) -> String {
    use base64::Engine;
    let bytes: &[u8] = match name {
        "temperature" => include_bytes!("../assets/sensors/temperature.png"),
        "temperature-warn" => include_bytes!("../assets/sensors/temperature-warn.png"),
        "load" => include_bytes!("../assets/sensors/load.png"),
        "load-warn" => include_bytes!("../assets/sensors/load-warn.png"),
        "power" => include_bytes!("../assets/sensors/power.png"),
        "power-warn" => include_bytes!("../assets/sensors/power-warn.png"),
        "fan" => include_bytes!("../assets/sensors/fan.png"),
        "fan-warn" => include_bytes!("../assets/sensors/fan-warn.png"),
        "clock" => include_bytes!("../assets/sensors/clock.png"),
        "clock-warn" => include_bytes!("../assets/sensors/clock-warn.png"),
        "generic-warn" => include_bytes!("../assets/sensors/generic-warn.png"),
        _ => include_bytes!("../assets/sensors/generic.png"),
    };
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Odświeża tytuły (wartości) i ikony akcji "Czujnik".
async fn sync_sensors(out: &mut OutboundEventManager, force: bool) -> EventHandlerResult {
    for (context, title, icon) in blocking(move || actions::sensor_updates(force)).await? {
        if let Some(icon) = icon {
            out.set_image(context.clone(), Some(sensor_icon_url(icon)), None).await?;
        }
        if let Some(title) = title {
            out.set_title(context, Some(title), None).await?;
        }
    }
    Ok(())
}

/// Co sekundę odświeża głośność (żeby było widać zmiany spoza MasterHUB-a) i czujniki.
async fn volume_poller() {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut lock = OUTBOUND_EVENT_MANAGER.lock().await;
        if let Some(out) = lock.as_mut() {
            if let Err(e) = sync_volume(out, false).await {
                log::warn!("Odświeżanie głośności: {e}");
            }
            for (context, image) in actions::clock_updates() {
                if let Err(e) = out.set_image(context, Some(image), None).await {
                    log::warn!("Zegar: {e}");
                }
            }
            if actions::sensors_active() {
                if let Err(e) = sync_sensors(out, false).await {
                    log::warn!("Odświeżanie czujników: {e}");
                }
            }
        }
    }
}

/// Uruchamia animację GIF; po przesłaniu pliku zapisuje w akcji już tylko jego ścieżkę.
async fn apply_gif(
    out: &mut OutboundEventManager,
    context: String,
    device: String,
    coordinates: Coordinates,
    settings: serde_json::Value,
) -> EventHandlerResult {
    let (row, column) = (coordinates.row, coordinates.column);
    if let Some(new) = blocking(move || actions::gif_apply(&device, row, column, &settings)).await? {
        out.set_settings(context, new).await?;
    }
    Ok(())
}

impl openaction::ActionEventHandler for ActionEventHandler {
    async fn will_appear(&self, event: AppearEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        let settings = event.payload.settings;
        match event.action.as_str() {
            actions::LIGHTING => {
                let context = event.context;
                blocking_brief(move || actions::lighting_appear(&context, &settings)).await;
            }
            actions::VOLUME => {
                actions::volume_register(&event.context, &settings);
                sync_volume(out, false).await?;
            }
            actions::GIF => apply_gif(out, event.context, event.device, event.payload.coordinates, settings).await?,
            actions::SENSOR => {
                actions::sensor_register(&event.context, &settings);
                sync_sensors(out, false).await?;
            }
            actions::CLOCK => actions::clock_register(&event.context, &event.device, &settings),
            _ => {}
        }
        Ok(())
    }

    async fn property_inspector_did_appear(
        &self,
        event: PropertyInspectorAppearEvent,
        out: &mut OutboundEventManager,
    ) -> EventHandlerResult {
        match event.action.as_str() {
            actions::LIGHTING => out.send_to_property_inspector(event.context, actions::led_state_json()).await?,
            actions::SENSOR => {
                let list = blocking(actions::sensor_list_json).await?;
                out.send_to_property_inspector(event.context, list).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn will_disappear(&self, event: AppearEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        match event.action.as_str() {
            actions::LIGHTING => actions::lighting_disappear(&event.context),
            actions::VOLUME => actions::volume_unregister(&event.context),
            actions::SENSOR => actions::sensor_unregister(&event.context),
            actions::CLOCK => {
                if let Some(device) = actions::clock_unregister(&event.context) {
                    out.rerender_images(device).await?;
                }
            }
            actions::GIF => {
                let c = event.payload.coordinates;
                let device = event.device.clone();
                // Czekamy na koniec wysyłanej klatki, zanim OpenDeck przyśle zwykłe ikony.
                if blocking(move || actions::gif_stop(&device, c.row, c.column)).await? {
                    // Klawisz wraca do zwykłych obrazów z OpenDecka.
                    out.rerender_images(event.device).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn did_receive_settings(&self, event: DidReceiveSettingsEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        let settings = event.payload.settings;
        match event.action.as_str() {
            actions::LIGHTING => {
                let context = event.context.clone();
                blocking_brief(move || actions::lighting_settings(&context, &settings)).await;
                // Panel dostaje odświeżony stan diod (np. po zmianie całego modułu).
                out.send_to_property_inspector(event.context, actions::led_state_json()).await?;
            }
            actions::VOLUME => {
                actions::volume_register(&event.context, &settings);
                sync_volume(out, false).await?;
            }
            actions::GIF => apply_gif(out, event.context, event.device, event.payload.coordinates, settings).await?,
            actions::SENSOR => {
                actions::sensor_register(&event.context, &settings);
                sync_sensors(out, false).await?;
            }
            actions::CLOCK => actions::clock_register(&event.context, &event.device, &settings),
            _ => {}
        }
        Ok(())
    }

    async fn key_down(&self, event: KeyEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        match event.action.as_str() {
            actions::LIGHTING => {
                let context = event.context;
                blocking_brief(move || actions::lighting_press(&context)).await;
            }
            actions::VOLUME => {
                let settings = event.payload.settings;
                blocking(move || actions::volume_toggle_mute(&settings)).await?;
                sync_volume(out, false).await?;
            }
            actions::GIF => {
                let settings = event.payload.settings;
                blocking(move || actions::gif_press(&settings)).await?;
            }
            actions::CLOCK => {
                let settings = event.payload.settings;
                blocking(move || actions::clock_press(&settings)).await?;
                // Wyciszenie z zegara – odświeżamy też tytuły akcji Głośność.
                sync_volume(out, false).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn dial_down(&self, event: DialPressEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        if event.action == actions::VOLUME {
            let settings = event.payload.settings;
            blocking(move || actions::volume_toggle_mute(&settings)).await?;
            sync_volume(out, false).await?;
        }
        Ok(())
    }

    async fn dial_rotate(&self, event: DialRotateEvent, out: &mut OutboundEventManager) -> EventHandlerResult {
        if event.action == actions::VOLUME {
            let (device, position) = (event.device, event.payload.coordinates.column);
            let (ticks, settings) = (event.payload.ticks, event.payload.settings);
            blocking(move || actions::volume_rotate(&device, position, ticks, &settings)).await?;
            sync_volume(out, false).await?;
        }
        Ok(())
    }
}

fn init_logging() {
    let path =
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("masterhub.log"))).unwrap_or_else(|| "masterhub.log".into());
    let config = simplelog::Config::default();
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = vec![simplelog::TermLogger::new(
        simplelog::LevelFilter::Info,
        config.clone(),
        simplelog::TerminalMode::Stdout,
        simplelog::ColorChoice::Never,
    )];
    if let Ok(file) = std::fs::File::create(path) {
        loggers.push(simplelog::WriteLogger::new(simplelog::LevelFilter::Info, config, file));
    }
    let _ = simplelog::CombinedLogger::init(loggers);
}

#[tokio::main]
async fn main() {
    init_logging();

    if let Err(e) = init_plugin(GlobalEventHandler, ActionEventHandler).await {
        log::error!("Błąd połączenia z OpenDeckiem: {e}");
    }

    // OpenDeck zamknął połączenie: oddajemy moduły w tryb sprzętowy.
    for (id, m) in MODULES.lock().unwrap().iter() {
        if let Err(e) = hub::set_sw_mode(&m.writer.lock().unwrap(), m.kind, false) {
            log::warn!("{id}: nie udało się przywrócić trybu sprzętowego: {e}");
        }
    }
    log::info!("Koniec pracy pluginu");
}
