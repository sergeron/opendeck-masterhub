//! Język napisów rysowanych przez plugin i paneli ustawień (polski / angielski).
//!
//! Ustawienie "language" w ustawieniach globalnych pluginu: "pl", "en" albo "auto".
//! "auto" = język paczki (plik `language.txt` obok programu, zapisywany przez package.py),
//! a bez niego język systemu.

use std::sync::{
    LazyLock, Mutex,
    atomic::{AtomicBool, Ordering},
};

static POLISH: AtomicBool = AtomicBool::new(false);
/// Ustawienie z panelu: "auto", "pl" albo "en".
static SETTING: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new("auto".into()));

/// Czy napisy mają być po polsku.
pub fn polish() -> bool {
    POLISH.load(Ordering::Relaxed)
}

/// Tekst w bieżącym języku.
pub fn tr<'a>(pl: &'a str, en: &'a str) -> &'a str {
    if polish() { pl } else { en }
}

/// Kod bieżącego języka ("pl" / "en").
pub fn code() -> &'static str {
    if polish() { "pl" } else { "en" }
}

pub fn setting() -> String {
    SETTING.lock().unwrap().clone()
}

/// Ustawia język; zwraca, czy się zmienił.
pub fn set(setting: &str) -> bool {
    let setting = match setting {
        "pl" | "en" => setting,
        _ => "auto",
    };
    *SETTING.lock().unwrap() = setting.to_string();
    let polish = match setting {
        "pl" => true,
        "en" => false,
        _ => default_polish(),
    };
    POLISH.swap(polish, Ordering::Relaxed) != polish
}

/// Język domyślny: z paczki (language.txt), a bez niej – z systemu.
fn default_polish() -> bool {
    let packaged = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read_to_string(p.with_file_name("language.txt")).ok());
    match packaged.as_deref().map(str::trim) {
        Some("pl") => true,
        Some("en") => false,
        _ => system_polish(),
    }
}

#[cfg(windows)]
fn system_polish() -> bool {
    // Język interfejsu Windows; 0x15 = polski (LANG_POLISH).
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3ff == 0x15
}

#[cfg(not(windows))]
fn system_polish() -> bool {
    let env = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    #[cfg(target_os = "macos")]
    let env = env.or_else(|| {
        // Aplikacje z Docka nie dostają LANG – pytamy o ustawienia regionalne systemu.
        let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLocale"]).output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    env.is_some_and(|v| v.starts_with("pl"))
}

/// Ustawia język domyślny przy starcie (zanim przyjdą ustawienia globalne).
pub fn init() {
    set("auto");
}
