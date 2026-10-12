//! Counter-Strike 2: dane z gry przez Game State Integration (GSI).
//!
//! CS2 sam wysyła stan gry (HTTP POST z JSON-em) na adres z pliku
//! `game/csgo/cfg/gamestate_integration_*.cfg` – to oficjalny mechanizm Valve (bez czytania
//! pamięci gry, więc bez ryzyka VAC). W trakcie meczu gra podaje tylko dane gracza i rundy.
//! Plugin sam zapisuje ten plik w folderze gry i nasłuchuje na 127.0.0.1:`PORT`.

use image::{Rgb, RgbImage};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{LazyLock, Mutex, Once},
    thread,
    time::{Duration, Instant},
};

use crate::{MODULES, hub, hub::Kind, i18n::tr, render, with_writer};

pub const PORT: u16 = 47231;
const TOKEN: &str = "opendeck-masterhub";
const CFG_NAME: &str = "gamestate_integration_opendeck_masterhub.cfg";
/// Bez żadnych danych dłużej niż tyle uznajemy, że gra jest wyłączona (heartbeat to 5 s).
const STALE: Duration = Duration::from_secs(15);
const BOMB_TIME: f32 = 40.0;
/// Polecenia dla gry: plugin zapisuje je do `CMD_CFG` i naciska `EXEC_KEY`, który w grze jest
/// przypisany do `exec` tego pliku (bind z `SETUP_CFG`). CS2 nie obsługuje `-netconport`.
const SETUP_CFG: &str = "opendeck_masterhub.cfg";
const CMD_CFG: &str = "opendeck_masterhub_cmd.cfg";
const EXEC_KEY: &str = "f13";

const CT: Rgb<u8> = Rgb([93, 140, 255]);
const TT: Rgb<u8> = Rgb([240, 160, 48]);
const GREEN: Rgb<u8> = Rgb([60, 200, 90]);
const YELLOW: Rgb<u8> = Rgb([235, 200, 40]);
const RED: Rgb<u8> = Rgb([225, 50, 50]);
const BLUE: Rgb<u8> = Rgb([60, 150, 255]);
const GREY: Rgb<u8> = Rgb([110, 110, 125]);

// --- Stan gry ----------------------------------------------------------------------------------

#[derive(Clone, Default, Debug)]
pub struct Game {
    received: Option<Instant>,
    /// Mapa jest w danych = trwa mecz (albo rozgrzewka).
    in_match: bool,
    map_name: String,
    map_phase: String,
    round: u32,
    round_phase: String,
    win_team: String,
    score_ct: u32,
    score_t: u32,
    team: String,
    /// Dane dotyczą nas (a nie obserwowanego po śmierci gracza).
    own: bool,
    health: u32,
    armor: u32,
    helmet: bool,
    flashed: u32,
    burning: u32,
    money: u32,
    round_kills: u32,
    round_killhs: u32,
    kills: i32,
    assists: i32,
    deaths: i32,
    mvps: i32,
    /// Broń w ręku: (nazwa, magazynek, pojemność magazynka, zapas).
    weapon: Option<(String, i32, i32, i32)>,
    /// Odliczanie fazy z gry: (faza, ile zostało s w chwili odebrania).
    countdown: Option<(String, f32)>,
    bomb: String,
    bomb_planted_at: Option<Instant>,
}

impl Game {
    fn active(&self) -> bool {
        self.in_match && self.received.is_some_and(|r| r.elapsed() < STALE)
    }

    fn alive(&self) -> bool {
        self.own && self.health > 0
    }

    fn team_color(&self) -> Rgb<u8> {
        match self.team.as_str() {
            "CT" => CT,
            "T" => TT,
            _ => GREY,
        }
    }

    /// Ile zostało z odliczania danej fazy (uwzględnia czas od odebrania danych).
    fn phase_left(&self, phase: &str) -> Option<f32> {
        let (p, left) = self.countdown.as_ref()?;
        let since = self.received?.elapsed().as_secs_f32();
        (p == phase).then(|| (left - since).max(0.0))
    }

    fn bomb_planted(&self) -> bool {
        self.bomb == "planted" && self.round_phase != "over"
    }

    fn bomb_left(&self) -> Option<f32> {
        if !self.bomb_planted() {
            return None;
        }
        let at = self.bomb_planted_at?;
        Some((BOMB_TIME - at.elapsed().as_secs_f32()).max(0.0))
    }

