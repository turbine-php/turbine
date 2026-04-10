#!/usr/bin/env bash
# setup.sh — One-time server setup on Hetzner CPX41 (Ubuntu 24.04)
#
# Uses the pre-built katisuhara/turbine-php Docker images to:
#   1. Run Turbine benchmarks (via docker run — no compilation needed)
#   2. Extract phalcon.so for the PHP-FPM baseline (avoids ~10 min of compilation)
#
# Installs: Docker, Nginx, PHP 8.4-FPM, Composer, wrk

set -euo pipefail

TURBINE_IMAGE_NTS="katisuhara/turbine-php:latest-php8.4-nts"
TURBINE_IMAGE_ZTS="katisuhara/turbine-php:latest-php8.4-zts"

log() { echo "[setup] $*"; }

# ── Docker (needed first — images are the source of PHP + extensions) ────────
log "Installing Docker..."
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl
curl -fsSL https://get.docker.com | sh

log "Pulling Turbine Docker images..."
docker pull "$TURBINE_IMAGE_NTS"
docker pull "$TURBINE_IMAGE_ZTS"

# ── Extract phalcon.so from NTS image — no compilation needed ────────────────
log "Extracting phalcon.so from Docker image..."
CONTAINER_ID=$(docker create "$TURBINE_IMAGE_NTS")
# Find where phalcon.so lives inside the image
PHALCON_PATH=$(docker run --rm "$TURBINE_IMAGE_NTS" \
    sh -c 'find /opt/php-embed/lib/php/extensions -name phalcon.so | head -1')
docker cp "${CONTAINER_ID}:${PHALCON_PATH}" /tmp/phalcon.so
docker rm "${CONTAINER_ID}" >/dev/null
log "phalcon.so extracted from image (path was: ${PHALCON_PATH})"

# ── System packages: Nginx, PHP-FPM, Composer, wrk ──────────────────────────
log "Adding ondrej/php PPA for PHP 8.4..."
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    software-properties-common gnupg
add-apt-repository -y ppa:ondrej/php
apt-get update -qq

log "Installing Nginx, PHP 8.4-FPM, Composer, wrk..."
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    nginx \
    php8.4-fpm php8.4-cli php8.4-mbstring php8.4-xml php8.4-curl php8.4-zip \
    php8.4-intl php8.4-bcmath php8.4-gd php8.4-sqlite3 \
    php8.4-tokenizer php8.4-fileinfo php8.4-opcache \
    composer wrk \
    git unzip jq

# ── Raw PHP application ──────────────────────────────────────────────────────
log "Creating raw PHP application..."
mkdir -p /var/www/raw
cat > /var/www/raw/index.php << 'PHPEOF'
<?php
header('Content-Type: text/plain');
echo "Hello, World!";
PHPEOF

# ── Laravel application ──────────────────────────────────────────────────────
log "Creating Laravel project (this takes a few minutes)..."
COMPOSER_ALLOW_SUPERUSER=1 composer create-project laravel/laravel /var/www/laravel \
    --quiet --no-interaction --prefer-dist

# Replace routes/web.php with a single lightweight benchmark route (no DB, no session)
cat > /var/www/laravel/routes/web.php << 'PHPEOF'
<?php
use Illuminate\Support\Facades\Route;

Route::get('/', fn() => response()->json(['status' => 'ok']));
PHPEOF

chown -R www-data:www-data /var/www/laravel/storage /var/www/laravel/bootstrap/cache
chmod -R ug+rw /var/www/laravel/storage /var/www/laravel/bootstrap/cache

# ── Phalcon micro application ────────────────────────────────────────────────
log "Creating Phalcon micro application..."
mkdir -p /var/www/phalcon
cat > /var/www/phalcon/index.php << 'PHPEOF'
<?php
use Phalcon\Mvc\Micro;

$app = new Micro();
$app->get('/', function () {
    echo json_encode(['status' => 'ok']);
});
$app->handle($_SERVER['REQUEST_URI'] ?? '/');
PHPEOF

