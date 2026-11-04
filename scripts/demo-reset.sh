#!/usr/bin/env bash
# Rebuild the demo from zero: fresh Switch stack, fresh room, fresh ledger.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

./scripts/stop-agents.sh >/dev/null 2>&1 || true
echo "  resetting the Switch stack"
docker compose down -v >/dev/null 2>&1 || true
docker compose up -d >/dev/null

for _ in $(seq 1 60); do
  curl -fsS "http://127.0.0.1:8000/health" >/dev/null 2>&1 && break
  sleep 2
done
sleep 6

rm -f .countersign/ledger.sqlite .countersign/state.json \
      .countersign/admin.cookie .countersign/agents.env \
      .countersign/mm.token .countersign/registration.key

echo "  creating the room"
./scripts/bootstrap.sh

echo "  starting the agents"
./scripts/start-agents.sh >/dev/null

echo "  waiting for the room"
for _ in $(seq 1 30); do
  if ./scripts/tail-room.sh 20 2>/dev/null | grep -q "Verifier online"; then break; fi
  sleep 2
done

./scripts/theme.sh || true
echo "  ready: http://127.0.0.1:4090 (ledger) · http://127.0.0.1:8065 (room)"
