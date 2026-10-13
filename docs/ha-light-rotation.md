# Stabilna zmiana jasności światła w Home Assistant

Przykłady używają identyfikatorów `light.example` i `script.masterhub_light_brightness`.
Zastąp je identyfikatorami lampy i skryptu ze swojej instancji Home Assistant.

Szablon `brightness_pct = state_attr(..., 'brightness') / 255 * 100 + 5 * ticks`
oblicza bezwzględny cel z aktualnie widocznego stanu HA. Gdy następne wywołanie
zaczyna się przed potwierdzeniem poprzedniego, odczyt może być nieaktualny.
Wywołania usługi w sprawdzonym pluginie HA nie czekają na zakończenie poprzedniego.
To możliwe wyjaśnienie zmiany „jaśniej, ciemniej, jaśniej” przy stałym kierunku obrotu.
Wyświetlanie feedbacku na LCD nie wysyła poleceń do lampy.

Rozwiązanie przenosi obliczenie jasności do jednego skryptu HA w trybie `queued`.
Każdy obrót przekazuje mu kroki, a skrypt odczytuje jasność dopiero wtedy, gdy
poprzednia zmiana została potwierdzona stanem encji. Ustawienie `transition: 0`
wyłącza przejście czasowe, jeśli integracja lampy je obsługuje.

## Instalacja skryptu

1. W HA otwórz **Ustawienia → Automatyzacje i sceny → Skrypty**, utwórz nowy skrypt
   i wybierz edycję w YAML.
2. Wklej cały plik
   [`masterhub_light_brightness.yaml`](../examples/home-assistant/masterhub_light_brightness.yaml).
   W `variables.light_entity` zamień `light.example` na identyfikator swojej lampy
   i zapisz skrypt. W ustawieniach encji nadaj mu identyfikator
   `script.masterhub_light_brightness` albo użyj faktycznie utworzonego identyfikatora
   w następnym kroku.

Dla konfiguracji plikowej zawartość YAML należy umieścić pod kluczem
`masterhub_light_brightness:` w `scripts.yaml`, z dodatkowym wcięciem dwóch spacji,
a następnie przeładować skrypty. Plik przykładowy ma format edytora pojedynczego
skryptu, nie całego `scripts.yaml`.

## Akcja Home Assistant w OpenDeck

Na zakładce wyświetlania wybierz **encję swojej lampy** (w przykładzie `light.example`)
i feedback **`$B1`**.
W ustawieniach **Rotation / Obrót** ustaw:

- Domain: **`script`**.
- Service: **`turn_on`** (czyli `script.turn_on` razem z domeną).
- Tick multiplier: **`1`**.
- Tick bucket size: **`100 ms`**.
- Service Data JSON:

```jinja
{
  "entity_id": "script.masterhub_light_brightness",
  "variables": {
    "ticks": {{ ticks | int }}
  }
}
```

Identyfikator skryptu jest wpisany w `entity_id` w Service Data JSON. Ten wariant
działa również wtedy, gdy plugin HA nie pokazuje listy Entity dla `script.turn_on`.
Jeśli skrypt ma inny identyfikator w HA, zastąp wartość `entity_id` jego faktycznym
identyfikatorem. Na zakładce Appearance nadal wyświetlana jest encja lampy.

Zapisz zmianę dla wszystkich enkoderów, które sterują tą samą lampą. Powinny
wywoływać ten sam skrypt, żeby korzystać z jednej kolejki. Dla kolejnej lampy
utwórz kopię skryptu z jej identyfikatorem w `variables.light_entity` i wpisz
identyfikator tej kopii w Service Data JSON przypisanego jej enkodera.

Wybrano jawne 100 ms, aby łączyć szybkie impulsy bez wywołania usługi na każdy
raport HID. W sprawdzonym kodzie pluginu HA `0` dodatkowo zastępuje się domyślnym
`300` przez `rotationTickBucketSizeMs || 300`; zachowanie może zależeć od wersji
zainstalowanego pluginu. Można później sprawdzić 50 ms z tą samą kolejką.

## Zachowanie i weryfikacja

Jeden krok daje zmianę o 5 punktów procentowych; paczka trzech kroków daje 15.
Przykładowy skrypt ogranicza jasność do zakresu 1–100%.
Procent na LCD nadal pochodzi z potwierdzonego stanu lampy. Zaokrąglenie jasności
do skali 0–255 może zmienić wyświetlany tekst o punkt procentowy.

Skrypt czeka najwyżej 3 sekundy na stan `on` i jasność w odległości do 2 jednostek
skali 0–255 od celu. Brak potwierdzenia anuluje kolejkę tego skryptu, zamiast
liczyć następny cel ze starego stanu. Kolejka mieści 100 wywołań. Przy wolnym
urządzeniu może być wyczuwalne oczekiwanie na wcześniejsze polecenia.

Po instalacji sprawdź szybki obrót w każdym kierunku i ślad wykonania skryptu w HA.
Przy obrocie w jednym kierunku kolejne cele powinny być monotoniczne. Jeśli kończy
się oczekiwanie po 3 sekundach, sprawdź sposób raportowania jasności przez integrację
lampy. Potwierdzenie w HA może być optymistyczne; nie mierzy fizycznej jasności żarówki.

Lokalna weryfikacja obejmuje składnię YAML/Jinja, ograniczenia jasności, zachowanie
kroku, oczekiwanie na właściwy cel i symulację opóźnionych odpowiedzi. Działanie
z konkretną integracją lampy należy sprawdzić we własnej instancji HA.

Źródła: [tryby skryptów HA](https://www.home-assistant.io/integrations/script/#configuration-variables),
[przekazywanie variables przez script.turn_on](https://www.home-assistant.io/integrations/script/#passing-values-to-a-script-in-yaml),
[oczekiwanie na szablon](https://www.home-assistant.io/docs/scripts/#wait-for-a-template-in-yaml),
[wywołania obrotu w pluginie HA](https://github.com/cgiesche/streamdeck-homeassistant/blob/7322fe0eac5279b3a09dcd0f4026c7b43df0636c/src/components/PluginComponent.vue#L130).
