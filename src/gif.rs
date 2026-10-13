//! Animowane GIF-y na klawiszach i ekranie pokrętła.
//!
//! Moduły nie animują same – tak jak oficjalna aplikacja wysyłamy kolejne klatki jako zwykłe
//! obrazy JPG. Klatki są kodowane raz (cache), a każda animacja ma własny wątek.
//! GIF może też zająć cały moduł 15xKey: klatka jest skalowana do rozmiaru panelu (z odstępami
//! między klawiszami) i cięta na 15 kafelków.

use base64::Engine;
use image::{AnimationDecoder, DynamicImage, codecs::gif::GifDecoder, imageops::FilterType};
use std::{
    collections::HashMap,
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{hub, hub::Kind, images, with_writer};

/// Oficjalna aplikacja nie odświeża klatek częściej niż co 66 ms.
const MIN_FRAME_MS: u64 = 66;
/// Animacje na cały moduł wysyłają 15 obrazów na klatkę – przy ciągłych ~225 obrazach/s moduł
/// potrafi się zawiesić i zresetować, więc ograniczamy je do 10 klatek/s.
const MIN_MODULE_FRAME_MS: u64 = 100;
/// Przerwa między kafelkami jednej klatki, żeby nie zasypywać modułu.
const TILE_PAUSE: Duration = Duration::from_millis(2);
/// Ograniczenie długości animacji (pamięć i czas kodowania).
const MAX_FRAMES: usize = 600;
/// Klucz animacji zajmującej cały moduł (zamiast numeru klawisza).
const WHOLE_MODULE: u8 = 255;

/// Sposób ułożenia animacji.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layout {
    /// Jeden klawisz / ekran pokrętła o boku `size`.
    Single { size: u32, cover: bool },
    /// Cały moduł: `cols`×`rows` klawiszy po `key` px, odstęp `gap` px między nimi.
    Grid { cols: u32, rows: u32, key: u32, gap: u32, cover: bool },
    /// Ta sama animacja na każdym z `count` klawiszy modułu.
    Copies { count: u8, size: u32, cover: bool },
}

/// Klatka: kafelki JPG (jeden dla `Single`, cols×rows dla `Grid`) i czas wyświetlania.
type Frames = Arc<Vec<(Vec<Vec<u8>>, Duration)>>;

static CACHE: LazyLock<Mutex<HashMap<(PathBuf, Layout), Frames>>> = LazyLock::new(Default::default);
/// Odtwarzana animacja: flaga zatrzymania, blokada trzymana podczas wysyłania klatki
/// (żeby zatrzymanie mogło poczekać na jej koniec) i klawisz akcji, która ją uruchomiła.
struct Player {
    stop: Arc<AtomicBool>,
    frame: Arc<Mutex<()>>,
    owner: u8,
}

impl Player {
    /// Ustawia flagę i czeka, aż bieżąca klatka zostanie dosłana.
    fn halt(&self) {
        self.stop.store(true, Ordering::Relaxed);
        drop(self.frame.lock());
    }
}

/// Odtwarzane animacje: (urządzenie, klawisz lub WHOLE_MODULE) -> odtwarzacz.
static PLAYERS: LazyLock<Mutex<HashMap<(String, u8), Player>>> = LazyLock::new(Default::default);

fn gif_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("gifs"))).unwrap_or_else(|| "gifs".into())
}

/// Zapisuje GIF przesłany z panelu (data URL) w katalogu pluginu i zwraca ścieżkę.
pub fn save_upload(data_url: &str) -> Result<PathBuf, String> {
    let (_, data) = data_url.split_once(',').ok_or("niepoprawny data URL")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| e.to_string())?;
    // Nazwa z prostego skrótu FNV-1a, żeby ten sam plik nie był zapisywany wielokrotnie.
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    let dir = gif_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{hash:016x}.gif"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn tiles(img: &DynamicImage, layout: Layout) -> Result<Vec<Vec<u8>>, String> {
    match layout {
        Layout::Single { size, cover } | Layout::Copies { size, cover, .. } => Ok(vec![images::fit_jpeg(img, size, cover, 85)?]),
        Layout::Grid { cols, rows, key, gap, cover } => {
            let (w, h) = (cols * key + (cols - 1) * gap, rows * key + (rows - 1) * gap);
            let canvas = if cover {
                img.resize_to_fill(w, h, FilterType::Triangle)
            } else {
                // Zmieść całość: wyśrodkuj na czarnym tle.
                let scaled = img.resize(w, h, FilterType::Triangle);
                let mut bg = DynamicImage::new_rgba8(w, h);
                let (x, y) = ((w - scaled.width()) / 2, (h - scaled.height()) / 2);
                image::imageops::overlay(&mut bg, &scaled, x as i64, y as i64);
                bg
            };
            let mut out = vec![];
            for r in 0..rows {
                for c in 0..cols {
                    let tile = canvas.crop_imm(c * (key + gap), r * (key + gap), key, key);
                    out.push(images::fit_jpeg(&tile, key, true, 85)?);
                }
            }
            Ok(out)
        }
    }
}

