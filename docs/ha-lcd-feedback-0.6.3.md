# Feedback Home Assistant na LCD MasterHUB — 0.6.3

Wersja 0.6.3 rozszerza feedback na wszystkie enkodery i usuwa dodatkowe klatki
animacji, które zwiększały ruch USB. [Opis poprzedniej wersji 0.6.2](ha-lcd-feedback-0.6.2.md)
pozostaje jako dokumentacja historyczna.

## Źródło feedbacku i ekran

- Akcja na enkoderze pokrętła LCD wyświetla feedback na tym pokrętle.
- Akcja na dowolnym knobie, rollerze lub faderze wyświetla feedback na podłączonych
  pokrętłach LCD. Bez podłączonego LCD nie ma ekranu, na którym można go pokazać.
- Pamięć i interakcja są przypisane do pary **urządzenie + pozycja enkodera**.
- Ostatni fizycznie obracany enkoder wybiera źródło dla wspólnego ekranu. Odpowiedzi
  innego enkodera aktualizują jego pamięć i nie przejmują ekranu.
- Feedback całego rzędu jest dzielony na pozycje przed konwersją `$B1`.
- Pusty obraz, zmiana profilu lub odłączenie modułu usuwa dane danego źródła.
  Wyczyszczenie jednej pozycji nie czyści innych ani własnej nakładki głośności.

`read_loop()` nadal przekazuje fizyczne ticki do OpenDecka. Dla pokrętła, knobów
i rollerów pojedynczy raport skoku daje tick `+1` albo `-1`; MasterHUB nie mnoży
go przez 5. Krok jasności pozostaje ustawieniem akcji HA. Fader przekazuje zmianę
pozycji, tak jak wcześniej.

## Aktualizacje LCD

Feedback, zegar, zwykłe obrazy i GIF przechodzą przez **jeden worker na LCD**.
Wybór klatki następuje przed zapisem, a sprawdzenie jej rewizji również **po
przejęciu blokady HID**, tuż przed pierwszym raportem obrazu. Zegar lub GIF nie może
przez osobną ścieżkę nadpisać aktywnego feedbacku.

Jedna potwierdzona wartość HA powoduje wysłanie jednej klatki. Usunięto animację
poziomów pośrednich. Każdy nowy cel pokazuje potwierdzony tekst oraz pierścień
odczytany z tego samego obrazu. Feedback używa JPEG jakości 80. Pozostałe własne
obrazy zachowują jakość 95.

Odstęp między początkami zapisów wynosi co najmniej 33 ms. Klatki napływające
jednocześnie są zbierane przez 8 ms. Przy wolnym USB worker kończy rozpoczęty obraz
i wybiera najnowszy oczekujący obraz. Nie tworzy kolejki starych procentów.
Jeśli HID zgłosi błąd, aktualna klatka jest ponawiana po 100 ms, również gdy kolejny
feedback zawiera identyczny procent. Nowsza klatka zastępuje oczekującą na ponowienie.
Przy odłączonym ekranie próby przywrócenia tła kończą się po trzech nieudanych zapisach.

Nakładka znika po 1500 ms od ostatniego obrotu lub nowego feedbacku. Oczekiwana
odpowiedź HA może nadejść do 5 s od obrotu, także po powrocie zegara. Po otrzymaniu
i wyświetleniu odpowiedzi odświeżenia zamkniętej interakcji nie otwierają nakładki.

### Reset układu `$B1`

Plugin HA wysyła `setFeedbackLayout`, a następnie `setFeedback`. OpenDeck po zmianie
układu odrysowuje go jeszcze bez właściwej wartości. W 0.6.2 taki obraz mógł zastąpić
procent i trafić do pamięci. W 0.6.3 rozpoznany `$B1` z pustym obszarem wartości jest
traktowany jako niepełny feedback: worker pomija klatkę i zachowuje ostatni poprawny
stan, aż nadejdzie rzeczywista wartość. Pozostałe układy nadal są dopasowywane do LCD
z zachowaniem proporcji.