    fn defuse_left(&self) -> Option<f32> {
        if !self.bomb_planted() { None } else { self.phase_left("defuse") }
    }

    fn update(&mut self, v: &Value) {
        let now = Instant::now();
        self.received = Some(now);
        let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let n = |v: &Value| v.as_u64().unwrap_or(0) as u32;
        let i = |v: &Value| v.as_i64().unwrap_or(0) as i32;

        let map = &v["map"];
        self.in_match = map.is_object();
        self.map_name = s(&map["name"]);
        self.map_phase = s(&map["phase"]);
        self.round = n(&map["round"]);
        self.score_ct = n(&map["team_ct"]["score"]);
        self.score_t = n(&map["team_t"]["score"]);

        let round = &v["round"];
        self.round_phase = s(&round["phase"]);
        self.win_team = s(&round["win_team"]);
        let bomb = s(&round["bomb"]);
        if bomb == "planted" && self.bomb != "planted" {
            self.bomb_planted_at = Some(now);
        }
        self.bomb = bomb;

        let player = &v["player"];
        self.own = !player["steamid"].is_null() && player["steamid"] == v["provider"]["steamid"];
        self.team = s(&player["team"]);
        let st = &player["state"];
        self.health = n(&st["health"]);
        self.armor = n(&st["armor"]);
        self.helmet = st["helmet"].as_bool().unwrap_or(false);
        self.flashed = n(&st["flashed"]);
        self.burning = n(&st["burning"]);
        self.money = n(&st["money"]);
        self.round_kills = n(&st["round_kills"]);
        self.round_killhs = n(&st["round_killhs"]);
        let ms = &player["match_stats"];
        self.kills = i(&ms["kills"]);
        self.assists = i(&ms["assists"]);
        self.deaths = i(&ms["deaths"]);
        self.mvps = i(&ms["mvps"]);
        self.weapon = player["weapons"].as_object().and_then(|w| {
            let active = w.values().find(|w| w["state"] == "active")?;
            Some((
                s(&active["name"]),
                active["ammo_clip"].as_i64().unwrap_or(-1) as i32,
                active["ammo_clip_max"].as_i64().unwrap_or(-1) as i32,
                active["ammo_reserve"].as_i64().unwrap_or(-1) as i32,
            ))
        });

        let pc = &v["phase_countdowns"];
        self.countdown = pc["phase"].as_str().map(|p| {
            let left = pc["phase_ends_in"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0);
            (p.to_string(), left)
        });
        // Odliczanie bomby z gry jest dokładniejsze niż liczone od chwili podłożenia.
        if let Some(left) = self.phase_left("bomb") {
            self.bomb_planted_at = now.checked_sub(Duration::from_secs_f32((BOMB_TIME - left).max(0.0)));
        }
    }
}

static GAME: LazyLock<Mutex<Game>> = LazyLock::new(Default::default);

fn game() -> Game {
    GAME.lock().unwrap().clone()
}

// --- Serwer GSI --------------------------------------------------------------------------------

static START: Once = Once::new();
/// Wynik instalacji pliku konfiguracyjnego (do pokazania w panelu).
static CFG_STATUS: LazyLock<Mutex<String>> = LazyLock::new(Default::default);
/// Folder `cfg` gry (po znalezieniu).
static CFG_DIR: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(Default::default);

/// Uruchamia serwer GSI i instaluje plik konfiguracyjny w grze (raz).
pub fn ensure_started() {
    START.call_once(|| {
        thread::spawn(|| {
            let status = install_cfg();
            log::info!("CS2: {status}");
            *CFG_STATUS.lock().unwrap() = status;
        });
        thread::spawn(serve);
    });
}

fn serve() {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("CS2: nie można nasłuchiwać na porcie {PORT}: {e}");
            return;
        }
    };
    for stream in listener.incoming().flatten() {
        thread::spawn(move || {
            if let Err(e) = handle(stream) {
                log::debug!("CS2: połączenie: {e}");
            }
        });
    }
}

/// Obsługuje połączenie od gry (HTTP/1.1, może przesłać kilka zapytań jedno po drugim).
fn handle(stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut length = 0usize;
        let mut line = String::new();
        let mut first = true;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let l = line.trim_end();
            if l.is_empty() && !first {
                break;
            }
            first = false;
            if let Some((name, value)) = l.split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0u8; length.min(1 << 20)];
        reader.read_exact(&mut body)?;
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Type: text/plain\r\n\r\n")?;
        let Ok(v) = serde_json::from_slice::<Value>(&body) else { continue };
        if v["auth"]["token"] != TOKEN {
            continue;
        }
        GAME.lock().unwrap().update(&v);
    }
}

