#!/usr/bin/env python3
"""
report.py — Convert benchmark JSON results into a Markdown report.

Usage:
    python3 report.py results.json <version> <date>
"""

import json
import sys
from pathlib import Path


def fmt_rps(value) -> str:
    try:
        return f"{int(value):,}"
    except (ValueError, TypeError):
        return str(value) if value else "—"


def fmt_ms(value) -> str:
    try:
        v = float(value)
        return f"{v:.1f} ms"
    except (ValueError, TypeError):
        return str(value) if value else "—"


def fmt_mem(value) -> str:
    try:
        v = int(float(value))
        if v >= 1024:
            return f"{v/1024:.1f} GiB"
        return f"{v} MiB"
    except (ValueError, TypeError):
        return str(value) if value and value != "N/A" else "—"


def fmt_cpu(value) -> str:
    try:
        return f"{float(value):.1f}%"
    except (ValueError, TypeError):
        return str(value) if value and value != "N/A" else "—"


def speedup(a_rps, b_rps) -> str:
    """Return 'Xa' speedup of a vs b."""
    try:
        ratio = float(a_rps) / float(b_rps)
        return f"{ratio:.1f}×"
    except (ValueError, TypeError, ZeroDivisionError):
        return "—"


SERVER_LABELS = {
    "turbine_nts_4w":       "Turbine NTS · 4w",
    "turbine_nts_8w":       "Turbine NTS · 8w",
    "turbine_nts_4w_p":     "Turbine NTS · 4w · persistent",
    "turbine_nts_8w_p":     "Turbine NTS · 8w · persistent",
    "turbine_zts_4w":       "Turbine ZTS · 4w",
    "turbine_zts_8w":       "Turbine ZTS · 8w",
    "frankenphp_4w":        "FrankenPHP · 4w",
    "frankenphp_8w":        "FrankenPHP · 8w",
    "frankenphp_4w_worker": "FrankenPHP · 4w · worker",
    "frankenphp_8w_worker": "FrankenPHP · 8w · worker",
    "nginx_fpm_4w":         "Nginx + FPM · 4w",
    "nginx_fpm_8w":         "Nginx + FPM · 8w",
}

SERVER_ORDER = [
    "turbine_nts_4w",       "turbine_nts_8w",
    "turbine_nts_4w_p",     "turbine_nts_8w_p",
    "turbine_zts_4w",       "turbine_zts_8w",
    "frankenphp_4w",        "frankenphp_8w",
    "frankenphp_4w_worker", "frankenphp_8w_worker",
    "nginx_fpm_4w",         "nginx_fpm_8w",
]


def render_table(scenario: dict) -> str:
    # Determine which servers are present in this scenario
    servers = [s for s in SERVER_ORDER if s in scenario]
    # Use 8w FPM as baseline (closest to classic nginx+fpm baseline)
    for _bk in ("nginx_fpm_8w", "nginx_fpm_4w", "nginx_fpm"):
        if _bk in scenario:
            baseline_key = _bk
            break
    else:
        baseline_key = servers[-1]
    baseline_rps = scenario.get(baseline_key, {}).get("rps", 0)

    header = "| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |"
    sep    = "|--------|------:|:-----------:|----:|----:|:-------:|---------:|"
    rows   = [header, sep]

    for key in servers:
        data  = scenario[key]
        label = SERVER_LABELS.get(key, key)
        rps   = fmt_rps(data.get("rps"))
        p50   = fmt_ms(data.get("latency_p50"))
        p99   = fmt_ms(data.get("latency_p99"))
        cpu   = fmt_cpu(data.get("avg_cpu_pct"))
        mem   = fmt_mem(data.get("peak_mem_mib"))
        vs    = speedup(data.get("rps", 0), baseline_rps) if key != baseline_key else "baseline"
        rows.append(f"| {label} | {rps} | {vs} | {p50} | {p99} | {cpu} | {mem} |")

    return "\n".join(rows)


