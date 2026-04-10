#!/usr/bin/env python3
"""
report.py — Convert benchmark JSON results into a Markdown report.

Usage:
    python3 report.py results.json <version> <date>
"""

import json
import sys
from pathlib import Path


def fmt_rps(value: str) -> str:
    """Format requests/sec with thousand separators."""
    try:
        n = float(value)
        return f"{n:,.0f}"
    except (ValueError, TypeError):
        return value or "—"


def speedup(turbine_rps: str, fpm_rps: str) -> str:
    """Return a human-readable speedup factor vs the FPM baseline."""
    try:
        t = float(turbine_rps)
        f = float(fpm_rps)
        if f == 0:
            return "N/A"
        ratio = t / f
        return f"{ratio:.1f}×"
    except (ValueError, TypeError):
        return "—"


def render_table(scenario: dict, fpm_key: str = "nginx_fpm") -> str:
    nts = scenario.get("turbine_nts", {})
    zts = scenario.get("turbine_zts", {})
    fpm = scenario.get(fpm_key, {})

    fpm_rps = fpm.get("rps", "0")

    rows = [
        ("Turbine NTS (process)", nts, fpm_rps),
        ("Turbine ZTS (thread)",  zts, fpm_rps),
        ("Nginx + PHP-FPM",       fpm, None),
    ]

    lines = [
        "| Server | Req/s | vs FPM | Latency (avg) | Transfer/s |",
        "|--------|------:|:------:|:-------------:|-----------:|",
    ]
    for label, data, base in rows:
        rps  = fmt_rps(data.get("rps", "0"))
        lat  = data.get("latency", "—")
        trf  = data.get("transfer", "—")
        vs   = speedup(data.get("rps", "0"), base) if base else "—"
        lines.append(f"| {label} | {rps} | {vs} | {lat} | {trf} |")

    return "\n".join(lines)


def render_report(data: dict, version: str, date: str) -> str:
    tool_params = data.get("parameters", {})
    threads     = tool_params.get("threads", 8)
    conns       = tool_params.get("connections", 100)
    duration    = tool_params.get("duration_seconds", 30)
    server      = data.get("server", "Hetzner CPX41")
    scenarios   = data.get("scenarios", {})

    raw     = scenarios.get("raw_php", {})
    laravel = scenarios.get("laravel", {})
    phalcon = scenarios.get("phalcon", {})

    raw_desc     = raw.get("description",     "Single PHP file returning plain-text Hello World")
    laravel_desc = laravel.get("description", "Laravel application, JSON response (no database)")
    phalcon_desc = phalcon.get("description", "Phalcon micro application, JSON response")

    lines = [
        "# Turbine Benchmark Results",
        "",
        f"**Version:** {version}  ",
        f"**Date:** {date}  ",
        f"**Server:** {server} (8 vCPU, 16 GB RAM, NVMe SSD)  ",
        f"**Tool:** [wrk](https://github.com/wg/wrk) — {duration}s · {threads} threads · {conns} connections  ",
        "",
        "> Nginx + PHP-FPM uses a static pool of 8 workers to match the Turbine worker count.",
        "> All Turbine results use `workers = 8` and `worker_mode = process` (NTS) or `thread` (ZTS).",
        "> No database queries. Measures raw HTTP throughput of the PHP + server stack.",
        "",
        "---",
        "",
        "## Raw PHP",
        "",
        f"_{raw_desc}._",
        "",
        render_table(raw),
        "",
        "## Laravel",
        "",
        f"_{laravel_desc}._",
        "",
        render_table(laravel),
        "",
        "## Phalcon",
        "",
        f"_{phalcon_desc}._",
        "",
        render_table(phalcon),
        "",
        "---",
        "",
        f"*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  ",
        f"*[View history](history/)*",
    ]

    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 4:
        print("Usage: report.py results.json <version> <date>", file=sys.stderr)
        sys.exit(1)

    results_path = Path(sys.argv[1])
    version      = sys.argv[2]
    date         = sys.argv[3]

    with results_path.open() as f:
        data = json.load(f)

    print(render_report(data, version, date), end="")


if __name__ == "__main__":
    main()
