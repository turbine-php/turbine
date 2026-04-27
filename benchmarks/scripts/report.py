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
        v = float(value)
        if v == 0:
            return "—"
        return f"{v:.1f}%"
    except (ValueError, TypeError):
        return str(value) if value and value != "N/A" else "—"


def fmt_errors(value) -> str:
    try:
        v = int(value)
        if v == 0:
            return "0"
        if v >= 1_000_000:
            return f"{v/1_000_000:.1f}M"
        if v >= 1_000:
            return f"{v/1_000:.1f}k"
        return str(v)
    except (ValueError, TypeError):
        return "—"


def is_suspect(data: dict) -> tuple[bool, str]:
    """Return (suspect, reason).  A benchmark row is suspect if:
    - preflight validation failed (too small body, non-2xx, or error page), OR
    - wrk reported any non-2xx responses during the run.
    """
    if data.get("preflight_ok") is False:
        return True, "preflight failed"
    non_2xx = data.get("req_non_2xx", 0) or 0
    if non_2xx > 0:
        bad = data.get("first_bad_status", 0) or 0
        ratio = non_2xx / max(1, (data.get("req_2xx", 0) + non_2xx))
        return True, f"{non_2xx} non-2xx ({ratio:.0%}), first={bad}"
    return False, ""


def fmt_rps_with_flag(data: dict) -> str:
    suspect, _ = is_suspect(data)
    rps = fmt_rps(data.get("rps"))
    return f"{rps} ⚠️" if suspect else rps


def fmt_status_col(data: dict) -> str:
    """Compact status column: ✅ / ⚠️ <reason>."""
    suspect, reason = is_suspect(data)
    if not suspect:
        return "✅"
    return f"⚠️ {reason}"


def speedup(a_rps, b_rps) -> str:
    """Return 'Xa' speedup of a vs b."""
    try:
        ratio = float(a_rps) / float(b_rps)
        return f"{ratio:.1f}×"
    except (ValueError, TypeError, ZeroDivisionError):
        return "—"


SERVER_LABELS = {
    # Turbine — fork_per_request (CGI-style cold start; stateless scenarios only)
    "turbine_nts_4w_fork":  "Turbine NTS · 4w · fork-per-request",
    "turbine_nts_8w_fork":  "Turbine NTS · 8w · fork-per-request",
    "turbine_zts_4w_fork":  "Turbine ZTS · 4w · fork-per-request",
    "turbine_zts_8w_fork":  "Turbine ZTS · 8w · fork-per-request",
    # Turbine — pool_reuse (PHP-FPM-equivalent: workers alive, full PHP lifecycle/req)
    "turbine_nts_4w_pool":  "Turbine NTS · 4w · pool-reuse (FPM-eq)",
    "turbine_nts_8w_pool":  "Turbine NTS · 8w · pool-reuse (FPM-eq)",
    "turbine_zts_4w_pool":  "Turbine ZTS · 4w · pool-reuse (FPM-eq)",
    "turbine_zts_8w_pool":  "Turbine ZTS · 8w · pool-reuse (FPM-eq)",
    # Turbine — persistent_app (boot framework once, reuse handler; Laravel/Symfony)
    "turbine_nts_4w_app":   "Turbine NTS · 4w · persistent-app",
    "turbine_nts_8w_app":   "Turbine NTS · 8w · persistent-app",
    "turbine_zts_4w_app":   "Turbine ZTS · 4w · persistent-app",
    "turbine_zts_8w_app":   "Turbine ZTS · 8w · persistent-app",
    # FrankenPHP & FPM (unchanged)
    "frankenphp_4w":        "FrankenPHP (ZTS) · 4w",
    "frankenphp_8w":        "FrankenPHP (ZTS) · 8w",
    "frankenphp_4w_worker": "FrankenPHP (ZTS) · 4w · worker",
    "frankenphp_8w_worker": "FrankenPHP (ZTS) · 8w · worker",
    "nginx_fpm_4w":         "Nginx + FPM · 4w",
    "nginx_fpm_8w":         "Nginx + FPM · 8w",
}

SERVER_ORDER = [
    "turbine_nts_4w_fork",  "turbine_nts_8w_fork",
    "turbine_zts_4w_fork",  "turbine_zts_8w_fork",
    "turbine_nts_4w_pool",  "turbine_nts_8w_pool",
    "turbine_zts_4w_pool",  "turbine_zts_8w_pool",
    "turbine_nts_4w_app",   "turbine_nts_8w_app",
    "turbine_zts_4w_app",   "turbine_zts_8w_app",
    "frankenphp_4w",        "frankenphp_8w",
    "frankenphp_4w_worker", "frankenphp_8w_worker",
    "nginx_fpm_4w",         "nginx_fpm_8w",
]


