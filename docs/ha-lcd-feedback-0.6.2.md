# Odtworzenie obsługi Home Assistant na LCD MasterHUB — wersja 0.6.2

## Cel i zakres

Obrót enkodera LCD steruje `light.pokoj_seba` przez istniejący plugin Home Assistant,
z krokiem ±5%. LCD pokazuje chwilowo okrągły wskaźnik jasności, a następnie wraca do
zegara. Odświeżanie profilu lub edycja innego panelu nie powinny samodzielnie włączać
nakładki.

Zmiany wykonano w pluginie `opendeck-masterhub`. Nie zmieniano kodu OpenDecka ani
pluginu Home Assistant. MasterHUB odbiera `setImage` zawierający gotowy obraz data URL;
nie dostaje bezpośrednio JSON-a `setFeedback` ani atrybutu `attributes.brightness`.

## 1. Odbiór obrazu i powiązanie z rzeczywistym obrotem — src/main.rs

- W `GlobalEventHandler::set_image()` usunięto bezwarunkowe ignorowanie `Encoder`.
- Feedback jest obsługiwany tylko dla podłączonego modułu `Kind::Dial`, pozycji `0`
  albo `None` (cały rząd, który w tym module zawiera jeden enkoder).
- Obraz jest przekazywany do `overlay::show_feedback(&event.device, image)`.
  Dekodowanie i zapis HID odbywają się poza wątkiem obsługującym zdarzenia OpenDecka.
- W `read_loop()`, dla fizycznego `Input::EncoderTwist(0, _)` z modułu LCD, przed
  wysłaniem zdarzenia do OpenDecka wywoływane jest `overlay::encoder_twist(id)`.
- Czyszczenie ekranu urządzenia albo pusty obraz enkodera wywołują
  `overlay::clear_feedback()`, aby usunąć dane poprzedniego przypisania.
- Dotychczasowa ścieżka obrazów zwykłych klawiszy pozostaje osobną ścieżką.

## 2. Konwersja standardowego $B1 na pierścień — src/images.rs

Dodano `DialFeedback::{Image, Ring}` i `RingFeedback`, który przechowuje poziom,
maskę oryginalnego tekstu wartości oraz opcjonalną maskę tytułu.

`dial_feedback()` dekoduje base64 i obraz JPEG/PNG przez istniejącą bibliotekę
`image`. Dla standardowego `$B1` rozpoznaje płótno 200×100 i następujące obszary:

```text
title:     x=16, y=10, width=136, height=24
value:     x=76, y=40, width=108, height=32
indicator: x=76, y=74, width=108, height=12
```

Poziom jest odczytywany z paska: na wierszach 79 i 80 analizowane są kolumny 76–183.
Sprawdzany jest szary/biały kolor paska, jasność 125–255 i czarne otoczenie.
Różnica największej i najmniejszej składowej RGB nie może przekraczać 35; jasność
poza paskiem w kolumnach 73 i 186 nie może przekraczać 60. Wypełnienie to ciągły
biały fragment od lewej, o średniej jasności powyżej 212. `level = fill / 108`.
Dokładność wypełnienia wynosi około 1 punktu procentowego.

Wyświetlany procent jest wycinany z oryginalnego obrazu, więc zachowuje tekst HA.
Nie ma OCR ani wyliczania tekstu procentu z przybliżonej długości paska. Maska tekstu
ma jasność `(max(luma - 32, 0) * 255) / 223`, a granice wycięcia wyznaczają piksele
maski powyżej 64. Wycięty tekst jest skalowany z zachowaniem proporcji i nakładany
na tło pierścienia; wartość zajmuje maksymalnie 64% szerokości i 26% wysokości LCD,
ze środkiem na 47% wysokości. Tytuł zajmuje maksymalnie 52% szerokości i 9% wysokości,
ze środkiem na 71% wysokości.

Pierścień jest rysowany istniejącym `render::gauge()` w rozdzielczości
`hub::DIAL_IMAGE_SIZE = 240`. Jeśli geometria, kolory albo tekst nie pasują do `$B1`,
plugin dopasowuje cały oryginalny obraz do 240×240 na czarnym tle, bez przycinania
i rozciągania proporcji.

## 3. Nakładka, kolejka i wygładzanie — src/overlay.rs

Stan jest osobny dla każdego identyfikatora urządzenia. `show_on()` wyświetla obraz
na wskazanym pokrętle; istniejące `show()` nadal rozsyła własny overlay głośności
do podłączonych pokręteł.

Każdy stan przechowuje najnowszą oczekującą klatkę, numer rewizji, ostatni feedback,
feedback w pamięci, okno interakcji, tryb nakładki, faktycznie wysłany poziom oraz
termin wygaszenia. Kluczowe zasady:

- Każdy obraz feedbacku jest zapamiętywany, ale sam `setImage` nie włącza nakładki.
- Fizyczny obrót otwiera okno odbioru feedbacku na 1500 ms dla tego urządzenia.
  Ostatni zapamiętany, potwierdzony stan może wtedy zostać pokazany od razu.
- Feedback poza oknem interakcji tylko aktualizuje pamięć. To usuwa przypadkowe
  wyświetlanie jasności podczas zmiany profilu lub edycji innych paneli.
- Jeden worker na urządzenie obsługuje konwersję i HID. `pending` przechowuje tylko
  ostatnią klatkę; duplikaty aktywnego feedbacku są pomijane.
- Worker budzi się przez `Condvar`, zamiast sprawdzać nowe obrazy co 30 ms.
- Rewizja jest sprawdzana po przygotowaniu klatki, przed zapisem, aby pominąć
  klatki zastąpione przez nowszą aktualizację.