Źródła: [wysyłanie layoutu i feedbacku przez HA](https://github.com/cgiesche/streamdeck-homeassistant/blob/7322fe0eac5279b3a09dcd0f4026c7b43df0636c/src/components/PluginComponent.vue),
[odrysowanie layoutu przez OpenDeck](https://github.com/nekename/OpenDeck/blob/66db15d9d5d41597772a4f209cd6a93b498fc7df/src-tauri/src/events/inbound/states.rs).

## Konfiguracja i liczba na LCD

Przypisz istniejącą akcję HA do dowolnego enkodera, ustaw krok jasności ±5%,
feedback `$B1` i **Tick bucket size = 50 ms**. Zegar przypisz do klawisza modułu LCD.
Kod OpenDecka i pluginu HA nie jest zmieniany przez tę paczkę.

MasterHUB otrzymuje gotowy obraz `setImage`, a nie atrybut `brightness`. Nie odczytuje
cyfr przez OCR i nie generuje przewidywanej liczby po samym obrocie. LCD pokazuje
tekst potwierdzony przez HA. Jeżeli HA pominie odpowiedzi pośrednie, LCD przechodzi
do ostatniej potwierdzonej wartości.

Domyślny [szablon HA dla światła](https://github.com/cgiesche/streamdeck-homeassistant/blob/7322fe0eac5279b3a09dcd0f4026c7b43df0636c/public/config/default-display-config.yml)
liczy `brightness / 255 * 100` i ucina część ułamkową przez `int`. Dlatego liczba
otrzymana z HA może wynosić np. 54% przy jasności `139/255 ≈ 54,51%`. MasterHUB
zachowuje tę liczbę. Samo wysyłanie klatek nie zmienia takiego tekstu na 55%.

Jeżeli na LCD mają być wyłącznie wielokrotności 5%, w **używanej konfiguracji
wyświetlania HA** zastąp `light.feedback` poniższym fragmentem, zachowując pozostałą
konfigurację. Nie jest to zmiana service data ani rzeczywistej jasności:

```yaml
light:
  feedbackLayout: '$B1'
  feedback: |
    {% set defaultBrightness = (255 if state == 'on' else 0) %}
    {% set brightness = attributes.brightness | default(defaultBrightness, true) %}
    {% set step = (brightness / 255 * 100 / 5) | round(0) %}
    {
      "indicator": {{ step * 5 }},
      "value": "{{ step * 5 }}%"
    }
```

Szablon zaokrągla potwierdzony stan do najbliższych 5%, a jego liczbę i pasek
MasterHUB wyświetla spójnie. Nie zastosowano go automatycznie do konfiguracji HA
użytkownika: w tym repozytorium nie ma instalacji ani ustawień tej zewnętrznej akcji.

## Budowanie i instalacja

Sprawdzone środowisko: Debian 12, Rust/Cargo 1.92.0 i lokalny MinGW-w64 GCC 12.2.
Z zainstalowanym Rustem i MinGW:

```sh
rustup target add x86_64-pc-windows-gnu
cargo test --locked
sh build-windows.sh
sh build-linux.sh
python3 package.py
```

Skrypty używają `$HOME/.cache/opendeck-masterhub-target`, chyba że ustawisz
`CARGO_TARGET_DIR`. Paczka to `dist/io.github.opendeck-masterhub.streamDeckPlugin`.
Zainstaluj ją przez **Plugins → Install from file** i ponownie uruchom OpenDeck.
Wersja manifestu powinna wynosić **0.6.3**.

Źródła: `dist/opendeck-masterhub-0.6.3-source.zip`. Archiwum zawiera assety, lockfile,
testy i obrazy referencyjne; nie zawiera `.git`, kompilacji ani binariów.

## Weryfikacja

33 testy obejmują wszystkie pozycje pokrętła, knobów i rollerów; wybór LCD; izolację
enkoderów; odświeżenia profilu; odpowiedź po 1500 ms; przerwanie starszego feedbacku;
reset layoutu bez procentu; serię 50% ↔ 55% przy wolnym USB bez klatek pośrednich;
ponawianie błędu HID; zastąpienie obrazu po oczekiwaniu na writer; obrót podczas
przywracania zegara; GIF i zegar przez wspólny worker; oraz odczyt `$B1` i zachowanie
tekstu.

Testy symulują błędy i opóźnienia. Rzeczywista przepustowość USB, odpowiedzi HA i
zachowanie fizycznego LCD wymagają sprawdzenia na urządzeniu.
