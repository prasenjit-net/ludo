#!/usr/bin/env bash
# Start a throwaway server with a short turn timer and run the end-to-end test against it.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build -p server
LUDO_ADDR=127.0.0.1:3100 LUDO_TURN_MS=500 LUDO_AWAY_TURN_MS=700 \
LUDO_ROOM_GRACE_MS=3000 LUDO_LOBBY_GRACE_MS=1500 LUDO_PING_MS=200 LUDO_DEAD_MS=1000 \
  ./target/debug/server >/dev/null &
pid=$!
trap 'kill $pid 2>/dev/null' EXIT
sleep 1
node server/tests/e2e.mjs
