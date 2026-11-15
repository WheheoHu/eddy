#!/usr/bin/env bash
# End-to-end check of eddy-server as it is deployed: builds the image, runs
# a throwaway container (its own name, port and volume, nothing shared with
# deploy/compose.yaml or the app's data) and drives it with two eddy-cli
# data directories standing in for two devices:
#
#   pair both -> sync -> stop the server -> change things offline ->
#   start it again -> both converge
#
# Needs Docker (OrbStack is fine) and a built pdfium (scripts/fetch-pdfium.sh).
#
#   scripts/test-server-smoke.sh          # clean up afterwards
#   scripts/test-server-smoke.sh --keep   # leave the container and data for a look
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
source scripts/env.sh

NAME=eddy-smoke
PORT="${EDDY_SMOKE_PORT:-8799}"
URL="http://127.0.0.1:$PORT"
WORK="$PWD/build/smoke"
KEEP=0
[[ "${1:-}" == "--keep" ]] && KEEP=1

step() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

cleanup() {
  status=$?
  if [[ $KEEP -eq 0 ]]; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker volume rm "$NAME-data" >/dev/null 2>&1 || true
    rm -rf "$WORK"
  else
    echo "kept: container $NAME on $URL, volume $NAME-data, $WORK"
  fi
  if [[ $status -eq 0 ]]; then echo "smoke test passed"; else echo "smoke test FAILED" >&2; fi
}
trap cleanup EXIT

wait_healthy() {
  for _ in $(seq 1 60); do
    if curl -fsS "$URL/v1/health" >/dev/null 2>&1; then return 0; fi
    sleep 0.5
  done
  docker logs --tail 30 "$NAME" >&2 || true
  fail "server did not become healthy"
}

step "build"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker volume rm "$NAME-data" >/dev/null 2>&1 || true
rm -rf "$WORK" && mkdir -p "$WORK"
HASH="$(git rev-parse --short HEAD)"
docker build -q -f deploy/Dockerfile --build-arg "EDDY_GIT_HASH=$HASH" -t eddy-server:smoke . >/dev/null
cargo build -q -p eddy-cli
CLI="$PWD/target/debug/eddy-cli"
a() { "$CLI" --data "$WORK/a" "$@"; }
b() { "$CLI" --data "$WORK/b" "$@"; }

step "start"
docker run -d --name "$NAME" --init -p "127.0.0.1:$PORT:8080" \
  -e "EDDY_MASTER_KEY=$(openssl rand -base64 32)" \
  -v "$NAME-data:/data" eddy-server:smoke >/dev/null
wait_healthy
version="$(docker exec "$NAME" eddy-server --version)"
echo "$version"
[[ "$version" == *"$HASH"* ]] || fail "the image does not report the commit it was built from"
"$CLI" --version

step "a client that sends no protocol version is told to update"
code="$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/info")"
[[ "$code" == 426 ]] || fail "expected 426, got $code"

step "pair two devices"
a connect "$URL" "$(docker exec "$NAME" eddy-server pairing-code 2>/dev/null)"
b connect "$URL" "$(docker exec "$NAME" eddy-server pairing-code 2>/dev/null)"
[[ "$(docker exec "$NAME" eddy-server devices | wc -l | tr -d ' ')" == 2 ]] || fail "expected two devices"

step "a change on A reaches B"
a tag smoke-online >/dev/null
a sync >/dev/null
b sync >/dev/null
b tags | grep -qx smoke-online || fail "B did not receive A's tag"

step "stop the server (it should exit on SIGTERM, not be killed)"
started=$(date +%s)
docker stop "$NAME" >/dev/null
took=$(( $(date +%s) - started ))
echo "stopped in ${took}s"
[[ $took -lt 8 ]] || fail "the server did not shut down on SIGTERM"
[[ "$(docker inspect -f '{{.State.ExitCode}}' "$NAME")" == 0 ]] || fail "the server did not exit cleanly"

step "work offline"
a tag smoke-offline >/dev/null
b tag smoke-offline-b >/dev/null
a tags | grep -qx smoke-offline || fail "the tag was not recorded locally"
if a sync >/dev/null 2>&1; then fail "sync should report the server as unreachable"; fi
a status | tee "$WORK/status-offline.txt"
grep -qx "server offline" "$WORK/status-offline.txt" || fail "status should say the server is offline"
grep -qx "pending uploads 1" "$WORK/status-offline.txt" || fail "the change should be waiting to upload"

step "start the server again: same identity, same devices, both converge"
docker start "$NAME" >/dev/null
wait_healthy
[[ "$(docker exec "$NAME" eddy-server devices | wc -l | tr -d ' ')" == 2 ]] || fail "devices were lost in the restart"
a sync >/dev/null
b sync >/dev/null
a sync >/dev/null
a status | tee "$WORK/status-online.txt"
grep -qx "server online" "$WORK/status-online.txt" || fail "status should say the server is online"
grep -qx "pending uploads 0" "$WORK/status-online.txt" || fail "everything should have been uploaded"
for tag in smoke-online smoke-offline smoke-offline-b; do
  a tags | grep -qx "$tag" || fail "A is missing $tag"
  b tags | grep -qx "$tag" || fail "B is missing $tag"
done

step "the container reports healthy"
for _ in $(seq 1 30); do
  health="$(docker inspect -f '{{.State.Health.Status}}' "$NAME")"
  [[ "$health" == healthy ]] && break
  sleep 1
done
[[ "$health" == healthy ]] || fail "container health is $health"
