#!/usr/bin/env bash
# run.sh — Execute all benchmarks and output JSON results to stdout
#
# Usage: bash run.sh [version] [image-tag] [connections] [duration]
#
#   version     — label for the results (default: dev)
#   image-tag   — Docker image tag suffix (default: latest)
#                 e.g. "latest" → katisuhara/turbine-php:latest-php8.4-{nts,zts}
#                      "v0.2.0-php8.4-nts" → used directly as NTS tag
#   connections — wrk concurrent connections (default: 100)
#   duration    — wrk duration in seconds per run (default: 30)

set -euo pipefail

VERSION="${1:-dev}"
IMAGE_TAG="${2:-latest}"
WRK_CONNECTIONS="${3:-100}"
WRK_DURATION="${4:-30}"

# ── wrk parameters ───────────────────────────────────────────────────────────
WRK_THREADS=8
WARMUP_DURATION=5
BENCH_PORT=8080

# Resolve image names: if IMAGE_TAG already contains 'nts'/'zts', use it as-is;
# otherwise treat it as a prefix for the standard naming convention.
if echo "$IMAGE_TAG" | grep -q "nts\|zts"; then
    TURBINE_IMAGE_NTS="katisuhara/turbine-php:${IMAGE_TAG}"
    TURBINE_IMAGE_ZTS="katisuhara/turbine-php:${IMAGE_TAG/nts/zts}"
else
    TURBINE_IMAGE_NTS="katisuhara/turbine-php:${IMAGE_TAG}-php8.4-nts"
    TURBINE_IMAGE_ZTS="katisuhara/turbine-php:${IMAGE_TAG}-php8.4-zts"
fi

# ── Helpers ──────────────────────────────────────────────────────────────────
log() { echo "[bench] $*" >&2; }

# Parse wrk text output into a JSON object
# Fields: rps, latency (avg), transfer
parse_wrk() {
    local raw="$1"
    local rps lat trf reqs
    rps=$(echo  "$raw" | grep -oP  'Requests/sec:\s+\K[\d.]+' || echo "0")
    lat=$(echo  "$raw" | grep -oP  'Latency\s+\K\S+'  | head -1 || echo "N/A")
    trf=$(echo  "$raw" | grep -oP  'Transfer/sec:\s+\K\S+' || echo "0")
    reqs=$(echo "$raw" | grep -oP  '(\d+) requests in' | grep -oP '^\d+' || echo "0")
    printf '{"rps":"%s","latency":"%s","transfer":"%s","total_requests":"%s"}' \
        "$rps" "$lat" "$trf" "$reqs"
}

# Run wrk and return parsed JSON
run_wrk() {
    local label="$1"
    local url="$2"
    log "  Warmup   ${label}..."
    wrk -t4 -c50 -d${WARMUP_DURATION}s "$url" >/dev/null 2>&1 || true
    log "  Benchmarking ${label} (${WRK_DURATION}s)..."
    local raw
    raw=$(wrk -t${WRK_THREADS} -c${WRK_CONNECTIONS} -d${WRK_DURATION}s "$url" 2>&1)
    log "  $(echo "$raw" | grep 'Requests/sec')"
    parse_wrk "$raw"
}

# Start a Turbine Docker container, run benchmark, stop container
bench_turbine() {
    local label="$1"
    local image="$2"
    local app_dir="$3"
    local toml="$4"
    local url="http://127.0.0.1:${BENCH_PORT}/"

    log "Starting ${label} container..."
    docker run -d --name turbine-bench \
        -p "${BENCH_PORT}:80" \
        -e PORT=80 \
        -v "${app_dir}:/var/www/html:ro" \
        -v "${toml}:/var/www/html/turbine.toml:ro" \
        "$image" >/dev/null

    # Wait for port to be accepting connections
    for i in $(seq 1 20); do
        curl -sf "$url" >/dev/null 2>&1 && break
        sleep 1
    done

    local result
    result=$(run_wrk "$label" "$url")

    docker stop turbine-bench  >/dev/null
    docker rm   turbine-bench  >/dev/null
    echo "$result"
}

# Benchmark Nginx + PHP-FPM (already running, fixed ports per scenario)
bench_fpm() {
    local label="$1"
    local port="$2"
    local url="http://127.0.0.1:${port}/"
    run_wrk "$label" "$url"
}

# ── Raw PHP ──────────────────────────────────────────────────────────────────
log "==> Scenario: Raw PHP (Hello World)"
RAW_NTS=$(bench_turbine \
    "turbine-nts/raw" \
    "$TURBINE_IMAGE_NTS" \
    "/var/www/raw" \
    "/etc/turbine/raw.toml")

RAW_ZTS=$(bench_turbine \
    "turbine-zts/raw" \
    "$TURBINE_IMAGE_ZTS" \
    "/var/www/raw" \
    "/etc/turbine/raw.toml")

RAW_FPM=$(bench_fpm "nginx-fpm/raw" 8803)

# ── Laravel ──────────────────────────────────────────────────────────────────
log "==> Scenario: Laravel (JSON endpoint)"
LARAVEL_NTS=$(bench_turbine \
    "turbine-nts/laravel" \
    "$TURBINE_IMAGE_NTS" \
    "/var/www/laravel" \
    "/etc/turbine/laravel.toml")

LARAVEL_ZTS=$(bench_turbine \
    "turbine-zts/laravel" \
    "$TURBINE_IMAGE_ZTS" \
    "/var/www/laravel" \
    "/etc/turbine/laravel.toml")

LARAVEL_FPM=$(bench_fpm "nginx-fpm/laravel" 8813)

# ── Phalcon ──────────────────────────────────────────────────────────────────
log "==> Scenario: Phalcon micro app (JSON endpoint)"
PHALCON_NTS=$(bench_turbine \
    "turbine-nts/phalcon" \
    "$TURBINE_IMAGE_NTS" \
    "/var/www/phalcon" \
    "/etc/turbine/phalcon-nts.toml")

PHALCON_ZTS=$(bench_turbine \
    "turbine-zts/phalcon" \
    "$TURBINE_IMAGE_ZTS" \
    "/var/www/phalcon" \
    "/etc/turbine/phalcon-zts.toml")

PHALCON_FPM=$(bench_fpm "nginx-fpm/phalcon" 8823)

# ── Output JSON ──────────────────────────────────────────────────────────────
cat << JSONEOF
{
  "version": "$VERSION",
  "date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "server": "Hetzner CPX41",
  "tool": "wrk",
  "parameters": {
    "threads": $WRK_THREADS,
    "connections": $WRK_CONNECTIONS,
    "duration_seconds": $WRK_DURATION,
    "turbine_image_nts": "$TURBINE_IMAGE_NTS",
    "turbine_image_zts": "$TURBINE_IMAGE_ZTS"
  },
  "scenarios": {
    "raw_php": {
      "description": "Single PHP file returning a plain-text Hello World response",
      "turbine_nts": $RAW_NTS,
      "turbine_zts": $RAW_ZTS,
      "nginx_fpm":   $RAW_FPM
    },
    "laravel": {
      "description": "Laravel application returning a JSON response (no database)",
      "turbine_nts": $LARAVEL_NTS,
      "turbine_zts": $LARAVEL_ZTS,
      "nginx_fpm":   $LARAVEL_FPM
    },
    "phalcon": {
      "description": "Phalcon micro application returning a JSON response",
      "turbine_nts": $PHALCON_NTS,
      "turbine_zts": $PHALCON_ZTS,
      "nginx_fpm":   $PHALCON_FPM
    }
  }
}
JSONEOF
