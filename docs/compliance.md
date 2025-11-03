# Compliance Playbook

This project bundles a light-weight test harness and a set of automation
scripts to exercise the Goose HTTP origin against common conformance and cache
validation suites.

All commands assume the repository root (`/Users/goosewin/Code/goose-http`).

## Prerequisites

- [Deno](https://deno.land/) ≥ 1.44 (used by the h1spec harness)
- [Bun](https://bun.sh/) ≥ 1.0 (for cache test tooling)
- Python 3.11+ (runtime helpers and REDBot CLI)
- Optional: Docker (only required if you embed these tools into a proxy/cache flow)

The helper scripts manage their own tooling clones under `target/tools/` and
write artefacts to `artifacts/`.

## Harness

```bash
scripts/compliance/harnessctl.sh start --host 127.0.0.1 --port 18080
# … run tests …
scripts/compliance/harnessctl.sh stop
```

## h1spec quick battery

Runs the upstream `uNetworkingAB/h1spec` Deno suite against the harness and
writes machine-readable output to `scripts/compliance/h1spec.json`.

```bash
scripts/compliance/run_h1spec.sh --start-harness
```

## CISPA direct-rule subset

`run_cispa_subset.py` re-implements a subset of cispa/http-conformance direct
tests (Host handling, invalid header values, unknown methods). The JSON report
is stored at `artifacts/http-conformance/report.json`.

```bash
scripts/compliance/run_cispa_subset.py --start-harness
```

## Cache surface checks

- `run_cache_profile.py` performs focused range/validator/no-store checks and
  emits `artifacts/cache-tests/profile.json`.
- `run_cache_tests.sh` contains a wrapper around the cache-tests.fyi CLI. The
  upstream suite targets caching proxies; the helper is provided as a starting
  point for experimentation and uses Bun for dependency management.

```bash
scripts/compliance/run_cache_profile.py --start-harness
# optional, requires bun + cache-tests upstream suite:
scripts/compliance/run_cache_tests.sh --start-harness
```

## REDBot lint

`run_redbot.sh` installs REDBot in a dedicated virtual environment and lints a
set of representative endpoints (`/`, `/cache/validator`, `/cache/immutable`,
`/json/time`, `/range/demo`). Results (HAR + summary) live in
`artifacts/redbot/`.

```bash
scripts/compliance/run_redbot.sh --start-harness
```

All artefacts live under `artifacts/`; feel free to wire these scripts into
your own automation if needed, but the primary workflow is to run them locally
while iterating on the server.