fn cfg_text() -> String {
    format!(
        r#""OpenDeck MasterHUB"
{{
	"uri"		"http://127.0.0.1:{PORT}/"
	"timeout"	"1.0"
	"buffer"	"0.0"
	"throttle"	"0.1"
	"heartbeat"	"5.0"
	"auth"
	{{
		"token"	"{TOKEN}"
	}}
	"data"
	{{
		"provider"				"1"
		"map"					"1"
		"round"					"1"
		"player_id"				"1"
		"player_state"			"1"
		"player_weapons"		"1"
		"player_match_stats"	"1"
		"phase_countdowns"		"1"
	}}
}}
"#
    )
}

/// Katalogi Steama, w których szukamy bibliotek gier.
fn steam_roots() -> Vec<PathBuf> {
    let mut roots = vec![];
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output();
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(path) = text.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, p)| p.trim().to_string())) {
                roots.push(PathBuf::from(path));
            }
        }
        roots.push(r"C:\Program Files (x86)\Steam".into());
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    roots
}

/// Folder `cfg` gry we wszystkich bibliotekach Steama.
fn find_cfg_dir() -> Option<PathBuf> {
    let mut libraries = vec![];
    for root in steam_roots() {
        libraries.push(root.clone());
        if let Ok(vdf) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            for line in vdf.lines() {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 4 && parts[1] == "path" {
                    libraries.push(PathBuf::from(parts[3].replace("\\\\", "\\")));
                }
            }
        }
    }
    libraries
        .into_iter()
        .map(|l| l.join("steamapps/common/Counter-Strike Global Offensive/game/csgo/cfg"))
        .find(|d| d.is_dir())
}

fn install_cfg() -> String {
    let Some(dir) = find_cfg_dir() else {
        return tr("nie znaleziono CS2 w bibliotekach Steama", "CS2 not found in the Steam libraries").into();
    };
    *CFG_DIR.lock().unwrap() = Some(dir.clone());
    let setup = format!(
        "// OpenDeck MasterHUB: {key} runs commands from MasterHUB buttons (e.g. buying).\n\
         // Run once in the console: exec opendeck_masterhub\n\
         bind \"{key}\" \"exec {cmd}\"\n\
         echo \"OpenDeck MasterHUB: ready ({key})\"\n",
        key = EXEC_KEY.to_uppercase(),
        cmd = CMD_CFG.trim_end_matches(".cfg"),
    );
    if let Err(e) = std::fs::write(dir.join(SETUP_CFG), setup) {
        log::error!("CS2: nie udało się zapisać {SETUP_CFG}: {e}");
    }
    let path = dir.join(CFG_NAME);
    let text = cfg_text();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return format!("{}: {}", tr("konfiguracja GSI", "GSI config"), path.display());
    }
    match std::fs::write(&path, text) {
        Ok(()) => format!(
            "{}: {}",
            tr(
                "zainstalowano konfigurację GSI (uruchom CS2 ponownie, jeśli był włączony)",
                "GSI config installed (restart CS2 if it was running)"
            ),
            path.display()
        ),
        Err(e) => format!("{} {}: {e}", tr("nie udało się zapisać", "could not write"), path.display()),
    }
}

/// Stan dla panelu ustawień.
pub fn status_json() -> Value {
    let g = game();
    let data = match g.received {
        None => tr("brak danych z gry – uruchom CS2", "no data from the game – start CS2").to_string(),
        Some(r) if r.elapsed() >= STALE => {
            format!("{} {} s", tr("gra nie wysyła danych od", "no data from the game for"), r.elapsed().as_secs())
        }
        Some(_) if !g.in_match => tr("gra połączona (menu)", "game connected (menu)").to_string(),
        Some(_) => format!("{}: {} ({})", tr("gra połączona", "game connected"), g.map_name, g.map_phase),
    };
    serde_json::json!({ "status": data, "cfg": *CFG_STATUS.lock().unwrap() })
}

