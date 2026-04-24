#!/usr/bin/env bash
# profile-docker.sh — Build and run a local profiling image of Turbine,
# capture a samply CPU profile, and write it to ./profile-out/.
#
# Output: ./profile-out/profile-<mode>-<scenario>-p<persistent>.json.gz
#         Open at https://profiler.firefox.com
#
# Usage:
#   bash scripts/profile-docker.sh [OPTIONS via env]
#
# Options:
#   MODE=nts|zts              (default: zts)
#   PHP_VERSION=8.5.1         (default: 8.5.1)
#   SCENARIO=mandelbrot|pdf50k|helloworld   (default: mandelbrot)
#   PERSIST=true|false        (default: true)
#   JIT_PERF_MAP=true|false   (default: true)
#   DURATION=30               (seconds of wrk)
#   CONNECTIONS=64
#   THREADS=4                 (wrk threads)
#   WORKERS=4                 (turbine workers)
#   RATE=1999                 (samply Hz)
#   PORT=8080                 (host port)
#   SKIP_BUILD=1              (skip docker build if image already exists)
#   IMAGE_TAG=turbine-profile (final tag = ${IMAGE_TAG}:${MODE})
#
# Requirements: docker + wrk (`brew install wrk`).

set -euo pipefail

MODE="${MODE:-zts}"
PHP_VERSION="${PHP_VERSION:-8.5.1}"
SCENARIO="${SCENARIO:-mandelbrot}"
PERSIST="${PERSIST:-true}"
JIT_PERF_MAP="${JIT_PERF_MAP:-true}"
DURATION="${DURATION:-30}"
CONNECTIONS="${CONNECTIONS:-64}"
THREADS="${THREADS:-4}"
WORKERS="${WORKERS:-4}"
RATE="${RATE:-1999}"
PORT="${PORT:-8080}"
SKIP_BUILD="${SKIP_BUILD:-0}"
IMAGE_TAG="${IMAGE_TAG:-turbine-profile}"

IMG="${IMAGE_TAG}:${MODE}"
CONTAINER="turbine-prof-local"
TAG="${MODE}-${SCENARIO}-p${PERSIST}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT_DIR="${REPO_ROOT}/profile-out"
WORK_DIR="$(mktemp -d -t turbine-prof.XXXXXX)"
mkdir -p "$OUT_DIR"

WM="process"
[[ "$MODE" == "zts" ]] && WM="thread"

cleanup() {
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT INT TERM

echo "==> Config"
printf '  %-16s %s\n' \
    image        "$IMG" \
    scenario     "$SCENARIO" \
    mode         "$MODE" \
    worker_mode  "$WM" \
    persistent   "$PERSIST" \
    workers      "$WORKERS" \
    jit_perf_map "$JIT_PERF_MAP" \
    duration     "${DURATION}s" \
    wrk          "${THREADS}t x ${CONNECTIONS}c" \
    rate         "${RATE}Hz" \
    output       "$OUT_DIR/profile-${TAG}.json.gz"

# ── Build the profiling image (natively for current arch) ──────────────────
if [[ "$SKIP_BUILD" != "1" ]] || ! docker image inspect "$IMG" >/dev/null 2>&1; then
    echo "==> Building $IMG (PHP $PHP_VERSION, $MODE)"
    ZTS="0"
    [[ "$MODE" == "zts" ]] && ZTS="1"
    docker build \
        -t "$IMG" \
        -f "$REPO_ROOT/docker/Dockerfile.profile" \
        --build-arg "PHP_VERSION=${PHP_VERSION}" \
        --build-arg "ZTS=${ZTS}" \
        "$REPO_ROOT"
else
    echo "==> Reusing existing image $IMG (SKIP_BUILD=1)"
fi

# ── Write turbine.toml override (per-run settings) ─────────────────────────
APP_DIR="$WORK_DIR/app"
mkdir -p "$APP_DIR"

JIT_INI=""
if [[ "$JIT_PERF_MAP" == "true" ]]; then
    JIT_INI=$'\n[php.ini]\n"opcache.jit_debug" = "0x20"\n'
fi

cat > "$APP_DIR/turbine.toml" << TOML
[server]
listen = "0.0.0.0:80"
workers = ${WORKERS}
worker_mode = "${WM}"
persistent_workers = ${PERSIST}
worker_max_requests = 0

[php]
memory_limit = "128M"
opcache_memory = 128

[logging]
level = "warn"

[security]
enabled = false

[cache]
enabled = false

[compression]
enabled = false
${JIT_INI}
TOML

# ── Start Turbine ──────────────────────────────────────────────────────────
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true

# Docker Desktop (macOS/Windows) runs a LinuxKit VM whose kernel has
# perf_event_paranoid=2 by default, which blocks samply's perf events.
# The canonical fix is to write 1 (or -1) to the sysctl *on the VM*.
# A privileged container with --pid=host sees the VM's /proc/sys and can
# do exactly that. Linux host: this is also fine (real host sysctl).
echo "==> Relaxing perf_event_paranoid on docker VM (one-shot)"
docker run --rm --privileged --pid=host alpine:3 sh -c '
  cur=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo ?)
  if [ "$cur" != "1" ] && [ "$cur" != "0" ] && [ "$cur" != "-1" ]; then
    echo 1 > /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true
    echo 0 > /proc/sys/kernel/kptr_restrict       2>/dev/null || true
  fi
  echo "perf_event_paranoid = $(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo ?)"
