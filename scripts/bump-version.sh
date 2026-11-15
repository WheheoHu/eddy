#!/usr/bin/env bash
# Sets the version of the core or of the Mac app and turns the changelog's
# "Unreleased" section into that release. They are versioned separately
# (docs/adr/0009): a UI-only change releases only the app.
#
#   scripts/bump-version.sh core 0.3.0   # Cargo workspace: engine, server, CLI
#   scripts/bump-version.sh mac 0.3.0    # Mac app; the build number goes up by one
#   scripts/bump-version.sh              # show both
#
# The changelogs follow https://keepachangelog.com/en/1.1.0/: entries are
# written under "## [Unreleased]" as changes are made; this script dates
# them as "## [X.Y.Z] - YYYY-MM-DD", starts an empty "Unreleased" and
# updates the links at the bottom. Run it when releasing, commit, merge to
# main, then tag that commit core-vX.Y.Z or mac-vX.Y.Z: pushing the tag
# runs .github/workflows/release.yml.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="https://github.com/WheheoHu/eddy"

core_version() { sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml; }
mac_version() { sed -n 's/^ *MARKETING_VERSION: "\(.*\)"/\1/p' apple/project.yml; }
mac_build() { sed -n 's/^ *CURRENT_PROJECT_VERSION: "\(.*\)"/\1/p' apple/project.yml; }

if [[ $# -eq 0 ]]; then
  echo "core $(core_version)"
  echo "mac $(mac_version) ($(mac_build))"
  exit 0
fi

target="${1:-}"
version="${2:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/bump-version.sh core|mac X.Y.Z" >&2
  exit 2
fi

# Moves what is under "## [Unreleased]" into "## [X.Y.Z] - date" and
# rewrites the link definitions: Unreleased compares from the new tag, the
# new version from the previous one (or points at its tag when it is the
# first).
release_changelog() {
  local file="$1" prefix="$2" note="$3"
  if grep -q "^## \[$version\]" "$file"; then
    echo "$file already has a section for $version"
    return
  fi
  local previous
  previous="$(sed -n 's/^## \[\([0-9][^]]*\)\].*/\1/p' "$file" | head -n 1)"
  awk -v v="$version" -v date="$(date +%Y-%m-%d)" -v note="$note" \
      -v repo="$REPO" -v prefix="$prefix" -v prev="$previous" '
    /^## \[Unreleased\]/ {
      print "## [Unreleased]\n\n## [" v "] - " date
      if (note != "") print "\n" note
      next
    }
    /^\[Unreleased\]: / {
      print "[Unreleased]: " repo "/compare/" prefix "-v" v "...HEAD"
      if (prev != "") print "[" v "]: " repo "/compare/" prefix "-v" prev "..." prefix "-v" v
      else print "[" v "]: " repo "/releases/tag/" prefix "-v" v
      next
    }
    { print }
  ' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
}

case "$target" in
  core)
    old="$(core_version)"
    sed -i '' "/^\[workspace.package\]/,/^\[/s/^version = \".*\"/version = \"$version\"/" Cargo.toml
    # Cargo.lock records the workspace crates' versions.
    cargo metadata --format-version 1 >/dev/null
    release_changelog CHANGELOG.md core ""
    echo "core $old -> $version"
    echo "If the sync protocol changed, PROTOCOL_VERSION in crates/eddy-sync/src/lib.rs must have been raised."
    ;;
  mac)
    old="$(mac_version)"
    build="$(mac_build)"
    if [[ "$old" != "$version" ]]; then
      build=$(( build + 1 ))
    fi
    sed -i '' "s/^\( *MARKETING_VERSION: \)\".*\"/\1\"$version\"/; s/^\( *CURRENT_PROJECT_VERSION: \)\".*\"/\1\"$build\"/" apple/project.yml
    release_changelog apple/CHANGELOG.md mac "Built with core $(core_version)."
    echo "mac $old -> $version (build $build)"
    ;;
  *)
    echo "usage: scripts/bump-version.sh core|mac X.Y.Z" >&2
    exit 2
    ;;
esac
