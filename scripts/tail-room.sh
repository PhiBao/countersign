#!/usr/bin/env bash
# Print the last N messages in the Countersign room.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a; . ./.env; set +a
N="${1:-8}"
TOKEN="$(cat .countersign/mm.token)"
CHANNEL="$(curl -s -b .countersign/admin.cookie "http://127.0.0.1:${API_HOST_PORT:-8000}/gateway/rooms/$(jq -r .room_id .countersign/state.json)" | jq -r .external_channel_id)"
curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:${MATTERMOST_HOST_PORT:-8065}/api/v4/channels/$CHANNEL/posts?per_page=$N" \
  | jq -r '.order[] as $id | .posts[$id] | "--- \(.user_id)\n\(.message)\n"'
