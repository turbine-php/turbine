#!/usr/bin/env bash
# run-verify-frankenphp.sh — Same verification benchmark, targeting FrankenPHP.
#
# Usage: bash run-verify-frankenphp.sh [connections] [duration]
#
# Runs the same 4 verification tests against FrankenPHP for direct comparison
# with run-verify.sh (Turbine).

set -euo pipefail

CONNECTIONS="${1:-64}"
DURATION="${2:-10}"
PORT=8098
WRK_THREADS=1
WARMUP_SEC=3
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PHP_DIR="${SCRIPT_DIR}/php"
VERIFY_SAMPLES=200

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

# ── Locate FrankenPHP ─────────────────────────────────────────────────────────
FRANKENPHP=$(command -v frankenphp 2>/dev/null || true)
if [[ -z "$FRANKENPHP" ]]; then
    fail "frankenphp not found. Install with: brew install frankenphp"
    exit 1
fi
log "FrankenPHP binary: ${FRANKENPHP}"
log "Version: $(frankenphp version 2>&1 | head -1)"

# ── Check wrk ─────────────────────────────────────────────────────────────────
if ! command -v wrk &>/dev/null; then
    fail "wrk not found. Install with: brew install wrk"
    exit 1
fi

# ── Create working directory ──────────────────────────────────────────────────
WORK_DIR=$(mktemp -d)
cp "$PHP_DIR"/verify_*.php "$WORK_DIR"/

# Caddyfile for FrankenPHP — 4 worker threads, no TLS, no admin
cat > "$WORK_DIR/Caddyfile" << CADDYEOF
{
    auto_https off
    admin off
    frankenphp {
        num_threads 4
    }
}

:${PORT} {
    root * ${WORK_DIR}
    php_server
}
CADDYEOF

# ── Cleanup ───────────────────────────────────────────────────────────────────
FRANKEN_PID=""
cleanup() {
    if [[ -n "$FRANKEN_PID" ]]; then
        kill "$FRANKEN_PID" 2>/dev/null || true
        wait "$FRANKEN_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR" 2>/dev/null || true
}
trap cleanup EXIT

# ── Test definitions ──────────────────────────────────────────────────────────
TESTS=(
    "verify_dynamic.php:Dynamic Counter+Nonce:json"
    "verify_echo.php:Echo Request Token:json"
    "verify_compute.php:SHA-256 Compute:json"
    "verify_payload.php:50KB Unique Payload:binary"
)

TOTAL_PASS=0
TOTAL_FAIL=0
ALL_RESULTS=()

echo "" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo -e "${BOLD}  FrankenPHP Verification Benchmark${NC}" >&2
echo -e "${BOLD}  Phase 1: ${VERIFY_SAMPLES} curl samples (uniqueness proof)${NC}" >&2
echo -e "${BOLD}  Phase 2: wrk ${DURATION}s · ${CONNECTIONS} connections (throughput)${NC}" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo "" >&2