fn decode(path: &PathBuf, layout: Layout) -> Result<Frames, String> {
    let key = (path.clone(), layout);
    if let Some(frames) = CACHE.lock().unwrap().get(&key) {
        return Ok(frames.clone());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut frames = vec![];
    for frame in decoder.into_frames().take(MAX_FRAMES) {
        let frame = frame.map_err(|e| e.to_string())?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = Duration::from_millis((num as u64 / den.max(1) as u64).max(MIN_FRAME_MS));
        frames.push((tiles(&DynamicImage::ImageRgba8(frame.into_buffer()), layout)?, delay));
    }
    if frames.is_empty() {
        return Err("GIF nie zawiera klatek".into());
    }
    let frames = Arc::new(frames);
    CACHE.lock().unwrap().insert(key, frames.clone());
    Ok(frames)
}

/// Czy klawisz jest zajęty przez animację (wtedy obrazy z OpenDecka są pomijane).
pub fn is_playing(device: &str, position: u8) -> bool {
    let players = PLAYERS.lock().unwrap();
    players.contains_key(&(device.to_string(), position)) || players.contains_key(&(device.to_string(), WHOLE_MODULE))
}

/// Zatrzymuje animację uruchomioną z klawisza `position` (pojedynczą albo na cały moduł)
/// i czeka na koniec wysyłanej klatki. Animacji na cały moduł z innego klawisza nie rusza.
/// Zwraca, czy coś zatrzymano.
pub fn stop(device: &str, position: u8, whole_module: bool) -> bool {
    let key = (device.to_string(), if whole_module { WHOLE_MODULE } else { position });
    let player = {
        let mut players = PLAYERS.lock().unwrap();
        match players.get(&key) {
            Some(p) if p.owner == position => players.remove(&key),
            _ => None,
        }
    };
    player.map(|p| p.halt()).is_some()
}

/// Uruchamia (lub restartuje) animację. Przy `Layout::Grid` zajmuje wszystkie klawisze modułu.
pub fn play(device: &str, kind: Kind, position: u8, path: PathBuf, layout: Layout) {
    let whole = matches!(layout, Layout::Grid { .. } | Layout::Copies { .. });
    stop(device, position, whole);
    let slot = if whole { WHOLE_MODULE } else { position };
    let stop_flag = Arc::new(AtomicBool::new(false));
    let frame_lock = Arc::new(Mutex::new(()));
    let player = Player { stop: stop_flag.clone(), frame: frame_lock.clone(), owner: position };
    // Nowa animacja na cały moduł zastępuje poprzednią, nawet z innego klawisza.
    let old = PLAYERS.lock().unwrap().insert((device.to_string(), slot), player);
    if let Some(old) = old {
        old.halt();
    }
    let min_frame = Duration::from_millis(if whole { MIN_MODULE_FRAME_MS } else { MIN_FRAME_MS });
    let device = device.to_string();
    thread::spawn(move || {
        let frames = match decode(&path, layout) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Nie udało się wczytać GIF-a: {e}");
                // Zwalniamy klawisze, żeby wróciły zwykłe obrazy z OpenDecka.
                let mut players = PLAYERS.lock().unwrap();
                if players.get(&(device.clone(), slot)).is_some_and(|p| Arc::ptr_eq(&p.stop, &stop_flag)) {
                    players.remove(&(device, slot));
                }
                return;
            }
        };
        log::info!("GIF na {device}/{slot}: {} klatek", frames.len());
        'outer: loop {
            for (tiles, delay) in frames.iter() {
                let guard = frame_lock.lock().unwrap();
                if stop_flag.load(Ordering::Relaxed) {
                    break 'outer;
                }
                let started = Instant::now();
                if kind == Kind::Dial {
                    // Jeden worker LCD wybiera aktualne tło albo nakładkę. GIF nie
                    // może nadpisać feedbacku po czekaniu na blokadę HID.
                    crate::overlay::set_background_jpeg(&device, tiles[0].clone());
                    drop(guard);
                    thread::sleep((*delay).max(min_frame));
                    continue;
                }
                let ok = with_writer(&device, |dev| {
                    match (kind, layout) {
                        (_, Layout::Single { .. }) => hub::upload_key_image(dev, position, &tiles[0])?,
                        (_, Layout::Grid { .. }) => {
                            for (i, tile) in tiles.iter().enumerate() {
                                hub::upload_key_image(dev, i as u8, tile)?;
                                thread::sleep(TILE_PAUSE);
                            }
                        }
                        (_, Layout::Copies { count, .. }) => {
                            for i in 0..count {
                                hub::upload_key_image(dev, i, &tiles[0])?;
                                thread::sleep(TILE_PAUSE);
                            }
                        }
                    }
                    Ok(())
                });
                drop(guard);
                // Po błędzie (moduł zajęty albo odłączony) dajemy mu chwilę, zamiast dosyłać dalej.
                let wait = if ok { (*delay).max(min_frame) } else { Duration::from_secs(1) };
                thread::sleep(wait.saturating_sub(started.elapsed()));
            }
            // Pojedyncza klatka: wysłana raz wystarczy.
            if frames.len() == 1 {
                break;
            }
        }
    });
}