- Zapis HID jest serializowany przez istniejący `with_writer()`.

Parametry animacji:

```rust
const SHOW_FOR: Duration = Duration::from_millis(1500);
const TRANSITION_FOR: Duration = Duration::from_millis(100);
const FRAME_EVERY: Duration = Duration::from_millis(33);
```

Animowany jest tylko pierścień. Duży procent od razu pokazuje potwierdzony tekst HA.
Przejście używa `t = min(elapsed / 100 ms, 1)` i wygładzenia `t * (2 - t)`.
Nowy cel zaczyna przejście od poziomu faktycznie wysłanego do LCD. Początek czasu
przejścia jest ustawiany na `now - FRAME_EVERY`, aby już pierwsza klatka przesuwała
pierścień. Dzięki temu ciągły feedback nie resetuje ruchu przed każdą drugą klatką.

Klatki animacji nie są kolejkowane. Przy wolnym HID worker oblicza obraz dla
aktualnego czasu i pomija klatki pośrednie. Nowy feedback ma pierwszeństwo.

Po wygaszeniu nakładki przywracany jest `DIRECT`, np. zegar, a gdy go brakuje —
`LAST_IMAGE`, czyli ostatni obraz klawisza od OpenDecka. Nowy obrót podczas przywracania
zegara ponownie wyświetla także identyczny procent. Czyszczenie feedbacku anuluje
nakładkę enkodera, zachowując osobną obsługę własnej nakładki głośności.

## 4. Renderowanie, wersja i pakowanie

- `src/render.rs`: udostępniono kolor `BG` i dodano `jpeg_with_quality()`.
  Feedback używa jakości JPEG 80, a własne obrazy pluginu dotychczasowej jakości 95.
- Dodano `build-windows.sh`: build Windows x64 na Linuksie przez
  `x86_64-pc-windows-gnu` i MinGW-w64. Wynik: `dist/bin/opendeck-masterhub.exe`.
- `package.py` wybiera Windows EXE najpierw z `dist/bin`, a następnie z
  `target/release`, obsługując także natywny build na Windowsie.
- Wersję ustawiono na `0.6.2` w `Cargo.toml`, wpisie projektu w `Cargo.lock`
  i `manifest.json`.
- Dodano opis w `README.md`, narzędzie `examples/feedback_preview.rs` oraz obrazy
  testowe `$B1` w `tests/fixtures/encoder-b1` wraz z opisem ich pochodzenia.
- Nie dodano nowych zależności Cargo.

## 5. Odtworzenie i kompilacja

Archiwum źródeł zawiera pełny stan projektu 0.6.2, wraz z assetami, lockfile'em
i obrazami testowymi. Nie zawiera `.git`, katalogów kompilacji ani plików binarnych.

Sprawdzone środowisko: Debian 12, Rust/Cargo 1.92.0, MinGW-w64 GCC 12.2.0.
Po rozpakowaniu archiwum, w katalogu projektu, z zainstalowanym Rustem:

```sh
sudo apt-get install gcc-mingw-w64-x86-64 python3
rustup target add x86_64-pc-windows-gnu
cargo test --locked
sh build-windows.sh
# Opcjonalnie, aby ta sama paczka zawierała także wersję Linux:
sh build-linux.sh
python3 package.py
```

Skrypty domyślnie trzymają kompilację w
`$HOME/.cache/opendeck-masterhub-target`; można ustawić `CARGO_TARGET_DIR`.
Wynikiem pakowania jest `dist/io.github.opendeck-masterhub.streamDeckPlugin`.
Zainstaluj go w OpenDeck przez **Plugins → Install from file**, następnie uruchom
ponownie OpenDeck.

Podgląd pojedynczej klatki, bez urządzenia:

```sh
mkdir -p dist
cargo run --locked --release --example feedback_preview -- \
  tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55.png
```

## 6. Konfiguracja HA i zegara

- Encodera LCD przypisz do istniejącej akcji HA sterującej `light.pokoj_seba`.
- Ustaw krok jasności ±5% i feedback `$B1`.
- Zegar przypisz do klawisza tego samego modułu LCD.
- W pluginie HA ustawiono **Tick bucket size = 50 ms** zamiast 300 ms.
  To ustawienie zewnętrznego pluginu, nie zmiana kodu MasterHuba. Skraca zbieranie
  obrotów przed komendą, lecz samo nie zapewnia płynności LCD.
- MasterHUB nie zmienia service data HA ani rzeczywistej jasności. Wygładzanie
  dotyczy prezentacji na ekranie.

## 7. Weryfikacja

Weryfikacja odtworzonej implementacji **12 października 2026**: **19 testów przeszło**, buildy release Windows
x64 i Linux przeszły. Archiwum instalacyjne sprawdzono pod kątem poprawności ZIP,
wersji manifestu i zgodności zawartych binariów z wynikami kompilacji.

Testy obejmują m.in. odczyt 0%/100% i wartości pośrednich z rzeczywistych obrazów
renderera `$B1`, zachowanie tekstu procentu, inne układy obrazów, duplikaty,
pomijanie starych klatek, płynność i przerwanie przejścia, symulację wolnego HID,
powrót zegara, obrót podczas przywracania zegara, izolację urządzeń oraz odświeżenia
profilu/edytora bez rzeczywistego obrotu.

Płynność finalnej wersji na fizycznym urządzeniu wymaga weryfikacji użytkownika;
testy symulacji nie mierzą realnej przepustowości USB ani czasu odpowiedzi HA.
