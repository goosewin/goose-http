#!/usr/bin/env python3
"""Quick cache-behaviour checks inspired by cache-tests.fyi."""

from __future__ import annotations

import argparse
import json
import pathlib
import time
from dataclasses import dataclass
from typing import Dict, List

import http.client


@dataclass
class CacheResult:
    path: str
    description: str
    assertions: List[str]
    passed: bool
    details: Dict[str, str]


def request(host: str, port: int, method: str, path: str, headers: Dict[str, str] | None = None) -> http.client.HTTPResponse:
    conn = http.client.HTTPConnection(host, port, timeout=5)
    conn.request(method, path, headers=headers or {})
    return conn.getresponse()


def check_cache_validator(host: str, port: int) -> List[CacheResult]:
    results: List[CacheResult] = []
    resp = request(host, port, "GET", "/cache/validator")
    body = resp.read()
    headers = {k.lower(): v for k, v in resp.getheaders()}
    ok = resp.status == 200 and "etag" in headers and "last-modified" in headers
    results.append(
        CacheResult(
            path="/cache/validator",
            description="Validator endpoint emits ETag/Last-Modified",
            assertions=["status == 200", "ETag present", "Last-Modified present"],
            passed=ok,
            details={
                "status": str(resp.status),
                "etag": headers.get("etag", ""),
                "last-modified": headers.get("last-modified", ""),
                "body": body.decode("utf-8", errors="replace"),
            },
        )
    )

    if "etag" in headers:
        conditional = request(
            host,
            port,
            "GET",
            "/cache/validator",
            headers={"If-None-Match": headers["etag"]},
        )
        conditional.read()
        results.append(
            CacheResult(
                path="/cache/validator",
                description="Conditional request with matching ETag yields 304",
                assertions=["status == 304"],
                passed=conditional.status == 304,
                details={"status": str(conditional.status)},
            )
        )
    return results


def check_cache_immutable(host: str, port: int) -> CacheResult:
    resp = request(host, port, "GET", "/cache/immutable")
    resp.read()
    headers = {k.lower(): v for k, v in resp.getheaders()}
    cache_control = headers.get("cache-control", "")
    passed = (
        resp.status == 200
        and "max-age=31536000" in cache_control
        and "immutable" in cache_control
    )
    return CacheResult(
        path="/cache/immutable",
        description="Immutable resource advertises long-lived cache-control",
        assertions=["status == 200", "max-age present", "immutable present"],
        passed=passed,
        details={
            "status": str(resp.status),
            "cache-control": cache_control,
            "etag": headers.get("etag", ""),
        },
    )


def check_json_no_store(host: str, port: int) -> CacheResult:
    resp = request(host, port, "GET", "/json/time")
    data = resp.read().decode("utf-8", errors="replace")
    headers = {k.lower(): v for k, v in resp.getheaders()}
    cache_control = headers.get("cache-control", "")
    passed = resp.status == 200 and "no-store" in cache_control
    return CacheResult(
        path="/json/time",
        description="Dynamic JSON endpoint is marked no-store",
        assertions=["status == 200", "Cache-Control includes no-store"],
        passed=passed,
        details={"status": str(resp.status), "cache-control": cache_control, "body": data},
    )


def check_range_support(host: str, port: int) -> CacheResult:
    resp = request(host, port, "GET", "/range/demo", headers={"Range": "bytes=0-3"})
    body = resp.read().decode("utf-8", errors="replace")
    headers = {k.lower(): v for k, v in resp.getheaders()}
    passed = resp.status == 206 and headers.get("content-range", "").startswith("bytes 0-3/")
    return CacheResult(
        path="/range/demo",
        description="Range requests return 206 with Content-Range",
        assertions=["status == 206", "Content-Range set"],
        passed=passed,
        details={
            "status": str(resp.status),
            "content-range": headers.get("content-range", ""),
            "body": body,
        },
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Cache surface checks for goose-http harness")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18080)
    parser.add_argument(
        "--output",
        default=str(
            pathlib.Path(__file__).resolve().parents[2]
            / "artifacts/cache-tests/profile.json"
        ),
    )
    parser.add_argument("--start-harness", action="store_true")

    args = parser.parse_args()

    harness_started = False
    if args.start_harness:
        harnessctl = pathlib.Path(__file__).resolve().parent / "harnessctl.sh"
        from subprocess import run

        result = run([str(harnessctl), "start", "--host", args.host, "--port", str(args.port)], capture_output=True, text=True)
        if result.returncode != 0:
            raise SystemExit(result.stderr.strip() or result.stdout.strip())
        harness_started = True
        time.sleep(0.5)

    try:
        results: List[CacheResult] = []
        results.extend(check_cache_validator(args.host, args.port))
        results.append(check_cache_immutable(args.host, args.port))
        results.append(check_json_no_store(args.host, args.port))
        results.append(check_range_support(args.host, args.port))
    finally:
        if harness_started:
            harnessctl = pathlib.Path(__file__).resolve().parent / "harnessctl.sh"
            from subprocess import run

            run([str(harnessctl), "stop"], capture_output=True, text=True)

    summary = {
        "timestamp": time.time(),
        "host": args.host,
        "port": args.port,
        "results": [r.__dict__ for r in results],
        "passed": sum(1 for r in results if r.passed),
        "total": len(results),
    }

    output = pathlib.Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2) + "\n")

    print(f"Stored cache profile at {output}")
    for result in results:
        status = "PASS" if result.passed else "FAIL"
        print(f"[{status}] {result.path}: {result.description}")

    if summary["passed"] != summary["total"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()