// --- Akcja -------------------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
struct Settings {
    /// auto | bomb | health | armor | money | ammo | kills | kda | score | buy
    show: String,
    /// Przedmiot do kupienia (nazwa z konsoli, np. "ak47") i jego cena.
    item: String,
    label: String,
    price: u32,
    /// Wciśnięcie: none | key (skrót klawiszowy) | console (polecenie w grze przez plik cfg + klawisz F13).
    press: String,
    key: String,
    command: String,
    /// Diody bazy, gałek i fadera: off | events (tylko zdarzenia) | full (też kolor drużyny).
    leds: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show: "auto".into(),
            item: String::new(),
            label: String::new(),
            price: 0,
            press: "none".into(),
            key: String::new(),
            command: String::new(),
            leds: "events".into(),
        }
    }
}

struct Instance {
    settings: Settings,
    /// Pokrętło, jeśli akcja stoi na jego klawiszu (obraz idzie prosto na ekran 240×240).
    dial: Option<String>,
    /// Ostatnie podpisy obrazu: dokładny (pokrętło) i zgrubny (podgląd w OpenDecku).
    fine: String,
    coarse: String,
}

static INSTANCES: LazyLock<Mutex<HashMap<String, Instance>>> = LazyLock::new(Default::default);

pub fn register(context: &str, device: &str, settings: &Value) {
    ensure_started();
    let settings: Settings = serde_json::from_value(settings.clone()).unwrap_or_default();
    let is_dial = MODULES.lock().unwrap().get(device).is_some_and(|m| m.kind == Kind::Dial);
    let dial = is_dial.then(|| device.to_string());
    INSTANCES
        .lock()
        .unwrap()
        .insert(context.to_string(), Instance { settings, dial, fine: String::new(), coarse: String::new() });
}

/// Zwraca pokrętło, jeśli akcja była na nim (trzeba wtedy przywrócić obraz OpenDecka).
pub fn unregister(context: &str) -> Option<String> {
    let dial = {
        let mut instances = INSTANCES.lock().unwrap();
        let dial = instances.remove(context).and_then(|i| i.dial);
        if instances.is_empty() {
            set_fx(Fx::None);
        }
        dial
    };
    dial.filter(|d| crate::overlay::clear_direct(d))
}

pub fn active() -> bool {
    !INSTANCES.lock().unwrap().is_empty()
}

/// Wciśnięcie klawisza akcji.
pub fn press(settings: &Value) {
    let s: Settings = serde_json::from_value(settings.clone()).unwrap_or_default();
    match s.press.as_str() {
        "key" if !s.key.trim().is_empty() => {
            if let Err(e) = crate::input::send_combo_held(s.key.trim(), 40) {
                log::error!("CS2: skrót {}: {e}", s.key);
            }
        }
        "console" => {
            let command = if s.command.trim().is_empty() && !s.item.is_empty() {
                format!("buy {}", s.item)
            } else {
                s.command.trim().to_string()
            };
            if !command.is_empty() {
                if let Err(e) = console(&command) {
                    log::error!("CS2: polecenie {command}: {e}");
                }
            }
        }
        _ => {}
    }
}

