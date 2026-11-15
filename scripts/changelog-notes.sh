#!/usr/bin/env bash
# Prints the changelog section of one released version, as release notes.
#
#   scripts/changelog-notes.sh core 0.2.0   # from CHANGELOG.md
#   scripts/changelog-notes.sh mac 0.2.0    # from apple/CHANGELOG.md
#
# Fails when the version has no section yet (it is still under
# "Unreleased": run scripts/bump-version.sh first) or the section is empty.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
  core) file=CHANGELOG.md ;;
  mac) file=apple/CHANGELOG.md ;;
  *) echo "usage: scripts/changelog-notes.sh core|mac X.Y.Z" >&2; exit 2 ;;
esac
version="${2:?usage: scripts/changelog-notes.sh core|mac X.Y.Z}"

notes="$(awk -v v="$version" '
  /^## \[/ { inside = (index($0, "## [" v "]") == 1); next }
  /^\[[^]]+\]: / { inside = 0 }
  inside { print }
' "$file" | sed -e '/./,$!d')"
if [[ -z "${notes//[[:space:]]/}" ]]; then
  echo "$file has no notes for $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
