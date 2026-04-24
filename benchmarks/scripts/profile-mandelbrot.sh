#!/usr/bin/env bash
# profile-mandelbrot.sh — Record a samply profile of Turbine serving a
# CPU-bound Mandelbrot PHP script.
#
# Output: a samply capture that opens in the Firefox Profiler (UI web).
#
# Usage:
#   bash benchmarks/scripts/profile-mandelbrot.sh [MODE] [DURATION_SEC] [CONNECTIONS] [THREADS]
#
# MODE: "nts" (default) or "zts". Picks vendor/php-embed{,-zts} and
#       worker_mode = process|thread accordingly.
# Defaults: nts, 20s, 32 connections, 4 wrk threads.
#
# Requires: samply (`cargo install --locked samply`) and wrk (`brew install wrk`).

set -euo pipefail

MODE="${1:-nts}"
DURATION="${2:-20}"
CONNECTIONS="${3:-32}"
THREADS="${4:-4}"
PORT=8099

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

# ── Pick PHP build based on MODE ─────────────────────────────────────────────
case "$MODE" in
    nts)
        PHP_PREFIX="$REPO_ROOT/vendor/php-embed"
        WORKER_MODE="process"
        ;;
    zts)
        PHP_PREFIX="$REPO_ROOT/vendor/php-embed-zts"
        WORKER_MODE="thread"
        ;;
    *)
        echo "MODE must be 'nts' or 'zts' (got: $MODE)"
        exit 1
        ;;
esac

[[ -x "$PHP_PREFIX/bin/php-config" ]] || {
    echo "php-config not found at $PHP_PREFIX/bin/php-config"
    exit 1
}

export PHP_CONFIG="$PHP_PREFIX/bin/php-config"
export LIBRARY_PATH="/opt/homebrew/opt/openssl@3/lib:${LIBRARY_PATH:-}"
# Ensure the dyld loader finds libphp at runtime (embed SAPI is a dylib).
export DYLD_LIBRARY_PATH="$PHP_PREFIX/lib:${DYLD_LIBRARY_PATH:-}"
export DYLD_FALLBACK_LIBRARY_PATH="$PHP_PREFIX/lib:${DYLD_FALLBACK_LIBRARY_PATH:-}"

# ── Preflight ────────────────────────────────────────────────────────────────
command -v samply >/dev/null || { echo "samply not found. Install: cargo install --locked samply"; exit 1; }
command -v wrk    >/dev/null || { echo "wrk not found. Install: brew install wrk"; exit 1; }

# ── Build profiling binary ───────────────────────────────────────────────────
# IMPORTANT: the bindings crate (turbine-php-sys) reads PHP_CONFIG at build
# time and links against *that* libphp. We rebuild per-mode into a separate
# target dir so nts/zts artifacts don't clobber each other.
TARGET_DIR="$REPO_ROOT/target/profiling-$MODE"
export CARGO_TARGET_DIR="$TARGET_DIR"

echo "[profile:$MODE] PHP_CONFIG = $PHP_CONFIG"
echo "[profile:$MODE] CARGO_TARGET_DIR = $CARGO_TARGET_DIR"
echo "[profile:$MODE] Building turbine with profile=profiling..."
cargo build --profile profiling -p turbine-core --bin turbine

BIN="$CARGO_TARGET_DIR/profiling/turbine"
[[ -x "$BIN" ]] || { echo "binary not found at $BIN"; exit 1; }

# ── Workspace: mandelbrot.php + turbine.toml ─────────────────────────────────
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

cat > "$WORK_DIR/mandelbrot.php" <<'PHP'
<?php
// Pure-PHP CPU loop — exercises the executor, not I/O.
function mandelbrot(int $iters = 300): int {
    $sum = 0;
    for ($y = -1.0; $y < 1.0; $y += 0.1) {
        for ($x = -2.0; $x < 1.0; $x += 0.08) {
            $cr = $x; $ci = $y;
            $zr = 0.0; $zi = 0.0;
            $i = 0;
            while ($i < $iters && ($zr * $zr + $zi * $zi) < 4.0) {
                $tmp = $zr * $zr - $zi * $zi + $cr;
                $zi = 2 * $zr * $zi + $ci;
                $zr = $tmp;
                $i++;
            }
            $sum += $i;
        }
    }
    return $sum;
}
header('Content-Type: text/plain');
echo mandelbrot();
PHP

# Diagnostics endpoint — reports opcache + JIT status so we can verify
# the interpreter isn't silently running without JIT in ZTS.
cat > "$WORK_DIR/jitstatus.php" <<'PHP'
<?php
header('Content-Type: application/json');
$out = [
    'sapi'         => PHP_SAPI,
    'zts'          => (bool) PHP_ZTS,
    'opcache_ext'  => extension_loaded('Zend OPcache'),
];
if (function_exists('opcache_get_status')) {
    $s = opcache_get_status(false);
    $out['opcache_enabled'] = $s['opcache_enabled'] ?? false;
    $out['jit_enabled']     = $s['jit']['enabled']   ?? false;
    $out['jit_on']          = $s['jit']['on']        ?? false;
    $out['jit_kind']        = $s['jit']['kind']      ?? null;
    $out['jit_buffer_size'] = $s['jit']['buffer_size'] ?? 0;
    $out['jit_buffer_free'] = $s['jit']['buffer_free'] ?? 0;
} else {
    $out['opcache_get_status'] = 'unavailable';
}
echo json_encode($out, JSON_PRETTY_PRINT);
PHP

