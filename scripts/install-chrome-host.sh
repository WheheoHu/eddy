#!/usr/bin/env bash
# Registers eddy-capture-host with Chrome (and other installed Chromium
# browsers) for development builds. The app does the same from its settings.
#
# Usage: scripts/install-chrome-host.sh [path/to/eddy-capture-host]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="${1:-$ROOT/apple/DerivedData/Build/Products/Debug/Eddy.app/Contents/Helpers/eddy-capture-host}"
ID="$(grep -o 'CHROME_EXTENSION_ID: &str = "[a-p]*"' "$ROOT/crates/eddy-core/src/native_host.rs" | cut -d'"' -f2)"

if [[ ! -x "$HOST" ]]; then
  echo "capture host not found: $HOST (run scripts/build-mac-app.sh first)" >&2
  exit 1
fi

SUPPORT="$HOME/Library/Application Support"
for browser in "Google/Chrome" "Chromium" "Microsoft Edge" "BraveSoftware/Brave-Browser" "Arc/User Data"; do
  if [[ "$browser" != "Google/Chrome" && ! -d "$SUPPORT/$browser" ]]; then
    continue
  fi
  dir="$SUPPORT/$browser/NativeMessagingHosts"
  mkdir -p "$dir"
  cat >"$dir/dev.eddy.capture.json" <<EOF
{
  "name": "dev.eddy.capture",
  "description": "Eddy page capture",
  "path": "$HOST",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$ID/"]
}
EOF
  echo "installed for $browser"
done
