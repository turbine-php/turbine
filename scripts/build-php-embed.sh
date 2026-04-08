#!/usr/bin/env bash
set -euo pipefail

# Build PHP with embed SAPI for Turbine Runtime
# Usage: ./scripts/build-php-embed.sh [PHP_VERSION]

PHP_VERSION="${1:-8.5.4}"
PHP_MAJOR_MINOR=$(echo "$PHP_VERSION" | cut -d. -f1,2)
BUILD_DIR="$(cd "$(dirname "$0")/.." && pwd)/vendor/php-build"
INSTALL_DIR="$(cd "$(dirname "$0")/.." && pwd)/vendor/php-embed"
NPROC=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)

# Set ZTS_BUILD=1 to compile a Thread-Safe (ZTS) build of PHP.
# ZTS enables the multi-thread worker model: one process, N pthreads,
# shared OPcache, zero IPC. Required for turbine thread pool mode.
# Default: NTS (non-thread-safe) — compatible with fork+pipe worker pool.
ZTS_BUILD="${ZTS_BUILD:-0}"

echo "=== Building PHP ${PHP_VERSION} with embed SAPI ==="
echo "Build dir:   ${BUILD_DIR}"
echo "Install dir: ${INSTALL_DIR}"
echo "Parallel:    ${NPROC} jobs"
echo "ZTS:         ${ZTS_BUILD}"
echo ""

# Dependencies (macOS)
if command -v brew &>/dev/null; then
    echo "--- Installing build dependencies via Homebrew ---"
    brew install --quiet \
        autoconf automake bison re2c pkg-config \
        libxml2 sqlite openssl@3 zlib curl libpng \
        oniguruma libzip icu4c libiconv \
        libsodium gmp libffi \
        libjpeg-turbo webp freetype 2>/dev/null || true
fi

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

# Download PHP source
TARBALL="php-${PHP_VERSION}.tar.xz"
if [ ! -f "${TARBALL}" ]; then
    echo "--- Downloading PHP ${PHP_VERSION} ---"
    curl -fSL "https://www.php.net/distributions/${TARBALL}" -o "${TARBALL}"
fi

# Extract
if [ ! -d "php-${PHP_VERSION}" ]; then
    echo "--- Extracting ---"
    tar xf "${TARBALL}"
fi

cd "php-${PHP_VERSION}"

# Detect Homebrew paths (Apple Silicon vs Intel)
BREW_PREFIX=$(brew --prefix 2>/dev/null || echo "/opt/homebrew")
OPENSSL_DIR=$(brew --prefix openssl@3 2>/dev/null || echo "${BREW_PREFIX}/opt/openssl@3")
ICU_DIR=$(brew --prefix icu4c 2>/dev/null || echo "${BREW_PREFIX}/opt/icu4c")
LIBXML2_DIR=$(brew --prefix libxml2 2>/dev/null || echo "${BREW_PREFIX}/opt/libxml2")
LIBZIP_DIR=$(brew --prefix libzip 2>/dev/null || echo "${BREW_PREFIX}/opt/libzip")
ONIG_DIR=$(brew --prefix oniguruma 2>/dev/null || echo "${BREW_PREFIX}/opt/oniguruma")
LIBICONV_DIR=$(brew --prefix libiconv 2>/dev/null || echo "${BREW_PREFIX}/opt/libiconv")
SODIUM_DIR=$(brew --prefix libsodium 2>/dev/null || echo "${BREW_PREFIX}/opt/libsodium")
GMP_DIR=$(brew --prefix gmp 2>/dev/null || echo "${BREW_PREFIX}/opt/gmp")
FFI_DIR=$(brew --prefix libffi 2>/dev/null || echo "${BREW_PREFIX}/opt/libffi")
JPEG_DIR=$(brew --prefix libjpeg-turbo 2>/dev/null || echo "${BREW_PREFIX}/opt/libjpeg-turbo")
WEBP_DIR=$(brew --prefix webp 2>/dev/null || echo "${BREW_PREFIX}/opt/webp")
FREETYPE_DIR=$(brew --prefix freetype 2>/dev/null || echo "${BREW_PREFIX}/opt/freetype")
BISON=$(brew --prefix bison 2>/dev/null)/bin/bison

