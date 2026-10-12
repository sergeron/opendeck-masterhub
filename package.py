"""Buduje paczki pluginu do instalacji w OpenDecku (Plugins -> Install from file) albo instaluje
plugin bezpośrednio. Paczki zawierają tylko pliki pluginu – bez logów i danych użytkownika.

Wersje językowe różnią się nazwami akcji na liście OpenDecka (manifest) i domyślnym językiem
paneli / napisów (language.txt); obie mają oba języki do wyboru w ustawieniach akcji:
- en: manifest.json bez zmian,
- pl: nazwy i opisy z pl.json wpisane do manifestu.

Pliki programu dla poszczególnych systemów:
- Windows: dist/bin/opendeck-masterhub.exe         (build-windows.sh na Linuksie)
           albo target/release/opendeck-masterhub.exe (cargo build --release na Windows)
- Linux:   dist/bin/opendeck-masterhub-linux        (build-linux.sh w WSL albo na Linuksie)
- macOS:   dist/bin/opendeck-masterhub-mac          (macos/build.sh na Macu)
Brakujące są pomijane (z ostrzeżeniem) – plugin działa wtedy tylko na pozostałych systemach.

Użycie:
  python package.py                  # obie paczki: dist/io.github.opendeck-masterhub-{en,pl}.streamDeckPlugin
  python package.py --lang pl        # tylko polska
  python package.py --install --lang pl   # instaluje do OpenDecka (zamknij najpierw OpenDecka)
"""

import argparse
import json
import os
import shutil
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).parent
FOLDER = "io.github.opendeck-masterhub.sdPlugin"
LANGS = ("en", "pl")

FILES = {
    "README.md": ROOT / "README.md",
    "pl.json": ROOT / "pl.json",
    "icon.png": ROOT / "assets" / "icon.png",
    "icon@2x.png": ROOT / "assets" / "icon@2x.png",
    "70-masterhub.rules": ROOT / "linux" / "70-masterhub.rules",
    # Wzorce cyfr feedbacku enkoderów (osadzone w programie) – licencje ich źródeł.
    "licenses/feedback-OFL.txt": ROOT / "assets" / "feedback" / "OFL.txt",
    "licenses/feedback-renderer-MIT.txt": ROOT / "assets" / "feedback" / "renderer-LICENSE.txt",
    "licenses/jpeg-encoder-MIT.txt": ROOT / "assets" / "feedback" / "jpeg-encoder-LICENSE-MIT.txt",
}

# Plik z build-windows.sh (budowanie na Linuksie) ma pierwszeństwo przed natywnym.
WINDOWS_EXE = ROOT / "dist" / "bin" / "opendeck-masterhub.exe"
BINARIES = {
    "opendeck-masterhub.exe": WINDOWS_EXE if WINDOWS_EXE.exists() else ROOT / "target" / "release" / "opendeck-masterhub.exe",
    "opendeck-masterhub-linux": ROOT / "dist" / "bin" / "opendeck-masterhub-linux",
    "opendeck-masterhub-mac": ROOT / "dist" / "bin" / "opendeck-masterhub-mac",
}
SYSTEMS = {"opendeck-masterhub.exe": "Windows", "opendeck-masterhub-linux": "Linux", "opendeck-masterhub-mac": "macOS"}


def manifest(lang):
    m = json.loads((ROOT / "manifest.json").read_text(encoding="utf-8"))
    if lang == "pl":
        pl = json.loads((ROOT / "pl.json").read_text(encoding="utf-8"))
        for key in ("Name", "Description"):
            m[key] = pl.get(key, m[key])
        for action in m["Actions"]:
            texts = pl.get(action["UUID"], {})
            for key in ("Name", "Tooltip"):
                action[key] = texts.get(key, action[key])
    return json.dumps(m, indent=2, ensure_ascii=False) + "\n"


def plugin_files(lang):
    """Pliki pluginu: nazwa w folderze pluginu -> (zawartość, czy plik programu)."""
    files = {name: (src.read_bytes(), False) for name, src in FILES.items()}
    files["manifest.json"] = (manifest(lang).encode("utf-8"), False)
    files["language.txt"] = (f"{lang}\n".encode(), False)
    for directory, prefix in ((ROOT / "assets" / "icons", "icons"), (ROOT / "pi", "pi")):
        for f in sorted(directory.iterdir()):
            files[f"{prefix}/{f.name}"] = (f.read_bytes(), False)
    for name, src in BINARIES.items():
        if src.exists():
            files[name] = (src.read_bytes(), True)
        else:
            print(f"UWAGA: brak {src} – plugin nie zadziała na systemie {SYSTEMS[name]}")
    return files


def package(lang):
    files = plugin_files(lang)
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    out = dist / f"io.github.opendeck-masterhub-{lang}.streamDeckPlugin"
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, (data, executable) in files.items():
            info = zipfile.ZipInfo(f"{FOLDER}/{name}", date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            # Prawo do uruchamiania dla Linuksa i macOS (OpenDeck odtwarza je przy rozpakowaniu).
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            z.writestr(info, data)
    systems = [SYSTEMS[n] for n in BINARIES if n in files]
    print(f"{out} ({out.stat().st_size // 1024} kB) – systemy: {', '.join(systems)}")


def plugins_dir():
    if sys.platform == "win32":
        return Path(os.environ["APPDATA"]) / "opendeck" / "plugins"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "opendeck" / "plugins"
    return Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "opendeck" / "plugins"


def install(lang):
    dest = plugins_dir() / FOLDER
    dest.mkdir(parents=True, exist_ok=True)
    # Stare panele i ikony mogły zostać po poprzedniej wersji – te katalogi wgrywamy od nowa.
    for sub in ("pi", "icons"):
        shutil.rmtree(dest / sub, ignore_errors=True)
    for name, (data, executable) in plugin_files(lang).items():
        path = dest / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        if executable and sys.platform != "win32":
            path.chmod(0o755)
    print(f"Zainstalowano ({lang}) w {dest}")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="Paczki / instalacja pluginu MasterHUB")
    parser.add_argument("--lang", choices=LANGS + ("all",), default="all")
    parser.add_argument("--install", action="store_true", help="zainstaluj do OpenDecka zamiast pakować")
    args = parser.parse_args()
    langs = LANGS if args.lang == "all" else (args.lang,)
    if args.install:
        if len(langs) != 1:
            parser.error("--install wymaga --lang en albo --lang pl")
        install(langs[0])
    else:
        for lang in langs:
            package(lang)


if __name__ == "__main__":
    main()
