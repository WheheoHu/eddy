#!/usr/bin/env bash
# Builds the Rust core, regenerates the Xcode project and builds Eddy.app.
#
# Usage: scripts/build-mac-app.sh [debug|release]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
source "$ROOT/scripts/env.sh"

"$ROOT/scripts/build-apple.sh" "$PROFILE"

cd "$ROOT/apple"
xcodegen generate --quiet

CONFIGURATION=Debug
if [[ "$PROFILE" == "release" ]]; then
  CONFIGURATION=Release
fi

mkdir -p "$ROOT/build"
LOG="$ROOT/build/xcodebuild.log"
if ! xcodebuild -project Eddy.xcodeproj -scheme Eddy -configuration "$CONFIGURATION" \
  -derivedDataPath "$ROOT/apple/DerivedData" -destination 'platform=macOS,arch=arm64' \
  build >"$LOG" 2>&1; then
  grep -E "error:" "$LOG" | sort -u >&2 || true
  echo "Build failed, full log: $LOG" >&2
  exit 1
fi
grep -E "warning: .*/apple/Eddy/" "$LOG" | sort -u || true

APP="$ROOT/apple/DerivedData/Build/Products/$CONFIGURATION/Eddy.app"
echo "Built $APP"
