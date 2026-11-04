#!/usr/bin/env bash
# Start both agents in the background, one PID file each.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a
. ./.env
. ./.countersign/agents.env
set +a

mkdir -p .countersign/logs
cargo build -q

start() {
  local role="$1" id="$2" key="$3"
  local pidfile=".countersign/$role.pid"
  if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "$role already running (pid $(cat "$pidfile"))"
    return
  fi
  SWITCH_AGENT_ID="$id" \
  SWITCH_AGENT_KEY="$key" \
  COUNTERSIGN_ROLE="$role" \
  COUNTERSIGN_ROLE_NAME="$role" \
    nohup ./target/debug/countersign >> ".countersign/logs/$role.log" 2>&1 &
  echo $! > "$pidfile"
  echo "$role started (pid $!)"
}

start fixer "$COUNTERSIGN_FIXER_ID" "$COUNTERSIGN_FIXER_KEY"
start verifier "$COUNTERSIGN_VERIFIER_ID" "$COUNTERSIGN_VERIFIER_KEY"
