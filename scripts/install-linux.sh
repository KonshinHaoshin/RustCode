#!/bin/bash

set -e

INSTALL_DIR="${1:-$HOME/.rustcode}"
BIN_DIR="$INSTALL_DIR/bin"
SOURCE_DIR="$(cd "$(dirname "$0")/.." && pwd)"

echo "==========================================="
echo "RustCode - Linux/macOS Source Install"
echo "==========================================="
echo "Install dir: $INSTALL_DIR"
echo

if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is required. Install Rust from https://rustup.rs/"
    exit 1
fi

mkdir -p "$BIN_DIR"

cd "$SOURCE_DIR"
cargo build --release

cp "$SOURCE_DIR/target/release/rustcode" "$BIN_DIR/rustcode"
chmod +x "$BIN_DIR/rustcode"

echo
echo "Installed: $BIN_DIR/rustcode"
echo "Add to PATH if needed:"
echo "  export PATH=\"$BIN_DIR:\$PATH\""
echo
echo "Quick start:"
echo "  rustcode --help"
echo "  rustcode config set provider deepseek"
echo "  rustcode config set api_key \"your-api-key\""
