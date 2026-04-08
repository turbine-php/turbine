#!/usr/bin/env bash
# Build Turbine with an embedded PHP application.
#
# Usage:
#   TURBINE_EMBED_DIR=/path/to/php-app ./scripts/build-embed.sh
#
# The PHP application files will be packed into the binary at compile time
# using Rust's include_dir! macro. At runtime, when [embed] enabled=true
# in turbine.toml, the files are extracted to a temporary directory and
# the server serves from there.
#
# This creates a single self-contained binary:
#   ./target/release/turbine

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -z "${TURBINE_EMBED_DIR:-}" ]; then
    echo "Error: TURBINE_EMBED_DIR must be set to the PHP app directory"
    echo "Usage: TURBINE_EMBED_DIR=/path/to/app ./scripts/build-embed.sh"
    exit 1
fi

EMBED_ABS="$(cd "$TURBINE_EMBED_DIR" && pwd)"

if [ ! -d "$EMBED_ABS" ]; then
    echo "Error: $EMBED_ABS is not a directory"
    exit 1
fi

cd "$PROJECT_ROOT"

echo "=== Turbine Embed Build ==="
echo "Embedding: $EMBED_ABS"

# Count files
FILE_COUNT=$(find "$EMBED_ABS" -type f | wc -l | tr -d ' ')
echo "Files: $FILE_COUNT"

# Set the env var that build.rs / the embed feature will read
export TURBINE_EMBED_DIR="$EMBED_ABS"

# Set PHP config if available
if [ -f "$PROJECT_ROOT/vendor/php-embed/bin/php-config" ]; then
    export PHP_CONFIG="$PROJECT_ROOT/vendor/php-embed/bin/php-config"
fi

echo "Building with embed feature..."
cargo build --release --features embed

BINARY="target/release/turbine"
SIZE=$(du -h "$BINARY" | cut -f1)

echo ""
echo "=== Build successful ==="
echo "Binary: $BINARY ($SIZE)"
echo ""
echo "Run with:"
echo "  ./$BINARY serve --config turbine.toml"
echo ""
echo "Make sure turbine.toml has:"
echo "  [embed]"
echo "  enabled = true"
