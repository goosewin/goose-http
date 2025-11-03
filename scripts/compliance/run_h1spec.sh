#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}" )/../.." && pwd)"
TARGET_DIR="$ROOT_DIR/target/compliance"
REPO_DIR="$ROOT_DIR/target/tools/h1spec"
LOG_FILE="$TARGET_DIR/h1spec.log"
JSON_FILE="$TARGET_DIR/h1spec.json"
HARNESSCTL="$ROOT_DIR/scripts/compliance/harnessctl.sh"

HOST="127.0.0.1"
PORT="18080"
START_HARNESS=false
UPDATE_REPO=false

usage() {
    cat <<'HELP'
Usage: run_h1spec.sh [options]

Options:
  --host HOST             Target host (default 127.0.0.1)
  --port PORT             Target port (default 18080)
  --output PATH           JSON output path (default target/compliance/h1spec.json)
  --repo-dir PATH         Override local h1spec checkout directory
  --start-harness         Start compliance harness automatically before running h1spec
  --update                Force pull the upstream h1spec repository before running
  --help                  Show this help message

This script wraps the upstream uNetworking h1spec battery (Deno script) and
produces a structured JSON report alongside the raw log output.
HELP
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)
            HOST="$2"; shift 2 ;;
        --port)
            PORT="$2"; shift 2 ;;
        --output)
            JSON_FILE="$2"; shift 2 ;;
        --repo-dir)
            REPO_DIR="$2"; shift 2 ;;
        --start-harness)
            START_HARNESS=true; shift ;;
        --update)
            UPDATE_REPO=true; shift ;;
        -h|--help)
            usage
            exit 0 ;;
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

require_cmd deno
require_cmd git
require_cmd python3

mkdir -p "$TARGET_DIR"

clone_or_update_repo() {
    if [[ -d "$REPO_DIR/.git" ]]; then
        if $UPDATE_REPO; then
            (cd "$REPO_DIR" && git pull --ff-only)
        fi
    else
        mkdir -p "$(dirname "$REPO_DIR")"
        git clone https://github.com/uNetworkingAB/h1spec "$REPO_DIR"
    fi
}

clone_or_update_repo

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

RUN_STATUS=0
set +e
deno run --allow-net "$REPO_DIR/http_test.ts" "$HOST" "$PORT" >"$LOG_FILE" 2>&1
RUN_STATUS=$?
set -e

cat "$LOG_FILE"

python3 - "$LOG_FILE" "$JSON_FILE" "$RUN_STATUS" <<'PY'
import ast
import json
import re
import sys
import time
from pathlib import Path

log_path = Path(sys.argv[1])
output_path = Path(sys.argv[2])
exit_code = int(sys.argv[3])

lines = log_path.read_text().splitlines()

tests = []
messages = []
summary = {
    "exit_code": exit_code,
    "timestamp": time.time(),
}

line_re = re.compile(r"^[✅❌] ")
expected_re = re.compile(r"Expected ranges: (\[.*\])")
summary_re = re.compile(r"(\d+) out of (\d+) tests passed")

for line in lines:
    stripped = line.strip()
    if not stripped:
        continue
    if line_re.match(stripped):
        passed = stripped.startswith("✅")
        body = stripped[1:].strip()
        if ": " in body:
            description, details = body.split(": ", 1)
        else:
            description, details = body, ""
        expected = None
        match = expected_re.search(details)
        if match:
            try:
                expected = ast.literal_eval(match.group(1))
            except Exception:
                expected = None
        tests.append({
            "description": description,
            "passed": passed,
            "details": details,
            "expected_ranges": expected,
            "raw": line,
        })
        continue
    match = summary_re.search(stripped)
    if match:
        summary["passed"] = int(match.group(1))
        summary["total"] = int(match.group(2))
        summary["raw"] = stripped
        continue
    messages.append(line)

payload = {
    "summary": summary,
    "tests": tests,
    "messages": messages,
}

output_path.parent.mkdir(parents=True, exist_ok=True)
output_path.write_text(json.dumps(payload, indent=2) + "\n")
PY

if $HARNESS_STARTED; then
    "$HARNESSCTL" stop >/dev/null
    HARNESS_STARTED=false
fi

exit "$RUN_STATUS"

