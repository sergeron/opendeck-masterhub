//! Skróty klawiszowe ("ctrl+shift+m", "media_play_pause", ...): implementacja zależna od systemu.

#[cfg(windows)]
#[path = "input_windows.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "input_enigo.rs"]
mod imp;

pub use imp::send_combo;
pub use imp::send_combo_held;
