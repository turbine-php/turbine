#!/usr/bin/env bash
# setup.sh — One-time server setup on Hetzner CCX33 (Ubuntu 24.04)
#
# Usage: bash setup.sh [php_version]
#   php_version — PHP major version, e.g. 8.4 or 8.5 (default: 8.4)
#
# Strategy:
#   - ALL servers run inside Docker containers — equal overhead, fair comparison
#   - Turbine NTS/ZTS and FrankenPHP use published Docker Hub images
#   - Nginx + PHP-FPM uses a locally-built image (bench-fpm) with Phalcon pre-installed
#   - Phalcon tested only on Turbine and Nginx+FPM (incompatible with FrankenPHP/ZTS)
#   - wrk (native) sends HTTP load; measured via Lua done() callback

set -euo pipefail

PHP_VERSION="${1:-8.4}"

TURBINE_IMAGE_NTS="katisuhara/turbine-php:latest-php${PHP_VERSION}-nts"
TURBINE_IMAGE_ZTS="katisuhara/turbine-php:latest-php${PHP_VERSION}-zts"
FRANKENPHP_IMAGE="dunglas/frankenphp:latest"
PHALCON_VERSION="5.11.1"
WRK_VERSION="master"

log() { echo "[setup] $*"; }

# ── 1. Docker ─────────────────────────────────────────────────────────────────
log "Installing Docker..."
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg
curl -fsSL https://get.docker.com | sh

# ── 2. Pull Turbine images ────────────────────────────────────────────────────
log "Pulling Turbine images..."
docker pull "$TURBINE_IMAGE_NTS"
docker pull "$TURBINE_IMAGE_ZTS"

# ── 3. Pull FrankenPHP image ─────────────────────────────────────────────────
log "Pulling FrankenPHP image (ZTS-based)..."
docker pull "$FRANKENPHP_IMAGE"

# ── 4. wrk HTTP benchmarking tool ──────────────────────────────────────────────
log "Installing wrk..."
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    wrk || {
    # Fallback: build from source if not in apt
    apt-get install -y --no-install-recommends libssl-dev build-essential git
    git clone --depth 1 https://github.com/wg/wrk.git /tmp/wrk-src
    make -C /tmp/wrk-src -j$(nproc)
    cp /tmp/wrk-src/wrk /usr/local/bin/wrk
    rm -rf /tmp/wrk-src
}
wrk --version 2>&1 | head -1 || true   # wrk exits 1 on --version but still prints info

# ── 5. PHP 8.4 CLI + Composer (for Laravel project creation on the host) ─────
# Need full PHP 8.4 with extensions from ondrej/php for `composer create-project`
log "Installing PHP 8.4 CLI + Composer from ondrej/php..."
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    software-properties-common
add-apt-repository -y ppa:ondrej/php
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    php8.4-cli php8.4-mbstring php8.4-xml php8.4-curl php8.4-zip \
    php8.4-intl php8.4-bcmath php8.4-gd php8.4-sqlite3 \
    php8.4-tokenizer php8.4-fileinfo \
    composer git unzip jq

# ── 5b. Build bench-fpm Docker image (nginx + php8.4-fpm + phalcon) ─────────
log "Building bench-fpm Docker image (nginx+php8.4-fpm+phalcon)..."
FPM_IMAGE="bench-fpm:latest"
docker build -t "$FPM_IMAGE" -f /root/bench/Dockerfile.fpm /root/bench
log "bench-fpm image built."

# ── 6. Application directories ───────────────────────────────────────────────
log "Creating raw PHP application..."
mkdir -p /var/www/raw/public
cat > /var/www/raw/index.php << 'PHPEOF'
<?php
header('Content-Type: text/plain');
echo "Hello, World!";
PHPEOF
# Copy to public/ for FrankenPHP (symlinks break across Docker mount boundaries)
cp /var/www/raw/index.php /var/www/raw/public/index.php

