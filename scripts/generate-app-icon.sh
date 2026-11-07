#!/usr/bin/env bash
# Regenerates the macOS asset catalog and a standalone ICNS from the approved master.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MASTER="$ROOT/apple/Artwork/AppIcon.png"
APPICONSET="$ROOT/apple/Eddy/Assets.xcassets/AppIcon.appiconset"
EXPORT_DIR="$ROOT/build/icons"
ICONSET="$EXPORT_DIR/Eddy.iconset"

mkdir -p "$APPICONSET" "$ICONSET"

# Generate every size directly from the master, keeping its transparent padding.
for size in 16 32 128 256 512; do
  for scale in 1 2; do
    pixels=$((size * scale))
    suffix=""
    if [[ "$scale" == 2 ]]; then
      suffix="@2x"
    fi
    filename="icon_${size}x${size}${suffix}.png"
    sips --resampleHeightWidth "$pixels" "$pixels" "$MASTER" \
      --out "$APPICONSET/$filename" >/dev/null
    cp "$APPICONSET/$filename" "$ICONSET/$filename"
  done
done

{
  printf '{\n  "images": [\n'
  separator=""
  for size in 16 32 128 256 512; do
    for scale in 1 2; do
      suffix=""
      if [[ "$scale" == 2 ]]; then
        suffix="@2x"
      fi
      printf '%s    { "filename": "icon_%sx%s%s.png", "idiom": "mac", "scale": "%sx", "size": "%sx%s" }' \
        "$separator" "$size" "$size" "$suffix" "$scale" "$size" "$size"
      separator=$',\n'
    done
  done
  printf '\n  ],\n  "info": { "author": "xcode", "version": 1 }\n}\n'
} >"$APPICONSET/Contents.json"

iconutil --convert icns "$ICONSET" --output "$EXPORT_DIR/Eddy.icns"

echo "Generated macOS AppIcon assets in $APPICONSET"
echo "Exported $EXPORT_DIR/Eddy.icns"
