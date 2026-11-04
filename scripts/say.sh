#!/usr/bin/env bash
# Post a message to the Countersign room as the Mattermost demo user.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a; . ./.env; set +a

TOKEN_FILE="$ROOT/.countersign/mm.token"
if [ ! -s "$TOKEN_FILE" ]; then
  TOKEN="$(curl -s -D - -o /dev/null -X POST "http://127.0.0.1:${MATTERMOST_HOST_PORT:-8065}/api/v4/users/login" \
    -H 'Content-Type: application/json' \
    -d "{\"login_id\":\"$MATTERMOST_USER\",\"password\":\"$MATTERMOST_USER_PASSWORD\"}" \
    | grep -i '^token:' | tr -d '\r' | cut -d' ' -f2)"
  printf '%s' "$TOKEN" > "$TOKEN_FILE"
fi
TOKEN="$(cat "$TOKEN_FILE")"

CHANNEL="$(curl -s -b .countersign/admin.cookie "http://127.0.0.1:${API_HOST_PORT:-8000}/gateway/rooms/$(jq -r .room_id .countersign/state.json)" | jq -r .external_channel_id)"
curl -s -X POST "http://127.0.0.1:${MATTERMOST_HOST_PORT:-8065}/api/v4/posts" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "$(jq -n --arg c "$CHANNEL" --arg m "$*" '{channel_id:$c,message:$m}')" | jq -r '.id // .message'
