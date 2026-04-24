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

# IMPORTANT: do NOT bind-mount /tmp from the macOS host. On Docker
# Desktop, virtiofs causes the LinuxKit kernel to report the backing
# host path (e.g. /run/host_virtiofs/private/var/folders/.../jit-8.dump)
# in the perf_event MMAP2 record, rather than the in-container path
# /tmp/jit-8.dump. samply then tries to open that non-existent path and
# silently fails to parse the jitdump, so no JIT$ symbols ever land in
# the profile. Using container-local /tmp makes the kernel report the
# plain /tmp/jit-<pid>.dump path, which samply can read.
#
# Output goes to a separate bind mount (/out) — unrelated to the JIT
# file paths, so the virtiofs path shows up for that mmap (harmless).
OUT_SHARED="$WORK_DIR/out"
mkdir -p "$OUT_SHARED"
chmod 1777 "$OUT_SHARED"

echo "==> Starting $CONTAINER (samply wraps turbine from PID 1 for jitdump)"
OUT_IN_CONTAINER="/out/profile-${TAG}.json.gz"
# Wrap turbine inside samply so samply sees every MMAP event (including
# the JIT buffer mmap and the /tmp/jit-%d.dump mmap) from the start.
# samply cannot reconstruct these when attached via -p after the fact
# (samply issue #127 — attach mode misses mmap events), which is why
# every JIT sample ended up as "0x8000xxx zero (deleted)" unresolved.
docker run -d --name "$CONTAINER" \
    --cap-add SYS_PTRACE \
    --cap-add PERFMON \
    --cap-add SYS_ADMIN \
    --security-opt seccomp=unconfined \
    -p "${PORT}:80" \
    -v "$APP_DIR/turbine.toml:/var/www/html/turbine.toml:ro" \
    -v "$OUT_SHARED:/out" \
    --entrypoint sh \
    "$IMG" \
    -c "exec samply record --save-only --jit-markers \
            --output '${OUT_IN_CONTAINER}' --rate ${RATE} \
            -- turbine serve -c /var/www/html/turbine.toml -r /var/www/html \
            > /out/samply.log 2>&1" \
    >/dev/null

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
echo "==> samply is already recording (PID 1 inside container)"

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
# Since samply is PID 1 (exec) and turbine is its child, we send
# SIGTERM to turbine. samply sees the child exit, finalises the
# profile, and then exits itself (which also exits the container).
echo "==> Sending SIGTERM to turbine (samply's child) to trigger flush"
docker exec "$CONTAINER" sh -c 'pkill -TERM -f "turbine serve" || true'

for i in $(seq 1 90); do
    if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
        echo "    container stopped after ${i}s"
        break
    fi
    sleep 1
done

if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
    echo "::warning:: container still running after 90s — sending SIGINT to PID 1 (samply)"
    docker kill --signal=INT "$CONTAINER" >/dev/null 2>&1 || true
    sleep 10
    if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
        docker kill "$CONTAINER" >/dev/null 2>&1 || true
        sleep 2
    fi
fi

# Prove the file exists (and its size) on the host side of the bind.
echo "==> Profile file on host bind:"
ls -lh "$OUT_SHARED/profile-${TAG}.json.gz" 2>/dev/null || echo "    (not found)"

# /out is a host bind ($OUT_SHARED) — read the files directly.
cp -f "$OUT_SHARED/samply.log" "$OUT_DIR/samply-${TAG}.log" 2>/dev/null || true
cp -f "$OUT_SHARED/profile-${TAG}.json.gz" "$OUT_DIR/profile-${TAG}.json.gz"
CP_RC=$?

# /tmp lives inside the container's own fs (not a bind), so use
# `docker cp`. Works on stopped containers too. We copy the whole /tmp
# into a staging dir and then pick out the JIT artefacts.
if [[ "$JIT_PERF_MAP" == "true" ]] && docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER"; then
    mkdir -p "$OUT_DIR/perf-maps-${TAG}"
    STAGE="$WORK_DIR/tmp-stage"
    rm -rf "$STAGE" && mkdir -p "$STAGE"
    docker cp "$CONTAINER:/tmp/." "$STAGE/" 2>/dev/null || true
    for f in "$STAGE"/perf-*.map "$STAGE"/jit-*.dump; do
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
