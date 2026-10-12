#!/bin/sh
# Buduje plugin dla Windows x64 na Linuksie przy użyciu MinGW-w64.
# Wymaga: rustup target add x86_64-pc-windows-gnu oraz x86_64-w64-mingw32-gcc.
# Wynik: dist/bin/opendeck-masterhub.exe (potem: python3 package.py).
set -e
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/opendeck-masterhub-target}"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
if ! command -v "$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER" >/dev/null 2>&1; then
    echo "Brak MinGW-w64: zainstaluj gcc-mingw-w64-x86-64 albo ustaw CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER." >&2
    exit 1
fi
cargo build --release --locked --target x86_64-pc-windows-gnu --bin opendeck-masterhub
mkdir -p dist/bin
cp "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/opendeck-masterhub.exe" dist/bin/opendeck-masterhub.exe
echo "Gotowe: dist/bin/opendeck-masterhub.exe"
