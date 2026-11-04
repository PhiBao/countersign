#!/usr/bin/env bash
# Generate a working .env for the local Switch stack with fresh secrets.
#
# Refuses to overwrite an existing .env unless FORCE=1.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -f .env ] && [ "${FORCE:-0}" != "1" ]; then
  echo ".env already exists. Set FORCE=1 to replace it." >&2
  exit 1
fi

[ -f .env.example ] || { echo ".env.example is missing" >&2; exit 1; }
command -v openssl >/dev/null || { echo "openssl is required" >&2; exit 1; }

python3 - "$ROOT" <<'PY'
import pathlib
import secrets
import sys

root = pathlib.Path(sys.argv[1])
text = (root / ".env.example").read_text()


def set_key(text: str, key: str, value: str) -> str:
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line.startswith(f"{key}="):
            lines[index] = f"{key}={value}"
            return "\n".join(lines) + "\n"
    return text + f"\n{key}={value}\n"


values = {
    "SWITCH_VERSION": "0.31.0",
    "COMPOSE_PROFILES": "collab,gateway",
    "DB_PASSWORD": secrets.token_hex(24),
    "DB_OWNER_PASSWORD": secrets.token_hex(24),
    "AGENT_REGISTRATION_TOKEN": secrets.token_hex(24),
    "GATEWAY_ADMIN_PASSWORD": secrets.token_urlsafe(18),
    "MATTERMOST_ADMIN_PASSWORD": secrets.token_hex(16),
    "MATTERMOST_USER_PASSWORD": secrets.token_hex(16),
    "SECRET_KEYS": "k1:" + secrets.token_hex(32),
}
for key, value in values.items():
    text = set_key(text, key, value)

# Countersign settings (optional; local landing is the default).
text = set_key(text, "COUNTERSIGN_PR_MODE", "local")
text = set_key(text, "COUNTERSIGN_GITHUB_REPO", "")

(root / ".env").write_text(text)
print((root / ".env").read_text().split("GATEWAY_ADMIN_EMAIL=")[1].splitlines()[0])
PY

chmod 600 .env
echo
echo "Wrote .env with fresh secrets."
echo "Gateway admin: $(grep '^GATEWAY_ADMIN_EMAIL=' .env | cut -d= -f2)"
echo "Mattermost user: $(grep '^MATTERMOST_USER=' .env | cut -d= -f2)"
echo
echo "Next: docker compose up -d && ./scripts/bootstrap.sh"
