#!/usr/bin/env bash
# Apply the Countersign palette to the demo Mattermost account.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
set -a; . ./.env; set +a

BASE="http://127.0.0.1:${MATTERMOST_HOST_PORT:-8065}"
TOKEN_FILE=".countersign/mm.token"
if [ ! -s "$TOKEN_FILE" ]; then
  TOKEN="$(curl -s -D - -o /dev/null -X POST "$BASE/api/v4/users/login" \
    -H 'Content-Type: application/json' \
    -d "{\"login_id\":\"$MATTERMOST_USER\",\"password\":\"$MATTERMOST_USER_PASSWORD\"}" \
    | grep -i '^token:' | tr -d '\r' | cut -d' ' -f2)"
  printf '%s' "$TOKEN" > "$TOKEN_FILE"
fi
TOKEN="$(cat "$TOKEN_FILE")"
USER_ID="$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/api/v4/users/me" | jq -r .id)"

THEME=$(jq -cn '{
  sidebarBg: "#141312",
  sidebarText: "#d9d2c2",
  sidebarUnreadText: "#f4f1e8",
  sidebarTextHoverBg: "#2a2724",
  sidebarTextActiveBorder: "#d9481c",
  sidebarTextActiveColor: "#f4f1e8",
  sidebarHeaderBg: "#141312",
  sidebarHeaderTextColor: "#f4f1e8",
  sidebarTeamBarBg: "#0d0c0b",
  centerChannelBg: "#f4f1e8",
  centerChannelColor: "#141312",
  newMessageSeparator: "#d9481c",
  linkColor: "#b83a14",
  buttonBg: "#141312",
  buttonColor: "#f4f1e8",
  errorTextColor: "#a32020",
  mentionBg: "#d9481c",
  mentionColor: "#f4f1e8",
  mentionHighlightBg: "#f3e3d8",
  mentionHighlightLink: "#a3350f",
  codeTheme: "github",
  centerChannelStyles: {},
  sidebarStyles: {}
}')

curl -s -X PUT "$BASE/api/v4/users/$USER_ID/preferences" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "$(jq -cn --arg uid "$USER_ID" --arg theme "$THEME" \
    '[{user_id:$uid, category:"theme", name:"theme", value:$theme}]')" >/dev/null
echo "  theme applied"