export PATH="${BISON%/*}:${PATH}"
export PKG_CONFIG_PATH="${OPENSSL_DIR}/lib/pkgconfig:${ICU_DIR}/lib/pkgconfig:${LIBXML2_DIR}/lib/pkgconfig:${LIBZIP_DIR}/lib/pkgconfig:${ONIG_DIR}/lib/pkgconfig:${LIBICONV_DIR}/lib/pkgconfig:${SODIUM_DIR}/lib/pkgconfig:${GMP_DIR}/lib/pkgconfig:${FFI_DIR}/lib/pkgconfig:${JPEG_DIR}/lib/pkgconfig:${WEBP_DIR}/lib/pkgconfig:${FREETYPE_DIR}/lib/pkgconfig:${PKG_CONFIG_PATH:-}"

echo "--- Configuring PHP ${PHP_VERSION} with embed SAPI ---"

# Build ZTS or NTS configure options
ZTS_FLAGS=""
if [ "${ZTS_BUILD}" = "1" ]; then
    ZTS_FLAGS="--enable-zts"
    echo "    ZTS (Thread-Safe) build enabled"
    # ZTS install goes to a separate dir so NTS and ZTS can coexist
    INSTALL_DIR="${INSTALL_DIR}-zts"
    echo "    Installing to: ${INSTALL_DIR}"
fi

./configure \
    --prefix="${INSTALL_DIR}" \
    --enable-embed=shared \
    --enable-opcache \
    --enable-mbstring \
    --enable-intl \
    --enable-bcmath \
    --enable-sockets \
    --enable-pcntl \
    --enable-gd \
    --enable-ftp=no \
    ${ZTS_FLAGS} \
    --with-iconv="${LIBICONV_DIR}" \
    --with-openssl="${OPENSSL_DIR}" \
    --with-zlib \
    --with-curl \
    --with-pdo-mysql=mysqlnd \
    --with-mysqli=mysqlnd \
    --with-pdo-sqlite \
    --with-sqlite3 \
    --with-zip="${LIBZIP_DIR}" \
    --with-libxml \
    --with-sodium \
    --with-gmp="${GMP_DIR}" \
    --with-ffi \
    --with-jpeg \
    --with-webp \
    --with-freetype \
    --with-config-file-path="${INSTALL_DIR}/etc" \
    --disable-cgi \
    --disable-phpdbg \
    2>&1 | tail -5

echo "--- Compiling (${NPROC} parallel jobs) ---"
make -j"${NPROC}" 2>&1 | tail -3

echo "--- Installing ---"
make install 2>&1 | tail -3

echo ""
echo "=== PHP embed SAPI built successfully ==="
echo "libphp location: ${INSTALL_DIR}/lib/libphp.dylib (macOS) or libphp.so (Linux)"
echo ""
echo "To use with Turbine, set:"
echo "  export PHP_CONFIG=${INSTALL_DIR}/bin/php-config"
echo ""
if [ "${ZTS_BUILD}" = "1" ]; then
    echo "ZTS build — enables thread pool worker model:"
    echo "  ZTS_BUILD=1 cargo build   (thread pool, shared OPcache, zero IPC)"
else
    echo "NTS build — fork+pipe worker pool (default):"
    echo "  cargo build   (persistent PHP worker mode)"
    echo ""
    echo "For ZTS thread pool mode, rebuild with:"
    echo "  ZTS_BUILD=1 ./scripts/build-php-embed.sh"
fi
echo ""
echo "Then build Turbine:"
echo "  cargo build"