' || echo "::warning:: could not relax perf_event_paranoid — samply may fail"

# Bind /tmp from a host dir instead of --tmpfs: `docker cp` on Docker
# Desktop for Mac cannot read tmpfs mounts (shows "Could not find the
# file" even though `docker exec ls` sees it). A host bind gives us
# direct access — we just cp from the host dir at the end. Bonus: perf
# maps + jit dumps land directly on the host, no cp needed.
TMP_SHARED="$WORK_DIR/tmp"
mkdir -p "$TMP_SHARED"
chmod 1777 "$TMP_SHARED"

echo "==> Starting $CONTAINER"
docker run -d --name "$CONTAINER" \
    --cap-add SYS_PTRACE \
    --cap-add PERFMON \
    --cap-add SYS_ADMIN \
    --security-opt seccomp=unconfined \
    -p "${PORT}:80" \
    -v "$APP_DIR/turbine.toml:/var/www/html/turbine.toml:ro" \
    -v "$TMP_SHARED:/tmp" \
    "$IMG" >/dev/null

# /proc/sys/kernel/perf_event_paranoid is read-only in Docker Desktop — skip.
# CAP_PERFMON + seccomp=unconfined is usually enough for samply's perf events.

code="000"
for i in $(seq 1 60); do
    code=$(curl -s -o /dev/null -w "%{http_code}" -m 2 "http://127.0.0.1:${PORT}/${SCENARIO}.php" || echo 000)
    [[ "$code" == "200" ]] && break
    sleep 0.5
done
if [[ "$code" != "200" ]]; then
    echo "::error:: Turbine did not return 200 (last: $code)" >&2
    docker logs --tail 80 "$CONTAINER" || true
    exit 1
fi
echo "==> jitstatus: $(curl -s http://127.0.0.1:${PORT}/jitstatus.php)"

# ── Warmup ─────────────────────────────────────────────────────────────────
if ! command -v wrk >/dev/null; then
    echo "::error:: wrk not found. Install with: brew install wrk" >&2
    exit 1
fi
echo "==> Warmup (5s)"
wrk -t2 -c8 -d5s "http://127.0.0.1:${PORT}/${SCENARIO}.php" >/dev/null

# ── Record profile ─────────────────────────────────────────────────────────
TURB_PID=$(docker exec "$CONTAINER" sh -c "pgrep -f 'turbine .*serve' | head -1")
if [[ -z "$TURB_PID" ]]; then
    echo "::error:: Could not locate turbine PID inside container" >&2
    docker logs --tail 80 "$CONTAINER" || true
    exit 1
fi
echo "==> samply attach to container PID $TURB_PID"

OUT_IN_CONTAINER="/tmp/profile-${TAG}.json.gz"
docker exec -d "$CONTAINER" sh -c \
    "samply record -v --jit-markers --save-only --output '${OUT_IN_CONTAINER}' \
        --rate ${RATE} --duration $((DURATION + 5)) \
        -p ${TURB_PID} > /tmp/samply.log 2>&1"

sleep 1

