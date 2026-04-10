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
