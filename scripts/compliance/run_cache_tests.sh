#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}" )/../.." && pwd)"
TOOLS_DIR="$ROOT_DIR/target/tools"
CACHE_REPO="$TOOLS_DIR/cache-tests"
ARTIFACT_DIR="$ROOT_DIR/artifacts/cache-tests"
LOG_FILE="$ARTIFACT_DIR/cache-tests.log"
JSON_FILE="$ARTIFACT_DIR/results.json"
HARNESSCTL="$ROOT_DIR/scripts/compliance/harnessctl.sh"

HOST="127.0.0.1"
PORT="18080"
START_HARNESS=false
UPDATE_REPO=false

usage() {
    cat <<'HELP'
Usage: run_cache_tests.sh [options]

Options:
  --host HOST             Target host (default 127.0.0.1)
  --port PORT             Target port (default 18080)
  --start-harness         Start the compliance harness automatically before testing
  --output PATH           Override JSON output path (default artifacts/cache-tests/results.json)
  --update                Pull latest cache-tests sources before running
  --help                  Show this help message

Requires bun >= 1.0. The script will install dependencies with bun install on first run.
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
        --output)
            JSON_FILE="$2"; shift 2 ;;
        --update)
            UPDATE_REPO=true; shift ;;
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

require_cmd bun
require_cmd git

mkdir -p "$ARTIFACT_DIR"
mkdir -p "$TOOLS_DIR"

if [[ ! -d "$CACHE_REPO/.git" ]]; then
    git clone https://github.com/http-tests/cache-tests "$CACHE_REPO"
fi

if $UPDATE_REPO; then
    (cd "$CACHE_REPO" && git pull --ff-only)
fi

(cd "$CACHE_REPO" && bun install >/dev/null)

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
fi

set +e
BUN_BASE="http://$HOST:$PORT/"
(cd "$CACHE_REPO" && npm_config_base="$BUN_BASE" bun run --silent cli) >"$JSON_FILE" 2>"$LOG_FILE"
STATUS=$?
set -e

cat "$LOG_FILE"

if $HARNESS_STARTED; then
    "$HARNESSCTL" stop >/dev/null
    HARNESS_STARTED=false
fi

if [[ $STATUS -ne 0 ]]; then
    echo "cache-tests CLI exited with status $STATUS" >&2
    exit $STATUS
fi

echo "Stored cache-tests results at $JSON_FILE"

