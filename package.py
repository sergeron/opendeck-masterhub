"""Buduje paczkę dist/io.github.opendeck-masterhub.streamDeckPlugin do instalacji w OpenDecku
(Plugins -> Install from file). Zawiera tylko pliki pluginu – bez logów i danych użytkownika.

Pliki programu dla poszczególnych systemów:
- Windows: dist/bin/opendeck-masterhub.exe         (build-windows.sh na Linuksie)
           albo target/release/opendeck-masterhub.exe (cargo build --release na Windows)
- Linux:   dist/bin/opendeck-masterhub-linux        (build-linux.sh w WSL albo na Linuksie)
- macOS:   dist/bin/opendeck-masterhub-mac          (macos/build.sh na Macu)
Brakujące są pomijane (z ostrzeżeniem) – plugin działa wtedy tylko na pozostałych systemach.

Użycie: python package.py
"""

import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).parent
FOLDER = "io.github.opendeck-masterhub.sdPlugin"

FILES = {
    "manifest.json": ROOT / "manifest.json",
    "README.md": ROOT / "README.md",
    "README.pl.md": ROOT / "README.pl.md",
    "docs/ha-lcd-feedback.md": ROOT / "docs" / "ha-lcd-feedback.md",
    "docs/ha-lcd-feedback-0.6.2.md": ROOT / "docs" / "ha-lcd-feedback-0.6.2.md",
    "docs/ha-lcd-feedback-0.6.3.md": ROOT / "docs" / "ha-lcd-feedback-0.6.3.md",
    "assets/feedback/README.md": ROOT / "assets" / "feedback" / "README.md",
    "licenses/feedback-OFL.txt": ROOT / "assets" / "feedback" / "OFL.txt",
    "licenses/feedback-renderer-MIT.txt": ROOT / "assets" / "feedback" / "renderer-LICENSE.txt",
    "licenses/jpeg-encoder-MIT.txt": ROOT / "assets" / "feedback" / "jpeg-encoder-LICENSE-MIT.txt",
    "icon.png": ROOT / "assets" / "icon.png",
    "icon@2x.png": ROOT / "assets" / "icon@2x.png",
    "70-masterhub.rules": ROOT / "linux" / "70-masterhub.rules",
}

BINARIES = {
    "opendeck-masterhub.exe": (
        ROOT / "dist" / "bin" / "opendeck-masterhub.exe",
        ROOT / "target" / "release" / "opendeck-masterhub.exe",
    ),
    "opendeck-masterhub-linux": (ROOT / "dist" / "bin" / "opendeck-masterhub-linux",),
    "opendeck-masterhub-mac": (ROOT / "dist" / "bin" / "opendeck-masterhub-mac",),
}


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    entries = dict(FILES)
    for directory, prefix in ((ROOT / "assets" / "icons", "icons"), (ROOT / "pi", "pi")):
        for f in sorted(directory.iterdir()):
            entries[f"{prefix}/{f.name}"] = f
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    out = dist / "io.github.opendeck-masterhub.streamDeckPlugin"
    systems = []
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, src in entries.items():
            z.write(src, f"{FOLDER}/{name}")
        for name, candidates in BINARIES.items():
            src = next((path for path in candidates if path.exists()), None)
            if src is None:
                print(f"UWAGA: brak {' lub '.join(map(str, candidates))} – paczka nie zadziała na tym systemie")
                continue
            info = zipfile.ZipInfo.from_file(src, f"{FOLDER}/{name}")
            info.compress_type = zipfile.ZIP_DEFLATED
            # Prawo do uruchamiania dla Linuksa i macOS (OpenDeck odtwarza je przy rozpakowaniu).
            info.external_attr = (0o100755 << 16)
            z.writestr(info, src.read_bytes())
            systems.append({"opendeck-masterhub.exe": "Windows", "opendeck-masterhub-linux": "Linux"}.get(name, "macOS"))
    print(f"{out} ({out.stat().st_size // 1024} kB) – systemy: {', '.join(systems)}")


if __name__ == "__main__":
    main()
