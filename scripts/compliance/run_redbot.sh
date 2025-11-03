#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}" )/../.." && pwd)"
VENVDIR="$ROOT_DIR/target/tools/redbot-venv"
REDBOT="$VENVDIR/bin/redbot"
HARNESSCTL="$ROOT_DIR/scripts/compliance/harnessctl.sh"
ARTIFACT_DIR="$ROOT_DIR/artifacts/redbot"
SUMMARY_FILE="$ARTIFACT_DIR/summary.json"

HOST="127.0.0.1"
PORT="18080"
START_HARNESS=false
UPDATE_ENV=false

usage() {
    cat <<'HELP'
Usage: run_redbot.sh [options]

Options:
  --host HOST             Target host (default 127.0.0.1)
  --port PORT             Target port (default 18080)
  --start-harness         Start the compliance harness automatically before linting
  --update-env            Re-create the REDBot virtual environment before running
  --help                  Show this help message

Outputs:
  HAR files for each URL under artifacts/redbot/
  A combined summary at artifacts/redbot/summary.json
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)
            HOST="$2"; shift 2 ;;
        --port)
            PORT="$2"; shift 2 ;;
        --start-harness)
            START_HARNESS=true; shift ;;
        --update-env)
            UPDATE_ENV=true; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2 ;;
    esac
done

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required command '$1' not found" >&2
        exit 1
    fi
}

require_cmd python3

if [[ $UPDATE_ENV == true || ! -x "$REDBOT" ]]; then
    rm -rf "$VENVDIR"
    python3 -m venv "$VENVDIR"
    "$VENVDIR/bin/pip" install --upgrade pip redbot >/dev/null
fi

mkdir -p "$ARTIFACT_DIR"

HARNESS_STARTED=false
cleanup() {
    if $HARNESS_STARTED; then
        "$HARNESSCTL" stop >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

if $START_HARNESS; then
    "$HARNESSCTL" start --host "$HOST" --port "$PORT" >/dev/null
    HARNESS_STARTED=true
    sleep 0.5
fi

URLS=(
    "/"
    "/cache/validator"
    "/cache/immutable"
    "/json/time"
    "/range/demo"
)

HAR_FILES=()
for path in "${URLS[@]}"; do
    safe_name=$(echo "$path" | tr '/?' '__')
    har_file="$ARTIFACT_DIR/redbot${safe_name}.har"
    "$REDBOT" --output-format har "http://$HOST:$PORT$path" >"$har_file"
    HAR_FILES+=("$har_file")
done

python3 - "$SUMMARY_FILE" "${HAR_FILES[@]}" <<'PY'
import json
import sys
from pathlib import Path

out_path = Path(sys.argv[1])
har_files = [Path(p) for p in sys.argv[2:]]

summary = {
    "timestamp": __import__("time").time(),
    "results": [],
}

for har_file in har_files:
    data = json.loads(har_file.read_text())
    entries = data.get("log", {}).get("entries", [])
    messages = []
    for entry in entries:
        for note in entry.get("_red_messages", []):
            messages.append({
                "note_id": note.get("note_id"),
                "subject": note.get("subject"),
                "category": note.get("category"),
                "level": note.get("level"),
                "summary": note.get("summary"),
            })
    summary["results"].append({
        "file": str(har_file.name),
        "url": entries[0]["request"]["url"] if entries else "",
        "status": entries[0]["response"]["status"] if entries else None,
        "notes": messages,
    })

out_path.parent.mkdir(parents=True, exist_ok=True)
out_path.write_text(json.dumps(summary, indent=2) + "\n")
PY

echo "Stored REDBot summary at $SUMMARY_FILE"

