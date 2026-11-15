#!/usr/bin/env bash
# Copies Eddy's data directory next to itself before something that changes
# it for good, like pairing the real installation with a server.
#
#   scripts/backup-app-data.sh            # ~/Library/Application Support/Eddy
#   scripts/backup-app-data.sh DIR        # another data directory
#
# Quit Eddy first: the database is copied as files. Secrets are in the
# keychain, not in this directory, and are not part of the copy.
# To go back: quit Eddy, move the data directory away, and rename the backup
# to its name.
set -euo pipefail

src="${1:-$HOME/Library/Application Support/Eddy}"
if [[ ! -d "$src" ]]; then
  echo "no data directory at $src" >&2
  exit 1
fi
if pgrep -x Eddy >/dev/null 2>&1; then
  echo "Eddy is running. Quit it first, so the copy is consistent." >&2
  exit 1
fi
dest="${src%/}.backup-$(date +%Y%m%d-%H%M%S)"
# Clones on APFS: instant, and takes no extra space until either side changes.
cp -cR "$src" "$dest" 2>/dev/null || cp -R "$src" "$dest"
echo "backed up to $dest ($(du -sh "$dest" | cut -f1))"
