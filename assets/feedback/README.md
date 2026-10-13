# Wzorce procentu ze standardowego `$B1`

`percent-templates.png` to atlas 1080×3904, komórki 108×32 w 10 kolumnach.
Wygenerowano go przez `streamdeck-strip-render`:
https://github.com/FrostyCoolSlug/streamdeck-strip-render/tree/dd71a18a232e9bfdad9a32f294765c01b0d416aa
z układem `$B1` OpenDecka:
https://github.com/nekename/OpenDeck/blob/66db15d9d5d41597772a4f209cd6a93b498fc7df/static/encoder_layouts/B1.json

Wzorce używają oryginalnej czcionki Noto Sans z renderera: wielkość 24,
waga 600, wyrównanie do prawej. Pole pochodzi z `[76,40,108,32]`.

Indeksy 0–999 zawierają `0%`…`999%`. Tylko 0–100 mogą zostać przerysowane lokalnie.
Indeksy 1000–1009 zawierają `00%`…`09%`, 1010–1109: `000%`…`099%`,
1110–1210: `-0%`…`-100%`. Służą jako wzorce odrzucenia, aby nie zmieniać np.
`101%` na `100%` ani nie usuwać zer lub minusa. Pozostałe komórki są puste.

Rozpoznawanie porównuje cały napis, razem ze znakiem `%`, dopuszcza różnicę wymiarów
do 2 pikseli i wymaga małego błędu oraz przewagi nad następnym dopasowaniem.
Pasek jasności nie służy do rozpoznania liczby. W przypadku niepewności plugin
zachowuje oryginalny tekst.

Wygenerowanie ponownie (w osobnym checkoutcie renderera):

```sh
cp /sciezka/do/pluginu/tools/generate-percent-templates.rs examples/masterhub_percent_templates.rs
cargo run --locked --release --example masterhub_percent_templates -- \
  /sciezka/do/OpenDeck/static/encoder_layouts/B1.json \
  /sciezka/do/pluginu/assets/feedback/percent-templates.png
```

Atlas jest osadzony w binarium. Kompilacja pluginu nie wymaga źródeł renderera
ani pliku czcionki. `OFL.txt` zawiera licencję Noto, a `renderer-LICENSE.txt`
licencję renderera. Oba pliki zachowano wraz z pochodzeniem atlasu.
