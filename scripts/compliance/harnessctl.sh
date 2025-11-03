#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="$ROOT_DIR/target/compliance"
PID_FILE="$TARGET_DIR/harness.pid"
LOG_FILE="$TARGET_DIR/harness.log"
BIN_DEBUG="$ROOT_DIR/target/debug/compliance_harness"

ensure_dependencies() {
    if ! command -v cargo >/dev/null 2>&1; then
        echo "error: cargo not found in PATH" >&2
        exit 1
    fi
    if ! command -v curl >/dev/null 2>&1; then
        echo "error: curl not found in PATH" >&2
        exit 1
    fi
}

read_pid() {
    if [[ -f "$PID_FILE" ]];
    then
        cat "$PID_FILE"
    else
        return 1
    fi
}

is_running() {
    local pid="$1"
    if [[ -z "$pid" ]]; then
        return 1
    fi
    if kill -0 "$pid" >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

start_harness() {
    local host="127.0.0.1"
    local port="18080"
    local shutdown_after=""

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --host)
                host="$2"; shift 2 ;;
            --port)
                port="$2"; shift 2 ;;
            --addr)
                local addr="$2"; shift 2
                host="${addr%%:*}"
                port="${addr##*:}"
                ;;
            --shutdown-after)
                shutdown_after="$2"; shift 2 ;;
            --)
                shift
                break
                ;;
            *)
                echo "unknown option: $1" >&2
                exit 2
                ;;
        esac
    done

    if [[ -f "$PID_FILE" ]]; then
        local existing_pid
        existing_pid="$(read_pid)"
        if is_running "$existing_pid"; then
            echo "compliance harness already running (pid $existing_pid)"
            exit 0
        fi
    fi

    ensure_dependencies

    mkdir -p "$TARGET_DIR"
    (cd "$ROOT_DIR" && cargo build --bin compliance_harness)

    local args=("--host" "$host" "--port" "$port")
    if [[ -n "$shutdown_after" ]]; then
        args+=("--shutdown-after" "$shutdown_after")
    fi

    "$BIN_DEBUG" "${args[@]}" "$@" >"$LOG_FILE" 2>&1 &
    local pid=$!
    echo "$pid" >"$PID_FILE"

    local attempt
    for attempt in {1..40}; do
        if curl --max-time 1 -fs "http://$host:$port/__health" >/dev/null 2>&1; then
            echo "compliance harness running on http://$host:$port"
            return 0
        fi
        sleep 0.25
        if ! is_running "$pid"; then
            echo "harness process exited unexpectedly" >&2
            tail -n 20 "$LOG_FILE" >&2 || true
            rm -f "$PID_FILE"
            exit 1
        fi
    done

    echo "timed out waiting for harness readiness" >&2
    tail -n 20 "$LOG_FILE" >&2 || true
    stop_harness
    exit 1
}

stop_harness() {
    if [[ ! -f "$PID_FILE" ]]; then
        echo "compliance harness not running"
        return 0
    fi

    local pid
    pid="$(read_pid)"
    if [[ -z "$pid" ]]; then
        rm -f "$PID_FILE"
        return 0
    fi

    if ! is_running "$pid"; then
        rm -f "$PID_FILE"
        echo "stale pid removed"
        return 0
    fi

    kill "$pid" >/dev/null 2>&1 || true

    for _ in {1..20}; do
        if ! is_running "$pid"; then
            rm -f "$PID_FILE"
            echo "compliance harness stopped"
            return 0
        fi
        sleep 0.25
    done

    echo "harness did not stop gracefully; sending SIGKILL" >&2
    kill -9 "$pid" >/dev/null 2>&1 || true
    rm -f "$PID_FILE"
}

status_harness() {
    if [[ -f "$PID_FILE" ]]; then
        local pid
        pid="$(read_pid)"
        if is_running "$pid"; then
            echo "compliance harness running (pid $pid)"
            return 0
        fi
        echo "compliance harness pid file present but process missing"
        return 1
    fi
    echo "compliance harness stopped"
    return 3
}

usage() {
    cat <<'HELP'
Usage: harnessctl.sh <command> [options]

Commands:
  start [--host HOST] [--port PORT] [--addr HOST:PORT] [--shutdown-after SECONDS]
  stop
  restart
  status

The harness exposes representative resources for RFC 9110/9111/9112 compliance suites.
HELP
}

main() {
    if [[ $# -lt 1 ]]; then
        usage >&2
        exit 2
    fi

    local command="$1"
    shift

    case "$command" in
        start)
            start_harness "$@"
            ;;
        stop)
            stop_harness
            ;;
        restart)
            stop_harness || true
            start_harness "$@"
            ;;
        status)
            status_harness
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "unknown command: $command" >&2
            usage >&2
            exit 2
            ;;
    esac
}

main "$@"

