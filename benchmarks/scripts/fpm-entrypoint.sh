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
php_admin_value[opcache.interned_strings_buffer] = 16
php_admin_value[opcache.max_accelerated_files]   = 10000
php_admin_value[opcache.revalidate_freq]         = 0
php_admin_value[opcache.save_comments]           = 1
php_admin_value[opcache.jit]                     = function
php_admin_value[opcache.jit_buffer_size]         = 64M
EOF

# ── Fix permissions for Laravel (storage + bootstrap/cache need to be writable) ──
if [ -d /var/www/html/storage ]; then
    chown -R www-data:www-data /var/www/html/storage /var/www/html/bootstrap/cache 2>/dev/null || true
    chmod -R ug+rw /var/www/html/storage /var/www/html/bootstrap/cache 2>/dev/null || true
fi

# ── Laravel runtime optimisation (config:cache + route:cache) ───────────────────
# Paths are correct inside the container (/var/www/html) so caching works here.
if [ -f /var/www/html/artisan ]; then
    echo "[fpm-entry] Detected Laravel — running config:cache + route:cache" >&2
    cd /var/www/html
    php artisan config:cache 2>&1 >&2 || true
    php artisan route:cache  2>&1 >&2 || true
    cd /
fi

# ── Inject document root into Nginx config ─────────────────────────────────────
# Use a temp copy so the original stays intact for debugging
cp /etc/nginx/sites-available/bench /etc/nginx/sites-available/bench-active
sed -i "s|APP_ROOT_PLACEHOLDER|${APP_ROOT}|g" /etc/nginx/sites-available/bench-active
ln -sf /etc/nginx/sites-available/bench-active /etc/nginx/sites-enabled/bench

# ── Ensure socket directory exists ──────────────────────────────────────────
mkdir -p /run/php
chown www-data:www-data /run/php

# ── Start PHP-FPM ──────────────────────────────────────────────────────────────
echo "[fpm-entry] APP_ROOT=${APP_ROOT} WORKERS=${WORKERS}" >&2
echo "[fpm-entry] Nginx config root:" >&2
grep 'root ' /etc/nginx/sites-available/bench-active >&2 || true
echo "[fpm-entry] Testing PHP-FPM config..." >&2
php-fpm8.4 -t 2>&1 || { echo "[fpm-entry] FPM config test FAILED" >&2; exit 1; }
php-fpm8.4 --nodaemonize &
FPM_PID=$!

# Wait for socket to appear
for i in $(seq 1 15); do
    [ -S /run/php/fpm.sock ] && break
    sleep 0.5
done
if [ ! -S /run/php/fpm.sock ]; then
    echo "[fpm-entry] ERROR: FPM socket never appeared at /run/php/fpm.sock" >&2
    ls -la /run/php/ >&2 || true
fi

echo "[fpm-entry] Testing Nginx config..." >&2
nginx -t 2>&1 || { echo "[fpm-entry] Nginx config test FAILED" >&2; exit 1; }

# ── Start Nginx in foreground ──────────────────────────────────────────────────
exec nginx -g "daemon off;"
