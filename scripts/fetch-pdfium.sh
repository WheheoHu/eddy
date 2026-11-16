#!/usr/bin/env bash
# Downloads the pdfium library that eddy-pdf uses.
#
# Prebuilt by https://github.com/paulocoutinhox/pdfium-lib (pdfium is
# BSD-3-Clause). The archive is pinned by release tag and sha256 and unpacked
# under build/pdfium/<platform>/. On macOS eddy-pdf links the static
# libpdfium.a: .cargo/config.toml points PDFIUM_STATIC_LIB_PATH_* at the
# unpacked lib directory, so every cargo command finds it without extra
# flags. On Windows eddy-pdf loads pdfium.dll from next to the executable at
# runtime: copy build/pdfium/windows/release/lib/x64/pdfium.dll there (the
# import library in the archive is not needed).
#
# Usage: scripts/fetch-pdfium.sh [macos|windows]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLATFORM="${1:-macos}"
RELEASE="8046d"

case "$PLATFORM" in
  macos) SHA256="1f718f8aeee279c8021de5d61ea9a604613fbfa2d83d79172d9086d711778554" ;;
  windows) SHA256="14d4a3e2bb90b6346877a26aeaac412401f16c5210d890f4f5819ec82903cc56" ;;
  *) echo "unknown platform: $PLATFORM" >&2; exit 1 ;;
esac

DEST="$ROOT/build/pdfium/$PLATFORM"
case "$PLATFORM" in
  macos) LIB="$DEST/release/lib/libpdfium.a" ;;
  windows) LIB="$DEST/release/lib/x64/pdfium.dll" ;;
esac
STAMP="$DEST/.release"

if [[ -f "$LIB" && -f "$STAMP" && "$(cat "$STAMP")" == "$RELEASE" ]]; then
  echo "pdfium $RELEASE ($PLATFORM) already at $DEST"
  exit 0
fi

mkdir -p "$ROOT/build/pdfium"
ZIP="$ROOT/build/pdfium/$PLATFORM-$RELEASE.zip"
URL="https://github.com/paulocoutinhox/pdfium-lib/releases/download/$RELEASE/$PLATFORM.zip"
if [[ ! -f "$ZIP" ]] || ! echo "$SHA256  $ZIP" | shasum -a 256 --check --status; then
  echo "downloading $URL"
  curl -fL --retry 3 -o "$ZIP.part" "$URL"
  mv "$ZIP.part" "$ZIP"
fi
echo "$SHA256  $ZIP" | shasum -a 256 --check --status || {
  echo "checksum mismatch for $ZIP" >&2
  rm -f "$ZIP"
  exit 1
}

rm -rf "$DEST"
mkdir -p "$DEST"
unzip -q "$ZIP" -d "$DEST"
echo "$RELEASE" >"$STAMP"
rm -f "$ZIP"
echo "pdfium $RELEASE ($PLATFORM) ready at $DEST"
