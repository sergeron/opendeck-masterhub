# MasterHUB for OpenDeck

**English** · [Polski](README.pl.md)

An unofficial [OpenDeck](https://github.com/nekename/OpenDeck) plugin that drives
**Cooler Master MasterHUB** modules directly over USB HID – no official Cooler Master software
needed. Not affiliated with or endorsed by Cooler Master.

## What works

| Module | In OpenDeck |
|---|---|
| 15xKey (IPS screens) | 5×3 grid, OpenDeck icons on the keys, OpenDeck brightness slider |
| 1xDial (dial with a round screen) | a key (press + 240×240 image on the screen) and an encoder (rotation) |
| 3xKnob | 3 encoders (rotate + press) |
| 2xRoller | 2 encoders |
| 5xFader | 5 encoders; with the "Volume" action the slider position is the volume |
| Base | lighting only (through the "Lighting" action) |

Every module is a separate OpenDeck device with its own profiles, so re-plugging modules
doesn't break assignments.

Actions in the **MasterHUB** category (their settings panels are in Polish):

- **Podświetlenie (Lighting)** – colour, effect, brightness and speed of the LEDs on the base,
  knobs and fader (each LED separately).
- **Głośność (Volume)** – system, microphone, the focused window's app or chosen apps;
  absolute on the fader, stepped on knobs / rollers / the dial, press to mute. While changing
  it briefly shows a round level gauge on the dial screen.
- **Animacja GIF (GIF animation)** – on a key, on the dial screen, stretched across the whole
  15xKey module or the same GIF on every key.
- **Zegar (Clock)** – time and date, e.g. on the dial's round screen. Pressing it can open
  a file / program / URL, send a keyboard shortcut or mute the system / microphone / focused app.
- **Czujnik (Sensor)** – temperatures, load, power, fans etc. from
  [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor).
  On Windows also **FPS and frame time of the focused window** from RivaTuner Statistics Server
  (e.g. bundled with MSI Afterburner) – works without LibreHardwareMonitor.

## LCD encoder feedback (0.6.4)

The LCD dial, all three knobs and both rollers can provide feedback on the connected
LCDs. Feedback memory is separate for each device and encoder position. The latest
physically rotated encoder owns the shared screen; profile refreshes keep the clock.

Standard **`$B1`** percentages **0–100%** are recognized from the original value text
and drawn locally at **240×240**, using the same system font as the volume gauge.
The number comes from the confirmed HA image, independently of the bar length.
Uncertain or custom text keeps its original image. The original title is preserved
at its pixel size; it is no longer enlarged. **Rotation never predicts a new value.**

A separate decoder prepares and caches the latest JPEG per encoder while the LCD
writer may be busy. Matching visual feedback reuses the prepared image. The LCD
worker no longer waits the extra 8 ms before uploading it, and ring geometry is
cached. Optimized baseline JPEG 4:4:4 at quality 80 reduces USB traffic; volume
and clock images keep their existing encoder. The new encoder is the pure Rust
`jpeg-encoder` crate, with no extra system libraries.

Assign the HA action to any dial, knob or roller encoder, keep **±5%**, **`$B1`** and
**Tick bucket size = 50 ms**, and put **Zegar (Clock)** on the LCD key. The screen
shows the percentage actually sent by HA. The [configuration and timing guide](docs/ha-lcd-feedback.md)
explains raw percentage rounding and the new `masterhub.log` measurements for
incoming feedback, preparation, writer lock and HID transfer.

A faster local renderer cannot remove the wait for a confirmed HA response.
USB and physical LCD timing still need a hardware check.

## Installation

All systems:

1. Install OpenDeck (2.14 or newer).
2. **Close the official Cooler Master MasterHUB app** if it's running – both would fight over
   the modules.
3. Download `io.github.opendeck-masterhub.streamDeckPlugin` from
   [Releases](../../releases) and in OpenDeck choose **Plugins → Install from file**.
4. The modules show up in OpenDeck's device list within a few seconds.

### Windows (10 / 11)

Nothing else is needed. The "Sensor" action needs LibreHardwareMonitor with its web server
enabled (**Options → Remote Web Server → Run**, port 8085); **Options → Run On Windows Startup**
is handy too.

### Linux (x86_64, glibc 2.34+: Ubuntu 22.04+, Debian 12+, Fedora 35+, Arch…)

1. **USB access** – without it a regular user can't open the modules. The rule file is in the
   plugin folder (`~/.config/opendeck/plugins/io.github.opendeck-masterhub.sdPlugin/`,
   for the Flatpak version `~/.var/app/me.amankhanna.opendeck/config/opendeck/plugins/…`):

   ```
   sudo cp 70-masterhub.rules /etc/udev/rules.d/
   sudo udevadm control --reload-rules && sudo udevadm trigger
   ```

   Then unplug and replug the MasterHUB.
2. Programs used by the actions (usually already installed):
   - `pactl` (package `pulseaudio-utils`, works with PipeWire too) – "Volume",
   - `xdotool` – the "focused window" volume target (X11 only, not available on Wayland),
   - `xdg-open` – opening files / URLs.
3. "Sensor" reads `/sys/class/hwmon` (temperatures, fans, power), CPU load and used memory –
   no extra programs.

Keyboard shortcuts work on X11 (on Wayland it depends on the desktop). OpenDeck from Flatpak
is supported – the plugin runs host programs through `flatpak-spawn --host`.

### macOS (11+, Apple Silicon and Intel)

- "Volume" controls only the system and microphone volume – macOS doesn't allow per-app volume.
- Keyboard shortcuts need permission: **System Settings → Privacy & Security → Accessibility** –
  add OpenDeck.
- "Sensor" doesn't work (macOS doesn't expose sensors without extra software).
- macOS may block the unsigned plugin binary on first start – then use
  **System Settings → Privacy & Security → "Open Anyway"**.
