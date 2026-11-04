#!/usr/bin/env bash
# Preflight for a Countersign demo. Read-only: it never starts or changes
# anything.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAIL=0
WARN=0

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; WARN=$((WARN + 1)); }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$*"; FAIL=$((FAIL + 1)); }

have() { command -v "$1" >/dev/null 2>&1; }

echo "Countersign preflight"
echo

echo "Environment"
[ -f .env ] && ok ".env present" || bad ".env missing — run ./scripts/init-env.sh"
if [ -f .env ]; then
  # shellcheck disable=SC1091
  set -a; . ./.env; set +a
  [ "${SWITCH_VERSION:-}" = "0.31.0" ] && ok "Switch pinned to 0.31.0" || warn "SWITCH_VERSION=${SWITCH_VERSION:-unset} (expected 0.31.0)"
  [ "${COUNTERSIGN_PR_MODE:-local}" = "gh" ] && ok "landing mode: real pull request" || ok "landing mode: local proof branch"
fi
[ -f countersign.toml ] && ok "countersign.toml present" || warn "no countersign.toml; using environment defaults"
have docker && ok "docker installed" || bad "docker missing"
have cargo && ok "cargo installed" || bad "cargo missing"
have jq && ok "jq installed" || bad "jq missing (bootstrap needs it)"

echo
echo "Stack"
if docker info >/dev/null 2>&1; then
  ok "Docker daemon reachable"
  for service in postgres switch gateway mattermost; do
    state="$(docker compose ps --format '{{.Service}} {{.State}}' 2>/dev/null | awk -v s="$service" '$1 == s {print $2}')"
    [ "$state" = "running" ] && ok "$service running" || bad "$service is ${state:-not running}"
  done
else
  bad "Docker daemon not reachable"
fi
curl -fsS "http://127.0.0.1:${API_HOST_PORT:-8000}/health" >/dev/null 2>&1 \
  && ok "Switch API healthy" || bad "Switch API not answering on ${API_HOST_PORT:-8000}"
curl -fsS "http://127.0.0.1:${MATTERMOST_HOST_PORT:-8065}/api/v4/system/ping" >/dev/null 2>&1 \
  && ok "Mattermost reachable" || bad "Mattermost not answering on ${MATTERMOST_HOST_PORT:-8065}"

echo
echo "Agents"
[ -f .countersign/agents.env ] && ok "agent credentials present" || bad "agent credentials missing — run ./scripts/bootstrap.sh"
for role in fixer verifier; do
  pidfile=".countersign/$role.pid"
  if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    ok "$role process running"
  else
    bad "$role not running — run ./scripts/start-agents.sh"
  fi
done
curl -fsS "http://127.0.0.1:${COUNTERSIGN_API_PORT:-4090}/api/health" >/dev/null 2>&1 \
  && ok "ledger viewer/API healthy" || warn "ledger viewer not answering (set COUNTERSIGN_API_PORT)"

echo
echo "Coding agent (the fixer)"
if have opencode; then
  ok "opencode installed"
  model="${COUNTERSIGN_FIXER_MODEL:-opencode-go/gpt-5.6-luna}"
  if opencode models 2>/dev/null | grep -qx "$model"; then
    ok "fixer model available: $model"
  else
    warn "fixer model not listed: $model (run 'opencode auth login')"
  fi
else
  warn "opencode not installed; the fixer cannot run"
fi
if have gh; then
  if gh auth status >/dev/null 2>&1; then
    ok "gh authenticated"
  else
    warn "gh not authenticated; pull-request landing will fall back to failure"
  fi
else
  warn "gh not installed; use COUNTERSIGN_PR_MODE=local"
fi

echo
echo "Build"
[ -x target/debug/countersign ] && ok "countersign binary built" || warn "binary not built; run cargo build"
[ -f viewer/dist/index.html ] && ok "viewer build present" || warn "viewer/dist missing; run pnpm build in viewer/"
cargo test -p countersign --quiet >/dev/null 2>&1 \
  && ok "countersign unit + integration tests pass" || bad "countersign tests fail"

echo
if [ "$FAIL" -eq 0 ]; then
  echo "Ready. $WARN warning(s)."
  exit 0
fi
echo "$FAIL blocker(s), $WARN warning(s)."
exit 1
