#!/usr/bin/env bash
# Bootstrap the Countersign room on a running Switch stack.
#
# Idempotent: re-running rotates the agents' API keys, keeps the same room if
# it already exists, and rewrites .countersign/agents.env.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a
. ./.env
set +a

BASE="http://127.0.0.1:${API_HOST_PORT:-8000}"
PR_MODE="${COUNTERSIGN_PR_MODE:-local}"
GITHUB_REPO="${COUNTERSIGN_GITHUB_REPO:-}"
STATE="$ROOT/.countersign"
mkdir -p "$STATE"

say() { printf '  %s\n' "$*"; }

say "waiting for Switch at $BASE"
for _ in $(seq 1 60); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then break; fi
  sleep 2
done
curl -fsS "$BASE/health" >/dev/null

say "signing in to the Gateway as $GATEWAY_ADMIN_EMAIL"
curl -fsS -c "$STATE/admin.cookie" -X POST "$BASE/gateway/auth/login" \
  -H 'content-type: application/json' \
  -d "$(jq -n --arg e "$GATEWAY_ADMIN_EMAIL" --arg p "$GATEWAY_ADMIN_PASSWORD" \
        '{email:$e,password:$p}')" >/dev/null

say "creating a registration key"
REG_KEY="$(curl -fsS -b "$STATE/admin.cookie" -X POST "$BASE/gateway/api-keys" \
  -H 'content-type: application/json' \
  -d '{"label":"countersign-bootstrap"}' | jq -r '.key')"
[ -n "$REG_KEY" ] && [ "$REG_KEY" != "null" ]

register() {
  local name="$1" description="$2"
  curl -fsS -X POST "$BASE/agents" \
    -H "Authorization: Bearer $REG_KEY" \
    -H 'content-type: application/json' \
    -d "$(jq -n --arg n "$name" --arg d "$description" '{
      name: $n,
      description: $d,
      connector_type: "custom",
      overwrite: true,
      integration_profile: {
        connection_model: "always_on",
        message_exchange: true,
        pre_invocation_mediation: [],
        post_invocation_mediation: [],
        event_reporting: []
      }
    }')"
}

say "registering countersign-fixer"
FIXER="$(register countersign-fixer "Proposes changes on a branch. Never lands its own work.")"
FIXER_ID="$(jq -r '.id' <<<"$FIXER")"
FIXER_KEY="$(jq -r '.api_key' <<<"$FIXER")"

say "registering countersign-verifier"
VERIFIER="$(register countersign-verifier "Independently reproduces and verifies proposed changes. Never writes code.")"
VERIFIER_ID="$(jq -r '.id' <<<"$VERIFIER")"
VERIFIER_KEY="$(jq -r '.api_key' <<<"$VERIFIER")"

say "opening both agents to room members"
for id in "$FIXER_ID" "$VERIFIER_ID"; do
  curl -fsS -b "$STATE/admin.cookie" -X PUT "$BASE/gateway/agents/$id/addressing-policy" \
    -H 'content-type: application/json' -d '{"policy":null}' >/dev/null
done

say "looking for an existing room"
ROOMS="$(curl -fsS -X POST "$BASE/agents/$VERIFIER_ID/ops/list_rooms" \
  -H "Authorization: Bearer $VERIFIER_KEY" \
  -H 'content-type: application/json' -d '{}')"
ROOM_ID="$(jq -r '.result[]? | select(.name == "checkout-incidents") | .room_id' <<<"$ROOMS" | head -1)"

if [ -z "$ROOM_ID" ] || [ "$ROOM_ID" = "null" ]; then
  say "creating the room and its roles"
  ROOM="$(curl -fsS -X POST "$BASE/agents/$VERIFIER_ID/ops/create_room" \
    -H "Authorization: Bearer $VERIFIER_KEY" \
    -H 'content-type: application/json' \
    -d "$(jq -n '{
      name: "checkout-incidents",
      description: "Small, verified changes to the checkout service. Agents propose; an independent verifier proves; a person countersigns.",
      agent_names: ["countersign-fixer", "countersign-verifier"],
      user_names: ["user"],
      channel_type: "channel_public",
      instructions: "This room ships small, verified changes to the checkout service in demo/orders.\n\nHow we work:\n- A person reports a problem. The fixer proposes a branch. It never lands its own work and never opens a pull request.\n- The verifier checks the proposal in a clean worktree: it reproduces the reported failure on main, runs the full suite on the branch, and refuses any branch that edits its own tests. It issues a receipt with the exact evidence.\n- A person countersigns a passing receipt. Only then does the change land.\n- If verification fails, the fixer may propose a correction. Nobody argues with a receipt; they produce a better branch.\n- Say what failed plainly. Never present a proposal as a fix until a receipt says PASS.",
      roles: [
        {
          name: "fixer",
          exclusive: false,
          instructions: "You propose changes. Work only on a branch in your own worktree. Never modify tests, never push, never open a pull request. Hand every proposal to @verifier with the branch name."
        },
        {
          name: "verifier",
          exclusive: true,
          instructions: "You are the independent check. Reproduce the reported failure on main, run the full suite on the branch in a clean worktree, and refuse branches that touch tests or files outside demo/orders. Post a receipt with the exact output. Never edit code. A receipt is the only thing a person can countersign."
        }
      ]
    }')")"
  ROOM_ID="$(jq -r '.result.id' <<<"$ROOM")"
  if [ -z "$ROOM_ID" ] || [ "$ROOM_ID" = "null" ]; then
    echo "room creation failed: $ROOM" >&2
    exit 1
  fi
else
  say "room already exists: $ROOM_ID"
fi

cat > "$STATE/agents.env" <<EOF
export SWITCH_URL="$BASE"
export COUNTERSIGN_REPO="$ROOT"
export COUNTERSIGN_LEDGER="$STATE/ledger.sqlite"
export COUNTERSIGN_API_PORT="\${COUNTERSIGN_API_PORT:-4090}"
export COUNTERSIGN_PR_MODE="$PR_MODE"
export COUNTERSIGN_GITHUB_REPO="$GITHUB_REPO"
export COUNTERSIGN_ROOM_ID="$ROOM_ID"

export COUNTERSIGN_FIXER_ID="$FIXER_ID"
export COUNTERSIGN_FIXER_KEY="$FIXER_KEY"
export COUNTERSIGN_VERIFIER_ID="$VERIFIER_ID"
export COUNTERSIGN_VERIFIER_KEY="$VERIFIER_KEY"
EOF

say "writing state"
jq -n --arg room "$ROOM_ID" --arg fixer "$FIXER_ID" --arg verifier "$VERIFIER_ID" \
  '{room_id:$room, fixer_agent_id:$fixer, verifier_agent_id:$verifier}' \
  > "$STATE/state.json"
chmod 600 "$STATE/agents.env" "$STATE/state.json"

say "done"
say "room id: $ROOM_ID"
say "next: ./scripts/run-agents.sh"