# ── Install phalcon.so into PHP-FPM (extracted from Docker image) ─────────────
log "Installing phalcon.so into PHP 8.4 extension dir..."
PHP_EXT_DIR=$(php8.4 -r 'echo ini_get("extension_dir");')
cp /tmp/phalcon.so "${PHP_EXT_DIR}/phalcon.so"
PHP_MODS_DIR="/etc/php/8.4/mods-available"
echo "extension=phalcon.so" > "${PHP_MODS_DIR}/phalcon.ini"
phpenmod -v 8.4 phalcon
log "Phalcon extension installed (no compilation needed)."

# ── PHP-FPM configuration ────────────────────────────────────────────────────
log "Configuring PHP-FPM (static pool, 8 workers)..."
sed -i 's/^pm = .*/pm = static/'             /etc/php/8.4/fpm/pool.d/www.conf
sed -i 's/^pm.max_children = .*/pm.max_children = 8/' /etc/php/8.4/fpm/pool.d/www.conf
# Disable slow log to avoid I/O overhead during bench
sed -i 's/^slowlog.*/#&/'                   /etc/php/8.4/fpm/pool.d/www.conf

# ── Nginx virtual hosts ───────────────────────────────────────────────────────
log "Configuring Nginx..."
rm -f /etc/nginx/sites-enabled/default

cat > /etc/nginx/sites-available/bench-raw << 'NGINXEOF'
server {
    listen 8803;
    root /var/www/raw;
    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.4-fpm.sock;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        include fastcgi_params;
    }
}
NGINXEOF

cat > /etc/nginx/sites-available/bench-laravel << 'NGINXEOF'
server {
    listen 8813;
    root /var/www/laravel/public;
    location / { try_files $uri $uri/ /index.php?$query_string; }
    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.4-fpm.sock;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        include fastcgi_params;
    }
}
NGINXEOF

cat > /etc/nginx/sites-available/bench-phalcon << 'NGINXEOF'
server {
    listen 8823;
    root /var/www/phalcon;
    location / { try_files $uri $uri/ /index.php?$query_string; }
    location ~ \.php$ {
        fastcgi_pass unix:/run/php/php8.4-fpm.sock;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
        include fastcgi_params;
    }
}
NGINXEOF

for site in bench-raw bench-laravel bench-phalcon; do
    ln -sf "/etc/nginx/sites-available/${site}" /etc/nginx/sites-enabled/
done

nginx -t
systemctl restart nginx php8.4-fpm

# ── Turbine config files ─────────────────────────────────────────────────────
log "Creating Turbine config files..."
mkdir -p /etc/turbine

# Raw PHP — process mode, no extensions needed
cat > /etc/turbine/raw.toml << 'TOMLEOF'
[server]
listen = "0.0.0.0:80"
workers = 8
worker_mode = "process"
request_timeout = 30

[php]
extensions = []

[logging]
level = "error"
TOMLEOF

# Laravel — process mode, no extra extensions
cat > /etc/turbine/laravel.toml << 'TOMLEOF'
[server]
listen = "0.0.0.0:80"
workers = 8
worker_mode = "process"
request_timeout = 30

[php]
extensions = []

[php.ini]
error_reporting = "0"
display_errors = "Off"
"date.timezone" = "UTC"

[logging]
level = "error"
TOMLEOF

# Phalcon NTS — process mode, Phalcon extension from image
cat > /etc/turbine/phalcon-nts.toml << 'TOMLEOF'
[server]
listen = "0.0.0.0:80"
workers = 8
worker_mode = "process"
request_timeout = 30

[php]
extensions = ["phalcon.so"]

[logging]
level = "error"
TOMLEOF

# Phalcon ZTS — thread mode, Phalcon extension from image
cat > /etc/turbine/phalcon-zts.toml << 'TOMLEOF'
[server]
listen = "0.0.0.0:80"
workers = 8
worker_mode = "thread"
request_timeout = 30

[php]
extensions = ["phalcon.so"]

[logging]
level = "error"
TOMLEOF

log "Setup complete!"
