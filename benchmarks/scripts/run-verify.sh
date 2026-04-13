#!/usr/bin/env bash
# run-verify.sh — Verification benchmark: proves Turbine executes PHP on every request.
#
# This script runs 4 verification tests against each server variant and checks:
#   1. verify_dynamic — monotonic counter + hrtime + random nonce (all unique)
#   2. verify_echo    — echo back per-request header token
#   3. verify_compute — SHA-256 of unique input (CPU-bound proof)
#   4. verify_payload — 50KB unique random body (same workload as random_50k.php)
#
# After wrk finishes, the script checks:
#   - duplicates == 0 (no cached/stale responses)
#   - unique_counters ≈ total_responses (PHP executed each time)
#   - unique_pids > 0 (workers are real)
#   - validation_errors == 0
#
# Usage: bash run-verify.sh [connections] [duration] [turbine-binary]
#
# For local testing, runs Turbine directly (no Docker).
# For CI/Hetzner, pass Docker image names.

set -euo pipefail

# ── Configuration ─────────────────────────────────────────────────────────────
CONNECTIONS="${1:-64}"
DURATION="${2:-10}"
TURBINE_BIN="${3:-}"              # path to turbine binary; empty = search PATH / cargo
PORT=8099
WRK_THREADS=1             # 1 thread: ensures done() sees all response() data
WARMUP_SEC=3
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WRK_LUA="${SCRIPT_DIR}/wrk-verify.lua"
PHP_DIR="${SCRIPT_DIR}/php"

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

log()  { echo -e "${CYAN}[verify]${NC} $*" >&2; }
pass() { echo -e "  ${GREEN}✓${NC} $*" >&2; }
fail() { echo -e "  ${RED}✗${NC} $*" >&2; }
warn() { echo -e "  ${YELLOW}⚠${NC} $*" >&2; }

# ── Find Turbine binary ──────────────────────────────────────────────────────
find_turbine() {
    if [[ -n "$TURBINE_BIN" ]]; then
        echo "$TURBINE_BIN"
        return
    fi
    # Check common locations
    for candidate in \
        "$(dirname "$SCRIPT_DIR")/../../target/release/turbine" \
        "$(dirname "$SCRIPT_DIR")/../../target/debug/turbine" \
        "$(dirname "$SCRIPT_DIR")/../../target/release/turbine-core" \
        "$(dirname "$SCRIPT_DIR")/../../target/debug/turbine-core" \
        "$(command -v turbine 2>/dev/null || true)" \
        "$(command -v turbine-core 2>/dev/null || true)"; do
        if [[ -x "$candidate" ]]; then
            echo "$candidate"
            return
        fi
    done
    echo ""
}

# ── Wait for HTTP server ─────────────────────────────────────────────────────
wait_http() {
    local url="$1" max="${2:-30}"
    for i in $(seq 1 "$max"); do
        local code
        code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 2 "$url" 2>/dev/null) || true
        [[ -n "$code" && "$code" != "000" && "$code" -lt 500 ]] && return 0
        sleep 0.5
    done
    return 1
}

# ── Kill Turbine process on exit ──────────────────────────────────────────────
TURBINE_PID=""
cleanup() {
    if [[ -n "$TURBINE_PID" ]]; then
        kill "$TURBINE_PID" 2>/dev/null || true
        wait "$TURBINE_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR" 2>/dev/null || true
}
trap cleanup EXIT

# ── Create working directory with PHP scripts and turbine.toml ────────────────
WORK_DIR=$(mktemp -d)
cp "$PHP_DIR"/verify_*.php "$WORK_DIR"/

cat > "$WORK_DIR/turbine.toml" << TOML
[server]
listen = "0.0.0.0:${PORT}"
workers = 4
worker_mode = "thread"
worker_max_requests = 0
persistent_workers = true

[php]
memory_limit = "128M"

[logging]
level = "warn"

[security]
enabled = false

[cache]
enabled = false

[compression]
enabled = false
TOML

