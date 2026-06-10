#!/data/data/com.termux/files/usr/bin/bash
# 3va installer for Termux (Android aarch64)
# Usage: curl -fsSL https://raw.githubusercontent.com/OdinoCano/3va/main/dist/termux/install.sh | bash

set -euo pipefail

VERSION="2.0.0"
REPO="OdinoCano/3va"
ARCHIVE="3va-v${VERSION}-aarch64-linux-android.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE}"
SHA256_URL="${URL}.sha256"
BIN_DIR="${PREFIX}/bin"

# Termux uses /data/data/com.termux/files/usr as prefix — $PREFIX is set by Termux.
if [ -z "${PREFIX:-}" ]; then
  echo "ERROR: \$PREFIX is not set. Run this script inside Termux." >&2
  exit 1
fi

arch=$(uname -m)
if [ "$arch" != "aarch64" ]; then
  echo "ERROR: 3va only provides a prebuilt binary for aarch64. Your arch is: $arch" >&2
  echo "       For other architectures, build from source: cargo install vvva_cli" >&2
  exit 1
fi

echo "[3va] Downloading v${VERSION} for aarch64-linux-android..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL"        -o "$TMP/$ARCHIVE"
curl -fsSL "$SHA256_URL" -o "$TMP/${ARCHIVE}.sha256"

echo "[3va] Verifying SHA256..."
EXPECTED=$(awk '{print $1}' "$TMP/${ARCHIVE}.sha256")
ACTUAL=$(sha256sum "$TMP/$ARCHIVE" | awk '{print $1}')
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "ERROR: SHA256 mismatch." >&2
  echo "  expected: $EXPECTED" >&2
  echo "  got:      $ACTUAL" >&2
  exit 1
fi
echo "[3va] SHA256 OK."

echo "[3va] Extracting..."
tar xzf "$TMP/$ARCHIVE" -C "$TMP"
install -Dm755 "$TMP/3va" "$BIN_DIR/3va"

echo "[3va] Installed to $BIN_DIR/3va"
echo "[3va] Run: 3va --version"
