#!/usr/bin/env bash
# Build a fully static Turbine binary using musl libc.
#
# Usage:
#   ./scripts/build-static-musl.sh
#
# Requires:
#   - Rust target x86_64-unknown-linux-musl installed
#   - PHP embed SAPI compiled with musl (or the vendor/php-embed directory)
#   - musl-tools (apt install musl-tools) or a musl cross-compiler
#
# The resulting binary will be at:
#   target/x86_64-unknown-linux-musl/release/turbine

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

echo "=== Turbine Static Build (musl) ==="

# Check for musl target
if ! rustup target list --installed | grep -q "x86_64-unknown-linux-musl"; then
    echo "Installing musl target..."
    rustup target add x86_64-unknown-linux-musl
fi

# Check for PHP embed
if [ -f "$PROJECT_ROOT/vendor/php-embed/bin/php-config" ]; then
    export PHP_CONFIG="$PROJECT_ROOT/vendor/php-embed/bin/php-config"
    echo "Using PHP embed from vendor/php-embed"
else
    echo "WARNING: vendor/php-embed not found."
    echo "  For a fully static build, compile PHP with:"
    echo "    --enable-embed=static --enable-static --disable-shared"
    echo "  See scripts/build-php-embed.sh for details."
fi

echo "Building release binary with musl..."
RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --target x86_64-unknown-linux-musl

BINARY="target/x86_64-unknown-linux-musl/release/turbine"

if [ -f "$BINARY" ]; then
    SIZE=$(du -h "$BINARY" | cut -f1)
    echo ""
    echo "=== Build successful ==="
    echo "Binary: $BINARY"
    echo "Size:   $SIZE"
    echo ""
    echo "Verify it's static:"
    file "$BINARY"
    ldd "$BINARY" 2>/dev/null || echo "(no dynamic dependencies — fully static)"
    echo ""
    echo "To compress with UPX:"
    echo "  upx --best --lzma $BINARY"
else
    echo "Build failed — binary not found"
    exit 1
fi
