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

## Feedback enkodera LCD (0.6.4)

Źródłem feedbacku może być pokrętło LCD, każdy z trzech knobów lub oba rollery.
Pamięć jest osobna dla urządzenia i pozycji. Ostatnio obracany enkoder wybiera źródło
wspólnego ekranu; odświeżenia profilu bez obrotu zachowują zegar.

Standardowe procenty **0–100% z `$B1`** są rozpoznawane z oryginalnego napisu
i rysowane lokalnie w **240×240**, tą samą czcionką systemową co głośność.
Liczba pochodzi z potwierdzonego obrazu HA, niezależnie od długości paska.
Niepewne i niestandardowe napisy zachowują oryginalny obraz. Tytuł jest zachowany
w rozmiarze oryginalnych pikseli, bez powiększania. **Obrót nie przewiduje wartości.**

Osobny dekoder przygotowuje najnowszy JPEG dla każdego enkodera podczas pracy HID.
Identyczny widok wykorzystuje gotowy obraz. Worker LCD nie czeka już dodatkowych
8 ms, a geometria pierścienia jest w pamięci. Zoptymalizowany baseline JPEG 4:4:4
z jakością 80 zmniejsza ruch USB. Głośność i zegar zachowują swój dotychczasowy
encoder JPEG. Dodano `jpeg-encoder` w czystym Rust, bez bibliotek systemowych.

Przypisz HA do dowolnego enkodera, zachowaj **±5%**, **`$B1`** i **Tick bucket size
= 50 ms**; **Zegar** zostaje na klawiszu LCD. Liczba na ekranie jest tą otrzymaną
z HA. [Instrukcja konfiguracji i pomiarów](docs/ha-lcd-feedback.md) opisuje zaokrąglenia
HA oraz nowe pomiary odbioru feedbacku, przygotowania, blokady writera i transferu
HID w `masterhub.log`.

Szybsze renderowanie lokalne nie usuwa oczekiwania na potwierdzoną odpowiedź HA.
Czas USB i zachowanie fizycznego LCD wymagają sprawdzenia na urządzeniu.

## Instalacja

Wspólne dla wszystkich systemów:

1. Zainstaluj OpenDecka (2.14 lub nowszy).
2. **Zamknij oficjalną aplikację Cooler Master MasterHUB**, jeśli działa – oba programy
   naraz walczyłyby o moduły.
3. Pobierz `io.github.opendeck-masterhub.streamDeckPlugin` z [Releases](../../releases)
   i w OpenDecku wybierz **Plugins → Install from file**.
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

Wymaga [Rusta](https://rustup.rs) (1.89+).

| System | Polecenie | Wynik |
|---|---|---|
| Windows | `cargo build --release`, potem `install.ps1` kopiuje go do OpenDecka (zamknij najpierw OpenDecka) | `target/release/opendeck-masterhub.exe` |
| Windows x64 z Linuksa | `sh build-windows.sh` | `dist/bin/opendeck-masterhub.exe` |
| Linux / WSL | `sh build-linux.sh` | `dist/bin/opendeck-masterhub-linux` |
| macOS | `sh macos/build.sh` (plik uniwersalny) | `dist/bin/opendeck-masterhub-mac` |

Build Windows na Debianie / Ubuntu wymaga `gcc-mingw-w64-x86-64` oraz
`rustup target add x86_64-pc-windows-gnu`. Skrypty Linux i Windows domyślnie trzymają
pliki kompilacji w `$HOME/.cache/opendeck-masterhub-target`; można ustawić
`CARGO_TARGET_DIR`.

Przed budowaniem uruchom `cargo test --locked`. Podgląd feedbacku bez urządzenia:

```sh
mkdir -p dist
cargo run --locked --release --example feedback_preview -- \
  tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png
```

`python3 package.py` pakuje dostępne pliki do `dist/io.github.opendeck-masterhub.streamDeckPlugin`.
Windows EXE jest wybierany najpierw z `dist/bin`, a następnie z `target/release`
(natywny build na Windowsie).
GitHub Actions buduje wszystkie trzy systemy przy każdym pushu; tag `v*` publikuje wydanie.

W `examples/` są małe narzędzia testowe sprzętu i grafiki (`cargo run --example …`).

## Autorzy i licencja

MIT – zobacz [LICENSE](LICENSE). Protokół HID: na podstawie
[cooltek-sdk-probe](https://github.com/projectzero7/cooltek-sdk-probe) (MIT) oraz własnej
analizy (ekran pokrętła, diody, GIF-y). Cooler Master i MasterHUB są znakami towarowymi
Cooler Master Technology Inc.