PHP_SCRIPT_LABELS = {
    "hello.php":      ("Hello World",        "Minimal `echo 'Hello World!'` response."),
    "html_50k.php":   ("HTML 50 KB",         "50 KB HTML response — SSR page simulation."),
    "pdf_50k.php":    ("PDF Binary 50 KB",   "50 KB `application/pdf` binary response."),
    "random_50k.php": ("Random 50 KB",       "50 KB incompressible random data — stress-tests compression bypass."),
}


def render_php_scripts_section(php_scenario: dict) -> str:
    """Render 4 sub-tables, one per PHP script."""
    scripts  = php_scenario.get("scripts", list(PHP_SCRIPT_LABELS.keys()))
    servers  = [s for s in SERVER_ORDER if s in php_scenario]
    baseline_key = "nginx_fpm" if "nginx_fpm" in php_scenario else servers[-1]

    lines = []
    for idx, script in enumerate(scripts):
        title, desc = PHP_SCRIPT_LABELS.get(script, (script, ""))
        lines += [f"### {title}", "", f"_{desc}_", ""]

        # Build a synthetic single-result scenario for render_table
        single: dict = {}
        for key in servers:
            arr = php_scenario.get(key)
            if isinstance(arr, list) and idx < len(arr):
                single[key] = arr[idx]
        single["description"] = desc

        if single:
            lines += [render_table(single), ""]

    return "\n".join(lines)


def render_report(data: dict, version: str, date: str) -> str:
    params    = data.get("parameters", {})
    conns     = params.get("connections", 100)
    duration  = params.get("duration_seconds", 30)
    workers_4 = params.get("workers_4w", 4)
    workers_8 = params.get("workers_8w", 8)
    mem_mb    = params.get("memory_limit_mb", 256)
    max_req   = params.get("max_requests_per_worker", 50000)
    server    = data.get("server", "Hetzner CPX41")
    tool      = data.get("tool", "bombardier")
    images    = data.get("images", {})
    scenarios = data.get("scenarios", {})

    raw        = scenarios.get("raw_php", {})
    laravel    = scenarios.get("laravel", {})
    phalcon    = scenarios.get("phalcon", {})
    php_scripts = scenarios.get("php_scripts", {})

    nts_img = images.get("turbine_nts", "")
    zts_img = images.get("turbine_zts", "")

    lines = [
        "# Turbine Benchmark Results",
        "",
        f"| | |",
        f"|---|---|",
        f"| **Version** | {version} |",
        f"| **Date** | {date} |",
        f"| **Server** | {server} |",
        f"| **Tool** | [{tool}](https://github.com/codesenberg/bombardier) |",
        f"| **Parameters** | {duration}s · {conns} connections |",
        f"| **Workers** | {workers_4}w and {workers_8}w variants (Turbine + FPM) |",
        f"| **Memory limit** | {mem_mb} MB per worker |",
        f"| **Max req/worker** | {max_req:,} |",
        f"| **Turbine NTS image** | `{nts_img}` |",
        f"| **Turbine ZTS image** | `{zts_img}` |",
        "",
        "> **Baseline**: Nginx + PHP-FPM · 8 workers.",
        "> **Persistent**: PHP worker process stays alive across requests (same as FrankenPHP worker mode).",
        "> CPU and memory metrics are collected via `docker stats` during the benchmark run.",
        "> Nginx + PHP-FPM runs natively (no docker stats).",
        "",
        "---",
        "",
        "## Raw PHP",
        "",
        f"_{raw.get('description', 'Single PHP file returning plain-text Hello World.')}_",
        "",
        render_table(raw),
        "",
        "## Laravel",
        "",
        f"_{laravel.get('description', 'Laravel framework, single JSON route, no database.')}_",
        "",
        render_table(laravel),
        "",
        "## Phalcon",
        "",
        f"_{phalcon.get('description', 'Phalcon micro application, single JSON route.')}_",
        "",
        f"> {phalcon.get('note', '')}",
        "",
        render_table(phalcon),
        "",
        "## PHP Scripts",
        "",
        f"_{php_scripts.get('description', 'Individual PHP scripts benchmarked per file.')}_",
        "",
        render_php_scripts_section(php_scripts),
        "---",
        "",
        "*Generated automatically — "
        "[benchmark workflow](/.github/workflows/benchmark.yml)*  ",
        "*[View history](history/)*",
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