log "Creating Phalcon micro application..."
mkdir -p /var/www/phalcon/public
cat > /var/www/phalcon/index.php << 'PHPEOF'
<?php
use Phalcon\Mvc\Micro;
$app = new Micro();
$app->get('/', function () {
    header('Content-Type: application/json');
    echo json_encode(['status' => 'ok', 'framework' => 'Phalcon', 'php' => PHP_VERSION]);
});
$app->get('/user/{id}', function ($id) {
    header('Content-Type: application/json');
    echo json_encode(['id' => (int) $id, 'name' => 'User ' . $id, 'email' => 'user' . $id . '@example.com']);
});
$app->post('/user', function () {
    header('Content-Type: application/json');
    http_response_code(201);
    echo json_encode(['status' => 'created', 'id' => random_int(1, 100000)]);
});
$app->handle($_SERVER['REQUEST_URI'] ?? '/');
PHPEOF
cp /var/www/phalcon/index.php /var/www/phalcon/public/index.php

log "Copying PHP benchmark scripts..."
mkdir -p /var/www/php-bench/public
cp /root/bench/php/*.php /var/www/php-bench/
cp /root/bench/php/*.php /var/www/php-bench/public/
# Minimal index for health-check (wait_http hits /)
echo '<?php http_response_code(200);' > /var/www/php-bench/index.php
cp /var/www/php-bench/index.php /var/www/php-bench/public/index.php

log "Creating Laravel 13 project (this may take a few minutes)..."
COMPOSER_ALLOW_SUPERUSER=1 composer create-project "laravel/laravel:^13.0" /var/www/laravel \
    --quiet --no-interaction --prefer-dist

# ── Fix .env BEFORE anything else ────────────────────────────────────────────
# Default Laravel uses SESSION_DRIVER=database which requires migrations to
# create the sessions table. Use file driver to avoid DB dependency entirely.
sed -i 's/SESSION_DRIVER=database/SESSION_DRIVER=file/' /var/www/laravel/.env
sed -i 's/SESSION_DRIVER=cookie/SESSION_DRIVER=file/'   /var/www/laravel/.env
sed -i 's/DB_CONNECTION=.*/DB_CONNECTION=sqlite/'        /var/www/laravel/.env

# Ensure SQLite DB file exists (migrations may need it)
touch /var/www/laravel/database/database.sqlite

# Standard benchmark routes: GET /, GET /user/:id, POST /user
# Routes return JSON with meaningful bodies so benchmarks measure real work.
cat > /var/www/laravel/routes/web.php << 'PHPEOF'
<?php
use Illuminate\Support\Facades\Route;

Route::get('/', fn() => response()->json(['status' => 'ok', 'framework' => 'Laravel', 'php' => PHP_VERSION]));
Route::get('/user/{id}', fn(string $id) => response()->json(['id' => (int) $id, 'name' => 'User ' . $id, 'email' => 'user' . $id . '@example.com']));
Route::post('/user', fn() => response()->json(['status' => 'created', 'id' => random_int(1, 100000)], 201));
PHPEOF

# Override bootstrap/app.php — strip session/CSRF middleware for stateless benchmark
# Uses Laravel 13 $middleware->web(remove: [...]) syntax
cat > /var/www/laravel/bootstrap/app.php << 'PHPEOF'
<?php
use Illuminate\Foundation\Application;
use Illuminate\Foundation\Configuration\Exceptions;
use Illuminate\Foundation\Configuration\Middleware;

return Application::configure(basePath: dirname(__DIR__))
    ->withRouting(
        web: __DIR__.'/../routes/web.php',
    )
    ->withMiddleware(function (Middleware $middleware) {
        $middleware->web(remove: [
            \Illuminate\Session\Middleware\StartSession::class,
            \Illuminate\View\Middleware\ShareErrorsFromSession::class,
            \Illuminate\Foundation\Http\Middleware\PreventRequestForgery::class,
            \Illuminate\Cookie\Middleware\EncryptCookies::class,
            \Illuminate\Cookie\Middleware\AddQueuedCookiesToResponse::class,
        ]);
    })
    ->withExceptions(function (Exceptions $exceptions) {
        //
    })->create();
PHPEOF

