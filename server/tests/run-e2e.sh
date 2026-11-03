#!/usr/bin/env bash
# Start a throwaway server with a short turn timer and run the end-to-end test against it.
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build -p server
LUDO_ADDR=127.0.0.1:3100 LUDO_TURN_MS=500 ./target/debug/server >/dev/null &
pid=$!
trap 'kill $pid 2>/dev/null' EXIT
sleep 1
node server/tests/e2e.mjs