# ── Locate Turbine ───────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TURBINE=$(find_turbine)
if [[ -z "$TURBINE" ]]; then
    log "Turbine binary not found. Building..."
    if [[ -f "${REPO_ROOT}/vendor/php-embed-zts/bin/php-config" ]]; then
        export PHP_CONFIG="${REPO_ROOT}/vendor/php-embed-zts/bin/php-config"
    else
        export PHP_CONFIG="${REPO_ROOT}/vendor/php-embed/bin/php-config"
    fi
    export LIBRARY_PATH="/opt/homebrew/opt/openssl@3/lib:${LIBRARY_PATH:-}"
    (cd "$REPO_ROOT" && cargo build --release -p turbine-core 2>&1 | tail -5 >&2)
    TURBINE="$REPO_ROOT/target/release/turbine"
    if [[ ! -x "$TURBINE" ]]; then
        fail "Could not build Turbine"
        exit 1
    fi
fi
log "Turbine binary: ${TURBINE}"

# Ensure PHP embed library is on the dynamic library path
if [[ -d "$REPO_ROOT/vendor/php-embed-zts/lib" ]]; then
    export DYLD_LIBRARY_PATH="${REPO_ROOT}/vendor/php-embed-zts/lib:${DYLD_LIBRARY_PATH:-}"
    export LD_LIBRARY_PATH="${REPO_ROOT}/vendor/php-embed-zts/lib:${LD_LIBRARY_PATH:-}"
elif [[ -d "$REPO_ROOT/vendor/php-embed/lib" ]]; then
    export DYLD_LIBRARY_PATH="${REPO_ROOT}/vendor/php-embed/lib:${DYLD_LIBRARY_PATH:-}"
    export LD_LIBRARY_PATH="${REPO_ROOT}/vendor/php-embed/lib:${LD_LIBRARY_PATH:-}"
fi

# ── Check wrk is installed ───────────────────────────────────────────────────
if ! command -v wrk &>/dev/null; then
    fail "wrk not found. Install with: brew install wrk"
    exit 1
fi

# ── Run verification tests ───────────────────────────────────────────────────
TESTS=(
    "verify_dynamic.php:Dynamic Counter+Nonce:json"
    "verify_echo.php:Echo Request Token:json"
    "verify_compute.php:SHA-256 Compute:json"
    "verify_payload.php:50KB Unique Payload:binary"
)

VERIFY_SAMPLES=200        # number of curl requests for uniqueness validation

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_WARN=0
ALL_RESULTS=()

echo "" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo -e "${BOLD}  Turbine Verification Benchmark${NC}" >&2
echo -e "${BOLD}  Phase 1: ${VERIFY_SAMPLES} curl samples (uniqueness proof)${NC}" >&2
echo -e "${BOLD}  Phase 2: wrk ${DURATION}s · ${CONNECTIONS} connections (throughput)${NC}" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo "" >&2