cat > "$WORK_DIR/turbine.toml" <<TOML
[server]
listen = "127.0.0.1:${PORT}"
workers = 4
worker_mode = "${WORKER_MODE}"
persistent_workers = true
worker_max_requests = 0

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

URL="http://127.0.0.1:${PORT}/mandelbrot.php"
PROFILE_OUT="$REPO_ROOT/target/profile-mandelbrot-$MODE.json.gz"

echo "[profile:$MODE] URL: $URL"
echo "[profile:$MODE] worker_mode=$WORKER_MODE, workers=4"
echo "[profile:$MODE] Warmup 3s, record ${DURATION}s @ ${CONNECTIONS} conn / ${THREADS} wrk threads"

# Launch under samply. --save-only writes the profile to disk instead of
# auto-opening the UI. --rate 1999 avoids aliasing with common periodic
# events (1000 Hz timer, etc.).
samply record \
    --save-only \
    --output "$PROFILE_OUT" \
    --rate 1999 \
    -- \
    "$BIN" serve --config "$WORK_DIR/turbine.toml" --root "$WORK_DIR" \
    > "$WORK_DIR/turbine.log" 2>&1 &

SAMPLY_PID=$!

# Wait for HTTP — and verify we actually get a 200 from the target URL,
# not just any response. Otherwise we'd silently benchmark a 404 loop.
READY=0
for i in $(seq 1 40); do
    code=$(curl -s -o /dev/null -w "%{http_code}" -m 2 "$URL" 2>/dev/null || echo "000")
    if [[ "$code" == "200" ]]; then
        READY=1
        break
    fi
    if ! kill -0 "$SAMPLY_PID" 2>/dev/null; then
        echo "[profile:$MODE] samply/turbine died during startup. Log:"
        tail -40 "$WORK_DIR/turbine.log"
        exit 1
    fi
    sleep 0.5
done

if [[ "$READY" != "1" ]]; then
    echo "[profile:$MODE] Could not get HTTP 200 from $URL (last code: $code). Log:"
    tail -40 "$WORK_DIR/turbine.log"
    kill -INT "$SAMPLY_PID" 2>/dev/null || true
    wait "$SAMPLY_PID" 2>/dev/null || true
    exit 1
fi

# Sanity check: response must be non-empty numeric mandelbrot output.
body_len=$(curl -s "$URL" | wc -c | tr -d ' ')
if [[ "$body_len" -lt 3 ]]; then
    echo "[profile:$MODE] /mandelbrot.php returned unexpectedly short body ($body_len bytes). Aborting."
    kill -INT "$SAMPLY_PID" 2>/dev/null || true
    wait "$SAMPLY_PID" 2>/dev/null || true
    exit 1
fi
echo "[profile:$MODE] Sanity check ok: mandelbrot returned $body_len bytes."

# Diagnostics — JIT/opcache status for the worker that handled the request.
echo "[profile:$MODE] --- Worker JIT/opcache status ---"
curl -s "http://127.0.0.1:${PORT}/jitstatus.php"
echo ""
echo "[profile:$MODE] ---------------------------------"

echo "[profile:$MODE] Server up. Warmup..."
wrk -t2 -c8 -d3s "$URL" >/dev/null

echo "[profile:$MODE] Recording..."
wrk -t"$THREADS" -c"$CONNECTIONS" -d"${DURATION}s" --latency "$URL"

echo "[profile:$MODE] Stopping turbine..."
# IMPORTANT: signal the turbine process (samply's child), NOT samply itself.
# If we kill samply, it aborts without writing the profile. When turbine
# exits cleanly, samply detects the child exit and finalises the capture.
TURBINE_PID=$(pgrep -P "$SAMPLY_PID" -f "turbine" | head -1 || true)
if [[ -z "$TURBINE_PID" ]]; then
    # Fall back to any descendant (samply may wrap via a shell on some OSes)
    TURBINE_PID=$(pgrep -f "target/profiling-$MODE/profiling/turbine" | head -1 || true)
fi

if [[ -n "$TURBINE_PID" ]]; then
    kill -TERM "$TURBINE_PID" 2>/dev/null || true
fi

# Wait up to 15s for samply to finalise the capture and exit.
for i in $(seq 1 30); do
    kill -0 "$SAMPLY_PID" 2>/dev/null || break
    sleep 0.5
done

if kill -0 "$SAMPLY_PID" 2>/dev/null; then
    echo "[profile:$MODE] samply still alive after 15s — escalating."
    # As a last resort: kill turbine harder, then samply.
    [[ -n "$TURBINE_PID" ]] && kill -KILL "$TURBINE_PID" 2>/dev/null || true
    sleep 1
    if kill -0 "$SAMPLY_PID" 2>/dev/null; then
        kill -TERM "$SAMPLY_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$SAMPLY_PID" 2>/dev/null || true
    fi
fi
wait "$SAMPLY_PID" 2>/dev/null || true

if [[ ! -s "$PROFILE_OUT" ]]; then
    echo "[profile:$MODE] Profile file missing or empty: $PROFILE_OUT"
    echo "[profile:$MODE] Turbine log tail:"
    tail -30 "$WORK_DIR/turbine.log"
    exit 1
fi

echo ""
echo "[profile:$MODE] ✓ Done. Profile: $PROFILE_OUT"
echo "[profile:$MODE] Open it with:"
echo "    samply load \"$PROFILE_OUT\""
