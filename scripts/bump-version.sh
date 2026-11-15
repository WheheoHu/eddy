#!/usr/bin/env bash
# Sets the version of the core or of the Mac app. They are versioned
# separately (docs/adr/0009): a UI-only change bumps only the app.
#
#   scripts/bump-version.sh core 0.3.0   # Cargo workspace: engine, server, CLI
#   scripts/bump-version.sh mac 0.3.0    # Mac app; the build number goes up by one
#   scripts/bump-version.sh              # show both
#
# It edits the version and starts a changelog section; write the entries,
# commit, and tag core-vX.Y.Z or mac-vX.Y.Z once the change is on main.
set -euo pipefail
cd "$(dirname "$0")/.."

core_version() { sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml; }
mac_version() { sed -n 's/^ *MARKETING_VERSION: "\(.*\)"/\1/p' apple/project.yml; }
mac_build() { sed -n 's/^ *CURRENT_PROJECT_VERSION: "\(.*\)"/\1/p' apple/project.yml; }

if [[ $# -eq 0 ]]; then
  echo "core $(core_version)"
  echo "mac  $(mac_version) ($(mac_build))"
  exit 0
fi

target="${1:-}"
version="${2:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/bump-version.sh core|mac X.Y.Z" >&2
  exit 2
fi

# Adds "## X.Y.Z (unreleased)" above the newest section of a changelog.
start_section() {
  local file="$1" note="$2"
  if grep -q "^## $version" "$file"; then
    return
  fi
  awk -v v="$version" -v note="$note" '
    !done && /^## / { print "## " v " (unreleased)\n"; if (note != "") print note "\n"; print "- \n"; done = 1 }
    { print }
  ' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
}

case "$target" in
  core)
    old="$(core_version)"
    sed -i '' "/^\[workspace.package\]/,/^\[/s/^version = \".*\"/version = \"$version\"/" Cargo.toml
    # Cargo.lock records the workspace crates' versions.
    cargo metadata --format-version 1 >/dev/null
    start_section CHANGELOG.md ""
    echo "core $old -> $version"
    echo "If the sync protocol changed, raise PROTOCOL_VERSION in crates/eddy-sync/src/lib.rs."
    ;;
  mac)
    old="$(mac_version)"
    build=$(( $(mac_build) + 1 ))
    sed -i '' "s/^\( *MARKETING_VERSION: \)\".*\"/\1\"$version\"/; s/^\( *CURRENT_PROJECT_VERSION: \)\".*\"/\1\"$build\"/" apple/project.yml
    start_section apple/CHANGELOG.md "Built with core $(core_version)."
    echo "mac $old -> $version (build $build)"
    ;;
  *)
    echo "usage: scripts/bump-version.sh core|mac X.Y.Z" >&2
    exit 2
    ;;
esac
