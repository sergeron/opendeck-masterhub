//! Feedback dowolnego enkodera na LCD. Jeden worker ekranu obsługuje również zegar,
//! zwykłe obrazy i GIF: tylko on wybiera i wysyła aktualną klatkę.

use image::RgbImage;
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, LazyLock, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::{MODULES, hub, hub::Kind, images, with_writer};

const SHOW_FOR: Duration = Duration::from_millis(1500);
// Odpowiedź HA może nadejść po zniknięciu poprzedniej nakładki.
const FEEDBACK_WAIT_FOR: Duration = Duration::from_secs(5);
// Odstęp dla tła i własnych obrazów. Potwierdzona odpowiedź enkodera ma
// pierwszeństwo: trafia do wolnego HID od razu, bez czekania na ten termin.
const FRAME_EVERY: Duration = Duration::from_millis(33);
const RETRY_AFTER: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Encoder {
    device: String,
    position: u8,
}

impl Encoder {
    fn new(device: &str, position: u8) -> Self {
        Self { device: device.into(), position }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FeedbackImage {
    data: Arc<str>,
    row: Option<(u8, u8)>,
}

#[derive(Default)]
struct Source {
    latest: Option<FeedbackImage>,
    cached: Option<FeedbackImage>,
    interaction_until: Option<Instant>,
    targets: Vec<String>,
    turn_revision: u64,
    awaiting_feedback: bool,
    ready: Option<Arc<ReadyFeedback>>,
    last_turn: Option<Instant>,
    /// Kolejność obrotów wszystkich enkoderów (rośnie z każdym obrotem).
    turn_order: u64,
    log_next_feedback: bool,
}

struct ReadyFeedback {
    image: FeedbackImage,
    model: Arc<images::DialFeedback>,
    jpeg: Arc<[u8]>,
    received_at: Instant,
    prepared_at: Instant,
    prepare_for: Duration,
}

impl ReadyFeedback {
    fn prepare(image: FeedbackImage, received_at: Instant) -> Result<Self, PrepareError> {
        let started = Instant::now();
        let model = images::dial_feedback_region(&image.data, hub::DIAL_IMAGE_SIZE, image.row).map_err(PrepareError::Decode)?;
        match model {
            images::DialFeedback::Incomplete => return Err(PrepareError::IncompleteFeedback),
            // Zwykła ikona z podpisem (akcja bez wsparcia Stream Decka+) – nie przejmuje LCD.
            images::DialFeedback::Image(_) => return Err(PrepareError::NotBar),
            images::DialFeedback::Ring(_) => {}
        }
        let jpeg = crate::render::feedback_jpeg(&model.render(hub::DIAL_IMAGE_SIZE)).map_err(PrepareError::Decode)?.into();
        Ok(Self { image, model: Arc::new(model), jpeg, received_at, prepared_at: Instant::now(), prepare_for: started.elapsed() })
    }

    fn received_again(&self, now: Instant) -> Self {
        Self {
            image: self.image.clone(),
            model: self.model.clone(),
            jpeg: self.jpeg.clone(),
            received_at: now,
            prepared_at: now,
            prepare_for: Duration::ZERO,
        }
    }
}

struct PendingFeedback {
    revision: u64,
    image: FeedbackImage,
    received_at: Instant,
}

#[derive(Clone)]
enum Frame {
    Image(Arc<RgbImage>),
    Feedback(FeedbackImage),
    ReadyFeedback(Arc<ReadyFeedback>),
    KeyImage(Option<Arc<str>>),
    Jpeg(Arc<[u8]>),
}

impl Frame {
    fn jpeg(&self) -> Result<Arc<[u8]>, PrepareError> {
        Ok(match self {
            Self::Image(image) => crate::render::jpeg(image).into(),
            Self::ReadyFeedback(ready) => ready.jpeg.clone(),
            Self::Feedback(image) => {
                let feedback = if image.row.is_some() {
                    images::dial_feedback_region(&image.data, hub::DIAL_IMAGE_SIZE, image.row).map_err(PrepareError::Decode)?
                } else {
                    images::dial_feedback(&image.data, hub::DIAL_IMAGE_SIZE).map_err(PrepareError::Decode)?
                };
                match feedback {
                    images::DialFeedback::Incomplete => return Err(PrepareError::IncompleteFeedback),
                    images::DialFeedback::Image(_) => return Err(PrepareError::NotBar),
                    images::DialFeedback::Ring(_) => {}
                }
                // Jedna klatka dla potwierdzonej wartości. Dodatkowe klatki animacji
                // mnożyły ruch HID i pokazywały poziomy pośrednie zamiast celu HA.
                crate::render::feedback_jpeg(&feedback.render(hub::DIAL_IMAGE_SIZE)).map_err(PrepareError::Decode)?.into()
            }
            Self::KeyImage(url) => images::to_jpeg(url.as_deref(), hub::DIAL_IMAGE_SIZE).map_err(PrepareError::Decode)?.into(),
            Self::Jpeg(jpeg) => jpeg.clone(),
        })
    }
}

#[derive(Debug)]
enum PrepareError {
    Decode(String),
    IncompleteFeedback,
    /// Obraz bez rozpoznanego paska – enkoder nie ma feedbacku do pokazania.
    NotBar,
}

#[derive(Clone, PartialEq, Eq)]
enum Owner {
    Encoder(Encoder),
    Volume,
}

#[derive(Default)]
struct Display {
    owner: Option<Owner>,
    overlay: Option<Frame>,
    reply_turn: Option<(Encoder, u64)>,
    until: Option<Instant>,
    revision: u64,
    pending_since: Option<Instant>,
    sent_overlay: Option<u64>,
    invalid_overlay: Option<u64>,
    direct: Option<Frame>,
    background: Option<Frame>,
    background_revision: u64,
    sent_background: Option<u64>,
    running: bool,
    requested: bool,
    report_upload: bool,
    write_timing: Option<WriteTiming>,
}

struct WriteTiming {
    wait: Duration,
    upload: Duration,
}

impl Display {
    fn active(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    fn current(&self, token: Token, now: Instant) -> bool {
        match token {
            Token::Overlay(revision) => self.active(now) && self.revision == revision,
            Token::Background { revision, overlay_revision } => {
                !self.active(now) && self.background_revision == revision && self.revision == overlay_revision
            }
        }
    }

    fn send_due(&self, token: Token, now: Instant, next_send: Instant, retry_at: Option<Instant>) -> Instant {
        let due = if matches!(token, Token::Overlay(_)) && self.reply_turn.is_some() { now } else { next_send };
        retry_at.map_or(due, |retry| due.max(retry))
    }

    fn queue(&mut self, owner: Owner, frame: Frame, now: Instant) {
        self.requested = true;
        let duplicate = self.owner.as_ref() == Some(&owner)
            && self.active(now)
            && match (&self.overlay, &frame) {
                (Some(Frame::Feedback(old)), Frame::Feedback(new)) => old == new,
                (Some(Frame::ReadyFeedback(old)), Frame::ReadyFeedback(new)) => old.model.same_visual(&new.model),
                _ => false,
            };
        if duplicate {
            return;
        }
        self.owner = Some(owner);
        self.overlay = Some(frame);
        self.reply_turn = None;
        self.revision = self.revision.wrapping_add(1);
        self.invalid_overlay = None;
        self.pending_since.get_or_insert(now);
        self.until = Some(now + SHOW_FOR);
    }

    fn expire(&mut self, now: Instant) {
        if self.until.is_some_and(|until| now >= until) {
            self.until = None;
            self.overlay = None;
            self.reply_turn = None;
            self.pending_since = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn needs_worker(&self, now: Instant) -> bool {
        self.until.is_some() || self.sent_overlay.is_some() || self.sent_background != Some(self.background_revision) || self.active(now)
    }
}

#[derive(Default)]
struct Registry {
    sources: HashMap<Encoder, Source>,
    displays: HashMap<String, Display>,
    pending_feedback: HashMap<Encoder, PendingFeedback>,
    feedback_revisions: HashMap<Encoder, (u64, FeedbackImage)>,
    feedback_serial: u64,
    turn_serial: u64,
    decoder_running: bool,
}

impl Registry {
    fn feedback(&mut self, encoder: Encoder, image: FeedbackImage, now: Instant) {
        // Kolejność obrotów enkoderów, które teraz zajmują LCD – nowszy obrót wygrywa.
        let turns: HashMap<Encoder, u64> = self.sources.iter().map(|(e, s)| (e.clone(), s.turn_order)).collect();
        let source = self.sources.entry(encoder.clone()).or_default();
        source.latest = Some(image.clone());
        if source.interaction_until.is_none_or(|until| now >= until) {
            return;
        }
        if !source.awaiting_feedback
            && !source.targets.iter().any(|target| self.displays.get(target).is_some_and(|display| display.active(now)))
        {
            // Pięć sekund jest oknem na spóźnioną ODPOWIEDŹ, a nie zgodą na
            // ponowne włączenie nakładki przez odświeżenie zakończonej interakcji.
            return;
        }
        for target in &source.targets {
            let display = self.displays.entry(target.clone()).or_default();
            // Nowszy obrót na innym enkoderze ma pierwszeństwo na wspólnym LCD. Pierwsza odpowiedź
            // po obrocie enkodera bez wcześniejszego paska zajmuje LCD tylko, gdy jest wolny.
            let owned = display.owner.as_ref() == Some(&Owner::Encoder(encoder.clone()));
            let older_owner = match &display.owner {
                Some(Owner::Encoder(owner)) => turns.get(owner).copied().unwrap_or(0) < source.turn_order,
                _ => false,
            };
            if owned || (source.awaiting_feedback && (!display.active(now) || older_owner)) {
                let frame = source
                    .ready
                    .as_ref()
                    .filter(|ready| ready.image == image)
                    .map_or_else(|| Frame::Feedback(image.clone()), |ready| Frame::ReadyFeedback(ready.clone()));
                display.queue(Owner::Encoder(encoder.clone()), frame, now);
                display.reply_turn = Some((encoder.clone(), source.turn_revision));
                if display.sent_overlay == Some(display.revision) {
                    source.awaiting_feedback = false;
                }
            }
        }
    }

    fn enqueue_feedback(&mut self, encoder: Encoder, image: FeedbackImage, now: Instant) {
        if self.feedback_revisions.get(&encoder).is_some_and(|(_, old)| old == &image) {
            if let Some(ready) = self.sources.get(&encoder).and_then(|source| source.ready.as_ref()).filter(|ready| ready.image == image) {
                let ready = Arc::new(ready.received_again(now));
                self.accept_ready(encoder, ready, now);
            }
            return;
        }
        self.feedback_serial = self.feedback_serial.wrapping_add(1);
        self.feedback_revisions.insert(encoder.clone(), (self.feedback_serial, image.clone()));
        self.pending_feedback.insert(encoder, PendingFeedback { revision: self.feedback_serial, image, received_at: now });
    }

    fn accept_ready(&mut self, encoder: Encoder, ready: Arc<ReadyFeedback>, now: Instant) {
        let source = self.sources.entry(encoder.clone()).or_default();
        source.cached = Some(ready.image.clone());
        source.ready = Some(ready.clone());
        if source.log_next_feedback && source.last_turn.is_some_and(|turn| ready.received_at >= turn) {
            log::info!(
                "{}[{}]: feedback po {:.1} ms od obrotu; przygotowanie {:.1} ms; JPEG {} B / {} raportów",
                encoder.device,
                encoder.position,
                ready.received_at.duration_since(source.last_turn.unwrap()).as_secs_f64() * 1000.0,
                ready.prepare_for.as_secs_f64() * 1000.0,
                ready.jpeg.len(),
                ready.jpeg.len().div_ceil(312)
            );
            source.log_next_feedback = false;
        }
        self.feedback(encoder, ready.image.clone(), now);
    }

    fn accept_ready_if_current(&mut self, encoder: Encoder, revision: u64, ready: Arc<ReadyFeedback>, now: Instant) {
        if self.feedback_revisions.get(&encoder).is_some_and(|(current, _)| *current == revision) {
            self.accept_ready(encoder, ready, now);
        }
    }

    /// Wynik dekodera: gotowy pasek albo `None` – zwykła ikona w miejscu dawnego paska
    /// (np. inny profil) usuwa zapamiętany feedback tego enkodera.
    fn deliver_if_current(&mut self, encoder: Encoder, revision: u64, ready: Option<Arc<ReadyFeedback>>, now: Instant) {
        match ready {
            Some(ready) => self.accept_ready_if_current(encoder, revision, ready, now),
            None if self.feedback_revisions.get(&encoder).is_some_and(|(current, _)| *current == revision) => {
                let (device, position) = (encoder.device.clone(), encoder.position);
                self.clear(&device, Some(position), now);
            }
            None => {}
        }
    }

    fn twist(&mut self, encoder: Encoder, targets: Vec<String>, now: Instant) {
        self.turn_serial += 1;
        let source = self.sources.entry(encoder.clone()).or_default();
        source.turn_order = self.turn_serial;
        let new_interaction = source.last_turn.is_none_or(|last| now.saturating_duration_since(last) >= SHOW_FOR);
        source.interaction_until = Some(now + FEEDBACK_WAIT_FOR);
        source.targets = targets;
        source.turn_revision = source.turn_revision.wrapping_add(1);
        source.awaiting_feedback = true;
        source.last_turn = Some(now);
        source.log_next_feedback |= new_interaction;
        if source.latest.is_none() && source.cached.is_none() {
            // Enkoder nie pokazał jeszcze paska (np. zwykła akcja) – czekamy na odpowiedź,
            // ale nie zasłaniamy zegara ani innej nakładki.
            return;
        }
        for target in &source.targets {
            let display = self.displays.entry(target.clone()).or_default();
            let owner = Owner::Encoder(encoder.clone());
            display.report_upload |= new_interaction || display.owner.as_ref() != Some(&owner) || !display.active(now);
            if display.owner.as_ref() == Some(&owner)
                && display.active(now)
                && display.overlay.is_some()
                && display.invalid_overlay.is_none()
            {
                // Obrót nie odtwarza starszego cache nad nowszym celem w kolejce.
            } else if let Some(cached) = source.latest.as_ref().or(source.cached.as_ref()) {
                // Wybór źródła i skopiowanie pamięci odbywają się pod jedną blokadą:
                // stara kopia cache nie może wyprzedzić nowego feedbacku.
                let frame = source
                    .ready
                    .as_ref()
                    .filter(|ready| ready.image == *cached)
                    .map_or_else(|| Frame::Feedback(cached.clone()), |ready| Frame::ReadyFeedback(ready.clone()));
                display.queue(owner, frame, now);
            } else if display.owner.as_ref() != Some(&owner) {
                display.owner = Some(owner);
                display.overlay = None;
                display.pending_since = None;
                display.revision = display.revision.wrapping_add(1);
            }
            display.until = Some(now + SHOW_FOR);
            display.reply_turn = None;
            display.requested = true;
        }
    }

    fn clear(&mut self, device: &str, position: Option<u8>, now: Instant) {
        let matches = |encoder: &Encoder| encoder.device == device && position.is_none_or(|p| p == encoder.position);
        self.sources.retain(|encoder, _| !matches(encoder));
        self.pending_feedback.retain(|encoder, _| !matches(encoder));
        self.feedback_revisions.retain(|encoder, _| !matches(encoder));
        for display in self.displays.values_mut() {
            if matches!(&display.owner, Some(Owner::Encoder(encoder)) if matches(encoder)) {
                display.owner = None;
                display.overlay = None;
                display.reply_turn = None;
                display.until = Some(now);
                display.pending_since = None;
                display.revision = display.revision.wrapping_add(1);
                display.invalid_overlay = None;
                display.requested = true;
            }
        }
    }

    fn claim_workers(&mut self, now: Instant) -> Vec<String> {
        self.displays
            .iter_mut()
            .filter_map(|(device, display)| {
                let requested = std::mem::take(&mut display.requested);
                if requested && !display.running && display.needs_worker(now) {
                    display.running = true;
                    Some(device.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<Registry>,
    changed: Condvar,
}

static SHARED: LazyLock<Arc<Shared>> = LazyLock::new(Default::default);

fn change(f: impl FnOnce(&mut Registry, Instant)) {
    let (starts, decode) = {
        let mut registry = SHARED.state.lock().unwrap();
        let now = Instant::now();
        f(&mut registry, now);
        let starts = registry.claim_workers(now);
        let decode = !registry.pending_feedback.is_empty() && !registry.decoder_running;
        if decode {
            registry.decoder_running = true;
        }
        (starts, decode)
    };
    SHARED.changed.notify_all();
    if decode {
        start_feedback_decoder();
    }
    for device in starts {
        let shared = SHARED.clone();
        thread::spawn(move || {
            worker(&shared, &device, |jpeg, token| {
                let mut outcome = Written::Superseded;
                let waiting = Instant::now();
                let mut timing = None;
                let ok = with_writer(&device, |dev| {
                    // Sprawdzamy po przejęciu blokady HID, tuż przed pierwszym raportem.
                    if shared.current(&device, token, Instant::now()) {
                        let started = Instant::now();
                        hub::upload_dial_image(dev, jpeg)?;
                        timing = Some(WriteTiming { wait: started.duration_since(waiting), upload: started.elapsed() });
                        outcome = Written::Sent;
                    }
                    Ok(())
                });
                if let Some(timing) = timing {
                    if let Some(display) = shared.state.lock().unwrap().displays.get_mut(&device) {
                        display.write_timing = Some(timing);
                    }
                }
                if ok { outcome } else { Written::Failed }
            })
        });
    }
}

fn start_feedback_decoder() {
    let shared = SHARED.clone();
    thread::spawn(move || {
        feedback_decoder(&shared, |encoder, revision, ready| {
            change(|registry, now| registry.deliver_if_current(encoder, revision, ready, now));
        });
    });
}

/// LCD ma własne źródło. Moduł bez ekranu korzysta z podłączonych LCD.
fn feedback_targets<'a>(device: &str, modules: impl Iterator<Item = (&'a str, Kind)>) -> Vec<String> {
    let modules: Vec<_> = modules.collect();
    let own_lcd = modules.iter().any(|&(id, kind)| id == device && kind == Kind::Dial);
    modules.into_iter().filter(|(id, kind)| *kind == Kind::Dial && (!own_lcd || *id == device)).map(|(id, _)| id.to_owned()).collect()
}

pub fn encoder_twist(device: &str, position: u8) {
    let targets = feedback_targets(device, MODULES.lock().unwrap().iter().map(|(id, module)| (id.as_str(), module.kind)));
    change(|registry, now| registry.twist(Encoder::new(device, position), targets, now));
}

pub fn show_feedback(device: &str, position: Option<u8>, count: u8, image: String) {
    let data: Arc<str> = image.into();
    change(|registry, now| {
        let positions: Vec<u8> = position.map_or_else(|| (0..count).collect(), |p| vec![p]);
        for p in positions {
            let row = (position.is_none() && count > 1).then_some((p, count));
            registry.enqueue_feedback(Encoder::new(device, p), FeedbackImage { data: data.clone(), row }, now);
        }
    });
}

/// Konwersja działa niezależnie od zajętego HID. Jedna najnowsza klatka na źródło,
/// obracany enkoder ma pierwszeństwo przed odświeżeniami pozostałych profili.
/// `None` = obraz bez paska: enkoder przestaje mieć feedback.
fn feedback_decoder(shared: &Shared, mut deliver: impl FnMut(Encoder, u64, Option<Arc<ReadyFeedback>>)) {
    loop {
        let job = {
            let mut registry = shared.state.lock().unwrap();
            let now = Instant::now();
            let next = registry
                .pending_feedback
                .iter()
                .min_by_key(|(encoder, pending)| {
                    let active =
                        registry.sources.get(*encoder).is_some_and(|source| source.interaction_until.is_some_and(|until| now < until));
                    (!active, pending.revision)
                })
                .map(|(encoder, _)| encoder.clone());
            match next {
                Some(encoder) => {
                    let pending = registry.pending_feedback.remove(&encoder).unwrap();
                    Some((encoder, pending))
                }
                None => {
                    registry.decoder_running = false;
                    None
                }
            }
        };
        let Some((encoder, pending)) = job else { return };
        match ReadyFeedback::prepare(pending.image, pending.received_at) {
            Ok(ready) => deliver(encoder, pending.revision, Some(Arc::new(ready))),
            Err(PrepareError::IncompleteFeedback) => {}
            Err(PrepareError::NotBar) => deliver(encoder, pending.revision, None),
            Err(PrepareError::Decode(error)) => log::error!("{}[{}]: feedback: {error}", encoder.device, encoder.position),
        }
    }
}

pub fn clear_feedback(device: &str, position: Option<u8>) {
    change(|registry, now| registry.clear(device, position, now));
}

pub fn show_on(device: &str, img: RgbImage) {
    change(|registry, now| registry.displays.entry(device.into()).or_default().queue(Owner::Volume, Frame::Image(Arc::new(img)), now));
}

pub fn show(img: RgbImage) {
    let targets: Vec<String> =
        MODULES.lock().unwrap().iter().filter(|(_, module)| module.kind == Kind::Dial).map(|(id, _)| id.clone()).collect();
    for device in targets {
        show_on(&device, img.clone());
    }
}

pub fn set_direct(device: &str, img: RgbImage) {
    change(|registry, _| {
        let display = registry.displays.entry(device.into()).or_default();
        display.direct = Some(Frame::Image(Arc::new(img)));
        display.background_revision = display.background_revision.wrapping_add(1);
        display.requested = true;
    });
}

pub fn clear_direct(device: &str) -> bool {
    let mut had_direct = false;
    change(|registry, _| {
        if let Some(display) = registry.displays.get_mut(device) {
            had_direct = display.direct.take().is_some();
            if had_direct {
                display.background_revision = display.background_revision.wrapping_add(1);
                display.requested = true;
            }
        }
    });
    had_direct
}

pub fn refresh_direct(device: &str) {
    change(|registry, _| {
        if let Some(display) = registry.displays.get_mut(device) {
            display.sent_background = None;
            display.requested = true;
        }
    });
}

pub fn remember(device: &str, image: Option<String>) {
    set_background(device, Frame::KeyImage(image.map(Arc::from)));
}

pub fn set_background_jpeg(device: &str, jpeg: Vec<u8>) {
    set_background(device, Frame::Jpeg(jpeg.into()));
}

fn set_background(device: &str, frame: Frame) {
    change(|registry, _| {
        let display = registry.displays.entry(device.into()).or_default();
        display.background = Some(frame);
        if display.direct.is_none() {
            display.background_revision = display.background_revision.wrapping_add(1);
            display.requested = true;
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Overlay(u64),
    Background { revision: u64, overlay_revision: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Written {
    Sent,
    Superseded,
    Failed,
}

impl Shared {
    fn current(&self, device: &str, token: Token, now: Instant) -> bool {
        self.state.lock().unwrap().displays.get(device).is_some_and(|display| display.current(token, now))
    }
}

fn worker(shared: &Shared, device: &str, mut write: impl FnMut(&[u8], Token) -> Written) {
    let mut next_send = Instant::now();
    let mut retry_at = None;
    let mut prepared: Option<(Token, Arc<[u8]>)> = None;
    let mut background_failures = 0;
    loop {
        let (frame, token, queued_at) = {
            let mut registry = shared.state.lock().unwrap();
            loop {
                let now = Instant::now();
                let display = registry.displays.get_mut(device).unwrap();
                display.expire(now);
                let work = if display.active(now) {
                    display
                        .overlay
                        .as_ref()
                        .filter(|_| display.sent_overlay != Some(display.revision) && display.invalid_overlay != Some(display.revision))
                        .map(|frame| (frame.clone(), Token::Overlay(display.revision)))
                } else if display.sent_overlay.is_some() || display.sent_background != Some(display.background_revision) {
                    Some((
                        display.direct.clone().or_else(|| display.background.clone()).unwrap_or(Frame::KeyImage(None)),
                        Token::Background { revision: display.background_revision, overlay_revision: display.revision },
                    ))
                } else {
                    display.running = false;
                    return;
                };
                if let Some(work) = work {
                    let due = display.send_due(work.1, now, next_send, retry_at);
                    if now >= due {
                        let mut queued_at = display.pending_since.unwrap_or(now);
                        if let Frame::ReadyFeedback(ready) = &work.0 {
                            queued_at = queued_at.max(ready.prepared_at);
                        }
                        break (work.0, work.1, queued_at);
                    }
                    let due = display.until.map_or(due, |until| until.min(due));
                    registry = shared.changed.wait_timeout(registry, due.saturating_duration_since(now)).unwrap().0;
                } else {
                    let until = display.until.unwrap();
                    registry = shared.changed.wait_timeout(registry, until.saturating_duration_since(now)).unwrap().0;
                }
            }
        };
        if prepared.as_ref().is_none_or(|(old, _)| *old != token) {
            match frame.jpeg() {
                Ok(jpeg) => prepared = Some((token, jpeg)),
                Err(error) => {
                    if let PrepareError::Decode(error) = &error {
                        log::error!("{device}: nie udało się przygotować LCD: {error}");
                    }
                    let mut registry = shared.state.lock().unwrap();
                    let display = registry.displays.get_mut(device).unwrap();
                    if !display.current(token, Instant::now()) {
                        continue;
                    }
                    let owner = display.owner.clone();
                    match token {
                        Token::Overlay(revision) => display.invalid_overlay = Some(revision),
                        Token::Background { .. } => {
                            display.running = false;
                            return;
                        }
                    }
                    if let (Some(Owner::Encoder(encoder)), Frame::Feedback(image)) = (owner, &frame) {
                        if let Some(source) = registry.sources.get_mut(&encoder) {
                            if source.latest.as_ref() == Some(image) {
                                source.latest = None;
                            }
                        }
                    }
                    shared.changed.notify_all();
                    continue;
                }
            }
            if let (Token::Overlay(revision), Frame::Feedback(image)) = (token, &frame) {
                let mut registry = shared.state.lock().unwrap();
                if let Some(display) = registry.displays.get(device).filter(|display| display.revision == revision) {
                    if let Some(Owner::Encoder(encoder)) = display.owner.clone() {
                        if let Some(source) = registry.sources.get_mut(&encoder) {
                            source.cached = Some(image.clone());
                        }
                    }
                }
            }
        }
        let started = Instant::now();
        let jpeg = &prepared.as_ref().unwrap().1;
        let outcome = write(jpeg, token);
        let mut registry = shared.state.lock().unwrap();
        let display = registry.displays.get_mut(device).unwrap();
        let mut acknowledged = None;
        match outcome {
            Written::Sent => {
                next_send = started + FRAME_EVERY;
                retry_at = None;
                background_failures = 0;
                match token {
                    Token::Overlay(revision) => {
                        display.sent_overlay = Some(revision);
                        if display.revision == revision {
                            display.pending_since = None;
                            acknowledged = display.reply_turn.clone();
                        }
                        if std::mem::take(&mut display.report_upload) {
                            let timing = display.write_timing.take();
                            log::info!(
                                "{device}: LCD kolejka {:.1} ms; blokada HID {:.1} ms; zapis {:.1} ms; {} B / {} raportów",
                                started.saturating_duration_since(queued_at).as_secs_f64() * 1000.0,
                                timing.as_ref().map_or(0.0, |timing| timing.wait.as_secs_f64() * 1000.0),
                                timing
                                    .as_ref()
                                    .map_or(started.elapsed().as_secs_f64() * 1000.0, |timing| timing.upload.as_secs_f64() * 1000.0),
                                jpeg.len(),
                                jpeg.len().div_ceil(312)
                            );
                        }
                    }
                    Token::Background { revision, .. } => {
                        display.sent_background = Some(revision);
                        // Jeśli obrót nastąpił w trakcie zapisu zegara, trzeba ponownie
                        // wysłać także identyczny feedback: LCD pokazuje już tło.
                        display.sent_overlay = None;
                    }
                }
                log::debug!("{device}: LCD {token:?}, {} B, zapis {:?}", jpeg.len(), started.elapsed());
            }
            Written::Superseded => prepared = None,
            Written::Failed => {
                // Nie oznaczamy klatki jako wysłanej. Taki sam feedback nadal czeka
                // i jest ponawiany bez następnego obrotu lub zmiany procentu.
                retry_at = Some(Instant::now() + RETRY_AFTER);
                if matches!(token, Token::Background { .. }) {
                    background_failures += 1;
                    if background_failures >= 3 && display.current(token, Instant::now()) {
                        display.running = false;
                        return;
                    }
                }
            }
        }
        if let Some((encoder, turn_revision)) = acknowledged {
            if let Some(source) = registry.sources.get_mut(&encoder) {
                if source.turn_revision == turn_revision {
                    source.awaiting_feedback = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::mpsc;

    fn feedback(bytes: &[u8]) -> FeedbackImage {
        FeedbackImage {
            data: format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)).into(),
            row: None,
        }
    }

    fn fifty() -> FeedbackImage {
        feedback(include_bytes!("../tests/fixtures/encoder-b1/b1-50.jpg"))
    }

    fn fifty_five() -> FeedbackImage {
        feedback(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg"))
    }

    fn shared_with_feedback(encoder: &Encoder, image: FeedbackImage) -> Shared {
        // W produkcji atlas i JPEG powstają w dekoderze, przed publikacją na LCD.
        // Inicjalizacja czcionek/wzorców nie jest częścią symulowanego timeoutu HID.
        let _ = Frame::Feedback(image.clone()).jpeg();
        let shared = Shared::default();
        let now = Instant::now();
        let mut registry = shared.state.lock().unwrap();
        registry.displays.insert(
            "lcd".into(),
            Display { background: Some(Frame::Jpeg(Arc::from(&b"clock"[..]))), sent_background: Some(0), ..Display::default() },
        );
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        registry.feedback(encoder.clone(), image, now);
        assert_eq!(registry.claim_workers(now), ["lcd"]);
        drop(registry);
        shared
    }

    fn end_overlay(shared: &Shared) {
        shared.state.lock().unwrap().displays.get_mut("lcd").unwrap().until = Some(Instant::now());
        shared.changed.notify_all();
    }

    fn queued_feedback(registry: &Registry, device: &str) -> Option<FeedbackImage> {
        match &registry.displays.get(device)?.overlay {
            Some(Frame::Feedback(image)) => Some(image.clone()),
            _ => None,
        }
    }

    #[test]
    fn dial_targets_itself_knobs_and_rollers_target_connected_lcds() {
        let modules = [
            ("lcd-a", Kind::Dial),
            ("knobs", Kind::Knob3),
            ("rollers", Kind::Roller2),
            ("lcd-b", Kind::Dial),
            ("faders", Kind::Fader),
            ("keys", Kind::Keys15),
        ];
        assert_eq!(feedback_targets("lcd-a", modules.into_iter()), ["lcd-a"]);
        for source in ["knobs", "rollers", "faders"] {
            assert_eq!(feedback_targets(source, modules.into_iter()), ["lcd-a", "lcd-b"]);
        }
        assert!(feedback_targets("knobs", [("knobs", Kind::Knob3)].into_iter()).is_empty());
    }

    #[test]
    fn every_knob_and_roller_position_can_show_its_own_feedback() {
        let now = Instant::now();
        for (device, count) in [("knobs", 3), ("rollers", 2), ("dial", 1)] {
            for position in 0..count {
                let mut registry = Registry::default();
                let encoder = Encoder::new(device, position);
                registry.feedback(encoder.clone(), fifty_five(), now);
                assert!(registry.displays.is_empty(), "setImage samo nie otwiera nakładki");
                registry.twist(encoder, vec!["lcd".into()], now);
                assert!(queued_feedback(&registry, "lcd") == Some(fifty_five()));
                assert!(registry.displays["lcd"].active(now));
            }
        }
    }

    #[test]
    fn profile_refresh_and_other_position_cannot_open_or_replace_overlay() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let active = Encoder::new("knobs", 1);
        let other = Encoder::new("knobs", 2);
        registry.feedback(active.clone(), fifty(), now);
        registry.feedback(other.clone(), fifty_five(), now);
        assert!(registry.claim_workers(now).is_empty());
        registry.twist(active.clone(), vec!["lcd".into()], now);
        registry.feedback(other.clone(), fifty_five(), now);
        assert!(queued_feedback(&registry, "lcd") == Some(fifty()));
        assert!(registry.sources[&other].interaction_until.is_none());
        registry.feedback(active, fifty_five(), now);
        assert!(queued_feedback(&registry, "lcd") == Some(fifty_five()));
    }

    #[test]
    fn latest_rotated_source_owns_lcd_even_if_old_source_replies_later() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let knob = Encoder::new("knobs", 2);
        let roller = Encoder::new("rollers", 1);
        registry.twist(knob.clone(), vec!["lcd".into()], now);
        registry.feedback(knob.clone(), fifty(), now);
        registry.twist(roller.clone(), vec!["lcd".into()], now);
        registry.feedback(roller, fifty_five(), now);
        registry.feedback(knob.clone(), fifty(), now + Duration::from_millis(100));
        assert!(queued_feedback(&registry, "lcd") == Some(fifty_five()));
        assert!(registry.sources[&knob].latest == Some(fifty()));
    }

    #[test]
    fn turning_an_encoder_without_a_bar_does_not_take_over_the_lcd() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let volume_knob = Encoder::new("knobs", 1);
        registry.twist(volume_knob.clone(), vec!["lcd".into()], now);
        assert!(registry.displays.get("lcd").is_none_or(|display| !display.active(now)));
        assert!(registry.claim_workers(now).is_empty());
        // Zegar ustawiony wcześniej zostaje tłem, a nie zastępuje go pusta nakładka.
        let display = registry.displays.entry("lcd".into()).or_default();
        assert!(display.overlay.is_none() && display.owner.is_none());
    }

    #[test]
    fn plain_icon_is_not_feedback() {
        let icon = Frame::Feedback(FeedbackImage { data: Arc::from(plain_icon_data_url()), row: None });
        assert!(matches!(icon.jpeg(), Err(PrepareError::NotBar)));
    }

    fn plain_icon_data_url() -> String {
        use base64::Engine;
        let img = RgbImage::from_fn(144, 144, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 90]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
    }

    #[test]
    fn late_ha_reply_is_accepted_after_the_old_1500ms_window() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let encoder = Encoder::new("rollers", 0);
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        registry.displays.entry("lcd".into()).or_default().expire(now + SHOW_FOR);
        registry.feedback(encoder.clone(), fifty_five(), now + Duration::from_secs(2));
        assert!(queued_feedback(&registry, "lcd") == Some(fifty_five()));
        assert!(registry.displays["lcd"].active(now + Duration::from_secs(2)));
        registry.displays.get_mut("lcd").unwrap().expire(now + Duration::from_secs(4));
        registry.feedback(encoder, fifty(), now + FEEDBACK_WAIT_FOR);
        assert!(queued_feedback(&registry, "lcd").is_none());
        assert!(!registry.displays["lcd"].active(now + FEEDBACK_WAIT_FOR));
    }

    #[test]
    fn burst_keeps_latest_duplicates_do_not_renew_timeout_and_twists_do() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let encoder = Encoder::new("knobs", 0);
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        for n in 0..100 {
            registry.feedback(encoder.clone(), FeedbackImage { data: format!("frame-{n}").into(), row: None }, now);
        }
        let revision = registry.displays["lcd"].revision;
        let until = registry.displays["lcd"].until;
        let latest = registry.sources[&encoder].latest.clone().unwrap();
        registry.feedback(encoder.clone(), latest.clone(), now + Duration::from_millis(100));
        assert_eq!(registry.displays["lcd"].revision, revision);
        assert_eq!(registry.displays["lcd"].until, until);
        assert!(queued_feedback(&registry, "lcd") == Some(latest));
        registry.twist(encoder, vec!["lcd".into()], now + Duration::from_millis(200));
        assert_eq!(registry.displays["lcd"].revision, revision);
        assert_eq!(registry.displays["lcd"].until, Some(now + Duration::from_millis(200) + SHOW_FOR));
    }

    #[test]
    fn clear_one_encoder_does_not_clear_other_encoder_or_volume() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let a = Encoder::new("knobs", 0);
        let b = Encoder::new("knobs", 1);
        registry.twist(a.clone(), vec!["lcd-a".into()], now);
        registry.twist(b.clone(), vec!["lcd-b".into()], now);
        registry.feedback(a.clone(), fifty(), now);
        registry.feedback(b.clone(), fifty_five(), now);
        registry.clear("knobs", Some(0), now);
        assert!(!registry.sources.contains_key(&a));
        assert!(queued_feedback(&registry, "lcd-a").is_none());
        assert!(queued_feedback(&registry, "lcd-b") == Some(fifty_five()));
        registry.displays.get_mut("lcd-a").unwrap().queue(Owner::Volume, Frame::Jpeg(Arc::from(&b"volume"[..])), now);
        registry.clear("knobs", None, now);
        assert!(registry.sources.is_empty());
        assert!(registry.displays["lcd-a"].active(now));
        assert!(registry.displays["lcd-a"].owner == Some(Owner::Volume));
    }

    #[test]
    fn clearing_profile_prevents_old_feedback_from_reopening_shared_lcd() {
        let now = Instant::now();
        let encoder = Encoder::new("rollers", 1);
        let mut registry = Registry::default();
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        registry.feedback(encoder.clone(), fifty_five(), now);
        registry.clear("rollers", None, now);
        registry.feedback(encoder.clone(), fifty(), now + Duration::from_millis(10));
        assert!(queued_feedback(&registry, "lcd").is_none());
        assert!(registry.sources[&encoder].interaction_until.is_none());
    }

    #[test]
    fn cached_value_cannot_roll_back_a_newer_confirmed_feedback_on_twist() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let encoder = Encoder::new("knobs", 1);
        registry.feedback(encoder.clone(), fifty(), now);
        registry.sources.get_mut(&encoder).unwrap().cached = Some(fifty());
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        registry.feedback(encoder.clone(), fifty_five(), now);
        let revision = registry.displays["lcd"].revision;
        registry.twist(encoder, vec!["lcd".into()], now + Duration::from_millis(1));
        assert!(queued_feedback(&registry, "lcd") == Some(fifty_five()));
        assert_eq!(registry.displays["lcd"].revision, revision);
    }

    #[test]
    fn five_percent_targets_send_only_confirmed_frames_even_with_slow_usb() {
        let encoder = Encoder::new("knobs", 2);
        let shared = shared_with_feedback(&encoder, fifty());
        let sequence = [fifty(), fifty_five(), fifty(), fifty_five()];
        let expected: Vec<_> = sequence.iter().map(|image| Frame::Feedback(image.clone()).jpeg().unwrap()).collect();
        let mut sent = Vec::new();
        worker(&shared, "lcd", |jpeg, token| {
            assert!(shared.current("lcd", token, Instant::now()));
            sent.push(jpeg.to_vec());
            if matches!(token, Token::Overlay(_)) {
                thread::sleep(Duration::from_millis(40));
                let next = sent.len();
                if let Some(image) = sequence.get(next) {
                    shared.state.lock().unwrap().feedback(encoder.clone(), image.clone(), Instant::now());
                } else {
                    end_overlay(&shared);
                }
            }
            Written::Sent
        });
        assert_eq!(sent.len(), expected.len() + 1, "brak dodatkowych klatek z poziomami pośrednimi");
        for (actual, expected) in sent.iter().zip(expected) {
            assert_eq!(actual.as_slice(), expected.as_ref());
        }
        assert_eq!(sent.last().unwrap(), b"clock");
        assert!(!shared.state.lock().unwrap().displays["lcd"].running);
    }

    #[test]
    fn confirmed_reply_bypasses_the_frame_interval_but_cached_images_do_not() {
        let encoder = Encoder::new("knobs", 1);
        let now = Instant::now();
        let next_send = now + FRAME_EVERY;
        let mut registry = Registry::default();
        registry.feedback(encoder.clone(), fifty(), now);
        registry.twist(encoder.clone(), vec!["lcd".into()], now);
        let display = &registry.displays["lcd"];
        assert_eq!(display.send_due(Token::Overlay(display.revision), now, next_send, None), next_send);

        registry.feedback(encoder, fifty_five(), now);
        let display = &registry.displays["lcd"];
        assert_eq!(display.send_due(Token::Overlay(display.revision), now, next_send, None), now);
        let background = Token::Background { revision: display.background_revision, overlay_revision: display.revision };
        assert_eq!(display.send_due(background, now, next_send, None), next_send);
    }

    #[test]
    fn confirmed_reply_respects_usb_error_backoff() {
        let encoder = Encoder::new("rollers", 0);
        let shared = shared_with_feedback(&encoder, fifty_five());
        let registry = shared.state.lock().unwrap();
        let display = &registry.displays["lcd"];
        let now = Instant::now();
        let retry_at = now + RETRY_AFTER;
        assert_eq!(display.send_due(Token::Overlay(display.revision), now, now + FRAME_EVERY, Some(retry_at)), retry_at);
    }

    #[test]
    fn failed_usb_write_retries_identical_feedback_without_a_new_rotation() {
        let encoder = Encoder::new("rollers", 1);
        let shared = shared_with_feedback(&encoder, fifty_five());
        let mut attempts = Vec::new();
        worker(&shared, "lcd", |jpeg, token| {
            assert!(shared.current("lcd", token, Instant::now()));
            attempts.push(jpeg.to_vec());
            if attempts.len() == 1 {
                shared.state.lock().unwrap().feedback(encoder.clone(), fifty_five(), Instant::now());
                return Written::Failed;
            }
            if matches!(token, Token::Overlay(_)) {
                end_overlay(&shared);
            }
            Written::Sent
        });
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0], attempts[1]);
        assert_eq!(attempts[2], b"clock");
    }

    #[test]
    fn replaced_frame_is_dropped_after_waiting_for_the_writer_lock() {
        let encoder = Encoder::new("rollers", 0);
        let shared = Arc::new(shared_with_feedback(&encoder, fifty()));
        let expected = Frame::Feedback(fifty_five()).jpeg().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (sent_tx, sent_rx) = mpsc::channel();
        let worker_shared = shared.clone();
        let handle = thread::spawn(move || {
            let mut first = true;
            worker(&worker_shared, "lcd", |jpeg, token| {
                if first {
                    first = false;
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                if !worker_shared.current("lcd", token, Instant::now()) {
                    return Written::Superseded;
                }
                sent_tx.send(jpeg.to_vec()).unwrap();
                if matches!(token, Token::Overlay(_)) {
                    end_overlay(&worker_shared);
                }
                Written::Sent
            });
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        shared.state.lock().unwrap().feedback(encoder, fifty_five(), Instant::now());
        shared.changed.notify_all();
        release_tx.send(()).unwrap();
        assert_eq!(sent_rx.recv_timeout(Duration::from_secs(2)).unwrap().as_slice(), expected.as_ref());
        assert_eq!(sent_rx.recv_timeout(Duration::from_secs(2)).unwrap(), b"clock");
        handle.join().unwrap();
        assert!(sent_rx.try_recv().is_err());
    }

    #[test]
    fn rotation_while_clock_is_written_redisplays_the_same_percentage() {
        let encoder = Encoder::new("knobs", 1);
        let shared = shared_with_feedback(&encoder, fifty_five());
        let expected = Frame::Feedback(fifty_five()).jpeg().unwrap();
        let mut sent = Vec::new();
        let mut clocks = 0;
        worker(&shared, "lcd", |jpeg, token| {
            assert!(shared.current("lcd", token, Instant::now()));
            sent.push(jpeg.to_vec());
            match token {
                Token::Overlay(_) => end_overlay(&shared),
                Token::Background { .. } => {
                    clocks += 1;
                    if clocks == 1 {
                        shared.state.lock().unwrap().twist(encoder.clone(), vec!["lcd".into()], Instant::now());
                    }
                }
            }
            Written::Sent
        });
        assert_eq!(sent, [expected.to_vec(), b"clock".to_vec(), expected.to_vec(), b"clock".to_vec()]);
    }

    #[test]
    fn clock_and_gif_updates_wait_until_feedback_finishes_then_latest_direct_wins() {
        let encoder = Encoder::new("knobs", 0);
        let shared = shared_with_feedback(&encoder, fifty_five());
        let mut sent = Vec::new();
        worker(&shared, "lcd", |jpeg, token| {
            assert!(shared.current("lcd", token, Instant::now()));
            sent.push(jpeg.to_vec());
            if matches!(token, Token::Overlay(_)) {
                {
                    let mut registry = shared.state.lock().unwrap();
                    let display = registry.displays.get_mut("lcd").unwrap();
                    display.background = Some(Frame::Jpeg(Arc::from(&b"gif"[..])));
                    display.direct = Some(Frame::Jpeg(Arc::from(&b"latest-clock"[..])));
                    display.background_revision += 1;
                }
                assert!(!shared.current("lcd", Token::Background { revision: 1, overlay_revision: 2 }, Instant::now()));
                end_overlay(&shared);
            }
            Written::Sent
        });
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1], b"latest-clock");
    }

    #[test]
    fn failed_background_stops_after_three_attempts_and_unrelated_updates_do_not_restart_it() {
        let shared = Shared::default();
        shared.state.lock().unwrap().displays.insert("lcd".into(), Display { running: true, ..Display::default() });
        let mut attempts = 0;
        worker(&shared, "lcd", |_, _| {
            attempts += 1;
            Written::Failed
        });
        assert_eq!(attempts, 3);
        let mut registry = shared.state.lock().unwrap();
        assert!(!registry.displays["lcd"].running);
        registry.feedback(Encoder::new("other-knobs", 0), fifty(), Instant::now());
        assert!(registry.claim_workers(Instant::now()).is_empty());
    }

    #[test]
    fn invalid_feedback_does_not_spin_or_prevent_restoring_the_clock() {
        let shared = shared_with_feedback(&Encoder::new("knobs", 2), FeedbackImage { data: "broken".into(), row: None });
        // Na LCD pozostała poprzednia, poprawna nakładka.
        shared.state.lock().unwrap().displays.get_mut("lcd").unwrap().sent_overlay = Some(0);
        let worker_shared = Arc::new(shared);
        let thread_shared = worker_shared.clone();
        let handle = thread::spawn(move || {
            let mut sent = Vec::new();
            worker(&thread_shared, "lcd", |jpeg, token| {
                assert!(thread_shared.current("lcd", token, Instant::now()));
                sent.push(jpeg.to_vec());
                Written::Sent
            });
            sent
        });
        {
            let registry = worker_shared.state.lock().unwrap();
            let (registry, timeout) = worker_shared
                .changed
                .wait_timeout_while(registry, Duration::from_secs(2), |registry| registry.displays["lcd"].invalid_overlay.is_none())
                .unwrap();
            assert!(!timeout.timed_out(), "błędny obraz powinien zostać odrzucony");
            drop(registry);
        }
        end_overlay(&worker_shared);
        assert_eq!(handle.join().unwrap(), [b"clock".to_vec()]);
    }

    #[test]
    fn newer_feedback_replaces_the_frame_waiting_for_a_usb_retry() {
        let encoder = Encoder::new("knobs", 2);
        let shared = shared_with_feedback(&encoder, fifty());
        let expected = Frame::Feedback(fifty_five()).jpeg().unwrap();
        let mut attempts = Vec::new();
        worker(&shared, "lcd", |jpeg, token| {
            assert!(shared.current("lcd", token, Instant::now()));
            attempts.push(jpeg.to_vec());
            if attempts.len() == 1 {
                shared.state.lock().unwrap().feedback(encoder.clone(), fifty_five(), Instant::now());
                return Written::Failed;
            }
            if matches!(token, Token::Overlay(_)) {
                end_overlay(&shared);
            }
            Written::Sent
        });
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[1].as_slice(), expected.as_ref());
        assert_eq!(attempts[2], b"clock");
    }

    #[test]
    fn old_clock_waiting_for_writer_cannot_overtake_new_rotation() {
        let _ = Frame::Feedback(fifty_five()).jpeg();
        let shared = Arc::new(Shared::default());
        shared
            .state
            .lock()
            .unwrap()
            .displays
            .insert("lcd".into(), Display { background: Some(Frame::Jpeg(Arc::from(&b"clock"[..]))), running: true, ..Display::default() });
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_shared = shared.clone();
        let handle = thread::spawn(move || {
            let mut first = true;
            let mut sent = Vec::new();
            worker(&worker_shared, "lcd", |jpeg, token| {
                if first {
                    first = false;
                    assert!(matches!(token, Token::Background { .. }));
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                if !worker_shared.current("lcd", token, Instant::now()) {
                    return Written::Superseded;
                }
                sent.push(jpeg.to_vec());
                if matches!(token, Token::Overlay(_)) {
                    end_overlay(&worker_shared);
                }
                Written::Sent
            });
            sent
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        {
            let mut registry = shared.state.lock().unwrap();
            let encoder = Encoder::new("rollers", 1);
            registry.twist(encoder.clone(), vec!["lcd".into()], Instant::now());
            registry.feedback(encoder, fifty_five(), Instant::now());
        }
        shared.changed.notify_all();
        release_tx.send(()).unwrap();
        let sent = handle.join().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].as_slice(), Frame::Feedback(fifty_five()).jpeg().unwrap().as_ref());
        assert_eq!(sent[1], b"clock");
    }

    #[test]
    fn completed_interaction_cannot_be_reopened_by_a_profile_refresh() {
        let encoder = Encoder::new("knobs", 1);
        let shared = shared_with_feedback(&encoder, fifty_five());
        worker(&shared, "lcd", |_, token| {
            if matches!(token, Token::Overlay(_)) {
                end_overlay(&shared);
            }
            Written::Sent
        });
        let mut registry = shared.state.lock().unwrap();
        assert!(!registry.sources[&encoder].awaiting_feedback);
        registry.feedback(encoder, fifty(), Instant::now());
        assert!(queued_feedback(&registry, "lcd").is_none());
        assert!(registry.claim_workers(Instant::now()).is_empty());
    }

    #[test]
    fn incomplete_b1_layout_does_not_replace_confirmed_value_or_cache() {
        let _ = Frame::Feedback(fifty_five()).jpeg();
        let mut strip = image::load_from_memory(include_bytes!("../tests/fixtures/encoder-b1/b1-55.jpg")).unwrap().to_rgb8();
        for y in 40..72 {
            for x in 76..184 {
                strip.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let incomplete = FeedbackImage { data: crate::render::png_data_url(&strip).into(), row: None };
        let encoder = Encoder::new("rollers", 1);
        let shared = Arc::new(shared_with_feedback(&encoder, incomplete));
        {
            let mut registry = shared.state.lock().unwrap();
            registry.sources.get_mut(&encoder).unwrap().cached = Some(fifty_five());
            registry.displays.get_mut("lcd").unwrap().sent_overlay = Some(0);
        }
        let worker_shared = shared.clone();
        let handle = thread::spawn(move || {
            let mut sent = Vec::new();
            worker(&worker_shared, "lcd", |jpeg, token| {
                assert!(worker_shared.current("lcd", token, Instant::now()));
                sent.push(jpeg.to_vec());
                if matches!(token, Token::Overlay(_)) {
                    end_overlay(&worker_shared);
                }
                Written::Sent
            });
            sent
        });
        {
            let registry = shared.state.lock().unwrap();
            let (mut registry, timeout) = shared
                .changed
                .wait_timeout_while(registry, Duration::from_secs(2), |registry| registry.displays["lcd"].invalid_overlay.is_none())
                .unwrap();
            assert!(!timeout.timed_out());
            assert!(registry.sources[&encoder].cached == Some(fifty_five()));
            assert!(registry.sources[&encoder].latest.is_none());
            registry.feedback(encoder, fifty(), Instant::now());
        }
        shared.changed.notify_all();
        let sent = handle.join().unwrap();
        assert_eq!(sent.len(), 2, "reset układu nie powinien wysyłać klatki");
        assert_eq!(sent[0].as_slice(), Frame::Feedback(fifty()).jpeg().unwrap().as_ref());
        assert_eq!(sent[1], b"clock");
    }

    fn decode_pending(shared: &Shared) {
        feedback_decoder(shared, |encoder, revision, ready| {
            shared.state.lock().unwrap().deliver_if_current(encoder, revision, ready, Instant::now());
            shared.changed.notify_all();
        });
    }

    #[test]
    fn profile_feedback_is_prepared_and_cached_without_showing_an_overlay() {
        let shared = Shared::default();
        let encoder = Encoder::new("knobs", 2);
        shared.state.lock().unwrap().enqueue_feedback(encoder.clone(), fifty_five(), Instant::now());
        decode_pending(&shared);
        let mut registry = shared.state.lock().unwrap();
        assert!(registry.displays.is_empty());
        let cached = registry.sources[&encoder].ready.clone().unwrap();
        assert!(matches!(cached.model.as_ref(), images::DialFeedback::Ring(ring) if ring.percent == Some(55)));
        registry.twist(encoder, vec!["lcd".into()], Instant::now());
        let Some(Frame::ReadyFeedback(queued)) = &registry.displays["lcd"].overlay else { panic!("JPEG powinien być gotowy") };
        assert!(Arc::ptr_eq(&queued.jpeg, &cached.jpeg), "obrót nie dekoduje ponownie data URL");
    }

    #[test]
    fn a_rotation_never_predicts_a_new_percent_before_ha_feedback() {
        let shared = Shared::default();
        let encoder = Encoder::new("rollers", 1);
        shared.state.lock().unwrap().enqueue_feedback(encoder.clone(), fifty(), Instant::now());
        decode_pending(&shared);
        let mut registry = shared.state.lock().unwrap();
        registry.twist(encoder.clone(), vec!["lcd".into()], Instant::now());
        registry.twist(encoder, vec!["lcd".into()], Instant::now());
        let Some(Frame::ReadyFeedback(ready)) = &registry.displays["lcd"].overlay else { panic!("potwierdzony stan") };
        assert!(matches!(ready.model.as_ref(), images::DialFeedback::Ring(ring) if ring.percent == Some(50)));
    }

    #[test]
    fn decoder_keeps_latest_per_source_and_prioritizes_the_rotated_encoder() {
        let shared = Shared::default();
        let old = Encoder::new("knobs", 0);
        let active = Encoder::new("rollers", 1);
        {
            let mut registry = shared.state.lock().unwrap();
            registry.enqueue_feedback(old.clone(), fifty(), Instant::now());
            registry.twist(active.clone(), vec!["lcd".into()], Instant::now());
            registry.enqueue_feedback(active.clone(), fifty(), Instant::now());
            registry.enqueue_feedback(active.clone(), fifty_five(), Instant::now());
        }
        let mut delivered = Vec::new();
        feedback_decoder(&shared, |encoder, _, ready| delivered.extend(ready.map(|ready| (encoder, ready))));
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].0, active);
        assert!(matches!(delivered[0].1.model.as_ref(), images::DialFeedback::Ring(ring) if ring.percent == Some(55)));
        assert_eq!(delivered[1].0, old);
    }

    #[test]
    fn decoding_from_an_old_assignment_cannot_repopulate_cache_after_profile_clear() {
        let shared = Shared::default();
        let encoder = Encoder::new("knobs", 1);
        let revision = {
            let mut registry = shared.state.lock().unwrap();
            registry.enqueue_feedback(encoder.clone(), fifty(), Instant::now());
            registry.feedback_revisions[&encoder].0
        };
        let old = Arc::new(ReadyFeedback::prepare(fifty(), Instant::now()).unwrap());
        {
            let mut registry = shared.state.lock().unwrap();
            registry.clear("knobs", None, Instant::now());
            registry.enqueue_feedback(encoder.clone(), fifty_five(), Instant::now());
            registry.accept_ready_if_current(encoder.clone(), revision, old, Instant::now());
            assert!(!registry.sources.contains_key(&encoder));
        }
        decode_pending(&shared);
        let registry = shared.state.lock().unwrap();
        assert!(matches!(registry.sources[&encoder].ready.as_ref().unwrap().model.as_ref(), images::DialFeedback::Ring(ring)
            if ring.percent == Some(55)));
    }

    #[test]
    fn equivalent_ready_feedback_does_not_upload_again_but_new_value_does() {
        let shared = Shared::default();
        let encoder = Encoder::new("knobs", 0);
        shared.state.lock().unwrap().enqueue_feedback(encoder.clone(), fifty(), Instant::now());
        decode_pending(&shared);
        let mut registry = shared.state.lock().unwrap();
        registry.twist(encoder.clone(), vec!["lcd".into()], Instant::now());
        let revision = registry.displays["lcd"].revision;
        let original = registry.sources[&encoder].ready.clone().unwrap();
        let mut repacked = original.received_again(Instant::now());
        repacked.image.data = "a different JPEG of the same feedback".into();
        registry.accept_ready(encoder.clone(), Arc::new(repacked), Instant::now());
        assert_eq!(registry.displays["lcd"].revision, revision);
        let changed = Arc::new(ReadyFeedback::prepare(fifty_five(), Instant::now()).unwrap());
        registry.accept_ready(encoder, changed, Instant::now());
        assert!(registry.displays["lcd"].revision > revision);
    }

    #[test]
    fn new_feedback_is_prepared_while_hid_is_blocked_and_only_latest_is_written() {
        let shared = Arc::new(Shared::default());
        let encoder = Encoder::new("rollers", 0);
        shared.state.lock().unwrap().enqueue_feedback(encoder.clone(), fifty(), Instant::now());
        decode_pending(&shared);
        {
            let mut registry = shared.state.lock().unwrap();
            registry.twist(encoder.clone(), vec!["lcd".into()], Instant::now());
            registry.displays.get_mut("lcd").unwrap().background = Some(Frame::Jpeg(Arc::from(&b"clock"[..])));
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_shared = shared.clone();
        let handle = thread::spawn(move || {
            let mut first = true;
            let mut sent = Vec::new();
            worker(&writer_shared, "lcd", |jpeg, token| {
                if first {
                    first = false;
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                if !writer_shared.current("lcd", token, Instant::now()) {
                    return Written::Superseded;
                }
                sent.push(jpeg.to_vec());
                if matches!(token, Token::Overlay(_)) {
                    end_overlay(&writer_shared);
                }
                Written::Sent
            });
            sent
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        shared.state.lock().unwrap().enqueue_feedback(encoder.clone(), fifty_five(), Instant::now());
        decode_pending(&shared);
        let newest = shared.state.lock().unwrap().sources[&encoder].ready.clone().unwrap();
        assert!(matches!(newest.model.as_ref(), images::DialFeedback::Ring(ring) if ring.percent == Some(55)));
        release_tx.send(()).unwrap();
        let sent = handle.join().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[0] == newest.jpeg.as_ref());
        assert_eq!(sent[1], b"clock");
    }

    #[test]
    fn public_feedback_api_prepares_cache_asynchronously_without_hid() {
        let image = fifty_five();
        let _ = ReadyFeedback::prepare(image.clone(), Instant::now()).unwrap();
        let device = "test-public-decoder-pipeline";
        let encoder = Encoder::new(device, 0);
        show_feedback(device, Some(0), 1, image.data.to_string());
        {
            let registry = SHARED.state.lock().unwrap();
            let (registry, timeout) = SHARED
                .changed
                .wait_timeout_while(registry, Duration::from_secs(2), |registry| {
                    registry.sources.get(&encoder).and_then(|source| source.ready.as_ref()).is_none()
                })
                .unwrap();
            assert!(!timeout.timed_out(), "rzeczywisty dekoder powinien dostarczyć klatkę");
            let cached = registry.sources[&encoder].ready.as_ref().unwrap();
            assert!(matches!(cached.model.as_ref(), images::DialFeedback::Ring(ring) if ring.percent == Some(55)));
            assert!(registry.displays.is_empty(), "odświeżenie profilu nie uruchamia HID");
        }
        clear_feedback(device, None);
        assert!(!SHARED.state.lock().unwrap().sources.contains_key(&encoder));
    }
}
