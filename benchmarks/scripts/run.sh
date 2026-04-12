#!/usr/bin/env bash
# run.sh — Execute all benchmarks and output a single JSON document to stdout.
#
# Usage: bash run.sh [version] [image-tag] [connections] [duration]
#
# Servers compared per scenario:
#   turbine_nts  — Turbine process mode (NTS Docker image)
#   turbine_zts  — Turbine thread  mode (ZTS Docker image)
#   frankenphp   — FrankenPHP (ZTS-based Docker image; NOT used for Phalcon)
#   nginx_fpm    — Nginx + PHP 8.4-FPM native, with Phalcon extension installed
#
# HTTP metrics: req/s, latency p50/p99/max (wrk + Lua JSON)
# System metrics: avg CPU%, peak memory MiB (docker stats streaming)

set -euo pipefail

VERSION="${1:-dev}"
IMAGE_TAG="${2:-latest}"
WRK_CONNECTIONS="${3:-256}"
WRK_DURATION="${4:-30}"

WARMUP_CONNECTIONS=20
WARMUP_DURATION=5
WRK_THREADS=4          # wrk loader threads (independent of PHP worker count)
WRK_LUA="/root/bench/wrk-report.lua"
BENCH_PORT=8080

# Per-run staging area: one JSON file per (scenario, server-variant)
RESULTS_DIR=$(mktemp -d)
trap 'rm -rf "$RESULTS_DIR"' EXIT

# save_result <scenario> <key> <json-string>
save_result() { mkdir -p "${RESULTS_DIR}/${1}"; printf '%s' "${3}" > "${RESULTS_DIR}/${1}/${2}.json"; }

# Resolve Docker image names
if echo "$IMAGE_TAG" | grep -qE "nts|zts"; then
    TURBINE_IMAGE_NTS="katisuhara/turbine-php:${IMAGE_TAG}"
    TURBINE_IMAGE_ZTS="katisuhara/turbine-php:${IMAGE_TAG/nts/zts}"
else
    TURBINE_IMAGE_NTS="katisuhara/turbine-php:${IMAGE_TAG}-php8.4-nts"
    TURBINE_IMAGE_ZTS="katisuhara/turbine-php:${IMAGE_TAG}-php8.4-zts"
fi
FRANKENPHP_IMAGE="dunglas/frankenphp:latest"
FPM_IMAGE="bench-fpm:latest"   # locally built nginx+php8.4-fpm+phalcon image
# Note: FrankenPHP is ZTS-based. Phalcon is NOT supported on FrankenPHP.

log() { echo "[bench] $*" >&2; }

# ── Wait for HTTP (accepts any response, 2xx to 4xx — just confirms server is up) ──
wait_http() {
    local url="$1"
    for i in $(seq 1 40); do
        local code
        code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "$url" 2>/dev/null)
        [[ -n "$code" && "$code" != "000" && "$code" -lt 500 ]] && return 0
        sleep 1
    done
    log "ERROR: server never became ready at ${url}"
    return 1
}

# ── Validate response body ─ log response for debugging, warn on suspect results ──
validate_response() {
    local label="$1"
    local url="$2"
    local body status
    status=$(curl -s -o /tmp/bench_body.txt -w "%{http_code}" --max-time 5 "$url" 2>/dev/null)
    body=$(head -c 500 /tmp/bench_body.txt 2>/dev/null)
    local size
    size=$(wc -c < /tmp/bench_body.txt 2>/dev/null | tr -d ' ')
    log "  VALIDATE ${label}: HTTP ${status}, ${size} bytes, body: ${body:0:120}"
    if [[ "$status" == "404" || "$status" == "403" || "$status" == "502" ]]; then
        log "  WARNING: ${label} returned HTTP ${status} — PHP may not be executing!"
    fi
    if [[ -z "$body" || "$body" == *"404"* || "$body" == *"Not Found"* ]]; then
        log "  WARNING: ${label} response looks like an error page"
    fi
    rm -f /tmp/bench_body.txt
}