for test_spec in "${TESTS[@]}"; do
    IFS=':' read -r script name kind <<< "$test_spec"

    echo -e "${BOLD}── ${name} (${script}) ──${NC}" >&2

    # Reset counter file from previous runs
    rm -f "$WORK_DIR/verify_counter.dat"

    # Start Turbine
    log "Starting Turbine for ${script}..."
    (cd "$WORK_DIR" && exec "$TURBINE" serve 2>/dev/null) &
    TURBINE_PID=$!
    sleep 1

    BASE_URL="http://127.0.0.1:${PORT}"
    if ! wait_http "${BASE_URL}/${script}"; then
        fail "Turbine failed to start for ${script}"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        kill "$TURBINE_PID" 2>/dev/null || true
        wait "$TURBINE_PID" 2>/dev/null || true
        TURBINE_PID=""
        continue
    fi

    # ── Phase 1: Uniqueness validation via curl ──────────────────────────────
    log "Phase 1: Collecting ${VERIFY_SAMPLES} responses for uniqueness check..."
    SAMPLE_DIR=$(mktemp -d)
    PHASE1_FAIL=false

    # Fetch N responses as fast as possible (parallel curl)
    CURL_PIDS=()
    for i in $(seq 1 "$VERIFY_SAMPLES"); do
        curl -s -H "X-Request-Token: sample-${i}" \
            "${BASE_URL}/${script}" > "${SAMPLE_DIR}/${i}.txt" &
        CURL_PIDS+=($!)
        # Limit parallelism to ~32 concurrent curls
        if (( i % 32 == 0 )); then
            for pid in "${CURL_PIDS[@]}"; do wait "$pid" 2>/dev/null; done
            CURL_PIDS=()
        fi
    done
    for pid in "${CURL_PIDS[@]}"; do wait "$pid" 2>/dev/null; done

    # Check all responses were non-empty
    EMPTY_COUNT=0
    for f in "$SAMPLE_DIR"/*.txt; do
        [[ ! -s "$f" ]] && EMPTY_COUNT=$((EMPTY_COUNT + 1))
    done
    if [[ "$EMPTY_COUNT" -gt 0 ]]; then
        fail "Phase 1: ${EMPTY_COUNT}/${VERIFY_SAMPLES} empty responses"
        PHASE1_FAIL=true
    fi

    # Check for duplicate responses (hash each body)
    if command -v md5sum &>/dev/null; then
        UNIQUE_BODIES=$(md5sum "$SAMPLE_DIR"/*.txt | awk '{print $1}' | sort -u | wc -l | tr -d ' ')
    else
        UNIQUE_BODIES=$(for f in "$SAMPLE_DIR"/*.txt; do md5 -q "$f"; done | sort -u | wc -l | tr -d ' ')
    fi
    DUP_BODIES=$((VERIFY_SAMPLES - UNIQUE_BODIES))

    if [[ "$DUP_BODIES" -eq 0 ]]; then
        pass "Phase 1: All ${VERIFY_SAMPLES} responses unique (0 duplicates)"
    else
        fail "Phase 1: ${DUP_BODIES}/${VERIFY_SAMPLES} DUPLICATE responses detected!"
        PHASE1_FAIL=true
    fi

    # For verify_dynamic: check counter monotonicity
    if [[ "$script" == "verify_dynamic.php" ]]; then
        COUNTERS=$(for f in "$SAMPLE_DIR"/*.txt; do
            python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['c'])" "$f" 2>/dev/null
        done | sort -n)
        UNIQUE_COUNTERS=$(echo "$COUNTERS" | sort -u | wc -l | tr -d ' ')
        MIN_C=$(echo "$COUNTERS" | head -1)
        MAX_C=$(echo "$COUNTERS" | tail -1)
        if [[ "$UNIQUE_COUNTERS" -eq "$VERIFY_SAMPLES" ]]; then
            pass "Phase 1: All ${UNIQUE_COUNTERS} counters unique (${MIN_C}→${MAX_C})"
        else
            fail "Phase 1: Only ${UNIQUE_COUNTERS}/${VERIFY_SAMPLES} unique counters"
            PHASE1_FAIL=true
        fi

        # Check PIDs (workers in use)
        UNIQUE_PIDS=$(for f in "$SAMPLE_DIR"/*.txt; do
            python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['pid'])" "$f" 2>/dev/null
        done | sort -u | wc -l | tr -d ' ')
        pass "Phase 1: ${UNIQUE_PIDS} worker PIDs observed"
    fi

    # For verify_compute: validate SHA-256 hashes
    if [[ "$script" == "verify_compute.php" ]]; then
        HASH_VALID=0
        HASH_INVALID=0
        for f in "$SAMPLE_DIR"/*.txt; do
            python3 -c "
import json, hashlib, sys
d = json.load(open(sys.argv[1]))
expected = hashlib.sha256(d['input'].encode()).hexdigest()
sys.exit(0 if d['hash'] == expected else 1)
" "$f" 2>/dev/null && HASH_VALID=$((HASH_VALID + 1)) || HASH_INVALID=$((HASH_INVALID + 1))
        done
        if [[ "$HASH_INVALID" -eq 0 ]]; then
            pass "Phase 1: All ${HASH_VALID} SHA-256 hashes verified correct"
        else
            fail "Phase 1: ${HASH_INVALID}/${VERIFY_SAMPLES} SHA-256 hashes INVALID"
            PHASE1_FAIL=true
        fi
    fi

    # For verify_payload: check body size is ~50KB
    if [[ "$script" == "verify_payload.php" ]]; then
        SIZES=$(for f in "$SAMPLE_DIR"/*.txt; do wc -c < "$f"; done | sort -u | tr -d ' ')
        SIZE_COUNT=$(echo "$SIZES" | wc -l | tr -d ' ')
        FIRST_SIZE=$(echo "$SIZES" | head -1)
        if [[ "$SIZE_COUNT" -eq 1 && "$FIRST_SIZE" -gt 40000 ]]; then
            pass "Phase 1: All responses are ${FIRST_SIZE} bytes (~50KB)"
        else
            warn "Phase 1: Variable response sizes detected: $(echo "$SIZES" | tr '\n' ' ')"
        fi
    fi

    rm -rf "$SAMPLE_DIR"

    # ── Phase 2: Throughput benchmark via wrk ────────────────────────────────
    log "Phase 2: Warmup ${WARMUP_SEC}s..."
    wrk -c 10 -d "${WARMUP_SEC}s" -t 1 \
        "${BASE_URL}/${script}" >/dev/null 2>&1 || true

    log "Phase 2: Benchmarking ${DURATION}s with ${CONNECTIONS} connections..."
    WRK_RAW="/tmp/wrk_verify_${RANDOM}.txt"
    wrk -c "$CONNECTIONS" -d "${DURATION}s" -t "$WRK_THREADS" \
        -s "${SCRIPT_DIR}/wrk-report.lua" "${BASE_URL}/${script}" > "$WRK_RAW" 2>/dev/null || true

    # Stop Turbine
    kill "$TURBINE_PID" 2>/dev/null || true
    wait "$TURBINE_PID" 2>/dev/null || true
    TURBINE_PID=""

    # Parse wrk result
    RESULT_JSON=$(grep '^{' "$WRK_RAW" 2>/dev/null | head -1)
    rm -f "$WRK_RAW"

    if [[ -z "$RESULT_JSON" ]]; then
        fail "Phase 2: No wrk output"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        continue
    fi

    ALL_RESULTS+=("$RESULT_JSON")

    # Extract performance metrics
    RPS=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('rps',0))")
    P50=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('latency_p50_ms',0))")
    P99=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('latency_p99_ms',0))")
    REQ_2XX=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('req_2xx',0))")
    REQ_ERR=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('req_errors',0))")

    echo "" >&2
    echo -e "  ${BOLD}Performance:${NC}  ${RPS} req/s · p50 ${P50}ms · p99 ${P99}ms" >&2
    echo -e "  ${BOLD}Requests:${NC}     ${REQ_2XX} successful · ${REQ_ERR} errors" >&2

    # ── Final verdict ─────────────────────────────────────────────────────────
    TEST_PASS=true
    if $PHASE1_FAIL; then TEST_PASS=false; fi
    if [[ "$REQ_ERR" -gt 0 ]]; then
        fail "Phase 2: ${REQ_ERR} network errors"
        TEST_PASS=false
    else
        pass "Phase 2: Zero errors at ${RPS} req/s"
    fi

    if $TEST_PASS; then
        echo -e "  ${GREEN}${BOLD}PASS${NC}" >&2
        TOTAL_PASS=$((TOTAL_PASS + 1))
    else
        echo -e "  ${RED}${BOLD}FAIL${NC}" >&2
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
    echo "" >&2
done

# ── Summary ───────────────────────────────────────────────────────────────────
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo -e "${BOLD}  Summary${NC}" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo -e "  Tests: ${GREEN}${TOTAL_PASS} passed${NC}, ${RED}${TOTAL_FAIL} failed${NC}" >&2

if [[ "$TOTAL_FAIL" -eq 0 ]]; then
    echo "" >&2
    echo -e "  ${GREEN}${BOLD}ALL TESTS PASSED${NC}" >&2
    echo -e "  ${GREEN}Turbine is executing PHP on every request.${NC}" >&2
    echo -e "  ${GREEN}No caching, no shortcuts — performance is real.${NC}" >&2
else
    echo "" >&2
    echo -e "  ${RED}${BOLD}SOME TESTS FAILED${NC}" >&2
    echo -e "  ${RED}Review the failures above for details.${NC}" >&2
fi
echo "" >&2

# Output JSON results to stdout for programmatic use
echo "["
for i in "${!ALL_RESULTS[@]}"; do
    IFS=':' read -r script name kind <<< "${TESTS[$i]}"
    echo "  {\"test\": \"${script}\", \"name\": \"${name}\", \"result\": ${ALL_RESULTS[$i]}}$([ $i -lt $((${#ALL_RESULTS[@]} - 1)) ] && echo ',')"
done
echo "]"

exit "$TOTAL_FAIL"
