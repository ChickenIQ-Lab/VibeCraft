#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="$ROOT_DIR/data/run"
LOG_DIR="$ROOT_DIR/data/logs"
PID_FILE="$RUN_DIR/vibecraft-server.pid"
LOG_FILE="$LOG_DIR/vibecraft-server.log"
BINARY="$ROOT_DIR/target/debug/vibecraft"

stop_pid() {
  local pid="$1"

  if [[ -z "$pid" ]]; then
    return
  fi

  if ! kill -0 "$pid" 2>/dev/null; then
    return
  fi

  kill "$pid"

  for _ in $(seq 1 50); do
    if ! kill -0 "$pid" 2>/dev/null; then
      return
    fi

    sleep 0.1
  done

  kill -9 "$pid" 2>/dev/null || true
}

stop_existing_server() {
  if [[ -f "$PID_FILE" ]]; then
    stop_pid "$(<"$PID_FILE")"
    rm -f "$PID_FILE"
  fi
}

mkdir -p "$RUN_DIR" "$LOG_DIR"
stop_existing_server

cargo build --quiet --manifest-path "$ROOT_DIR/Cargo.toml"

: > "$LOG_FILE"
nohup "$BINARY" > "$LOG_FILE" 2>&1 < /dev/null &
server_pid=$!
printf '%s\n' "$server_pid" > "$PID_FILE"

printf 'VibeCraft restarted. pid=%s log=%s\n' "$server_pid" "$LOG_FILE"