# From here on: never abort on a non-zero exit. We want to harvest every
# file we can and print diagnostics if anything is missing. Previously an
# unexpected 130 (SIGPIPE/SIGINT propagated from wrk|tee or a docker exec)
# was silently killing the script before the profile was copied out.
set +e

echo "==> wrk -t${THREADS} -c${CONNECTIONS} -d${DURATION}s http://127.0.0.1:${PORT}/${SCENARIO}.php"
wrk -t"${THREADS}" -c"${CONNECTIONS}" -d"${DURATION}s" --latency \
    "http://127.0.0.1:${PORT}/${SCENARIO}.php" > "$OUT_DIR/wrk-${TAG}.txt" 2>&1
cat "$OUT_DIR/wrk-${TAG}.txt"

# samply 0.13.1 ignores --duration when attached to an external PID —
# it literally logs "until Ctrl+C" and waits for a signal. SIGINT is
# the normal stop-and-save path (NOT cancel); SIGTERM/SIGKILL truncates.
echo "==> Sending SIGINT to samply to stop+flush"
docker exec "$CONTAINER" sh -c 'pkill -INT -f "samply record" || true'

# Wait up to 90s for samply to finish writing the gz.
for i in $(seq 1 90); do
    if ! docker exec "$CONTAINER" sh -c 'pgrep -f "samply record" >/dev/null 2>&1'; then
        echo "    samply exited after ${i}s"
        break
    fi
    sleep 1
done

if docker exec "$CONTAINER" sh -c 'pgrep -f "samply record" >/dev/null 2>&1'; then
    echo "::warning:: samply still alive after 90s — forcing SIGTERM then SIGKILL"
    docker exec "$CONTAINER" sh -c 'pkill -TERM -f "samply record" || true'
    sleep 3
    docker exec "$CONTAINER" sh -c 'pkill -KILL -f "samply record" || true'
    sleep 1
fi

# Prove the file exists (and its size) inside the container before cp.
echo "==> Profile file inside container:"
docker exec "$CONTAINER" ls -lh "$OUT_IN_CONTAINER" || true

# /tmp is now a host bind ($TMP_SHARED) — read the files directly, no
# `docker cp` (which is broken for tmpfs and flaky in general on
# Docker Desktop Mac).
cp -f "$TMP_SHARED/samply.log" "$OUT_DIR/samply-${TAG}.log" 2>/dev/null || true
cp -f "$TMP_SHARED/profile-${TAG}.json.gz" "$OUT_DIR/profile-${TAG}.json.gz"
CP_RC=$?

if [[ "$JIT_PERF_MAP" == "true" ]]; then
    mkdir -p "$OUT_DIR/perf-maps-${TAG}"
    for f in "$TMP_SHARED"/perf-*.map "$TMP_SHARED"/jit-*.dump; do
        [[ -e "$f" ]] && cp -f "$f" "$OUT_DIR/perf-maps-${TAG}/"
    done
fi

if [[ "$CP_RC" -ne 0 || ! -s "$OUT_DIR/profile-${TAG}.json.gz" ]]; then
    echo ""
    echo "::error:: Profile missing or empty: $OUT_DIR/profile-${TAG}.json.gz"
    echo ""
    echo "---- /tmp inside container ----"
    docker exec "$CONTAINER" ls -la /tmp/ 2>/dev/null || true
    echo ""
    echo "---- samply.log (last 40 lines) ----"
    if [[ -s "$OUT_DIR/samply-${TAG}.log" ]]; then
        tail -40 "$OUT_DIR/samply-${TAG}.log"
    else
        docker exec "$CONTAINER" sh -c 'cat /tmp/samply.log 2>/dev/null' || true
    fi
    echo ""
    echo "---- turbine logs (last 40 lines) ----"
    docker logs --tail 40 "$CONTAINER" 2>&1 || true
    echo ""
    echo "Container kept for inspection: $CONTAINER"
    echo "  docker exec -it $CONTAINER sh"
    echo "  docker rm -f $CONTAINER  # when done"
    trap 'rm -rf "$WORK_DIR"' EXIT
    exit 1
fi

echo ""
echo "==> Done."
ls -lh "$OUT_DIR/profile-${TAG}.json.gz"
echo ""
echo "Open https://profiler.firefox.com and load:"
echo "    $OUT_DIR/profile-${TAG}.json.gz"
