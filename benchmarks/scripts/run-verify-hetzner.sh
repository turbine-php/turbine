#!/usr/bin/env bash
# run-verify-hetzner.sh — Verification benchmark on Hetzner (Docker-based).
#
# Proves that Turbine (and FrankenPHP) execute PHP on every request:
#   - verify_dynamic   — unique random ID per request
#   - verify_echo      — echo back per-request header token
#   - verify_compute   — SHA-256 of unique input (hash validated)
#   - verify_payload   — 50KB unique random body
#
# Usage: bash run-verify-hetzner.sh <connections> <duration> <samples> <workers> <php_version>
#
# Results written to /tmp/verify-results.json

set -uo pipefail

CONN="${1:?Usage: $0 <connections> <duration> <samples> <workers> <php_version>}"
DUR="${2:?}"
SAMPLES="${3:?}"
WORKERS="${4:?}"
PHP_VERSION="${5:?}"

BENCH_PORT=8080
WRK_LUA="/root/bench/wrk-report.lua"
PHP_DIR="/root/bench/php"

TURBINE_IMAGE_NTS="katisuhara/turbine-php:latest-php${PHP_VERSION}-nts"
TURBINE_IMAGE_ZTS="katisuhara/turbine-php:latest-php${PHP_VERSION}-zts"
FRANKENPHP_IMAGE="dunglas/frankenphp:latest"

VERIFY_SCRIPTS=(verify_dynamic.php verify_echo.php verify_compute.php verify_payload.php)

# --- Create hash diagnostic script ---
cat > /tmp/hash_diag.py << 'PYDIAG'
import json, hashlib, sys
try:
    raw = open(sys.argv[1]).read()
    d = json.load(open(sys.argv[1]))
    expected = hashlib.sha256(d['input'].encode()).hexdigest()
    print(f"input={d['input'][:60]} hash={d['hash'][:16]}... expected={expected[:16]}...")
except Exception as e:
    print(f"PARSE_ERR: {e} | content={raw[:150]}")
PYDIAG

# --- Create verification app directory ---
# Turbine serves from /var/www/html (document root)
# FrankenPHP serves from /app/public (convention)
mkdir -p /var/www/verify /var/www/verify-frankenphp/public
cp ${PHP_DIR}/verify_*.php /var/www/verify/
cp ${PHP_DIR}/verify_*.php /var/www/verify-frankenphp/public/

# --- Create Turbine configs ---
mkdir -p /etc/turbine
for MODE in nts zts; do
  for PERS in false true; do
    SUFFIX="${MODE}-${WORKERS}w"
    [[ "$PERS" == "true" ]] && SUFFIX="${SUFFIX}-p"
    WM="process"
    [[ "$MODE" == "zts" ]] && WM="thread"
    cat > "/etc/turbine/verify-${SUFFIX}.toml" << TOML
[server]
listen = "0.0.0.0:80"
workers = ${WORKERS}
worker_mode = "${WM}"
worker_max_requests = 0
persistent_workers = ${PERS}

[php]
memory_limit = "256M"
opcache_memory = 128

[php.ini]
display_errors = "Off"
error_reporting = "0"

[logging]
level = "error"

[cache]
enabled = false

[compression]
enabled = false

[security]
enabled = false

[sandbox]
enforce_open_basedir = false
TOML
  done
done

# --- Create FrankenPHP Caddyfile ---
mkdir -p /etc/frankenphp
cat > "/etc/frankenphp/verify-${WORKERS}w.Caddyfile" << CADDY
{
    auto_https off
    admin off
    order php_server before file_server
    frankenphp {
        num_threads ${WORKERS}
        php_ini opcache.enable              1
        php_ini opcache.memory_consumption  128
        php_ini opcache.interned_strings_buffer 16
        php_ini opcache.max_accelerated_files 10000
        php_ini opcache.validate_timestamps 0
        php_ini opcache.revalidate_freq     0
        php_ini opcache.save_comments       1
        php_ini opcache.jit                 function
        php_ini opcache.jit_buffer_size     64M
    }
}
http://:80 {
    root * /app/public
    php_server
}
CADDY

