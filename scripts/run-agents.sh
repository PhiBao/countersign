#!/usr/bin/env bash
# Run both Countersign agents in the foreground (Ctrl-C stops both).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a
. ./.env
. ./.countersign/agents.env
set +a

mkdir -p .countersign/logs
cargo build -q

export SWITCH_AGENT_ID="$COUNTERSIGN_FIXER_ID"
export SWITCH_AGENT_KEY="$COUNTERSIGN_FIXER_KEY"
export COUNTERSIGN_ROLE=fixer
export COUNTERSIGN_ROLE_NAME=fixer
./target/debug/countersign 2>&1 | tee -a .countersign/logs/fixer.log &
FIXER_PID=$!

export SWITCH_AGENT_ID="$COUNTERSIGN_VERIFIER_ID"
export SWITCH_AGENT_KEY="$COUNTERSIGN_VERIFIER_KEY"
export COUNTERSIGN_ROLE=verifier
export COUNTERSIGN_ROLE_NAME=verifier
./target/debug/countersign 2>&1 | tee -a .countersign/logs/verifier.log &
VERIFIER_PID=$!

trap 'kill $FIXER_PID $VERIFIER_PID 2>/dev/null || true' INT TERM
wait
