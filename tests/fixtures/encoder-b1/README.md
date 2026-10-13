Te obrazy JPEG 200×100 generuje renderer używany przez OpenDeck:

- układ `$B1`: https://github.com/nekename/OpenDeck/blob/66db15d9d5d41597772a4f209cd6a93b498fc7df/static/encoder_layouts/B1.json
- renderer: https://github.com/FrostyCoolSlug/streamdeck-strip-render/tree/dd71a18a232e9bfdad9a32f294765c01b0d416aa

Generator wywołuje `streamdeck_strip_render::render_to_image` na tym układzie,
ustawiając przykładowy tytuł encji, `value = "<procent>%"` oraz `indicator.value = procent`.
Ikona w [16,40,48,48] jest żółtym kołem (RGB 255,210,50, promień 14, środek 24,20).
Obraz jest zapisywany przez `DynamicImage::write_to(..., ImageFormat::Jpeg)`, tak samo
jak feedback wysyłany do pluginu urządzenia w OpenDecku.

Testy obejmują rozpoznanie paska po kompresji JPEG, 0%/100%, wartości tuż przy końcach,
zachowanie dokładnego tekstu wartości i pozostawienie niestandardowych układów.