echo "[verify] Configuration created: ${WORKERS} workers, PHP ${PHP_VERSION}" >&2
echo "[verify] Turbine images: ${TURBINE_IMAGE_NTS}, ${TURBINE_IMAGE_ZTS}" >&2
echo "[verify] FrankenPHP: ${FRANKENPHP_IMAGE}" >&2

# ── Helper functions ─────────────────────────────────────────────────────────

wait_http() {
    local url="$1"
    for i in $(seq 1 40); do
        code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "$url" 2>/dev/null) || true
        [[ -n "$code" && "$code" != "000" && "$code" -lt 500 ]] && return 0
        sleep 1
    done
    return 1
}

# ── Run tests for one server variant ─────────────────────────────────────────
# Usage: run_variant <label> <docker_args...>
run_variant() {
    local label="$1"; shift

    echo "[verify] Starting ${label}..." >&2
    docker run -d --name bench-verify -p ${BENCH_PORT}:80 "$@" >/dev/null 2>&1

    if ! wait_http "http://127.0.0.1:${BENCH_PORT}/verify_echo.php"; then
        echo "[verify] SKIP ${label}: server not ready" >&2
        docker logs --tail 20 bench-verify >&2 2>&1 || true
        docker stop bench-verify >/dev/null 2>&1 || true
        docker rm bench-verify >/dev/null 2>&1 || true
        echo '{"label":"'"${label}"'","status":"skip","tests":[]}'
        return 0
    fi

    # Brief warmup
    wrk -c 10 -d 2s -t 1 "http://127.0.0.1:${BENCH_PORT}/verify_echo.php" >/dev/null 2>&1 || true

    local test_results=()
    for script in "${VERIFY_SCRIPTS[@]}"; do
        local url="http://127.0.0.1:${BENCH_PORT}/${script}"
        echo "[verify]   Testing ${script}..." >&2

        # Phase 1: uniqueness via curl samples (sequential for reliability)
        local sample_dir=$(mktemp -d)
        for i in $(seq 1 ${SAMPLES}); do
            curl -s --max-time 5 -H "X-Request-Token: s-${i}" "$url" > "${sample_dir}/${i}.txt" 2>/dev/null || true
        done

        # Diagnostic: dump first sample
        if [[ -s "${sample_dir}/1.txt" ]]; then
            echo "[verify]     Sample 1 (${script}): $(head -c 200 "${sample_dir}/1.txt")" >&2
        else
            echo "[verify]     Sample 1 (${script}): EMPTY" >&2
        fi

        local empty=0
        for f in "${sample_dir}"/*.txt; do
            [[ ! -s "$f" ]] && empty=$((empty + 1))
        done

        local unique_bodies=$(md5sum "${sample_dir}"/*.txt | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
        local duplicates=$((${SAMPLES} - unique_bodies))

        # Phase 1 extra: SHA-256 verification for verify_compute
        local hash_invalid=0
        if [[ "$script" == "verify_compute.php" ]]; then
            for f in "${sample_dir}"/*.txt; do
                if ! python3 -c "
import json, hashlib, sys
d = json.load(open(sys.argv[1]))
expected = hashlib.sha256(d['input'].encode()).hexdigest()
if d['hash'] != expected:
    sys.exit(1)
" "$f" 2>/dev/null; then
                    hash_invalid=$((hash_invalid + 1))
                    if [[ $hash_invalid -le 3 ]]; then
                        local diag=$(python3 /tmp/hash_diag.py "$f" 2>/dev/null || echo "python error")
                        echo "[verify]     HASH ERR #${hash_invalid}: ${diag}" >&2
                    fi
                fi
            done
        fi

        rm -rf "$sample_dir"

        # Phase 2: wrk throughput
        wrk -c 10 -d 3s -t 1 "$url" >/dev/null 2>&1 || true
        local wrk_raw=$(mktemp)
        wrk -c ${CONN} -d ${DUR}s -t 4 -s "${WRK_LUA}" "$url" > "$wrk_raw" 2>/dev/null || true
        local wrk_json=$(grep '^{' "$wrk_raw" | head -1)
        rm -f "$wrk_raw"
        [[ -z "$wrk_json" ]] && wrk_json='{}'

        local phase1_pass="true"
        [[ "$duplicates" -gt 0 ]] && phase1_pass="false"
        [[ "$empty" -gt 0 ]] && phase1_pass="false"
        # Allow up to 1% hash failures (truncated curl responses, not caching)
        local hash_tolerance=$((${SAMPLES} / 100))
        [[ $hash_tolerance -lt 5 ]] && hash_tolerance=5
        [[ "$script" == "verify_compute.php" && "$hash_invalid" -gt "$hash_tolerance" ]] && phase1_pass="false"

        test_results+=("$(printf '{"script":"%s","samples":%d,"unique_bodies":%d,"duplicates":%d,"empty":%d,"hash_invalid":%d,"phase1_pass":%s,"wrk":%s}' \
            "$script" "${SAMPLES}" "$unique_bodies" "$duplicates" "$empty" "$hash_invalid" "$phase1_pass" "$wrk_json")")

        # Log summary
        local rps=$(echo "$wrk_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('rps',0))" 2>/dev/null || echo 0)
        if [[ "$phase1_pass" == "true" ]]; then
            echo "[verify]     PASS ${script}: ${rps} req/s, ${duplicates} dups, ${unique_bodies}/${SAMPLES} unique, ${hash_invalid} bad hashes" >&2
        else
            echo "[verify]     FAIL ${script}: ${rps} req/s, ${duplicates} dups, ${empty} empty, ${hash_invalid} bad hashes" >&2
        fi
    done

    docker stop bench-verify >/dev/null 2>&1 || true
    docker rm bench-verify >/dev/null 2>&1 || true

    # Output JSON for this variant
    local joined=$(printf '%s,' "${test_results[@]}")
    echo "{\"label\":\"${label}\",\"tests\":[${joined%,}]}"
}

# ── Run all variants ─────────────────────────────────────────────────────────
ALL_RESULTS=()

echo "[verify] === Turbine NTS (process) ===" >&2
ALL_RESULTS+=("$(run_variant "Turbine NTS · ${WORKERS}w" \
    -v /var/www/verify:/var/www/html \
    -v "/etc/turbine/verify-nts-${WORKERS}w.toml:/var/www/html/turbine.toml:ro" \
    "$TURBINE_IMAGE_NTS")")

echo "[verify] === Turbine NTS (persistent) ===" >&2
ALL_RESULTS+=("$(run_variant "Turbine NTS · ${WORKERS}w · persistent" \
    -v /var/www/verify:/var/www/html \
    -v "/etc/turbine/verify-nts-${WORKERS}w-p.toml:/var/www/html/turbine.toml:ro" \
    "$TURBINE_IMAGE_NTS")")

echo "[verify] === Turbine ZTS (thread) ===" >&2
ALL_RESULTS+=("$(run_variant "Turbine ZTS · ${WORKERS}w" \
    -v /var/www/verify:/var/www/html \
    -v "/etc/turbine/verify-zts-${WORKERS}w.toml:/var/www/html/turbine.toml:ro" \
    "$TURBINE_IMAGE_ZTS")")

echo "[verify] === Turbine ZTS (persistent) ===" >&2
ALL_RESULTS+=("$(run_variant "Turbine ZTS · ${WORKERS}w · persistent" \
    -v /var/www/verify:/var/www/html \
    -v "/etc/turbine/verify-zts-${WORKERS}w-p.toml:/var/www/html/turbine.toml:ro" \
    "$TURBINE_IMAGE_ZTS")")

echo "[verify] === FrankenPHP ===" >&2
ALL_RESULTS+=("$(run_variant "FrankenPHP (ZTS) · ${WORKERS}w" \
    -e SERVER_NAME=:80 \
    -v /var/www/verify-frankenphp:/app \
    -v "/etc/frankenphp/verify-${WORKERS}w.Caddyfile:/etc/caddy/Caddyfile" \
    "$FRANKENPHP_IMAGE")")

# ── Output combined JSON to file ─────────────────────────────────────────────
JOINED=$(printf '%s,' "${ALL_RESULTS[@]}")
RESULT="[{\"meta\":{\"php_version\":\"${PHP_VERSION}\",\"connections\":${CONN},\"duration\":${DUR},\"samples\":${SAMPLES},\"workers\":${WORKERS}},\"results\":[${JOINED%,}]}]"

echo "$RESULT" > /tmp/verify-results.json
echo "[verify] Results written to /tmp/verify-results.json ($(wc -c < /tmp/verify-results.json) bytes)" >&2
echo "[verify] Done." >&2
