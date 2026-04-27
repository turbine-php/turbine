#!/usr/bin/env python3
"""
compare.py — Compare two benchmark JSON files and detect regressions.

Usage:
    python3 compare.py <current.json> <previous.json> [--threshold-pct N] [--strict]

Behaviour:
    - Walks every (scenario, server-key) pair present in BOTH files.
    - Skips suspect rows (preflight_ok=False or req_non_2xx>0) on either side.
    - Skips fork-per-request rows (CLASS_FORK) — those are diagnostic and
      noisy by design; gating on them would create false regressions.
    - For the remaining rows, computes (current_rps - previous_rps) / previous_rps.
    - Prints a Markdown table of all comparisons sorted by delta (worst first).
    - Exit code:
        0  if no regressions exceed the threshold
        0  with WARNING printed if any regression exceeds threshold AND --strict
           is not set (default — warning only, suitable for release branches)
        2  if any regression exceeds threshold AND --strict is set
        1  on usage error or missing files

Threshold defaults to 10% — meaning current must be at least 90% of previous.
Improvements (positive deltas) are never flagged.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

# Keys we never gate on — they're diagnostic-only and have higher noise floor
# (CLASS_FORK in report.py: single-executor mode, by definition unstable and
# not comparable to FPM/FrankenPHP).
DIAGNOSTIC_KEYS = {
    "turbine_nts_8w_fork",
    "turbine_zts_8w_fork",
}


def is_suspect(row: dict) -> bool:
    if not isinstance(row, dict):
        return True
    if row.get("preflight_ok") is False:
        return True
    if (row.get("req_non_2xx") or 0) > 0:
        return True
    if (row.get("rps") or 0) <= 0:
        return True
    return False


def iter_pairs(current: dict, previous: dict):
    cur_scenarios = current.get("scenarios", {})
    prv_scenarios = previous.get("scenarios", {})
    for scenario in sorted(set(cur_scenarios) & set(prv_scenarios)):
        cur = cur_scenarios.get(scenario, {}) or {}
        prv = prv_scenarios.get(scenario, {}) or {}
        if not isinstance(cur, dict) or not isinstance(prv, dict):
            continue
        common_keys = set(cur) & set(prv)
        for key in sorted(common_keys):
            if key in DIAGNOSTIC_KEYS:
                continue
            cur_row = cur[key]
            prv_row = prv[key]
            # php_scripts: row is a list-of-dicts (one per script). Compare
            # the per-script rows positionally; aggregate worst delta.
            if isinstance(cur_row, list) and isinstance(prv_row, list):
                for idx, (c, p) in enumerate(zip(cur_row, prv_row)):
                    yield (f"{scenario}/{key}#{idx}", c, p)
                continue
            if not isinstance(cur_row, dict) or not isinstance(prv_row, dict):
                continue
            yield (f"{scenario}/{key}", cur_row, prv_row)


def main() -> int:
    args = sys.argv[1:]
    threshold_pct = 10.0
    strict = False

    positional: list[str] = []
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--strict":
            strict = True
        elif a == "--threshold-pct":
            i += 1
            if i >= len(args):
                print("error: --threshold-pct requires a value", file=sys.stderr)
                return 1
            try:
                threshold_pct = float(args[i])
            except ValueError:
                print(f"error: invalid threshold {args[i]!r}", file=sys.stderr)
                return 1
        elif a.startswith("--threshold-pct="):
            try:
                threshold_pct = float(a.split("=", 1)[1])
            except ValueError:
                print(f"error: invalid threshold {a!r}", file=sys.stderr)
                return 1
        else:
            positional.append(a)
        i += 1

    if len(positional) < 2:
        print("Usage: compare.py <current.json> <previous.json> "
              "[--threshold-pct N] [--strict]", file=sys.stderr)
        return 1

    cur_path = Path(positional[0])
    prv_path = Path(positional[1])
    if not cur_path.exists():
        print(f"error: {cur_path} does not exist", file=sys.stderr)
        return 1
    if not prv_path.exists():
        # Missing baseline is not a regression; just print info and pass.
        print(f"info: previous results file {prv_path} not found — skipping comparison.")
        return 0

    with cur_path.open() as f:
        current = json.load(f)
    with prv_path.open() as f:
        previous = json.load(f)

    rows: list[tuple[str, int, int, float, str]] = []
    skipped: list[tuple[str, str]] = []

    for label, cur_row, prv_row in iter_pairs(current, previous):
        if is_suspect(cur_row):
            skipped.append((label, "current suspect"))
            continue
        if is_suspect(prv_row):
            skipped.append((label, "previous suspect"))
            continue
        cur_rps = int(cur_row.get("rps") or 0)
        prv_rps = int(prv_row.get("rps") or 0)
        if prv_rps <= 0:
            continue
        delta = (cur_rps - prv_rps) / prv_rps * 100.0
        flag = ""
        if delta < -threshold_pct:
            flag = f"REGRESSION (≥{threshold_pct:.0f}% drop)"
        elif delta < 0:
            flag = "minor drop"
        rows.append((label, prv_rps, cur_rps, delta, flag))

    rows.sort(key=lambda r: r[3])

    cur_tag = current.get("version") or current.get("tag") or "current"
    prv_tag = previous.get("version") or previous.get("tag") or "previous"

    print(f"# Benchmark regression check")
    print()
    print(f"- Current:  `{cur_path.name}` ({cur_tag})")
    print(f"- Previous: `{prv_path.name}` ({prv_tag})")
    print(f"- Threshold: {threshold_pct:.1f}% drop in req/s")
    print(f"- Strict mode: {'yes (will fail on regression)' if strict else 'no (warn only)'}")
    print()

    if not rows:
        print("_No comparable rows found._")
        return 0

    print("| Scenario / Server | Previous req/s | Current req/s | Δ% | Flag |")
    print("|---|---:|---:|---:|---|")
    for label, prv_rps, cur_rps, delta, flag in rows:
        sign = "+" if delta >= 0 else ""
        print(f"| {label} | {prv_rps:,} | {cur_rps:,} | {sign}{delta:.1f}% | {flag} |")

    if skipped:
        print()
        print(f"<details><summary>Skipped {len(skipped)} rows (suspect / non-2xx)</summary>")
        print()
        for label, reason in skipped:
            print(f"- `{label}` — {reason}")
        print()
        print("</details>")

    regressions = [r for r in rows if r[3] < -threshold_pct]
    if regressions:
        print()
        print(f"### ⚠️ {len(regressions)} regression(s) exceed {threshold_pct:.1f}% threshold")
        for label, prv_rps, cur_rps, delta, _ in regressions:
            print(f"- **{label}**: {prv_rps:,} → {cur_rps:,} req/s ({delta:+.1f}%)")
        if strict:
            return 2
        print()
        print("_Strict mode disabled — exit 0 anyway. Pass `--strict` to fail the job._")
        return 0

    print()
    print(f"### ✅ No regressions exceeding {threshold_pct:.1f}% threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
