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

Actions in the **MasterHUB** category:

- **Lighting** – colour, effect, brightness and speed of the LEDs on the base,
  knobs and fader (each LED separately).
- **Volume** – system, microphone, the focused window's app or chosen apps;
  absolute on the fader, stepped on knobs / rollers / the dial, press to mute. While changing
  it briefly shows a round level gauge on the dial screen.
- **GIF animation** – on a key, on the dial screen, stretched across the whole
  15xKey module or the same GIF on every key.
- **Clock** – time and date, e.g. on the dial's round screen. Pressing it can open
  a file / program / URL, send a keyboard shortcut or mute the system / microphone / focused app.
- **Sensor** – temperatures, load, power, fans etc. from
  [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor).
  On Windows also **FPS and frame time of the focused window** from RivaTuner Statistics Server
  (e.g. bundled with MSI Afterburner) – works without LibreHardwareMonitor.

## Encoder feedback on the dial screen

Stream Deck+ plugins that draw a bar on the touch strip above an encoder (e.g. the
[Home Assistant](https://github.com/cgiesche/streamdeck-homeassistant) light brightness, layout
**`$B1`**) can show it on the MasterHUB dial's round screen while you turn that encoder – on the
dial itself, any knob or any roller. Only recognized bar layouts are shown; encoders with plain
icons and encoders running this plugin's own "Volume" action (which has its own gauge) never take
over the screen, so the clock stays put. Standard **0–100%** values are redrawn sharply at
**240×240**; the number is the one confirmed by the other plugin, turning never predicts it.
After 1.5 s without changes the screen returns to the clock / key image.

Confirmed encoder feedback goes to the free LCD writer immediately, without the 33 ms spacing
used for background and volume images. Audio, clock and sensor polling run independently;
their reads and rendering leave the OpenDeck output available for encoder events. JPEG quality
and the sharp 240×240 text stay unchanged. Waiting for HA and an ongoing USB transfer still
contributes to the response time.

For Home Assistant: keep **±5%** and **`$B1`**. If the actual light alternates brighter
and dimmer while turning in one direction, use the
[serialized light script and configuration](docs/ha-light-rotation.md); its preset uses
**Tick multiplier = 1** and **Tick bucket size = 100 ms**. The
[configuration and timing guide](docs/ha-lcd-feedback.md) explains HA's percentage rounding and
the `masterhub.log` measurements.

## Languages

The plugin speaks **English and Polish**: settings panels, texts drawn on the screens
(volume overlay, clock date…) and sensor messages. The language follows the package you
install and can be changed in any action's settings (**Język / Language**: Auto / Polski /
English – shared by all actions). Releases contain two packages that differ only in the action
names shown in OpenDeck's action list (OpenDeck reads them from the manifest and has no Polish
UI of its own):

- `io.github.opendeck-masterhub-en.streamDeckPlugin` – English names,
- `io.github.opendeck-masterhub-pl.streamDeckPlugin` – Polish names.

## Installation

All systems:

1. Install OpenDeck (2.14 or newer).
2. **Close the official Cooler Master MasterHUB app** if it's running – both would fight over
   the modules.
3. Download `io.github.opendeck-masterhub-en.streamDeckPlugin` (or `-pl`) from
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

Requires [Rust](https://rustup.rs) (1.89+) and Python 3 for packaging. Code comments are in Polish.

| System | Command | Output |
|---|---|---|
| Windows | `cargo build --release` | `target/release/opendeck-masterhub.exe` |
| Linux / WSL | `sh build-linux.sh` | `dist/bin/opendeck-masterhub-linux` |
| macOS | `sh macos/build.sh` (universal binary) | `dist/bin/opendeck-masterhub-mac` |

`python package.py` then bundles whatever binaries exist into
`dist/io.github.opendeck-masterhub-{en,pl}.streamDeckPlugin` (`--lang en` / `--lang pl` for just one).
On Windows `install.ps1` builds and installs straight into OpenDeck (close OpenDeck first);
the action names follow the Windows display language, `-Lang en` / `-Lang pl` overrides it.
Other systems: `python3 package.py --install --lang en`.
GitHub Actions builds all three systems on every push; pushing a `v*` tag publishes a release.

Translations: panel texts are in the HTML (Polish) with English in `data-en` attributes
(`pi/i18n.js`), texts drawn by the plugin use `i18n::tr("polski", "English")`, and the Polish
action names live in `pl.json` (Stream Deck localization format).

For a Windows build on Linux (Debian / Ubuntu) install `gcc-mingw-w64-x86-64`, run
`rustup target add x86_64-pc-windows-gnu` and `sh build-windows.sh` – the result goes to
`dist/bin/opendeck-masterhub.exe`, which `package.py` prefers over `target/release`.
Run `cargo test --locked` before building. A feedback preview without hardware:
`cargo run --release --example feedback_preview -- tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png`.

`examples/` contains small hardware and rendering test tools (`cargo run --example …`).

## Credits and license

MIT – see [LICENSE](LICENSE). The HID protocol is based on
[cooltek-sdk-probe](https://github.com/projectzero7/cooltek-sdk-probe) (MIT) plus own analysis
(dial screen, LEDs, GIFs). Cooler Master and MasterHUB are trademarks of Cooler Master
Technology Inc.
