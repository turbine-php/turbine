#!/usr/bin/env bash
# Build Turbine with Profile-Guided Optimization.
#
# Workflow:
#   1. Instrumented build  — collects branch / function-call frequencies.
#   2. Run a representative workload (wrk against a sample PHP app).
#   3. Merge raw .profraw files into a single .profdata.
#   4. Final optimized build consumes .profdata to reorder functions,
#      inline hot paths, and bias branch prediction.
#
# Typical gain: 8–15 % throughput, 5–10 % lower p99 latency.
#
# Prerequisites:
#   - rustc nightly or stable with the target's llvm-tools-preview component:
#       rustup component add llvm-tools-preview
#   - wrk installed (brew install wrk / apt install wrk)
#   - A running sample PHP workload (see benchmarks/).
#
# Usage:
#   scripts/build-pgo.sh [profile-target]
#
#   profile-target defaults to "http://127.0.0.1:8080/"

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE_URL="${1:-http://127.0.0.1:8080/}"
PROFILE_DIR="$ROOT/target/pgo-data"
PROFDATA_FILE="$ROOT/target/merged.profdata"

if [[ -z "${PHP_CONFIG:-}" ]]; then
    export PHP_CONFIG="$ROOT/vendor/php-embed/bin/php-config"
fi

# Locate llvm-profdata shipped with rustc.
LLVM_PROFDATA="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's|host: ||p')/bin/llvm-profdata"
if [[ ! -x "$LLVM_PROFDATA" ]]; then
    echo "llvm-profdata not found. Run: rustup component add llvm-tools-preview" >&2
    exit 1
fi

echo "==> [1/4] Cleaning previous PGO data"
rm -rf "$PROFILE_DIR" "$PROFDATA_FILE"
mkdir -p "$PROFILE_DIR"

echo "==> [2/4] Instrumented build"
RUSTFLAGS="-Cprofile-generate=$PROFILE_DIR -L/opt/homebrew/opt/openssl@3/lib" \
    cargo build --release -p turbine-core

BINARY="$ROOT/target/release/turbine"

echo "==> [3/4] Running workload to collect profile data"
echo "         Start your PHP app under: $BINARY serve --root <your-app>"
echo "         Then in another terminal hit: $PROFILE_URL"
echo
echo "         Example 60s warm-up:"
echo "             wrk -t4 -c64 -d60s '$PROFILE_URL'"
echo
read -r -p "Press Enter once the workload has finished..."

echo "==> Merging profile data"
"$LLVM_PROFDATA" merge -o "$PROFDATA_FILE" "$PROFILE_DIR"

echo "==> [4/4] Optimized build using collected profile"
RUSTFLAGS="-Cprofile-use=$PROFDATA_FILE -Cllvm-args=-pgo-warn-missing-function -L/opt/homebrew/opt/openssl@3/lib" \
    cargo build --release -p turbine-core

echo
echo "PGO build complete: $BINARY"
echo "Typical improvements: 8–15% throughput, 5–10% p99 latency."
