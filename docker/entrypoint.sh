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

# Resolve the config path: prefer mounted turbine.toml, fall back to template
TEMPLATE="/etc/turbine/turbine.toml.tmpl"
EFFECTIVE_CONFIG="${APP_ROOT}/turbine.toml"

if [ -f "${EFFECTIVE_CONFIG}" ]; then
    # File is mounted — apply substitution to a writable copy
    envsubst '${PORT} ${APP_ROOT}' < "${EFFECTIVE_CONFIG}" > /tmp/turbine.toml
    EFFECTIVE_CONFIG=/tmp/turbine.toml
else
    # No mounted config — use baked-in template
    envsubst '${PORT} ${APP_ROOT}' < "${TEMPLATE}" > /tmp/turbine.toml
    EFFECTIVE_CONFIG=/tmp/turbine.toml
fi

exec turbine serve -c "${EFFECTIVE_CONFIG}" -r "${APP_ROOT}" "$@"
