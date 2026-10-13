#!/bin/sh
# Buduje plik pluginu dla Linuksa (x86_64). Działa na Linuksie i w WSL.
# Wymaga Rusta (https://rustup.rs); nie potrzebuje bibliotek systemowych poza glibc.
# Wynik: dist/bin/opendeck-masterhub-linux (potem: python3 package.py)
set -e
cd "$(dirname "$0")"
# Na dysku Windowsa (WSL /mnt/...) kompilacja jest wolna – pliki pośrednie trzymamy w $HOME.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/opendeck-masterhub-target}"
cargo build --release --locked --bin opendeck-masterhub
mkdir -p dist/bin
cp "$CARGO_TARGET_DIR/release/opendeck-masterhub" dist/bin/opendeck-masterhub-linux
echo "Gotowe: dist/bin/opendeck-masterhub-linux"