- The macOS build hasn't been tested with real hardware yet.

## Good to know

- Rotation directions and roller order are set up after the unit the plugin was written on.
- The plugin doesn't update module firmware.
- Plugin log: `masterhub.log` in the plugin folder
  (Windows: `%APPDATA%\opendeck\plugins\io.github.opendeck-masterhub.sdPlugin\`).

## Building

Requires [Rust](https://rustup.rs) (1.89+). Code comments are in Polish.

| System | Command | Output |
|---|---|---|
| Windows | `cargo build --release`, then `install.ps1` copies it into OpenDeck (close OpenDeck first) | `target/release/opendeck-masterhub.exe` |
| Windows x64 built on Linux | `sh build-windows.sh` | `dist/bin/opendeck-masterhub.exe` |
| Linux / WSL | `sh build-linux.sh` | `dist/bin/opendeck-masterhub-linux` |
| macOS | `sh macos/build.sh` (universal binary) | `dist/bin/opendeck-masterhub-mac` |

For the Windows cross-build on Debian / Ubuntu, install
`gcc-mingw-w64-x86-64` and add the Rust target with
`rustup target add x86_64-pc-windows-gnu`. The Linux and Windows scripts keep build
files in `$HOME/.cache/opendeck-masterhub-target` by default; `CARGO_TARGET_DIR`
overrides this location.

Run `cargo test --locked` before building. A feedback preview without hardware:

```sh
mkdir -p dist
cargo run --locked --release --example feedback_preview -- \
  tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png
```

`python3 package.py` then bundles whatever binaries exist into
`dist/io.github.opendeck-masterhub.streamDeckPlugin`. GitHub Actions builds all three systems
on every push; pushing a `v*` tag publishes a release.
Packaging prefers the Windows executable in `dist/bin`, then falls back to
`target/release` for a native Windows build.

`examples/` contains small hardware and rendering test tools (`cargo run --example …`).

## Credits and license

MIT – see [LICENSE](LICENSE). The HID protocol is based on
[cooltek-sdk-probe](https://github.com/projectzero7/cooltek-sdk-probe) (MIT) plus own analysis
(dial screen, LEDs, GIFs). Cooler Master and MasterHUB are trademarks of Cooler Master
Technology Inc.
