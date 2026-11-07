#!/usr/bin/env bash
# Builds the Rust core, regenerates the Xcode project and builds Eddy.app.
#
# Usage: scripts/build-mac-app.sh [debug|release]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-debug}"
SCHEME="${SCHEME:-Eddy}"
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
if ! xcodebuild -project Eddy.xcodeproj -scheme "$SCHEME" -configuration "$CONFIGURATION" \
  -derivedDataPath "$ROOT/apple/DerivedData" -destination 'platform=macOS,arch=arm64' \
  build >"$LOG" 2>&1; then
  grep -E "error:" "$LOG" | sort -u >&2 || true
  echo "Build failed, full log: $LOG" >&2
  exit 1
fi
grep -E "warning: .*/apple/Eddy/" "$LOG" | sort -u || true

# Incremental builds can leave the bundle directory's mtime unchanged even when
# its icon resources change. Notify Finder and Dock so they drop the cached icon.
touch "$ROOT/apple/DerivedData/Build/Products/$CONFIGURATION/$SCHEME.app"

echo "Built $SCHEME ($CONFIGURATION) in $ROOT/apple/DerivedData/Build/Products/$CONFIGURATION"
