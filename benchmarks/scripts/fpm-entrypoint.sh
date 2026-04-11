#!/bin/sh
# fpm-entrypoint.sh — Configure and start PHP-FPM + Nginx at container runtime.
# Env vars:
#   WORKERS   — pm.max_children (default: 4)
#   APP_ROOT  — Nginx document root (default: /var/www/html/public)

set -e

WORKERS="${WORKERS:-4}"
APP_ROOT="${APP_ROOT:-/var/www/html/public}"

# ── Write FPM pool ─────────────────────────────────────────────────────────────
cat > /etc/php/8.4/fpm/pool.d/bench.conf << EOF
[bench]
user  = www-data
group = www-data
listen = /run/php/fpm.sock
listen.owner = www-data
listen.group = www-data
pm = static
pm.max_children = ${WORKERS}
pm.max_requests = 50000
php_admin_value[memory_limit]               = 256M
php_admin_value[opcache.memory_consumption] = 128
php_admin_value[opcache.enable]             = 1
php_admin_value[opcache.validate_timestamps]= 0
EOF

# ── Inject document root into Nginx config ─────────────────────────────────────
sed -i "s|APP_ROOT_PLACEHOLDER|${APP_ROOT}|g" /etc/nginx/sites-available/bench

# ── Start PHP-FPM ──────────────────────────────────────────────────────────────
php-fpm8.4 --nodaemonize &
FPM_PID=$!

# Wait for socket to appear
for i in $(seq 1 15); do
    [ -S /run/php/fpm.sock ] && break
    sleep 0.5
done

# ── Start Nginx in foreground ──────────────────────────────────────────────────
exec nginx -g "daemon off;"