# ── Collect docker stats while benchmark runs ─────────────────────────────────
# Uses --no-stream in a loop to produce clean newline-delimited output.
# Streaming mode uses \r (carriage return) which corrupts the stats file.
start_stats() {
    local container="$1"
    local outfile="$2"
    (
        while docker inspect "$container" >/dev/null 2>&1; do
            docker stats --no-stream --format "{{.CPUPerc}},{{.MemUsage}}" "$container" 2>/dev/null
            sleep 1
        done
    ) > "$outfile" &
    echo $!
}

# Parse stats file → "avg_cpu_pct peak_mem_mib"
parse_stats() {
    local file="$1"
    python3 - "$file" << 'PYEOF'
import sys, re
cpus, mems = [], []
for line in open(sys.argv[1]):
    # strip ANSI escape sequences and whitespace
    line = re.sub(r'\x1b\[[0-9;]*m', '', line).strip()
    if not line: continue
    parts = line.split(',', 1)
    if len(parts) < 2: continue
    try: cpus.append(float(parts[0].replace('%', '')))
    except: pass
    m = re.match(r'([\d.]+)\s*(GiB|MiB|KiB|B)', parts[1].split('/')[0].strip())
    if m:
        v, u = float(m.group(1)), m.group(2)
        if u == 'GiB': v *= 1024
        elif u == 'KiB': v /= 1024
        elif u == 'B': v /= 1048576
        mems.append(v)
# Skip first 2 samples (startup noise with 0.00%)
if len(cpus) > 3:
    cpus = cpus[2:]
avg_cpu = sum(cpus)/len(cpus) if cpus else 0
peak_mem = max(mems) if mems else 0
print(f'{avg_cpu:.1f} {peak_mem:.0f}')
PYEOF
}

# ── Parse wrk+Lua JSON output → compact result JSON ─────────────────────────
# wrk-report.lua outputs: {"rps":N,"latency_p50_ms":X,"latency_p99_ms":X,"latency_max_ms":X,
#                          "req_2xx":N,"req_errors":N}
parse_wrk() {
    local file="$1"
    local avg_cpu="$2"
    local peak_mem="$3"
    python3 - "$file" "$avg_cpu" "$peak_mem" << 'PYEOF'
import sys, json
try:
    data = json.load(open(sys.argv[1]))
except Exception:
    print(json.dumps({'rps':0,'latency_p50':0,'latency_p99':0,'latency_max':0,
                      'req_2xx':0,'req_errors':0,
                      'avg_cpu_pct':sys.argv[2],'peak_mem_mib':sys.argv[3],
                      'error':'no_data'}))
    sys.exit(0)
print(json.dumps({
    'rps':          int(data.get('rps', 0)),
    'latency_p50':  round(float(data.get('latency_p50_ms', 0)), 2),
    'latency_p99':  round(float(data.get('latency_p99_ms', 0)), 2),
    'latency_max':  round(float(data.get('latency_max_ms', 0)), 2),
    'req_2xx':      int(data.get('req_2xx', 0)),
    'req_errors':   int(data.get('req_errors', 0)),
    'avg_cpu_pct':  round(float(sys.argv[2]), 1) if sys.argv[2] not in ('N/A', '') else None,
    'peak_mem_mib': round(float(sys.argv[3]))     if sys.argv[3] not in ('N/A', '') else None,
}))
PYEOF
}

