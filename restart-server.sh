#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="/tmp/vibecraft-server.pid"
LOG_FILE="/tmp/vibecraft-server.log"
BINARY="$ROOT_DIR/target/debug/vibecraft"

stop_existing_server() {
  if [[ ! -f "$PID_FILE" ]]; then
    return
  fi

  local pid
  pid="$(<"$PID_FILE")"

  if [[ -z "$pid" ]]; then
    rm -f "$PID_FILE"
    return
  fi

  if ! kill -0 "$pid" 2>/dev/null; then
    rm -f "$PID_FILE"
    return
  fi

  kill "$pid"

  for _ in $(seq 1 50); do
    if ! kill -0 "$pid" 2>/dev/null; then
      rm -f "$PID_FILE"
      return
    fi

    sleep 0.1
  done

  kill -9 "$pid" 2>/dev/null || true
  rm -f "$PID_FILE"
}

stop_existing_server

cargo build --quiet --manifest-path "$ROOT_DIR/Cargo.toml"

: > "$LOG_FILE"
nohup "$BINARY" > "$LOG_FILE" 2>&1 < /dev/null &
server_pid=$!
printf '%s\n' "$server_pid" > "$PID_FILE"

printf 'VibeCraft restarted. pid=%s log=%s\n' "$server_pid" "$LOG_FILE"
