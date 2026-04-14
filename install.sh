#!/usr/bin/env bash

set -e

REPO="KonshinHaoshin/claude-code-rust"
INSTALL_PATH="${INSTALL_PATH:-$HOME/.local/bin}"

detect_os() {
    case "$OSTYPE" in
        linux-gnu*) OS="linux" ;;
        darwin*) OS="macos" ;;
        *) echo "Unsupported OS: $OSTYPE"; exit 1 ;;
    esac

    if [[ "$(uname -m)" == "arm64" || "$(uname -m)" == "aarch64" ]]; then
        ARCH="aarch64"
    else
        ARCH="x86_64"
    fi
}

detect_os
mkdir -p "$INSTALL_PATH"

VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | cut -d'"' -f4 || true)"
if [[ -z "$VERSION" ]]; then
    VERSION="v0.1.0"
fi

ASSET="rustcode-${OS}-${ARCH}"
TARGET="$INSTALL_PATH/rustcode"

echo "Downloading $ASSET from $REPO@$VERSION"
curl -fsSL "https://github.com/$REPO/releases/download/$VERSION/$ASSET" -o "$TARGET"
chmod +x "$TARGET"

echo
echo "Installed: $TARGET"
echo "Run: rustcode --help"