def render_table(scenario: dict) -> str:
    # Determine which servers are present in this scenario
    servers = [s for s in SERVER_ORDER if s in scenario]
    if not servers:
        return "_No data available._"
    # Use 8w FPM as baseline (closest to classic nginx+fpm baseline)
    for _bk in ("nginx_fpm_8w", "nginx_fpm_4w", "nginx_fpm"):
        if _bk in scenario:
            baseline_key = _bk
            break
    else:
        baseline_key = servers[-1]
    baseline_rps = scenario.get(baseline_key, {}).get("rps", 0)

    header = "| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |"
    sep    = "|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|"
    rows   = [header, sep]

    for key in servers:
        data  = scenario[key]
        label = SERVER_LABELS.get(key, key)
        rps   = fmt_rps_with_flag(data)
        p50   = fmt_ms(data.get("latency_p50"))
        p99   = fmt_ms(data.get("latency_p99"))
        cpu   = fmt_cpu(data.get("avg_cpu_pct"))
        mem   = fmt_mem(data.get("peak_mem_mib"))
        errs  = fmt_errors(data.get("req_errors"))
        status = fmt_status_col(data)
        vs    = speedup(data.get("rps", 0), baseline_rps) if key != baseline_key else "baseline"
        rows.append(f"| {label} | {rps} | {vs} | {p50} | {p99} | {cpu} | {mem} | {errs} | {status} |")

    return "\n".join(rows)


PHP_SCRIPT_LABELS = {
    "html_50k.php":   ("HTML 50 KB",         "50 KB HTML response — SSR page simulation."),
    "pdf_50k.php":    ("PDF Binary 50 KB",   "50 KB `application/pdf` binary response."),
    "random_50k.php": ("Random 50 KB",       "50 KB incompressible random data — stress-tests compression bypass."),
}


def render_php_scripts_section(php_scenario: dict) -> str:
    """Render 4 sub-tables, one per PHP script."""
    scripts  = php_scenario.get("scripts", list(PHP_SCRIPT_LABELS.keys()))
    servers  = [s for s in SERVER_ORDER if s in php_scenario]
    if not servers:
        return "_No data available._"
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
        if any(k for k in single if k in SERVER_ORDER):
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
    tool      = data.get("tool", "wrk")
    images    = data.get("images", {})
    scenarios = data.get("scenarios", {})

    raw        = scenarios.get("raw_php", {})
    laravel    = scenarios.get("laravel", {})
    symfony    = scenarios.get("symfony", {})
    phalcon    = scenarios.get("phalcon", {})
    php_scripts = scenarios.get("php_scripts", {})

    nts_img = images.get("turbine_nts", "")
    zts_img = images.get("turbine_zts", "")
    php_ver = data.get("php_version", "")

    lines = [
        "# Turbine Benchmark Results",
        "",
        f"| | |",
        f"|---|---|",
        f"| **Version** | {version} |",
        f"| **Date** | {date} |",
        f"| **PHP** | {php_ver} |",
        f"| **Server** | {server} |",
        f"| **Tool** | [{tool}](https://github.com/wg/wrk) |",
        f"| **Parameters** | {duration}s · {conns} connections |",
        f"| **Workers** | {workers_4}w and {workers_8}w variants (Turbine + FPM) |",
        f"| **Memory limit** | {mem_mb} MB per worker |",
        f"| **Max req/worker** | {max_req:,} |",
        f"| **Turbine NTS image** | `{nts_img}` |",
        f"| **Turbine ZTS image** | `{zts_img}` |",
        "",
        "> **Baseline**: Nginx + PHP-FPM · 8 workers.",
        "> **NTS**: Non-thread-safe PHP — process mode (fork per worker).",
        "> **ZTS**: Thread-safe PHP — thread mode (shared memory, lock-free dispatch).",
        "> **Lifecycle modes** (Turbine):",
        "> - **fork-per-request** — fresh PHP process per request (CGI-style). "
        "Shown only for stateless scenarios; **not architecturally comparable** to FPM/FrankenPHP, "
        "which both reuse processes between requests.",
        "> - **pool-reuse (FPM-eq)** — workers stay alive, full PHP lifecycle per request. "
        "This is the apples-to-apples comparison vs Nginx+FPM and FrankenPHP regular mode.",
        "> - **persistent-app** — framework boots once via `worker_boot`, handler reused. "
        "Comparable to FrankenPHP **worker** mode and Swoole.",
        "> **FrankenPHP** uses ZTS PHP internally and does **not** support Phalcon.",
        "> All servers (including FPM) run inside Docker containers for equal overhead.",
        "> CPU and memory metrics are collected via `docker stats --no-stream` during benchmark.",
        "",
        "> **⚠️ Disclaimer:** These benchmarks are synthetic and may not reflect real-world performance. "
        "Results depend heavily on architecture, application design, dependencies, stack, "
        "and workload characteristics. The goal is **not** to declare any runtime better or worse — "
        "choosing the right tool depends on many factors beyond raw throughput.",
        "",
        "> **Status column:** `✅` = preflight passed and all responses were 2xx. "
        "`⚠️` = the row returned non-2xx responses or failed preflight validation "
        "(tiny response body, wrong content-type, 404/5xx before the run). "
        "Req/s for flagged rows is **not comparable** to healthy rows — the server "
        "may be returning fast error pages.",
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
        "## Symfony",
        "",
        f"_{symfony.get('description', 'Symfony framework, mixed JSON routes.')}_",
        "",
        render_table(symfony),
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
