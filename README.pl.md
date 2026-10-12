# MasterHUB dla OpenDecka

[English](README.md) · **Polski**

Nieoficjalny plugin do [OpenDecka](https://github.com/nekename/OpenDeck), który obsługuje
moduły **Cooler Master MasterHUB** bezpośrednio przez USB HID – bez oficjalnej aplikacji
Cooler Mastera. Nie jest związany z Cooler Masterem.

## Co działa

| Moduł | W OpenDecku |
|---|---|
| 15xKey (ekrany IPS) | siatka 5×3, ikony z OpenDecka na klawiszach, suwak jasności OpenDecka |
| 1xDial (pokrętło z ekranem) | klawisz (wciśnięcie + obraz 240×240 na ekranie) i enkoder (obrót) |
| 3xKnob | 3 enkodery (obrót + wciśnięcie) |
| 2xRoller | 2 enkodery |
| 5xFader | 5 enkoderów; z akcją „Głośność” pozycja suwaka = głośność |
| Baza | tylko podświetlenie (przez akcję „Podświetlenie”) |

Każdy moduł jest osobnym urządzeniem w OpenDecku z własnymi profilami – przepinanie modułów
nie rusza przypisań.

Akcje w kategorii **MasterHUB**:

- **Podświetlenie** – kolor, efekt, jasność i prędkość diod bazy, gałek i fadera
  (osobno dla każdej diody).
- **Głośność** – system, mikrofon, aplikacja aktywnego okna albo wybrane aplikacje; na faderze
  bezwzględnie, na gałkach / rolkach / pokrętle krokowo, wciśnięcie wycisza. Przy zmianie na
  chwilę pokazuje okrągły wskaźnik poziomu na ekranie pokrętła.
- **Animacja GIF** – na klawiszu, na ekranie pokrętła, rozciągnięta na cały moduł 15xKey
  albo ta sama na każdym klawiszu.
- **Zegar** – godzina i data, np. na okrągłym ekranie pokrętła. Wciśnięcie może otworzyć
  plik / program / URL, wysłać skrót klawiszowy albo wyciszyć system / mikrofon / aktywne okno.
- **Czujnik** – temperatury, obciążenie, moc, wentylatory itd. z
  [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor).
  Na Windows także **FPS i czas klatki aktywnego okna** z RivaTuner Statistics Server
  (np. razem z MSI Afterburnerem) – działa bez LibreHardwareMonitor.

## Obraz z enkoderów na ekranie pokrętła

Pluginy ze wsparciem Stream Decka+, które rysują pasek na wyświetlaczu nad pokrętłem (np. jasność
światła z [Home Assistanta](https://github.com/cgiesche/streamdeck-homeassistant), układ **`$B1`**),
mogą pokazywać go na okrągłym ekranie pokrętła MasterHUB-a, gdy kręcisz tym enkoderem – samym
pokrętłem, dowolną gałką albo rolką. Pokazywane są tylko rozpoznane układy z paskiem; enkodery
ze zwykłą ikoną i enkodery z akcją „Głośność” tego pluginu (ma własny wskaźnik) nigdy nie
przejmują ekranu, więc zegar zostaje. Standardowe wartości **0–100%** są rysowane ostro
w **240×240**; liczba pochodzi z drugiego pluginu, obrót jej nie przewiduje. Po 1,5 s bez zmian
ekran wraca do zegara / obrazu klawisza.

Dla Home Assistanta: zostaw **±5%**, **`$B1`** i **Tick bucket size = 50 ms**.
[Instrukcja konfiguracji i pomiarów](docs/ha-lcd-feedback.md) opisuje zaokrąglenia HA oraz
pomiary w `masterhub.log`.

## Języki

Plugin mówi **po polsku i po angielsku**: panele ustawień, napisy rysowane na ekranach
(nakładka głośności, data na zegarze…) i komunikaty czujników. Język zależy od zainstalowanej
paczki i można go zmienić w ustawieniach dowolnej akcji (**Język / Language**: Auto / Polski /
English – wspólny dla wszystkich akcji). Wydania zawierają dwie paczki, które różnią się tylko
nazwami akcji na liście OpenDecka (OpenDeck czyta je z manifestu i nie ma polskiej wersji):

- `io.github.opendeck-masterhub-pl.streamDeckPlugin` – polskie nazwy,
- `io.github.opendeck-masterhub-en.streamDeckPlugin` – angielskie nazwy.

## Instalacja

Wspólne dla wszystkich systemów:

1. Zainstaluj OpenDecka (2.14 lub nowszy).
2. **Zamknij oficjalną aplikację Cooler Master MasterHUB**, jeśli działa – oba programy
   naraz walczyłyby o moduły.
3. Pobierz `io.github.opendeck-masterhub-pl.streamDeckPlugin` (albo `-en`) z
   [Releases](../../releases) i w OpenDecku wybierz **Plugins → Install from file**.
4. Moduły pojawią się na liście urządzeń OpenDecka w ciągu kilku sekund.

### Windows (10 / 11)

Nic więcej nie trzeba. Akcja „Czujnik” wymaga LibreHardwareMonitor z włączonym serwerem WWW
(**Options → Remote Web Server → Run**, port 8085); warto też zaznaczyć
**Options → Run On Windows Startup**.

### Linux (x86_64, glibc 2.34+: Ubuntu 22.04+, Debian 12+, Fedora 35+, Arch…)

1. **Dostęp do USB** – bez tego zwykły użytkownik nie otworzy modułów. Plik reguły jest
   w folderze pluginu (`~/.config/opendeck/plugins/io.github.opendeck-masterhub.sdPlugin/`,
   w wersji Flatpak `~/.var/app/me.amankhanna.opendeck/config/opendeck/plugins/…`):

   ```
   sudo cp 70-masterhub.rules /etc/udev/rules.d/
   sudo udevadm control --reload-rules && sudo udevadm trigger
   ```

   Potem odłącz i podłącz MasterHUB.
2. Programy używane przez akcje (zwykle już są w systemie):
   - `pactl` (pakiet `pulseaudio-utils`, działa też z PipeWire) – akcja „Głośność”,
   - `xdotool` – cel „Aktywne okno” (tylko X11; na Wayland niedostępny),
   - `xdg-open` – otwieranie plików / adresów.
3. „Czujnik” czyta `/sys/class/hwmon` (temperatury, wentylatory, moc), obciążenie CPU
   i zajętą pamięć – bez dodatkowych programów.

Skróty klawiszowe działają na X11 (na Wayland zależnie od środowiska). OpenDeck z Flatpaka
jest obsługiwany – plugin uruchamia programy systemowe przez `flatpak-spawn --host`.

### macOS (11+, Apple Silicon i Intel)

- „Głośność” steruje tylko głośnością systemu i mikrofonu – macOS nie pozwala zmieniać
  głośności pojedynczych aplikacji.
- Skróty klawiszowe wymagają zgody: **Ustawienia systemowe → Prywatność i ochrona →
  Dostępność** – dopisz OpenDecka.
- „Czujnik” nie działa (macOS nie udostępnia czujników bez dodatkowych programów).
- Przy pierwszym uruchomieniu macOS może zablokować niepodpisany plik pluginu – wtedy
  **Ustawienia systemowe → Prywatność i ochrona → „Otwórz mimo to”**.
- Wersja dla macOS nie była jeszcze testowana z prawdziwym sprzętem.

## Dobrze wiedzieć

- Kierunki obrotu i kolejność rolek są ustawione według egzemplarza, na którym plugin powstał.
- Plugin nie aktualizuje firmware'u modułów.
- Log pluginu: `masterhub.log` w folderze pluginu
  (Windows: `%APPDATA%\opendeck\plugins\io.github.opendeck-masterhub.sdPlugin\`).

## Budowanie

Wymaga [Rusta](https://rustup.rs) (1.89+) i Pythona 3 do pakowania. Komentarze w kodzie są po polsku.

| System | Polecenie | Wynik |
|---|---|---|
| Windows | `cargo build --release` | `target/release/opendeck-masterhub.exe` |
| Linux / WSL | `sh build-linux.sh` | `dist/bin/opendeck-masterhub-linux` |
| macOS | `sh macos/build.sh` (plik uniwersalny) | `dist/bin/opendeck-masterhub-mac` |

`python package.py` pakuje dostępne pliki do `dist/io.github.opendeck-masterhub-{en,pl}.streamDeckPlugin`
(`--lang en` / `--lang pl` – tylko jedna). Na Windows `install.ps1` buduje i instaluje od razu
do OpenDecka (zamknij najpierw OpenDecka); nazwy akcji według języka Windows, `-Lang en` /
`-Lang pl` to zmienia. Inne systemy: `python3 package.py --install --lang pl`.

Build dla Windows na Linuksie (Debian / Ubuntu): zainstaluj `gcc-mingw-w64-x86-64`, uruchom
`rustup target add x86_64-pc-windows-gnu` i `sh build-windows.sh` – wynik trafia do
`dist/bin/opendeck-masterhub.exe`, który `package.py` bierze przed `target/release`.
Przed budowaniem uruchom `cargo test --locked`. Podgląd obrazu z enkodera bez urządzenia:
`cargo run --release --example feedback_preview -- tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png`.
GitHub Actions buduje wszystkie trzy systemy przy każdym pushu; tag `v*` publikuje wydanie.

Tłumaczenia: teksty paneli są w HTML (po polsku), angielskie w atrybutach `data-en`
(`pi/i18n.js`); napisy rysowane przez plugin – `i18n::tr("polski", "English")`; polskie nazwy
akcji – `pl.json` (format tłumaczeń Stream Decka).

W `examples/` są małe narzędzia testowe sprzętu i grafiki (`cargo run --example …`).

## Autorzy i licencja

MIT – zobacz [LICENSE](LICENSE). Protokół HID: na podstawie
[cooltek-sdk-probe](https://github.com/projectzero7/cooltek-sdk-probe) (MIT) oraz własnej
analizy (ekran pokrętła, diody, GIF-y). Cooler Master i MasterHUB są znakami towarowymi
Cooler Master Technology Inc.
