#!/usr/bin/env python3
"""
Subset of cispa/http-conformance direct tests tailored for goose-http.

The upstream suite requires a substantial Python environment (Poetry, PostgreSQL,
patched dpkt, mitmproxy). To keep feedback loops tight, this runner reproduces a
handful of representative direct tests derived from the "Direct" activity rules
in cispa/http-conformance/testcases.py.

Tests implemented:
 - code_400_after_bad_host_request (no host, duplicate host, invalid host)
 - reject_fields_contaning_cr_lf_nul (NUL/CR/LF in header value)
 - code_501_unknown_methods (unknown method should yield 501)

The script connects to the target using raw TCP sockets to mirror the upstream
behaviour. Results are persisted as JSON in artifacts/http-conformance/.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import socket
import time
from contextlib import contextmanager
from dataclasses import dataclass
from typing import List, Optional


@dataclass
class TestResult:
    rule: str
    case: str
    description: str
    expected: List[int]
    status: Optional[int]
    passed: bool
    raw_response: str


class RawHttpClient:
    def __init__(self, host: str, port: int, timeout: float = 2.0) -> None:
        self.host = host
        self.port = port
        self.timeout = timeout

    @contextmanager
    def connect(self) -> socket.socket:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(self.timeout)
        try:
            s.connect((self.host, self.port))
            yield s
        finally:
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            s.close()

    def send(self, request: bytes) -> str:
        with self.connect() as conn:
            conn.sendall(request)
            response = bytearray()
            try:
                while True:
                    chunk = conn.recv(4096)
                    if not chunk:
                        break
                    response.extend(chunk)
            except socket.timeout:
                pass
            return response.decode("utf-8", errors="replace")


def parse_status(raw_response: str) -> Optional[int]:
    if not raw_response:
        return None
    first_line = raw_response.split("\r\n", 1)[0]
    parts = first_line.split()
    if len(parts) >= 2 and parts[0].startswith("HTTP/"):
        try:
            return int(parts[1])
        except ValueError:
            return None
    return None


def build_request(
    method: str,
    path: str,
    host: str,
    headers: Optional[List[str]] = None,
    body: str = "",
    include_host: bool = True,
) -> bytes:
    request_line = f"{method} {path} HTTP/1.1\r\n"
    header_lines = headers or []
    payload = request_line
    for header in header_lines:
        payload += header
        if not header.endswith("\r\n"):
            payload += "\r\n"
    if include_host and not any(h.lower().startswith("host:") for h in header_lines):
        payload += f"Host: {host}\r\n"
    payload += "Connection: close\r\n"
    if body:
        payload += f"Content-Length: {len(body.encode('utf-8'))}\r\n"
    payload += "\r\n"
    payload += body
    return payload.encode("utf-8", errors="surrogateescape")


def run_code_400_bad_host_tests(client: RawHttpClient, host: str) -> List[TestResult]:
    cases = [
        ("no_host", "Request without Host header", [], False),
        (
            "duplicate_host",
            "Request with duplicate Host headers",
            [f"Host: {host}\r\n", f"Host: {host}\r\n"],
            False,
        ),
        (
            "invalid_host",
            "Request with invalid Host",
            ["Host: invalid host name\r\n"],
            False,
        ),
    ]
    results: List[TestResult] = []
    for slug, description, extra_headers, include_host in cases:
        headers = extra_headers.copy()
        request = build_request("GET", "/", host, headers, include_host=include_host)
        raw = client.send(request)
        status = parse_status(raw)
        passed = status == 400
        results.append(
            TestResult(
                rule="code_400_after_bad_host_request",
                case=slug,
                description=description,
                expected=[400],
                status=status,
                passed=passed,
                raw_response=raw,
            )
        )
    return results


def run_reject_invalid_value_test(client: RawHttpClient, host: str) -> List[TestResult]:
    invalid_bytes = ["\x00", "\r", "\n"]
    results: List[TestResult] = []
    for inv in invalid_bytes:
        header_value = f"Invalid: a{inv}\r\n"
        request = build_request("GET", "/", host, [header_value])
        raw = client.send(request)
        status = parse_status(raw)
        passed = status == 400
        results.append(
            TestResult(
                rule="reject_fields_contaning_cr_lf_nul",
                case=f"value_{repr(inv)}",
                description="Reject header values containing CR/LF/NUL",
                expected=[400],
                status=status,
                passed=passed,
                raw_response=raw,
            )
        )
    return results


def run_unknown_method_test(client: RawHttpClient, host: str) -> TestResult:
    request = build_request("FOO", "/", host)
    raw = client.send(request)
    status = parse_status(raw)
    passed = status == 501
    return TestResult(
        rule="code_501_unknown_methods",
        case="unknown_method",
        description="Unknown HTTP method should return 501",
        expected=[501],
        status=status,
        passed=passed,
        raw_response=raw,
    )


def load_tests(host: str, port: int) -> List[TestResult]:
    client = RawHttpClient(host, port)
    results: List[TestResult] = []
    results.extend(run_code_400_bad_host_tests(client, host))
    results.extend(run_reject_invalid_value_test(client, host))
    results.append(run_unknown_method_test(client, host))
    return results


def start_harness(host: str, port: int) -> None:
    harnessctl = pathlib.Path(__file__).resolve().parent / "harnessctl.sh"
    if not harnessctl.exists():
        raise SystemExit("harnessctl.sh not found; cannot auto-start harness")
    from subprocess import run

    result = run(
        [str(harnessctl), "start", "--host", host, "--port", str(port)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(result.stderr.strip() or result.stdout.strip())


def stop_harness() -> None:
    harnessctl = pathlib.Path(__file__).resolve().parent / "harnessctl.sh"
    from subprocess import run

    run([str(harnessctl), "stop"], capture_output=True, text=True)


def main() -> None:
    parser = argparse.ArgumentParser(description="Run a subset of cispa direct tests")
    parser.add_argument("--host", default="127.0.0.1", help="Target host (default 127.0.0.1)")
    parser.add_argument("--port", type=int, default=18080, help="Target port (default 18080)")
    parser.add_argument(
        "--output",
        default=str(
            pathlib.Path(__file__).resolve().parents[2]
            / "artifacts/http-conformance/report.json"
        ),
        help="Path to JSON report",
    )
    parser.add_argument(
        "--start-harness",
        action="store_true",
        help="Start the compliance harness automatically before running tests",
    )

    args = parser.parse_args()

    output_path = pathlib.Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    if args.start_harness:
        start_harness(args.host, args.port)
        time.sleep(0.5)
        harness_started = True
    else:
        harness_started = False

    try:
        results = load_tests(args.host, args.port)
    finally:
        if harness_started:
            stop_harness()

    summary = {
        "timestamp": time.time(),
        "host": args.host,
        "port": args.port,
        "tests": [result.__dict__ for result in results],
        "passed": sum(1 for r in results if r.passed),
        "total": len(results),
    }

    output_path.write_text(json.dumps(summary, indent=2) + "\n")

    print(f"Stored HTTP conformance subset report at {output_path}")
    for result in results:
        status = result.status if result.status is not None else "<no response>"
        prefix = "PASS" if result.passed else "FAIL"
        print(f"[{prefix}] {result.rule}/{result.case}: got {status}, expected {result.expected}")

    if summary["passed"] != summary["total"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()