# Clear any caches left by post-create scripts (packages.php, services.php, etc.)
# These may contain absolute paths to /var/www/laravel/ that break inside containers
# where the app is mounted at /var/www/html/
cd /var/www/laravel
php artisan config:clear  2>/dev/null || true
php artisan route:clear   2>/dev/null || true
php artisan view:clear    2>/dev/null || true
php artisan cache:clear   2>/dev/null || true
php artisan package:discover --ansi 2>/dev/null || true
cd /

# NOTE: Do NOT run config:cache or route:cache here.
# Setup runs at /var/www/laravel but containers mount it at /var/www/html.
# Cached config bakes absolute paths (/var/www/laravel/resources/...) that
# break inside the container where open_basedir restricts to /var/www/html.

chown -R www-data:www-data \
    /var/www/laravel/storage \
    /var/www/laravel/bootstrap/cache
chmod -R ug+rw \
    /var/www/laravel/storage \
    /var/www/laravel/bootstrap/cache

# Turbine persistent-worker files for Laravel
# turbine-boot.php: runs ONCE per worker at startup — bootstraps Laravel
cat > /var/www/laravel/turbine-boot.php << 'PHPEOF'
<?php
declare(strict_types=1);
define('LARAVEL_START', microtime(true));
require __DIR__.'/vendor/autoload.php';
$GLOBALS['__turbine_app'] = require_once __DIR__.'/bootstrap/app.php';
$GLOBALS['__turbine_kernel'] = $GLOBALS['__turbine_app']
    ->make(\Illuminate\Contracts\Http\Kernel::class);
PHPEOF

# turbine-handler.php: runs on EVERY request in persistent mode
cat > /var/www/laravel/turbine-handler.php << 'PHPEOF'
<?php
declare(strict_types=1);
$request  = \Illuminate\Http\Request::capture();
$response = $GLOBALS['__turbine_kernel']->handle($request);
$response->send();
$GLOBALS['__turbine_kernel']->terminate($request, $response);
gc_collect_cycles();
PHPEOF

# turbine-cleanup.php: runs AFTER every request in persistent mode
# Clears session, auth, scoped instances and facades to prevent state leaks
cat > /var/www/laravel/turbine-cleanup.php << 'PHPEOF'
<?php
declare(strict_types=1);
$app = $GLOBALS['__turbine_app'];
if (method_exists($app, 'resetScope')) { $app->resetScope(); }
if (method_exists($app, 'forgetScopedInstances')) { $app->forgetScopedInstances(); }
if ($app->resolved('session')) {
    try { $s = $app->make('session')->driver(); $s->flush(); $s->regenerate(); } catch (\Throwable $e) {}
}
$app->forgetInstance('session.store');
if ($app->resolved('auth.driver')) { $app->forgetInstance('auth.driver'); }
if ($app->resolved('auth')) { $app->make('auth')->forgetGuards(); }
\Illuminate\Support\Facades\Facade::clearResolvedInstances();
PHPEOF

# ── 7. Turbine config files (all apps × modes × worker counts) ────────────────
# Naming: {app}-{nts|zts}-{N}w[-p].toml
#   nts = worker_mode "process"   persistent_workers false/true
#   zts = worker_mode "thread"    (no persistent variant needed)
# PHP: memory_limit=256M, opcache=128M, worker_max_requests=50000
log "Creating Turbine config files..."
mkdir -p /etc/turbine

make_turbine_toml() {
    local file="$1" workers="$2" mode="$3" persistent="$4"
    local extensions="${5:-[]}"
    local extra_ini="${6:-}"
    local extra_server="${7:-}"      # additional [server] keys (worker_boot, etc.)
    local extra_sections="${8:-}"    # additional TOML sections ([sandbox], etc.)
    cat > "$file" << TOML
[server]
listen = "0.0.0.0:80"
workers = ${workers}
worker_mode = "${mode}"
worker_max_requests = 50000
persistent_workers = ${persistent}
request_timeout = 30
${extra_server}
[php]
memory_limit = "256M"
opcache_memory = 128
extensions = ${extensions}
${extra_ini}
[logging]
level = "error"

[cache]
enabled = false
${extra_sections}
TOML
}

