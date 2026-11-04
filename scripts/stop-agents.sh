#!/usr/bin/env bash
# Stop the background agents.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
for role in fixer verifier; do
  pidfile=".countersign/$role.pid"
  if [ -f "$pidfile" ]; then
    pid="$(cat "$pidfile")"
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      echo "stopped $role ($pid)"
    fi
    rm -f "$pidfile"
  fi
done