for test_spec in "${TESTS[@]}"; do
    IFS=':' read -r script name kind <<< "$test_spec"

    echo -e "${BOLD}── ${name} (${script}) ──${NC}" >&2

    # Reset counter file
    rm -f "$WORK_DIR/verify_counter.dat"

    # Start FrankenPHP
    log "Starting FrankenPHP for ${script}..."
    (cd "$WORK_DIR" && exec "$FRANKENPHP" run --config "$WORK_DIR/Caddyfile" 2>/dev/null) &
    FRANKEN_PID=$!
    sleep 2

    BASE_URL="http://127.0.0.1:${PORT}"
    if ! wait_http "${BASE_URL}/${script}"; then
        fail "FrankenPHP failed to start for ${script}"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        kill "$FRANKEN_PID" 2>/dev/null || true
        wait "$FRANKEN_PID" 2>/dev/null || true
        FRANKEN_PID=""
        continue
    fi

    # ── Phase 1: Uniqueness validation via curl ──────────────────────────────
    log "Phase 1: Collecting ${VERIFY_SAMPLES} responses for uniqueness check..."
    SAMPLE_DIR=$(mktemp -d)
    PHASE1_FAIL=false

    CURL_PIDS=()
    for i in $(seq 1 "$VERIFY_SAMPLES"); do
        curl -s -H "X-Request-Token: sample-${i}" \
            "${BASE_URL}/${script}" > "${SAMPLE_DIR}/${i}.txt" &
        CURL_PIDS+=($!)
        if (( i % 32 == 0 )); then
            for pid in "${CURL_PIDS[@]}"; do wait "$pid" 2>/dev/null; done
            CURL_PIDS=()
        fi
    done
    for pid in "${CURL_PIDS[@]}"; do wait "$pid" 2>/dev/null; done

    # Check empty responses
    EMPTY_COUNT=0
    for f in "$SAMPLE_DIR"/*.txt; do
        [[ ! -s "$f" ]] && EMPTY_COUNT=$((EMPTY_COUNT + 1))
    done
    if [[ "$EMPTY_COUNT" -gt 0 ]]; then
        fail "Phase 1: ${EMPTY_COUNT}/${VERIFY_SAMPLES} empty responses"
        PHASE1_FAIL=true
    fi

    # Check duplicates
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

    # verify_dynamic: counter check
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

        UNIQUE_PIDS=$(for f in "$SAMPLE_DIR"/*.txt; do
            python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['pid'])" "$f" 2>/dev/null
        done | sort -u | wc -l | tr -d ' ')
        pass "Phase 1: ${UNIQUE_PIDS} worker PIDs observed"
    fi

    # verify_compute: SHA-256 validation
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

    # verify_payload: size check
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
    WRK_RAW="/tmp/wrk_franken_${RANDOM}.txt"
    wrk -c "$CONNECTIONS" -d "${DURATION}s" -t "$WRK_THREADS" \
        -s "${SCRIPT_DIR}/wrk-report.lua" "${BASE_URL}/${script}" > "$WRK_RAW" 2>/dev/null || true

    # Stop FrankenPHP
    kill "$FRANKEN_PID" 2>/dev/null || true
    wait "$FRANKEN_PID" 2>/dev/null || true
    FRANKEN_PID=""

    RESULT_JSON=$(grep '^{' "$WRK_RAW" 2>/dev/null | head -1)
    rm -f "$WRK_RAW"

    if [[ -z "$RESULT_JSON" ]]; then
        fail "Phase 2: No wrk output"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
        continue
    fi

    ALL_RESULTS+=("$RESULT_JSON")

    RPS=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('rps',0))")
    P50=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('latency_p50_ms',0))")
    P99=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('latency_p99_ms',0))")
    REQ_2XX=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('req_2xx',0))")
    REQ_ERR=$(echo "$RESULT_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('req_errors',0))")

    echo "" >&2
    echo -e "  ${BOLD}Performance:${NC}  ${RPS} req/s · p50 ${P50}ms · p99 ${P99}ms" >&2
    echo -e "  ${BOLD}Requests:${NC}     ${REQ_2XX} successful · ${REQ_ERR} errors" >&2

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
echo -e "${BOLD}  Summary (FrankenPHP)${NC}" >&2
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}" >&2
echo -e "  Tests: ${GREEN}${TOTAL_PASS} passed${NC}, ${RED}${TOTAL_FAIL} failed${NC}" >&2

if [[ "$TOTAL_FAIL" -eq 0 ]]; then
    echo "" >&2
    echo -e "  ${GREEN}${BOLD}ALL TESTS PASSED${NC}" >&2
    echo -e "  ${GREEN}FrankenPHP is executing PHP on every request.${NC}" >&2
else
    echo "" >&2
    echo -e "  ${RED}${BOLD}SOME TESTS FAILED${NC}" >&2
fi
echo "" >&2

# JSON output
echo "["
for i in "${!ALL_RESULTS[@]}"; do
    IFS=':' read -r script name kind <<< "${TESTS[$i]}"
    echo "  {\"test\": \"${script}\", \"name\": \"${name}\", \"server\": \"FrankenPHP\", \"result\": ${ALL_RESULTS[$i]}}$([ $i -lt $((${#ALL_RESULTS[@]} - 1)) ] && echo ',')"
done
echo "]"

exit "$TOTAL_FAIL"
