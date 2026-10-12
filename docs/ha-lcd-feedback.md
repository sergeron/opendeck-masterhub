# Feedback Home Assistant na LCD MasterHUB — 0.6.4

Procent standardowego `$B1` jest rysowany lokalnie w 240×240 tą samą czcionką,
co głośność. Obie ścieżki wysyłają JPEG; LCD nie otrzymuje osobnej komendy tekstowej.
Wcześniej procent HA był powiększany z wycinka małego JPEG 200×100. To powodowało
pikselizację i utratę ostrości.

**Liczba pozostaje wyłącznie potwierdzonym procentem z HA.** Obrót pokazuje ostatni
potwierdzony stan, a nowa liczba pojawia się po odebraniu nowego feedbacku. Nie ma
przewidywania ±5% ani obliczania tekstu z przybliżonej długości paska.

[Historia 0.6.3](ha-lcd-feedback-0.6.3.md) · [Historia 0.6.2](ha-lcd-feedback-0.6.2.md)

## Ostrość i rozpoznawanie wartości

Dla standardowego białego `$B1` plugin rozpoznaje cały napis `0%`…`100%` z pola
wartości `[76,40,108,32]`. Wzorce pochodzą z tego samego renderera Noto Sans, który
używa OpenDeck. Porównanie obejmuje znak `%`, wymiary i maskę całego napisu.
Wymagany jest mały błąd oraz jednoznaczna przewaga nad następnym dopasowaniem.

Wzorce spoza zakresu, ujemne oraz z zerami wiodącymi są klasami odrzucenia.
Niepewne i niestandardowe wartości zachowują oryginalną maskę. To ograniczone
rozpoznawanie znanego zapisu procentu, bez modelu OCR ani usługi sieciowej.
Po rozpoznaniu liczba jest rysowana czcionką systemową (Windows: Segoe UI/Arial;
Linux: DejaVu/Noto/Liberation). Tytuł jest zachowany jako obraz bez powiększania.
Brak czcionki systemowej również powoduje zachowanie oryginalnego tekstu.

Puste pole wartości po `setFeedbackLayout` nadal jest pomijane do nadejścia
właściwego `setFeedback`. Inne układy są dopasowywane z zachowaniem proporcji.
Pochodzenie, licencje i generator atlasu: [assets/feedback](../assets/feedback/README.md).

## Kolejka i zapis ekranu

Jeden dekoder przygotowuje najnowszy feedback każdego enkodera niezależnie od
zajętego HID. Obracany enkoder ma pierwszeństwo przed odświeżeniami innych profili.
Gotowy JPEG jest wspólny dla docelowych LCD i pozostaje w pamięci. Obrót oraz
ponowienie błędu HID nie dekodują ponownie tego samego obrazu.

Jeden worker na LCD nadal wybiera między feedbackiem, zegarem, klawiszem i GIF.
Rewizja jest sprawdzana po przejęciu blokady HID. Wyczyszczenie lub zmiana profilu
unieważnia również trwającą konwersję; stary wynik nie wraca do nowego przypisania.
Nie tworzy się kolejka nieaktualnych procentów. Identyczny wynik wizualny nie jest
ponownie wysyłany, nawet jeśli JPEG wejściowy lub niewyświetlana ikona się zmieniły.

Usunięto dodatkowe oczekiwanie 8 ms przed zapisem. Maksymalna częstotliwość początku
zapisów pozostaje ograniczona odstępem 33 ms. Geometria pierścienia jest obliczana
raz i ponownie wykorzystywana. Nieudana klatka jest ponawiana po 100 ms.

Feedback używa **baseline JPEG 4:4:4, jakość 80, optymalizowane tablice Huffmana**.
Dodano jedną zależność `jpeg-encoder` 0.7.1 w czystym Rust, bez bibliotek systemowych.
Nie używa się progresywnego JPEG ani kodowania 4:2:0. Głośność i zegar zachowują
swój dotychczasowy encoder jakości 95.

## Moduły i konfiguracja

Pokrętło LCD używa własnego ekranu. Każdy knob, roller lub fader korzysta z
podłączonych LCD. Pamięć jest osobna dla pary urządzenie + pozycja, a ostatnio
obracany enkoder wybiera źródło wspólnego ekranu. Odświeżenie bez obrotu nie otwiera
nakładki. Po 1,5 s wraca zegar; na oczekiwaną odpowiedź plugin czeka do 5 s.

Zachowaj akcję HA z krokiem ±5%, feedback `$B1` i **Tick bucket size = 50 ms**.
Zegar przypisz do klawisza LCD. Kod OpenDecka oraz pluginu HA nie jest zmieniany.

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

## Jak sprawdzić opóźnienie

Po ponownym uruchomieniu OpenDecka `masterhub.log` powinien zawierać
`Plugin MasterHUB 0.6.4 gotowy`. Pozwala to sprawdzić również wersję faktycznie
uruchomionego EXE, a nie tylko manifestu.

Na początku interakcji pojawiają się dwa pomiary:

- `feedback po ... ms od obrotu; przygotowanie ... ms; JPEG ... B / ... raportów`:
  czas od ostatniego obrotu do otrzymania obrazu z OpenDecka oraz lokalna konwersja;
- `LCD kolejka ... ms; blokada HID ... ms; zapis ... ms`: oczekiwanie gotowej klatki,
  przejęcie writera i faktyczny czas wysyłania obrazu.

Pierwszy pomiar obejmuje plugin HA, komunikację ze światłem oraz renderowanie
OpenDecka. Bez identyfikatora żądania nie jest dokładnym pomiarem jednej usługi HA.
Lokalna poprawka nie usuwa tego oczekiwania: zgodnie z wybranym zachowaniem ekran
pokazuje wyłącznie potwierdzone dane. Jeśli odpowiedź nadal przychodzi wolno, log
pozwala wskazać, czy problem jest przed MasterHUBem, w przygotowaniu czy w USB.

## Budowanie i instalacja

Rust/Cargo **1.89+**, sprawdzone z 1.92.0 na Debianie 12 i MinGW-w64 GCC 12.2.

```sh
rustup target add x86_64-pc-windows-gnu
cargo test --locked
sh build-windows.sh
sh build-linux.sh
python3 package.py
```

Zainstaluj `dist/io.github.opendeck-masterhub-0.6.4.streamDeckPlugin` przez
**Plugins → Install from file**, następnie uruchom ponownie OpenDeck.
Źródła: `dist/opendeck-masterhub-0.6.4-source.zip`.

Podgląd faktycznej kompresji feedbacku, bez urządzenia:

```sh
cargo run --locked --release --example feedback_preview -- \
  tests/fixtures/encoder-b1/b1-55.jpg dist/feedback-55-0.6.4.png
```

## Weryfikacja i ograniczenia

43 testy obejmują między innymi wszystkie 0–100% przy JPEG 75/95, zachowanie liczby
przy innym pasku, odrzucenie wartości ujemnych/spoza zakresu/zer wiodących,
przygotowanie podczas blokady HID, wybór najnowszej klatki, brak przewidywania
po obrocie, deduplikację wyglądu, usunięcie starego wyniku po zmianie profilu,
obsługę knobów/rollerów oraz poprawne dekodowanie zoptymalizowanego JPEG.

Próbka 55% w tym środowisku: **8836 → 8245 B**, czyli **29 → 27 raportów HID**.
To pomiar wielkości klatki, nie obietnica takiego samego zysku czasu USB.
Ostrość sprawdzono na gotowych obrazach; rzeczywista latencja HA, USB i fizycznego
LCD nadal wymaga pomiaru na urządzeniu.