# ── Benchmark a Docker container ──────────────────────────────────────────────
# Usage: bench_container <label> <image> [path] [docker args...]
# path defaults to /
bench_container() {
    local label="$1"
    local image="$2"
    local path="${3:-/}"
    shift 3
    local docker_args=("$@")
    local url="http://127.0.0.1:${BENCH_PORT}${path}"
    local stats_file="/tmp/stats_${RANDOM}.txt"
    local result_file="/tmp/result_${RANDOM}.json"

    log "Starting ${label}..."
    docker run -d --name bench-server \
        -p "${BENCH_PORT}:80" \
        "${docker_args[@]}" \
        "$image" >/dev/null

    if ! wait_http "http://127.0.0.1:${BENCH_PORT}/"; then
        log "  SKIP ${label}: server never became ready (check image/config)"
        log "  --- Container logs (last 30 lines) ---"
        docker logs --tail 30 bench-server >&2 2>&1 || true
        log "  --- End container logs ---"
        docker stop bench-server >/dev/null 2>&1 || true
        docker rm   bench-server >/dev/null 2>&1 || true
        parse_wrk /dev/null "N/A" "N/A"
        return 0
    fi

    validate_response "$label" "$url"

    log "  Warmup ${label}..."
    wrk -c "$WARMUP_CONNECTIONS" -d "${WARMUP_DURATION}s" -t 2 \
        "$url" >/dev/null 2>&1 || true

    log "  Benchmarking ${label} (${WRK_DURATION}s, ${WRK_CONNECTIONS} conn)..."
    local stats_pid
    stats_pid=$(start_stats bench-server "$stats_file")

    local wrk_raw="/tmp/wrk_raw_${RANDOM}.txt"
    wrk \
        -c "$WRK_CONNECTIONS" \
        -d "${WRK_DURATION}s" \
        -t "$WRK_THREADS" \
        -s "$WRK_LUA" \
        "$url" > "$wrk_raw" 2>/dev/null || true
    grep '^{' "$wrk_raw" > "$result_file" 2>/dev/null || echo '{}' > "$result_file"
    rm -f "$wrk_raw"

    kill "$stats_pid" 2>/dev/null || true
    wait "$stats_pid" 2>/dev/null || true

    docker stop bench-server >/dev/null
    docker rm   bench-server >/dev/null

    local stats
    stats=$(parse_stats "$stats_file")
    local avg_cpu peak_mem
    avg_cpu=$(echo "$stats" | awk '{print $1}')
    peak_mem=$(echo "$stats" | awk '{print $2}')

    log "  ${label}: $(python3 -c "import json; d=json.load(open('$result_file')); print(d.get('rps',0))" 2>/dev/null || echo '?') req/s"

    parse_wrk "$result_file" "$avg_cpu" "$peak_mem"
    rm -f "$stats_file" "$result_file"
}