/// Wykonuje polecenie w grze: zapisuje je do pliku cfg i naciska klawisz z bindem `exec`.
fn console(command: &str) -> Result<(), String> {
    let dir = CFG_DIR.lock().unwrap().clone().ok_or(tr("nie znaleziono folderu cfg CS2", "CS2 cfg folder not found"))?;
    std::fs::write(dir.join(CMD_CFG), format!("{command}
")).map_err(|e| e.to_string())?;
    crate::input::send_combo_held(EXEC_KEY, 40)
}

/// Wynik rysowania: (podpis zgrubny, podpis dokładny, funkcja rysująca).
struct View {
    coarse: String,
    fine: String,
    draw: Box<dyn Fn(u32) -> RgbImage>,
}

fn view(s: &Settings, g: &Game) -> View {
    let simple = |key: String, draw: Box<dyn Fn(u32) -> RgbImage>| View { coarse: key.clone(), fine: key, draw };
    if !g.active() {
        let label = idle_label(s);
        return simple(
            format!("idle {label}"),
            Box::new(move |size| render::tile(size, &label, "—", Some("CS2"), GREY, None, None)),
        );
    }
    match s.show.as_str() {
        "bomb" => bomb_view(g).unwrap_or_else(|| {
            let label = tr("BOMBA", "BOMB").to_string();
            simple("bomb-none".into(), Box::new(move |size| render::tile(size, &label, "—", None, GREY, None, None)))
        }),
        "health" => {
            let (hp, alive) = (g.health, g.alive());
            simple(
                format!("hp {hp} {alive}"),
                Box::new(move |size| {
                    let value = if alive { hp.to_string() } else { "—".into() };
                    render::tile(size, "HP", &value, None, health_color(hp), None, Some(hp as f32 / 100.0))
                }),
            )
        }
        "armor" => {
            let (armor, helmet) = (g.armor, g.helmet);
            simple(
                format!("armor {armor} {helmet}"),
                Box::new(move |size| {
                    let sub = if helmet { Some(tr("+ hełm", "+ helmet")) } else { None };
                    render::tile(size, tr("PANCERZ", "ARMOR"), &armor.to_string(), sub, BLUE, None, Some(armor as f32 / 100.0))
                }),
            )
        }
        "money" => {
            let (money, color) = (g.money, g.team_color());
            simple(
                format!("money {money}"),
                Box::new(move |size| render::tile(size, tr("KASA", "MONEY"), &format!("${money}"), None, color, None, None)),
            )
        }
        "ammo" => {
            let w = g.weapon.clone();
            simple(
                format!("ammo {w:?}"),
                Box::new(move |size| match &w {
                    Some((name, clip, max, reserve)) if *clip >= 0 => {
                        let progress = (*max > 0).then(|| *clip as f32 / *max as f32);
                        let color = if *max > 0 && *clip * 4 <= *max { RED } else { GREEN };
                        render::tile(size, &weapon_name(name), &clip.to_string(), Some(&format!("/ {reserve}")), color, None, progress)
                    }
                    Some((name, ..)) => render::tile(size, &weapon_name(name), "—", None, GREY, None, None),
                    None => render::tile(size, tr("BROŃ", "WEAPON"), "—", None, GREY, None, None),
                }),
            )
        }
        "kills" => {
            let (k, hs) = (g.round_kills, g.round_killhs);
            simple(
                format!("rk {k} {hs}"),
                Box::new(move |size| {
                    let sub = (hs > 0).then(|| format!("{hs} HS"));
                    let color = if k >= 3 { YELLOW } else { GREEN };
                    render::tile(size, tr("W RUNDZIE", "THIS ROUND"), &k.to_string(), sub.as_deref(), color, None, None)
                }),
            )
        }
        "kda" => {
            let (k, a, d, mvp) = (g.kills, g.assists, g.deaths, g.mvps);
            simple(
                format!("kda {k} {a} {d} {mvp}"),
                Box::new(move |size| {
                    let sub = format!("A {a} · MVP {mvp}");
                    render::tile(size, "K / D", &format!("{k}/{d}"), Some(&sub), GREEN, None, None)
                }),
            )
        }
        "score" => {
            let (ct, t, round, team) = (g.score_ct, g.score_t, g.round, g.team.clone());
            simple(
                format!("score {ct} {t} {round} {team}"),
                Box::new(move |size| {
                    // Najpierw wynik naszej drużyny.
                    let (value, color) = if team == "T" { (format!("{t}:{ct}"), TT) } else { (format!("{ct}:{t}"), CT) };
                    render::tile(size, tr("WYNIK", "SCORE"), &value, Some(&format!("{} {}", tr("runda", "round"), round + 1)), color, None, None)
                }),
            )
        }
        "buy" => {
            let label = if s.label.trim().is_empty() { s.item.to_uppercase() } else { s.label.trim().to_string() };
            let (price, money, alive) = (s.price, g.money, g.alive());
            let afford = money >= price;
            simple(
                format!("buy {label} {price} {afford} {alive}"),
                Box::new(move |size| {
                    let (accent, tint) = match (alive, afford) {
                        (false, _) => (GREY, None),
                        (true, true) => (GREEN, Some(GREEN)),
                        (true, false) => (RED, Some(RED)),
                    };
                    render::tile(size, &label, &format!("${price}"), None, accent, tint, None)
                }),
            )
        }
        _ => auto_view(g),
    }
}

fn idle_label(s: &Settings) -> String {
    match s.show.as_str() {
        "bomb" => tr("BOMBA", "BOMB"),
        "health" => "HP",
        "armor" => tr("PANCERZ", "ARMOR"),
        "money" => tr("KASA", "MONEY"),
        "ammo" => tr("AMUNICJA", "AMMO"),
        "kills" => tr("W RUNDZIE", "THIS ROUND"),
        "kda" => "K / D",
        "score" => tr("WYNIK", "SCORE"),
        "buy" if !s.label.trim().is_empty() => s.label.trim(),
        "buy" => return s.item.to_uppercase(),
        _ => "CS2",
    }
    .to_string()
}

fn health_color(hp: u32) -> Rgb<u8> {
    if hp > 60 {
        GREEN
    } else if hp > 25 {
        YELLOW
    } else {
        RED
    }
}

/// Licznik bomby (i rozbrajania).
fn bomb_view(g: &Game) -> Option<View> {
    let left = g.bomb_left()?;
    let defuse = g.defuse_left();
    // Dokładny podpis co 0,2 s (płynny pierścień na pokrętle), zgrubny co sekundę.
    let fine = format!("bomb {:.0} {:?}", left * 5.0, defuse.map(|d| (d * 5.0) as i32));
    let coarse = format!("bomb {} {}", left.ceil(), defuse.is_some());
    Some(View {
        coarse,
        fine,
        draw: Box::new(move |size| {
            let value = if left < 10.0 { format!("{left:.1}") } else { format!("{left:.0}") };
            match defuse {
                Some(d) => render::ring(
                    size,
                    left / BOMB_TIME,
                    BLUE,
                    render::TRACK,
                    Some(tr("ROZBRAJANIE", "DEFUSING")),
                    &value,
                    &format!("{} {d:.1} s", tr("jeszcze", "left")),
                ),
                None => render::ring(size, left / BOMB_TIME, RED, render::TRACK, Some(tr("BOMBA", "BOMB")), &value, tr("sekund", "seconds")),
            }
        }),
    })
}

/// Podsumowanie na okrągły ekran: bomba > koniec rundy > zakupy > HP > obserwowanie.
fn auto_view(g: &Game) -> View {
    if let Some(v) = bomb_view(g) {
        return v;
    }
    let simple = |key: String, draw: Box<dyn Fn(u32) -> RgbImage>| View { coarse: key.clone(), fine: key, draw };
    let team = g.team.clone();
    let (ct, t) = (g.score_ct, g.score_t);
    let (mine, theirs) = if team == "T" { (t, ct) } else { (ct, t) };
    if g.map_phase == "warmup" {
        let money = g.money;
        return simple(
            format!("warmup {money}"),
            Box::new(move |size| render::ring(size, 1.0, GREY, render::TRACK, Some(tr("ROZGRZEWKA", "WARMUP")), &format!("${money}"), "")),
        );
    }
    if g.round_phase == "over" {
        let won = !team.is_empty() && g.win_team == team;
        let lost = !team.is_empty() && !g.win_team.is_empty() && !won;
        return simple(
            format!("over {mine} {theirs} {won} {lost}"),
            Box::new(move |size| {
                let (color, label) = if won {
                    (GREEN, tr("wygrana runda", "round won"))
                } else if lost {
                    (RED, tr("przegrana runda", "round lost"))
                } else {
                    (GREY, tr("koniec rundy", "round over"))
                };
                render::ring(size, 1.0, color, render::TRACK, Some(tr("WYNIK", "SCORE")), &format!("{mine}:{theirs}"), label)
            }),
        );
    }
    if g.round_phase == "freezetime" {
        let money = g.money;
        let left = g.phase_left("freezetime").map(|l| l.ceil() as u32);
        let color = g.team_color();
        return simple(
            format!("freeze {money} {left:?}"),
            Box::new(move |size| {
                let label = left.map(|l| format!("{} {l} s", tr("start za", "starts in"))).unwrap_or_else(|| tr("zakupy", "buy time").into());
                render::ring(size, 1.0, color, render::TRACK, Some(&format!("{mine}:{theirs}")), &format!("${money}"), &label)
            }),
        );
    }
    if g.alive() {
        let (hp, armor, helmet, money) = (g.health, g.armor, g.helmet, g.money);
        return simple(
            format!("alive {hp} {armor} {helmet} {money}"),
            Box::new(move |size| {
                let label = format!("{} {armor}{} · ${money}", tr("pancerz", "armor"), if helmet { "+H" } else { "" });
                render::ring(size, hp as f32 / 100.0, health_color(hp), render::TRACK, Some("HP"), &hp.to_string(), &label)
            }),
        );
    }
    let (k, d, money) = (g.kills, g.deaths, g.money);
    simple(
        format!("dead {k} {d} {money} {mine} {theirs}"),
        Box::new(move |size| {
            render::ring(size, 0.0, GREY, render::TRACK, Some(&format!("{mine}:{theirs}")), &format!("${money}"), &format!("K/D {k}/{d}"))
        }),
    )
}

/// Ładna nazwa broni z nazwy w grze ("weapon_m4a1_silencer" -> "M4A1-S").
fn weapon_name(raw: &str) -> String {
    let n = raw.trim_start_matches("weapon_");
    match n {
        "ak47" => "AK-47",
        "m4a1" => "M4A4",
        "m4a1_silencer" => "M4A1-S",
        "usp_silencer" => "USP-S",
        "hkp2000" => "P2000",
        "glock" => "Glock",
        "deagle" => "Deagle",
        "revolver" => "R8",
        "elite" => "Dualies",
        "fiveseven" => "Five-SeveN",
        "cz75a" => "CZ75",
        "tec9" => "Tec-9",
        "mp5sd" => "MP5-SD",
        "bizon" => "PP-Bizon",
        "galilar" => "Galil",
        "sg556" => "SG 553",
        "ssg08" => "Scout",
        "xm1014" => "XM1014",
        "mag7" => "MAG-7",
        "sawedoff" => "Sawed-Off",
        "taser" => "Zeus",
        "hegrenade" => "HE",
        "flashbang" => "Flash",
        "smokegrenade" => "Smoke",
        "molotov" => "Molotov",
        "incgrenade" => "Incendiary",
        "decoy" => "Decoy",
        "c4" => "C4",
        n if n.starts_with("knife") || n == "bayonet" => tr("Nóż", "Knife"),
        n => return n.to_uppercase(),
    }
    .to_string()
}

/// Nowe obrazy akcji (kontekst, data URL) dla OpenDecka. Obrazy na pokrętle wysyła od razu
/// bezpośrednio (240×240). Steruje też diodami.
pub fn updates() -> Vec<(String, String)> {
    let g = game();
    let mut out = vec![];
    let mut leds = "off";
    for (context, inst) in INSTANCES.lock().unwrap().iter_mut() {
        leds = match (leds, inst.settings.leds.as_str()) {
            ("full", _) | (_, "full") => "full",
            ("events", _) | (_, "events") => "events",
            _ => "off",
        };
        let mut v = view(&inst.settings, &g);
        // Zmiana języka też wymusza przerysowanie.
        v.fine.insert_str(0, crate::i18n::code());
        v.coarse.insert_str(0, crate::i18n::code());
        if let Some(device) = &inst.dial {
            if v.fine != inst.fine {
                inst.fine = v.fine.clone();
                crate::overlay::set_direct(device, (v.draw)(hub::DIAL_IMAGE_SIZE));
            }
        }
        if v.coarse != inst.coarse {
            inst.coarse = v.coarse;
            let size = if inst.dial.is_some() { hub::DIAL_IMAGE_SIZE } else { 144 };
            out.push((context.clone(), render::png_data_url(&(v.draw)(size))));
        }
    }
    set_fx(if leds == "off" { Fx::None } else { fx(&g, leds == "full") });
    out
}

/// Czy trzeba odświeżać często (trwa mecz – zmienia się bomba, odliczanie, stan gracza).
pub fn busy() -> bool {
    GAME.lock().unwrap().received.is_some_and(|r| r.elapsed() < STALE + Duration::from_secs(2))
}

// --- Diody -------------------------------------------------------------------------------------

/// Efekt na diodach bazy, gałek i fadera.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fx {
    /// Brak – diody wracają do ustawień użytkownika.
    None,
    /// (tryb, kolor, jasność, prędkość)
    Set(u8, [u8; 3], u8, u8),
}

static LAST_FX: LazyLock<Mutex<Fx>> = LazyLock::new(|| Mutex::new(Fx::None));

fn rgb(c: Rgb<u8>) -> [u8; 3] {
    c.0
}

fn fx(g: &Game, full: bool) -> Fx {
    const STATIC: u8 = 1;
    const BREATHE: u8 = 2;
    const STROBE: u8 = 3;
    if !g.active() {
        return Fx::None;
    }
    if g.own && g.flashed > 0 {
        // Oślepienie: biały, tym jaśniejszy, im mocniejszy flash (0–255).
        let b = (g.flashed * 100 / 255).max(20) as u8;
        return Fx::Set(STATIC, [255, 255, 255], (b / 10) * 10, 50);
    }
    if g.defuse_left().is_some() {
        return Fx::Set(BREATHE, rgb(BLUE), 100, 85);
    }
    if let Some(left) = g.bomb_left() {
        return match left {
            l if l > 20.0 => Fx::Set(BREATHE, rgb(RED), 100, 40),
            l if l > 10.0 => Fx::Set(BREATHE, rgb(RED), 100, 70),
            l if l > 5.0 => Fx::Set(BREATHE, rgb(RED), 100, 95),
            _ => Fx::Set(STROBE, rgb(RED), 100, 90),
        };
    }
    if g.round_phase == "over" && !g.team.is_empty() && !g.win_team.is_empty() {
        return if g.win_team == g.team { Fx::Set(STATIC, rgb(GREEN), 100, 50) } else { Fx::Set(STATIC, rgb(RED), 40, 50) };
    }
    if g.own && g.burning > 0 && g.health > 0 {
        return Fx::Set(BREATHE, [255, 110, 0], 100, 90);
    }
    if g.alive() && g.health <= 25 {
        return Fx::Set(BREATHE, rgb(RED), 100, 60);
    }
    if full && !g.team.is_empty() {
        let brightness = if g.alive() || g.round_phase == "freezetime" { 100 } else { 25 };
        return Fx::Set(STATIC, rgb(g.team_color()), brightness, 50);
    }
    Fx::None
}

fn set_fx(fx: Fx) {
    let mut last = LAST_FX.lock().unwrap();
    if *last == fx {
        return;
    }
    *last = fx;
    match fx {
        Fx::None => crate::actions::restore_led_state(),
        Fx::Set(mode, color, brightness, speed) => {
            let modules: Vec<(String, Kind)> = MODULES
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, m)| m.kind.led_module().is_some())
                .map(|(id, m)| (id.clone(), m.kind))
                .collect();
            for (id, kind) in modules {
                with_writer(&id, |dev| {
                    hub::set_led_color(dev, kind, None, color)?;
                    hub::set_led_brightness(dev, kind, None, brightness)?;
                    hub::set_led_speed(dev, kind, speed)?;
                    hub::set_led_mode(dev, kind, None, mode)
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(bomb: &str, phase: &str, health: u32) -> Value {
        serde_json::json!({
            "provider": { "steamid": "1" },
            "auth": { "token": TOKEN },
            "map": { "name": "de_mirage", "phase": "live", "round": 14, "team_ct": { "score": 8 }, "team_t": { "score": 6 } },
            "round": { "phase": phase, "bomb": bomb },
            "player": {
                "steamid": "1", "team": "CT",
                "state": { "health": health, "armor": 87, "helmet": true, "flashed": 0, "burning": 0, "money": 4350, "round_kills": 2, "round_killhs": 1 },
                "match_stats": { "kills": 17, "assists": 3, "deaths": 9, "mvps": 2 },
                "weapons": { "weapon_0": { "name": "weapon_knife", "state": "holstered" },
                             "weapon_1": { "name": "weapon_m4a1_silencer", "ammo_clip": 7, "ammo_clip_max": 20, "ammo_reserve": 40, "state": "active" } }
            },
            "phase_countdowns": { "phase": if bomb == "planted" { "bomb" } else { "freezetime" }, "phase_ends_in": "8.4" }
        })
    }

    /// Zapisuje podglądy do katalogu z CS2_PREVIEW (cargo test -- --nocapture).
    #[test]
    fn previews() {
        let Some(dir) = std::env::var_os("CS2_PREVIEW").filter(|d| !d.is_empty()).map(PathBuf::from) else { return };
        let cases = [("hp", "", "live", 64), ("bomb", "planted", "live", 64), ("freeze", "", "freezetime", 100), ("dead", "", "live", 0), ("lowhp", "", "live", 18)];
        for (name, bomb, phase, hp) in cases {
            let mut g = Game::default();
            g.update(&sample(bomb, phase, hp));
            for show in ["auto", "health", "armor", "money", "ammo", "kills", "kda", "score", "buy", "bomb"] {
                if name != "hp" && show != "auto" {
                    continue;
                }
                let s = Settings { show: show.into(), item: "ak47".into(), label: "AK-47".into(), price: 2700, ..Default::default() };
                let v = view(&s, &g);
                let size = if show == "auto" { hub::DIAL_IMAGE_SIZE } else { 144 };
                (v.draw)(size).save(dir.join(format!("{name}-{show}.png"))).unwrap();
            }
        }
    }
}
