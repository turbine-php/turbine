#!/bin/sh
# Turbine entrypoint — substitutes ${PORT} in the turbine.toml template
# before starting the server.
#
# Set PORT via env var (default 80):
#   docker run -e PORT=8080 ...
#   or environment: in docker-compose

set -e

export PORT="${PORT:-80}"
export APP_ROOT="${APP_ROOT:-/var/www/html}"
# Extension dir is written at build time by php-config --extension-dir
export PHP_EXTENSION_DIR
PHP_EXTENSION_DIR=$(cat /opt/php-embed/ext-dir.txt 2>/dev/null || /opt/php-embed/bin/php-config --extension-dir)

# ── Framework runtime optimisation ────────────────────────────────────────────
# Without config/route caching, every request in per-request mode re-parses
# dozens of PHP files (Laravel bootstrap ~15-25ms vs ~2ms cached). FPM does
# this too in its entrypoint — we must match to be fair in per-request mode
# AND to give real users a sensible default.  Set TURBINE_SKIP_FRAMEWORK_CACHE=1
# to opt out (e.g. when mounting read-only code and caching at build time).
PHP_BIN="/opt/php-embed/bin/php"
if [ -z "${TURBINE_SKIP_FRAMEWORK_CACHE}" ] && [ -x "${PHP_BIN}" ]; then
    # Laravel
    if [ -f "${APP_ROOT}/artisan" ]; then
        echo "[turbine-entry] Detected Laravel — running config:cache + route:cache + view:cache" >&2
        ( cd "${APP_ROOT}" && \
            "${PHP_BIN}" artisan config:cache 2>&1 >&2 || true; \
            "${PHP_BIN}" artisan route:cache  2>&1 >&2 || true; \
            "${PHP_BIN}" artisan view:cache   2>&1 >&2 || true )
    fi
    # Symfony
    if [ -f "${APP_ROOT}/bin/console" ]; then
        echo "[turbine-entry] Detected Symfony — warming prod cache" >&2
        ( cd "${APP_ROOT}" && \
            "${PHP_BIN}" bin/console cache:clear  --env=prod --no-debug 2>&1 >&2 || true; \
            "${PHP_BIN}" bin/console cache:warmup --env=prod --no-debug 2>&1 >&2 || true )
    fi
fi

# Resolve the config path: prefer mounted turbine.toml, fall back to template
TEMPLATE="/etc/turbine/turbine.toml.tmpl"
EFFECTIVE_CONFIG="${APP_ROOT}/turbine.toml"

if [ -f "${EFFECTIVE_CONFIG}" ]; then
    # File is mounted — apply substitution to a writable copy
    envsubst '${PORT} ${APP_ROOT} ${PHP_EXTENSION_DIR}' < "${EFFECTIVE_CONFIG}" > /tmp/turbine.toml
    EFFECTIVE_CONFIG=/tmp/turbine.toml
else
    # No mounted config — use baked-in template
    envsubst '${PORT} ${APP_ROOT} ${PHP_EXTENSION_DIR}' < "${TEMPLATE}" > /tmp/turbine.toml
    EFFECTIVE_CONFIG=/tmp/turbine.toml
fi

exec turbine serve -c "${EFFECTIVE_CONFIG}" -r "${APP_ROOT}" "$@"