# ── Benchmark a set of PHP scripts inside one container (start once, N runs) ──
# Usage: bench_php_scripts <label> <image> [docker args...] -- <script1> [script2 ...]
# docker args are passed to `docker run`; scripts are the PHP filenames to hit.
bench_php_scripts() {
    local label="$1"
    local image="$2"
    shift 2

    local docker_args=()
    while [[ $# -gt 0 && "$1" != "--" ]]; do
        docker_args+=("$1"); shift
    done
    [[ "${1:-}" == "--" ]] && shift
    local scripts=("$@")

    log "Starting ${label} container for PHP script benchmarks..."
    docker run -d --name bench-server \
        -p "${BENCH_PORT}:80" \
        "${docker_args[@]}" \
        "$image" >/dev/null

    if ! wait_http "http://127.0.0.1:${BENCH_PORT}/"; then
        log "  SKIP ${label}: server never became ready"
        docker stop bench-server >/dev/null 2>&1 || true
        docker rm   bench-server >/dev/null 2>&1 || true
        local null_result
        null_result=$(parse_wrk /dev/null "N/A" "N/A")
        local joined=""
        for _ in "${scripts[@]}"; do
            joined+="${null_result},"
        done
        echo "[${joined%,}]"
        return 0
    fi

    # Validate first script to confirm PHP is executing
    validate_response "${label}/${scripts[0]}" "http://127.0.0.1:${BENCH_PORT}/${scripts[0]}"

    local results=()
    for script in "${scripts[@]}"; do
        local url="http://127.0.0.1:${BENCH_PORT}/${script}"
        local stats_file="/tmp/stats_${RANDOM}.txt"
        local result_file="/tmp/result_${RANDOM}.json"

        log "  Warmup ${label}/${script}..."
        wrk -c "$WARMUP_CONNECTIONS" -d "${WARMUP_DURATION}s" -t 2 \
            "$url" >/dev/null 2>&1 || true

        log "  Benchmarking ${label}/${script} (${WRK_DURATION}s)..."
        local stats_pid
        stats_pid=$(start_stats bench-server "$stats_file")
        local wrk_raw="/tmp/wrk_raw_${RANDOM}.txt"
        wrk -c "$WRK_CONNECTIONS" -d "${WRK_DURATION}s" -t "$WRK_THREADS" \
            -s "$WRK_LUA" "$url" > "$wrk_raw" 2>/dev/null || true
        grep '^{' "$wrk_raw" > "$result_file" 2>/dev/null || echo '{}' > "$result_file"
        rm -f "$wrk_raw"
        kill "$stats_pid" 2>/dev/null || true
        wait "$stats_pid" 2>/dev/null || true

        local stats avg_cpu peak_mem
        stats=$(parse_stats "$stats_file")
        avg_cpu=$(echo "$stats" | awk '{print $1}')
        peak_mem=$(echo "$stats" | awk '{print $2}')
        results+=("$(parse_wrk "$result_file" "$avg_cpu" "$peak_mem")")
        rm -f "$stats_file" "$result_file"
    done

    docker stop bench-server >/dev/null
    docker rm   bench-server >/dev/null

    # Output as JSON array preserving order
    local joined
    joined=$(printf '%s,' "${results[@]}")
    echo "[${joined%,}]"
}

# ─────────────────────────────────────────────────────────────────────────────
# Benchmark matrix
#   Workers:     4 and 8
#   Turbine NTS: process mode, persistent=false and persistent=true
#   Turbine ZTS: thread  mode (no persistent variant — threads already share state)
#   FrankenPHP:  regular mode (num_threads N) and worker mode (N persistent workers)
#   Nginx+FPM:   bench-fpm Docker image, 4w and 8w (equal overhead to all others)
# ─────────────────────────────────────────────────────────────────────────────

PHP_SCRIPTS=(hello.php html_50k.php pdf_50k.php random_50k.php)

# ─── Raw PHP ─────────────────────────────────────────────────────────────────
log "==> Scenario: Raw PHP"
for W in 4 8; do
    for P in "" "-p"; do
        KEY="turbine_nts_${W}w${P//-/_}"
        save_result raw_php "$KEY" \
            "$(bench_container "nts${P}/${W}w/raw" "$TURBINE_IMAGE_NTS" "/" \
                -v /var/www/raw:/var/www/html \
                -v "/etc/turbine/raw-nts-${W}w${P}.toml:/var/www/html/turbine.toml:ro")"
    done
    save_result raw_php "turbine_zts_${W}w" \
        "$(bench_container "zts/${W}w/raw" "$TURBINE_IMAGE_ZTS" "/" \
            -v /var/www/raw:/var/www/html \
            -v "/etc/turbine/raw-zts-${W}w.toml:/var/www/html/turbine.toml:ro")"
    save_result raw_php "frankenphp_${W}w" \
        "$(bench_container "frankenphp/${W}w/raw" "$FRANKENPHP_IMAGE" "/" \
            -e SERVER_NAME=:80 \
            -v /var/www/raw:/app \
            -v "/etc/frankenphp/raw-${W}w.Caddyfile:/etc/caddy/Caddyfile")"
    save_result raw_php "frankenphp_${W}w_worker" \
        "$(bench_container "frankenphp/${W}w-worker/raw" "$FRANKENPHP_IMAGE" "/" \
            -e SERVER_NAME=:80 \
            -v /var/www/raw:/app \
            -v "/etc/frankenphp/raw-${W}w-worker.Caddyfile:/etc/caddy/Caddyfile")"
    save_result raw_php "nginx_fpm_${W}w" \
        "$(bench_container "fpm/${W}w/raw" "$FPM_IMAGE" "/" \
            -e WORKERS=${W} \
            -e APP_ROOT=/var/www/html \
            -v /var/www/raw:/var/www/html)"
done

# ─── PHP Scripts ─────────────────────────────────────────────────────────────
log "==> Scenario: PHP scripts (hello, html_50k, pdf_50k, random_50k)"
for W in 4 8; do
    for P in "" "-p"; do
        KEY="turbine_nts_${W}w${P//-/_}"
        save_result php_scripts "$KEY" \
            "$(bench_php_scripts "nts${P}/${W}w/php-bench" "$TURBINE_IMAGE_NTS" \
                -v /var/www/php-bench:/var/www/html \
                -v "/etc/turbine/php-bench-nts-${W}w${P}.toml:/var/www/html/turbine.toml:ro" \
                -- "${PHP_SCRIPTS[@]}")"
    done
    save_result php_scripts "turbine_zts_${W}w" \
        "$(bench_php_scripts "zts/${W}w/php-bench" "$TURBINE_IMAGE_ZTS" \
            -v /var/www/php-bench:/var/www/html \
            -v "/etc/turbine/php-bench-zts-${W}w.toml:/var/www/html/turbine.toml:ro" \
            -- "${PHP_SCRIPTS[@]}")"
    save_result php_scripts "frankenphp_${W}w" \
        "$(bench_php_scripts "frankenphp/${W}w/php-bench" "$FRANKENPHP_IMAGE" \
            -e SERVER_NAME=:80 \
            -v /var/www/php-bench:/app \
            -v "/etc/frankenphp/php-bench-${W}w.Caddyfile:/etc/caddy/Caddyfile" \
            -- "${PHP_SCRIPTS[@]}")"
    save_result php_scripts "frankenphp_${W}w_worker" \
        "$(bench_php_scripts "frankenphp/${W}w-worker/php-bench" "$FRANKENPHP_IMAGE" \
            -e SERVER_NAME=:80 \
            -v /var/www/php-bench:/app \
            -v "/etc/frankenphp/php-bench-${W}w-worker.Caddyfile:/etc/caddy/Caddyfile" \
            -- "${PHP_SCRIPTS[@]}")"
    save_result php_scripts "nginx_fpm_${W}w" \
        "$(bench_php_scripts "fpm/${W}w/php-bench" "$FPM_IMAGE" \
            -e WORKERS=${W} \
            -e APP_ROOT=/var/www/html \
            -v /var/www/php-bench:/var/www/html \
            -- "${PHP_SCRIPTS[@]}")"
done

# ─── Laravel ─────────────────────────────────────────────────────────────────
log "==> Scenario: Laravel (JSON endpoint)"
for W in 4 8; do
    for P in "" "-p"; do
        KEY="turbine_nts_${W}w${P//-/_}"
        save_result laravel "$KEY" \
            "$(bench_container "nts${P}/${W}w/laravel" "$TURBINE_IMAGE_NTS" "/" \
                -v /var/www/laravel:/var/www/html \
                -v "/etc/turbine/laravel-nts-${W}w${P}.toml:/var/www/html/turbine.toml:ro")"
    done
    save_result laravel "turbine_zts_${W}w" \
        "$(bench_container "zts/${W}w/laravel" "$TURBINE_IMAGE_ZTS" "/" \
            -v /var/www/laravel:/var/www/html \
            -v "/etc/turbine/laravel-zts-${W}w.toml:/var/www/html/turbine.toml:ro")"
    save_result laravel "frankenphp_${W}w" \
        "$(bench_container "frankenphp/${W}w/laravel" "$FRANKENPHP_IMAGE" "/" \
            -e SERVER_NAME=:80 \
            -v /var/www/laravel:/app \
            -v "/etc/frankenphp/laravel-${W}w.Caddyfile:/etc/caddy/Caddyfile")"
    save_result laravel "frankenphp_${W}w_worker" \
        "$(bench_container "frankenphp/${W}w-worker/laravel" "$FRANKENPHP_IMAGE" "/" \
            -e SERVER_NAME=:80 \
            -v /var/www/laravel:/app \
            -v "/etc/frankenphp/laravel-${W}w-worker.Caddyfile:/etc/caddy/Caddyfile")"
    save_result laravel "nginx_fpm_${W}w" \
        "$(bench_container "fpm/${W}w/laravel" "$FPM_IMAGE" "/" \
            -e WORKERS=${W} \
            -e APP_ROOT=/var/www/html/public \
            -v /var/www/laravel:/var/www/html)"
done

# ─── Laravel note: full app dir is mounted (not just public/) so autoloader works ───
# Turbine uses [sandbox] front_controller=true to route to public/index.php

# ─── Phalcon (Turbine only + Nginx+FPM — Phalcon incompatible with FrankenPHP) ───────
log "==> Scenario: Phalcon micro app (JSON endpoint)"
for W in 4 8; do
    for P in "" "-p"; do
        KEY="turbine_nts_${W}w${P//-/_}"
        save_result phalcon "$KEY" \
            "$(bench_container "nts${P}/${W}w/phalcon" "$TURBINE_IMAGE_NTS" "/" \
                -v /var/www/phalcon:/var/www/html \
                -v "/etc/turbine/phalcon-nts-${W}w${P}.toml:/var/www/html/turbine.toml:ro")"
    done
    save_result phalcon "turbine_zts_${W}w" \
        "$(bench_container "zts/${W}w/phalcon" "$TURBINE_IMAGE_ZTS" "/" \
            -v /var/www/phalcon:/var/www/html \
            -v "/etc/turbine/phalcon-zts-${W}w.toml:/var/www/html/turbine.toml:ro")"
    save_result phalcon "nginx_fpm_${W}w" \
        "$(bench_container "fpm/${W}w/phalcon" "$FPM_IMAGE" "/" \
            -e WORKERS=${W} \
            -e APP_ROOT=/var/www/html \
            -v /var/www/phalcon:/var/www/html)"
done

# ─────────────────────────────────────────────────────────────────────────────
# JSON output — assembled by Python from per-server result files
# ─────────────────────────────────────────────────────────────────────────────
BENCH_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)

python3 - << PYEOF
import json, os

results_dir = '${RESULTS_DIR}'

SERVER_ORDER = [
    'turbine_nts_4w',        'turbine_nts_8w',
    'turbine_nts_4w_p',      'turbine_nts_8w_p',
    'turbine_zts_4w',        'turbine_zts_8w',
    'frankenphp_4w',         'frankenphp_8w',
    'frankenphp_4w_worker',  'frankenphp_8w_worker',
    'nginx_fpm_4w',          'nginx_fpm_8w',
]

SCENARIO_META = {
    'raw_php': {
        'description': 'Single PHP file returning plain-text Hello World',
    },
    'php_scripts': {
        'description': 'Individual scripts: Hello World, 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)',
        'scripts': ['hello.php', 'html_50k.php', 'pdf_50k.php', 'random_50k.php'],
    },
    'laravel': {
        'description': 'Laravel framework, single JSON route, no database',
    },
    'phalcon': {
        'description': 'Phalcon micro application, single JSON route',
        'note': 'FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)',
    },
}

scenarios = {}
for sname, meta in SCENARIO_META.items():
    sdir = os.path.join(results_dir, sname)
    if not os.path.isdir(sdir):
        continue
    s = dict(meta)
    for key in SERVER_ORDER:
        fpath = os.path.join(sdir, key + '.json')
        if os.path.exists(fpath):
            with open(fpath) as f:
                s[key] = json.load(f)
    scenarios[sname] = s

doc = {
    'version': '${VERSION}',
    'date':    '${BENCH_DATE}',
    'server':  'Hetzner CCX33 (8 vCPU dedicated / 32 GB RAM / NVMe)',
    'tool':    'wrk',
    'images': {
        'turbine_nts': '${TURBINE_IMAGE_NTS}',
        'turbine_zts': '${TURBINE_IMAGE_ZTS}',
        'frankenphp':  '${FRANKENPHP_IMAGE}',
    },
    'parameters': {
        'connections':             int('${WRK_CONNECTIONS}'),
        'duration_seconds':        int('${WRK_DURATION}'),
        'workers_4w':              4,
        'workers_8w':              8,
        'memory_limit_mb':         256,
        'max_requests_per_worker': 50000,
    },
    'scenarios': scenarios,
}
print(json.dumps(doc, indent=2))
PYEOF