for W in 4 8; do
    # Raw PHP + PHP-bench scripts (stateless)
    for APP in raw php-bench; do
        make_turbine_toml /etc/turbine/${APP}-nts-${W}w.toml   ${W} process false
        make_turbine_toml /etc/turbine/${APP}-nts-${W}w-p.toml ${W} process true
        make_turbine_toml /etc/turbine/${APP}-zts-${W}w.toml   ${W} thread  false
        make_turbine_toml /etc/turbine/${APP}-zts-${W}w-p.toml ${W} thread  true
    done

    # Laravel — needs [sandbox] for framework detection and full app dir
    LARAVEL_INI=$'[php.ini]\nerror_reporting = "0"\ndisplay_errors = "Off"\n"date.timezone" = "UTC"'
    LARAVEL_SANDBOX=$'[sandbox]\nexecution_mode = "framework"\nfront_controller = true'
    LARAVEL_BOOT=$'worker_boot = "turbine-boot.php"\nworker_handler = "turbine-handler.php"\nworker_cleanup = "turbine-cleanup.php"'
    make_turbine_toml /etc/turbine/laravel-nts-${W}w.toml   ${W} process false "[]" "$LARAVEL_INI" "" "$LARAVEL_SANDBOX"
    make_turbine_toml /etc/turbine/laravel-nts-${W}w-p.toml ${W} process true  "[]" "$LARAVEL_INI" "$LARAVEL_BOOT" "$LARAVEL_SANDBOX"
    make_turbine_toml /etc/turbine/laravel-zts-${W}w.toml   ${W} thread  false "[]" "$LARAVEL_INI" "" "$LARAVEL_SANDBOX"
    make_turbine_toml /etc/turbine/laravel-zts-${W}w-p.toml ${W} thread  true  "[]" "$LARAVEL_INI" "$LARAVEL_BOOT" "$LARAVEL_SANDBOX"

    # Phalcon (requires phalcon extension)
    make_turbine_toml /etc/turbine/phalcon-nts-${W}w.toml   ${W} process false '["phalcon.so"]'
    make_turbine_toml /etc/turbine/phalcon-nts-${W}w-p.toml ${W} process true  '["phalcon.so"]'
    make_turbine_toml /etc/turbine/phalcon-zts-${W}w.toml   ${W} thread  false '["phalcon.so"]'
    make_turbine_toml /etc/turbine/phalcon-zts-${W}w-p.toml ${W} thread  true  '["phalcon.so"]'
done

# ── 8. FrankenPHP worker scripts ──────────────────────────────────────────────
log "Creating FrankenPHP worker scripts..."

# Raw PHP worker — handler stays in memory, avoids per-request PHP startup
cat > /var/www/raw/worker.php << 'PHPEOF'
<?php
$handler = static function (): void {
    header('Content-Type: text/plain');
    echo "Hello, World!";
};
while (\frankenphp_handle_request($handler));
PHPEOF
# Copy (not symlink) — symlinks break across Docker mount boundaries
cp /var/www/raw/worker.php /var/www/raw/public/worker.php

# PHP-bench worker — dispatches to the requested .php file via URI
cat > /var/www/php-bench/worker.php << 'PHPEOF'
<?php
$handler = static function (): void {
    $raw  = parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH) ?? '/';
    $file = realpath(__DIR__ . $raw);
    if ($file !== false
        && str_starts_with($file, __DIR__ . '/')
        && str_ends_with($file, '.php')
        && basename($file) !== 'worker.php'
    ) {
        require $file;
        return;
    }
    http_response_code(404);
};
while (\frankenphp_handle_request($handler));
PHPEOF
cp /var/www/php-bench/worker.php /var/www/php-bench/public/worker.php

# Laravel worker — bootstrap once, handle many requests
# Pattern matches the proven local config in dev/laravel-test/public/frankenphp-worker.php
cat > /var/www/laravel/public/worker.php << 'PHPEOF'
<?php
require __DIR__ . '/../vendor/autoload.php';

