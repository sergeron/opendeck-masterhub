//! Protokół HID modułów Cooler Master MasterHUB.
//!
//! Każdy moduł to osobne urządzenie HID (VID 0x2516). Raporty wejściowe mają format:
//! `01 <tag kanału> <długość u16 LE> <id zdarzenia> <długość> <dane...>`.
//! Źródło: https://github.com/projectzero7/cooltek-sdk-probe + własne pomiary (pokrętło).

use hidapi::{HidDevice, HidError};

pub const VID: u16 = 0x2516;

/// Rozmiar ikony na klawiszu modułu 15xKey.
pub const KEY_IMAGE_SIZE: u32 = 72;
/// Rozmiar okrągłego ekranu pokrętła (aplikacja CM rysuje na płótnie 240×240).
pub const DIAL_IMAGE_SIZE: u32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Base,
    Keys15,
    Fader,
    Knob3,
    Roller2,
    Dial,
}

impl Kind {
    pub fn from_pid(pid: u16) -> Option<Kind> {
        Some(match pid {
            0x01f6 => Kind::Base,
            0x01fe => Kind::Keys15,
            0x0200 => Kind::Fader,
            0x0202 => Kind::Knob3,
            0x0204 => Kind::Roller2,
            0x0206 => Kind::Dial,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Base => "MasterHUB Base",
            Kind::Keys15 => "MasterHUB 15xKey",
            Kind::Fader => "MasterHUB 5xFader",
            Kind::Knob3 => "MasterHUB 3xKnob",
            Kind::Roller2 => "MasterHUB 2xRoller",
            Kind::Dial => "MasterHUB 1xDial",
        }
    }

    /// (wiersze, kolumny, enkodery) w układzie OpenDecka.
    ///
    /// Moduły bez klawiszy zgłaszają tyle kolumn, ile mają enkoderów: OpenDeck liczy szerokość
    /// rzędu enkoderów jako `kolumny * 132 px`, a przy 0 wierszach i tak nie tworzy klawiszy.
    pub fn layout(self) -> (u8, u8, u8) {
        match self {
            Kind::Base => (0, 0, 0),
            Kind::Keys15 => (3, 5, 0),
            Kind::Fader => (0, 5, 5),
            Kind::Knob3 => (0, 3, 3),
            Kind::Roller2 => (0, 2, 2),
            // Klawisz = wciśnięcie (ekran na środku), enkoder = obrót.
            Kind::Dial => (1, 1, 1),
        }
    }

    pub fn has_screen(self) -> bool {
        matches!(self, Kind::Keys15 | Kind::Dial)
    }

    /// Feature report z numerem seryjnym.
    fn serial_report(self) -> u8 {
        match self {
            Kind::Keys15 => 0x3a,
            Kind::Dial => 0x4a,
            _ => 0x30,
        }
    }

    /// Komenda "Set SW Mode"; bez niej te moduły nie wysyłają zdarzeń.
    fn sw_mode_command(self) -> Option<u8> {
        match self {
            Kind::Keys15 => Some(0x3a),
            Kind::Dial => Some(0x49),
            _ => None,
        }
    }
}

pub fn read_serial(dev: &HidDevice, kind: Kind) -> Option<String> {
    let mut raw = [0u8; 0x20];
    raw[0] = kind.serial_report();
    dev.get_feature_report(&mut raw).ok()?;
    let len = (raw[1] as usize).min(raw.len() - 2);
    let s = String::from_utf8_lossy(&raw[2..2 + len]);
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

pub fn set_sw_mode(dev: &HidDevice, kind: Kind, on: bool) -> Result<(), HidError> {
    let Some(cmd) = kind.sw_mode_command() else { return Ok(()) };
    let mut raw = [0u8; 0x20];
    raw[0] = 0x03;
    raw[1] = cmd;
    raw[2] = on as u8;
    dev.send_feature_report(&raw)
}

/// Jasność podświetlenia ekranów (15xKey, pokrętło), 0–100.
pub fn set_backlight(dev: &HidDevice, kind: Kind, brightness: u8) -> Result<(), HidError> {
    let mut raw = [0u8; 0x20];
    raw[0] = 0x03;
    raw[1] = if kind == Kind::Dial { 0x44 } else { 0x34 };
    raw[2] = brightness.min(100);
    dev.send_feature_report(&raw)
}

/// Wysyła obraz JPG na klawisz `index` (0–14).
pub fn upload_key_image(dev: &HidDevice, index: u8, jpeg: &[u8]) -> Result<(), HidError> {
    upload_image(dev, 0x36, index, jpeg)
}

/// Wysyła obraz JPG 240×240 na okrągły ekran pokrętła.
/// Odczytane z CoolTEK_SDK_CM_M131E (Display_Picture_On_Screen): ten sam format co klawisze,
/// tylko komenda 0x45.
pub fn upload_dial_image(dev: &HidDevice, jpeg: &[u8]) -> Result<(), HidError> {
    upload_image(dev, 0x45, 0, jpeg)
}

/// Format: 02 <komenda> <nr kawałka u16> <długość u16> <indeks> <ostatni?> <dane...>
fn upload_image(dev: &HidDevice, command: u8, index: u8, jpeg: &[u8]) -> Result<(), HidError> {
    const REPORT_LEN: usize = 0x140;
    const CHUNK_MAX: usize = REPORT_LEN - 8;
    for (chunk_index, chunk) in jpeg.chunks(CHUNK_MAX).enumerate() {
        let mut report = [0u8; REPORT_LEN];
        report[0] = 0x02;
        report[1] = command;
        report[2..4].copy_from_slice(&(chunk_index as u16).to_le_bytes());
        report[4..6].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
        report[6] = index;
        report[7] = ((chunk_index + 1) * CHUNK_MAX >= jpeg.len()) as u8;
        report[8..8 + chunk.len()].copy_from_slice(chunk);
        dev.write(&report)?;
    }
    Ok(())
}

// --- Diody LED modułów M0 (baza, fader, gałki) -----------------------------------------------
// Raport wyjściowy 64 B: 02 <komenda> <moduł> <indeks> <parametry...>
// Moduł: 0 = baza (jedna dioda), 1 = fader (5), 2 = gałki (3). Indeks 0xff = wszystkie.

// Tryby efektów: 0 wyłączone, 1 stały, 2 oddychanie, 3 stroboskop, 4 spring, 5 cykl kolorów,
// 6 losowy, 7 fala, 8 woda, 9 tęcza (kolor działa tylko w trybach 1–4).

impl Kind {
    /// Numer modułu w komendach LED; `None` = moduł bez diod.
    pub fn led_module(self) -> Option<u8> {
        match self {
            Kind::Base => Some(0),
            Kind::Fader => Some(1),
            Kind::Knob3 => Some(2),
            _ => None,
        }
    }

    pub fn led_count(self) -> u8 {
        match self {
            Kind::Base => 1,
            Kind::Fader => 5,
            Kind::Knob3 => 3,
            _ => 0,
        }
    }
}

/// Indeks diody: `None` = wszystkie. Baza ma tylko indeks 0.
fn led_index(kind: Kind, index: Option<u8>) -> u8 {
    match (kind, index) {
        (Kind::Base, _) => 0,
        (_, Some(i)) => i,
        (_, None) => 0xff,
    }
}

fn led_command(dev: &HidDevice, kind: Kind, command: u8, index: Option<u8>, params: &[u8]) -> Result<(), HidError> {
    let Some(module) = kind.led_module() else { return Ok(()) };
    let mut raw = [0u8; 0x40];
    raw[0] = 0x02;
    raw[1] = command;
    raw[2] = module;
    raw[3] = led_index(kind, index);
    raw[4..4 + params.len()].copy_from_slice(params);
    dev.write(&raw).map(|_| ())
}

/// Tryb efektu (0–9, patrz `LED_MODES`). Nie zapisuje w pamięci modułu.
pub fn set_led_mode(dev: &HidDevice, kind: Kind, index: Option<u8>, mode: u8) -> Result<(), HidError> {
    led_command(dev, kind, 0x2c, index, &[mode.min(9), 0])
}

pub fn set_led_color(dev: &HidDevice, kind: Kind, index: Option<u8>, rgb: [u8; 3]) -> Result<(), HidError> {
    led_command(dev, kind, 0x2d, index, &[rgb[0], rgb[1], rgb[2], 0])
}

pub fn set_led_brightness(dev: &HidDevice, kind: Kind, index: Option<u8>, brightness: u8) -> Result<(), HidError> {
    led_command(dev, kind, 0x2e, index, &[brightness.min(100)])
}

/// Prędkość efektów 0–100 (dla całego modułu).
pub fn set_led_speed(dev: &HidDevice, kind: Kind, speed: u8) -> Result<(), HidError> {
    if kind.led_module().is_none() {
        return Ok(());
    }
    let mut raw = [0u8; 0x40];
    raw[0] = 0x02;
    raw[1] = 0x30;
    raw[2] = speed.min(100);
    dev.write(&raw).map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    KeyDown(u8),
    KeyUp(u8),
    EncoderDown(u8),
    EncoderUp(u8),
    /// Obrót o `ticks` kroków (dodatnie = w prawo / w górę).
    EncoderTwist(u8, i16),
    /// Bezwzględna pozycja suwaka 0–100 (dodatkowo do EncoderTwist).
    FaderValue(u8, u8),
}

/// Zamienia raporty modułu na zdarzenia, pamiętając poprzedni stan przycisków i suwaków.
pub struct Decoder {
    kind: Kind,
    held: Vec<u8>,
    faders: Vec<Option<u8>>,
}

impl Decoder {
    pub fn new(kind: Kind) -> Self {
        Decoder { kind, held: vec![], faders: vec![] }
    }

    pub fn decode(&mut self, pkt: &[u8]) -> Vec<Input> {
        let mut out = vec![];
        if pkt.len() < 6 || pkt[0] != 0x01 {
            return out;
        }
        let (id, len) = (pkt[4], pkt[5] as usize);
        let Some(data) = pkt.get(6..6 + len) else { return out };
        let twist = |v: u8, positive: u8| if v == positive { 1 } else { -1 };
        match (self.kind, id) {
            (Kind::Keys15, 0x02) => {
                self.edges(data, &mut out, Input::KeyDown, Input::KeyUp);
            }
            (Kind::Knob3, 0x00) => {
                for (i, &v) in data.iter().enumerate().filter(|(_, v)| **v <= 1) {
                    out.push(Input::EncoderTwist(i as u8, twist(v, 1)));
                }
            }
            (Kind::Knob3, 0x03) => {
                self.edges(data, &mut out, Input::EncoderDown, Input::EncoderUp);
            }
            (Kind::Roller2, 0x01) => {
                // 0 = w górę. Raport podaje rolki od prawej, a OpenDeck numeruje od lewej.
                let count = data.len() as u8;
                for (i, &v) in data.iter().enumerate().filter(|(_, v)| **v <= 1) {
                    out.push(Input::EncoderTwist(count - 1 - i as u8, twist(v, 0)));
                }
            }
            (Kind::Dial, 0x03) => {
                // Odwrotnie niż gałki: 0 = w prawo.
                if let Some(&v) = data.first().filter(|v| **v <= 1) {
                    out.push(Input::EncoderTwist(0, twist(v, 0)));
                }
            }
            (Kind::Dial, 0x04) => {
                self.edges(&data[..data.len().min(1)], &mut out, Input::KeyDown, Input::KeyUp);
            }
            (Kind::Fader, 0x02) => {
                self.faders.resize(data.len(), None);
                for (i, (&v, last)) in data.iter().zip(self.faders.iter_mut()).enumerate() {
                    if v > 100 {
                        continue;
                    }
                    match *last {
                        // Pierwszy odczyt ustala punkt odniesienia.
                        None => *last = Some(v),
                        // Pomijamy drgania o 1, chyba że suwak dojechał do końca zakresu.
                        Some(prev) if v.abs_diff(prev) >= 2 || (v != prev && (v == 0 || v == 100)) => {
                            *last = Some(v);
                            // Najpierw pozycja, żeby akcja głośności miała ją już przy obrocie.
                            out.push(Input::FaderValue(i as u8, v));
                            out.push(Input::EncoderTwist(i as u8, v as i16 - prev as i16));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn edges(&mut self, now: &[u8], out: &mut Vec<Input>, down: fn(u8) -> Input, up: fn(u8) -> Input) {
        self.held.resize(now.len(), 0);
        for (i, (&n, p)) in now.iter().zip(self.held.iter_mut()).enumerate() {
            match (n == 1, *p == 1) {
                (true, false) => out.push(down(i as u8)),
                (false, true) => out.push(up(i as u8)),
                _ => {}
            }
            *p = n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(id: u8, data: &[u8]) -> Vec<u8> {
        let mut packet = vec![0x01, 0, 0, 0, id, data.len() as u8];
        packet.extend(data);
        packet
    }

    #[test]
    fn every_rotary_detent_is_forwarded_as_one_tick_in_both_directions() {
        for (kind, id, count, positive) in [(Kind::Dial, 0x03, 1, 0), (Kind::Knob3, 0x00, 3, 1), (Kind::Roller2, 0x01, 2, 0)] {
            for raw_position in 0..count {
                let position = if kind == Kind::Roller2 { count - 1 - raw_position } else { raw_position };
                for (value, ticks) in [(positive, 1), (1 - positive, -1)] {
                    let mut data = vec![0xff; count];
                    data[raw_position] = value;
                    let mut decoder = Decoder::new(kind);
                    for _ in 0..20 {
                        assert_eq!(decoder.decode(&packet(id, &data)), [Input::EncoderTwist(position as u8, ticks)]);
                    }
                }
            }
        }
    }
}
