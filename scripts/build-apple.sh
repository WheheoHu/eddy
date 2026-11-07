#!/usr/bin/env bash
# Builds the Rust core for Apple platforms, generates Swift bindings and
# packages EddyFFI.xcframework for apple/EddyKit.
#
# Usage: scripts/build-apple.sh [debug|release]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
source "$ROOT/scripts/env.sh"

cd "$ROOT"
CARGO_FLAGS=()
if [[ "$PROFILE" == "release" ]]; then
  CARGO_FLAGS+=(--release)
fi

TARGET=aarch64-apple-darwin
export MACOSX_DEPLOYMENT_TARGET=15.0

cargo build -p eddy-ffi -p eddy-capture-host --target "$TARGET" ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}
OUT="target/$TARGET/$PROFILE"

GEN="build/uniffi"
rm -rf "$GEN" && mkdir -p "$GEN"
cargo run -q -p uniffi-bindgen -- generate \
  --library "$OUT/libeddy_ffi.dylib" \
  --language swift \
  --out-dir "$GEN"

HEADERS="build/xcframework-headers"
rm -rf "$HEADERS" && mkdir -p "$HEADERS"
cp "$GEN"/EddyFFI.h "$HEADERS/"
cp "$GEN"/EddyFFI.modulemap "$HEADERS/module.modulemap"

KIT="apple/EddyKit"
rm -rf "$KIT/EddyFFI.xcframework"
xcodebuild -create-xcframework \
  -library "$OUT/libeddy_ffi.a" -headers "$HEADERS" \
  -output "$KIT/EddyFFI.xcframework" >/dev/null

mkdir -p "$KIT/Sources/EddyKit/Generated"
rm -f "$KIT/Sources/EddyKit/Generated/"*.swift
cp "$GEN"/*.swift "$KIT/Sources/EddyKit/Generated/"

mkdir -p build/helpers
cp "$OUT/eddy-capture-host" build/helpers/

echo "EddyFFI.xcframework ($PROFILE) ready at $KIT"