$app = require __DIR__ . '/../bootstrap/app.php';
$kernel = $app->make(Illuminate\Contracts\Http\Kernel::class);

$running = true;

while ($running && ($running = \frankenphp_handle_request(function () use ($kernel) {
    $request = Illuminate\Http\Request::capture();
    $response = $kernel->handle($request);
    $response->send();
    $kernel->terminate($request, $response);
}))) {
    gc_collect_cycles();
}
PHPEOF

# Phalcon worker — Micro app persists between requests
cat > /var/www/phalcon/public/worker.php << 'PHPEOF'
<?php
use Phalcon\Mvc\Micro;
$app = new Micro();
$app->get('/', static function (): void {
    header('Content-Type: application/json');
    echo json_encode(['status' => 'ok', 'framework' => 'Phalcon', 'php' => PHP_VERSION]);
});
$app->get('/user/{id}', static function ($id): void {
    header('Content-Type: application/json');
    echo json_encode(['id' => (int) $id, 'name' => 'User ' . $id, 'email' => 'user' . $id . '@example.com']);
});
$app->post('/user', static function (): void {
    header('Content-Type: application/json');
    http_response_code(201);
    echo json_encode(['status' => 'created', 'id' => random_int(1, 100000)]);
});
$handler = static function () use ($app): void {
    $app->handle($_SERVER['REQUEST_URI'] ?? '/');
};
while (\frankenphp_handle_request($handler));
PHPEOF

# NOTE: phalcon worker.php is kept for reference but NOT used in benchmarks
# since Phalcon is incompatible with FrankenPHP's ZTS threading model.

# ── 9. FrankenPHP Caddyfile templates ────────────────────────────────────────
# Only for apps that run on FrankenPHP: raw, laravel, php-bench (NOT phalcon)
log "Creating FrankenPHP Caddyfile templates..."
mkdir -p /etc/frankenphp

make_caddyfile() {
    local file="$1" num_threads="$2" root="$3" worker_script="${4:-}"
    {
        echo "{"
        echo "    auto_https off"
        echo "    admin off"
        echo "    order php_server before file_server"
        echo "    frankenphp {"
        if [[ -n "$worker_script" ]]; then
            # Worker mode: declare worker in global frankenphp block
            # (matches proven local config in dev/laravel-test/Caddyfile.worker)
            echo "        worker ${worker_script} ${num_threads}"
        else
            # Non-worker mode: limit PHP threads to N
            echo "        num_threads ${num_threads}"
        fi
        echo "        # Match Turbine OPcache+JIT settings for fair comparison"
        echo "        php_ini opcache.enable              1"
        echo "        php_ini opcache.memory_consumption  128"
        echo "        php_ini opcache.interned_strings_buffer 16"
        echo "        php_ini opcache.max_accelerated_files 10000"
        echo "        php_ini opcache.validate_timestamps 0"
        echo "        php_ini opcache.revalidate_freq     0"
        echo "        php_ini opcache.save_comments       1"
        echo "        php_ini opcache.jit                 function"
        echo "        php_ini opcache.jit_buffer_size     64M"
        echo "    }"
        echo "}"
        echo "http://:80 {"
        echo "    root * ${root}"
        echo "    php_server"
        echo "}"
    } > "$file"
}

for W in 4 8; do
    # Apps that run on FrankenPHP (raw, laravel, php-bench — NOT phalcon)
    for APP in raw laravel; do
        make_caddyfile /etc/frankenphp/${APP}-${W}w.Caddyfile         ${W} /app/public
        make_caddyfile /etc/frankenphp/${APP}-${W}w-worker.Caddyfile  ${W} /app/public /app/public/worker.php
    done
    # PHP-bench: uses public/ subdir like other apps for FrankenPHP compatibility
    make_caddyfile /etc/frankenphp/php-bench-${W}w.Caddyfile        ${W} /app/public
    make_caddyfile /etc/frankenphp/php-bench-${W}w-worker.Caddyfile ${W} /app/public /app/public/worker.php
done

log "Setup complete!"
