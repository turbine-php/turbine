#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# Turbine Build Script
# Interactive build for the Turbine PHP runtime
# https://github.com/turbine-php/turbine
# ============================================================================

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_ROOT"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; }
header(){ echo ""; echo -e "${BOLD}── $* ──${NC}"; }

# ── Detect OS ───────────────────────────────────────────────────────────────

OS="unknown"
case "$(uname -s)" in
    Darwin*) OS="macos" ;;
    Linux*)  OS="linux" ;;
esac

ARCH="$(uname -m)"
NPROC=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)

echo ""
echo -e "${BOLD}╔══════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║         Turbine Build Script             ║${NC}"
echo -e "${BOLD}║   High-Performance PHP Runtime in Rust   ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════╝${NC}"
echo ""
echo "  OS:       ${OS} (${ARCH})"
echo "  CPUs:     ${NPROC}"
echo "  Project:  ${PROJECT_ROOT}"
echo ""

# ── Check Requirements ──────────────────────────────────────────────────────

header "Checking requirements"

MISSING=()

# Rust
if command -v rustc &>/dev/null; then
    RUST_VER=$(rustc --version | awk '{print $2}')
    ok "Rust ${RUST_VER}"
else
    fail "Rust not found — install from https://rustup.rs"
    MISSING+=("rust")
fi

# Cargo
if command -v cargo &>/dev/null; then
    ok "Cargo $(cargo --version | awk '{print $2}')"
else
    fail "Cargo not found"
    MISSING+=("cargo")
fi

# C compiler
if command -v cc &>/dev/null; then
    ok "C compiler (cc)"
elif command -v gcc &>/dev/null; then
    ok "C compiler (gcc)"
else
    fail "C compiler not found — install Xcode CLI tools (macOS) or build-essential (Linux)"
    MISSING+=("cc")
fi

# pkg-config
if command -v pkg-config &>/dev/null; then
    ok "pkg-config"
else
    fail "pkg-config not found"
    MISSING+=("pkg-config")
fi

# curl
if command -v curl &>/dev/null; then
    ok "curl"
else
    fail "curl not found"
    MISSING+=("curl")
fi

# autoconf (needed for PHP build)
if command -v autoconf &>/dev/null; then
    ok "autoconf"
else
    warn "autoconf not found — needed to compile PHP from source"
    MISSING+=("autoconf")
fi

# bison
if command -v bison &>/dev/null; then
    ok "bison"
elif [ "$OS" = "macos" ] && brew --prefix bison &>/dev/null 2>&1; then
    ok "bison (Homebrew)"
else
    warn "bison not found — needed to compile PHP from source"
    MISSING+=("bison")
fi

# re2c
if command -v re2c &>/dev/null; then
    ok "re2c"
else
    warn "re2c not found — needed to compile PHP from source"
    MISSING+=("re2c")
fi

# Homebrew (macOS only)
if [ "$OS" = "macos" ]; then
    if command -v brew &>/dev/null; then
        ok "Homebrew"
    else
        fail "Homebrew not found — install from https://brew.sh"
        MISSING+=("brew")
    fi
fi

# Check existing PHP embed installations
NTS_EXISTS=false
ZTS_EXISTS=false

if [ -f "$PROJECT_ROOT/vendor/php-embed/bin/php-config" ]; then
    NTS_VER=$("$PROJECT_ROOT/vendor/php-embed/bin/php-config" --version 2>/dev/null || echo "unknown")
    ok "PHP embed NTS ${NTS_VER} found at vendor/php-embed/"
    NTS_EXISTS=true
else
    info "PHP embed NTS not found at vendor/php-embed/"
fi

if [ -f "$PROJECT_ROOT/vendor/php-embed-zts/bin/php-config" ]; then
    ZTS_VER=$("$PROJECT_ROOT/vendor/php-embed-zts/bin/php-config" --version 2>/dev/null || echo "unknown")
    ok "PHP embed ZTS ${ZTS_VER} found at vendor/php-embed-zts/"
    ZTS_EXISTS=true
else
    info "PHP embed ZTS not found at vendor/php-embed-zts/"
fi

# Abort on critical missing
if [[ " ${MISSING[*]:-} " =~ " rust " ]] || [[ " ${MISSING[*]:-} " =~ " cargo " ]]; then
    echo ""
    fail "Cannot continue without Rust/Cargo. Install from https://rustup.rs"
    exit 1
fi

# ── Install missing dependencies ────────────────────────────────────────────

if [ ${#MISSING[@]} -gt 0 ] && [ "$OS" = "macos" ] && command -v brew &>/dev/null; then
    echo ""
    BREW_PKGS=()
    for pkg in "${MISSING[@]}"; do
        case "$pkg" in
            autoconf|bison|re2c|pkg-config|curl) BREW_PKGS+=("$pkg") ;;
        esac
    done

    if [ ${#BREW_PKGS[@]} -gt 0 ]; then
        echo -n "Install missing dependencies via Homebrew (${BREW_PKGS[*]})? [Y/n] "
        read -r INSTALL_DEPS
        if [ "$(echo "$INSTALL_DEPS" | tr "[:upper:]" "[:lower:]")" != "n" ]; then
            brew install --quiet "${BREW_PKGS[@]}" 2>/dev/null || true
            ok "Dependencies installed"
        fi
    fi
fi

# ── Choose build mode ──────────────────────────────────────────────────────

# Interactive radio selector (single choice)
# Usage: radio_select "result_var" "label1|label2|..." "desc1|desc2|..." [default_index]
radio_select() {
    local _result_var="$1"
    IFS='|' read -r -a _labels <<< "$2"
    IFS='|' read -r -a _descs  <<< "$3"
    local _count=${#_labels[@]}
    local _cursor="${4:-0}"
    local _selected="$_cursor"

    tput civis 2>/dev/null || true

    _draw_radio() {
        if [ "${_first_draw:-1}" = "0" ]; then
            printf '\033[%dA' "$_count"
        fi
        _first_draw=0

        for (( i=0; i<_count; i++ )); do
            local _dot=" "
            local _arrow="  "
            if [ "$i" = "$_selected" ]; then _dot="●"; else _dot="○"; fi
            if [ "$i" = "$_cursor" ]; then _arrow="▸ "; fi

            printf '\033[2K'
            if [ "$i" = "$_cursor" ]; then
                printf "  ${_arrow}\033[1m${_dot} ${_labels[$i]}\033[0m"
            else
                printf "  ${_arrow}${_dot} ${_labels[$i]}"
            fi
            if [ -n "${_descs[$i]:-}" ]; then
                printf "  \033[2m${_descs[$i]}\033[0m"
            fi
            printf '\n'
        done
    }

    _first_draw=1
    _draw_radio

    while true; do
        IFS= read -rsn1 _key
        case "$_key" in
            ' ')
                _selected=$_cursor
                _draw_radio
                ;;
            '')
                _selected=$_cursor
                break
                ;;
            $'\x1b')
                read -rsn2 _seq
                case "$_seq" in
                    '[A')
                        if [ "$_cursor" -gt 0 ]; then ((_cursor--))
                        else _cursor=$((_count - 1)); fi
                        _draw_radio
                        ;;
                    '[B')
                        if [ "$_cursor" -lt "$((_count - 1))" ]; then ((_cursor++))
                        else _cursor=0; fi
                        _draw_radio
                        ;;
                esac
                ;;
        esac
    done

    tput cnorm 2>/dev/null || true
    eval "$_result_var='$_selected'"
}

header "Build mode"

echo "  Turbine supports two worker modes."
echo "  (↑/↓ navigate, Enter select)"
echo ""

radio_select BUILD_MODE_IDX \
    "Process mode (NTS)|Thread mode (ZTS)|Both (NTS + ZTS)" \
    "Fork-based workers — stability, isolation|In-memory channels — max throughput|Build both, link ZTS (supports both modes)"

BUILD_NTS=false
BUILD_ZTS=false
LINK_ZTS=false

case "$BUILD_MODE_IDX" in
    0) BUILD_NTS=true ;;
    1) BUILD_ZTS=true; LINK_ZTS=true ;;
    2) BUILD_NTS=true; BUILD_ZTS=true; LINK_ZTS=true ;;
esac

# ── PHP version ─────────────────────────────────────────────────────────────

DEFAULT_PHP_VER="8.4.6"
echo ""
echo -n "PHP version (default: ${DEFAULT_PHP_VER}): "
read -r PHP_VERSION
PHP_VERSION="${PHP_VERSION:-$DEFAULT_PHP_VER}"

# ── PHP extensions ──────────────────────────────────────────────────────────

# Interactive checkbox selector
# Usage: checkbox_select "result_var" "label1|label2|..." "desc1|desc2|..."
# Returns space-separated indices (0-based) of selected items in result_var.
checkbox_select() {
    local _result_var="$1"
    IFS='|' read -r -a _labels <<< "$2"
    IFS='|' read -r -a _descs  <<< "$3"
    local _count=${#_labels[@]}
    local -a _selected=()
    local _cursor=0

    # Initialize all unselected
    for (( i=0; i<_count; i++ )); do _selected+=("false"); done

    # Hide cursor
    tput civis 2>/dev/null || true

    # Draw function
    _draw_menu() {
        # Move cursor up to redraw (except first draw)
        if [ "${_first_draw:-1}" = "0" ]; then
            printf '\033[%dA' "$_count"
        fi
        _first_draw=0

        for (( i=0; i<_count; i++ )); do
            local _check=" "
            local _arrow="  "
            if [ "${_selected[$i]}" = "true" ]; then _check="✔"; fi
            if [ "$i" = "$_cursor" ]; then _arrow="▸ "; fi

            # Clear line
            printf '\033[2K'
            if [ "$i" = "$_cursor" ]; then
                # Highlighted line
                printf "  ${_arrow}\033[1m[${_check}] ${_labels[$i]}\033[0m"
            else
                printf "  ${_arrow}[${_check}] ${_labels[$i]}"
            fi
            # Description
            if [ -n "${_descs[$i]:-}" ]; then
                printf "  \033[2m${_descs[$i]}\033[0m"
            fi
            printf '\n'
        done
    }

    _first_draw=1
    _draw_menu

    # Read keys
    while true; do
        # Read single char (raw mode)
        IFS= read -rsn1 _key

        case "$_key" in
            # Space — toggle
            ' ')
                if [ "${_selected[$_cursor]}" = "true" ]; then
                    _selected[$_cursor]="false"
                else
                    _selected[$_cursor]="true"
                fi
                _draw_menu
                ;;
            # Enter — confirm
            '')
                break
                ;;
            # Escape sequence (arrows)
            $'\x1b')
                read -rsn2 _seq
                case "$_seq" in
                    '[A') # Up
                        if [ "$_cursor" -gt 0 ]; then
                            ((_cursor--))
                        else
                            _cursor=$((_count - 1))
                        fi
                        _draw_menu
                        ;;
                    '[B') # Down
                        if [ "$_cursor" -lt "$((_count - 1))" ]; then
                            ((_cursor++))
                        else
                            _cursor=0
                        fi
                        _draw_menu
                        ;;
                esac
                ;;
            # 'a' or 'A' — select all
            a|A)
                for (( i=0; i<_count; i++ )); do _selected[$i]="true"; done
                _draw_menu
                ;;
            # 'n' or 'N' — deselect all
            n|N)
                for (( i=0; i<_count; i++ )); do _selected[$i]="false"; done
                _draw_menu
                ;;
        esac
    done

    # Show cursor
    tput cnorm 2>/dev/null || true

    # Build result
    local _result=""
    for (( i=0; i<_count; i++ )); do
        if [ "${_selected[$i]}" = "true" ]; then
            _result="${_result} ${i}"
        fi
    done
    eval "$_result_var='${_result# }'"
}

header "PHP extensions"

echo "  The following extensions are always included (compiled into PHP):"
echo "    opcache, mbstring, intl, bcmath, sockets, pcntl, gd, openssl,"
echo "    curl, pdo_mysql, mysqli, pdo_sqlite, sqlite3, zip, sodium,"
echo "    gmp, ffi, libxml, zlib, json, session, fileinfo, iconv"
echo ""
echo "  Select optional PECL extensions to install:"
echo "  (↑/↓ navigate, Space toggle, A select all, N clear, Enter confirm)"
echo ""

checkbox_select SELECTED_EXT_INDICES \
    "Phalcon|Redis|Imagick|APCu|Xdebug" \
    "High-performance PHP framework (C extension)|PHP Redis client|ImageMagick bindings|User data cache|Debugger and profiler (dev only)"

INSTALL_PHALCON=false
INSTALL_REDIS=false
INSTALL_IMAGICK=false
INSTALL_APCU=false
INSTALL_XDEBUG=false

for idx in $SELECTED_EXT_INDICES; do
    case "$idx" in
        0) INSTALL_PHALCON=true ;;
        1) INSTALL_REDIS=true ;;
        2) INSTALL_IMAGICK=true ;;
        3) INSTALL_APCU=true ;;
        4) INSTALL_XDEBUG=true ;;
    esac
done

# Check system dependencies for selected extensions
if $INSTALL_IMAGICK; then
    if [ "$OS" = "macos" ]; then
        if ! brew list imagemagick &>/dev/null 2>&1; then
            info "Imagick requires ImageMagick library"
            echo -n "  Install ImageMagick via Homebrew? [Y/n] "
            read -r INSTALL_IM
            if [ "$(echo "$INSTALL_IM" | tr "[:upper:]" "[:lower:]")" != "n" ]; then
                brew install --quiet imagemagick 2>/dev/null || true
            fi
        fi
    elif [ "$OS" = "linux" ]; then
        if ! pkg-config --exists MagickWand 2>/dev/null; then
            warn "ImageMagick development headers not found"
            info "  Install with: apt install libmagickwand-dev"
        fi
    fi
fi

# ── Release or debug ────────────────────────────────────────────────────────

echo ""
echo -n "Build Turbine in release mode? [Y/n] "
read -r RELEASE_MODE
RELEASE_MODE="${RELEASE_MODE:-y}"
CARGO_PROFILE=""
if [ "$(echo "$RELEASE_MODE" | tr "[:upper:]" "[:lower:]")" != "n" ]; then
    CARGO_PROFILE="--release"
    BINARY_DIR="target/release"
else
    BINARY_DIR="target/debug"
fi

# ── Summary ─────────────────────────────────────────────────────────────────

header "Build plan"

echo "  PHP version:    ${PHP_VERSION}"
if $BUILD_NTS; then echo "  Build NTS:      yes → vendor/php-embed/"; fi
if $BUILD_ZTS; then echo "  Build ZTS:      yes → vendor/php-embed-zts/"; fi
if $LINK_ZTS; then
    echo "  Link against:   ZTS (thread + process modes)"
else
    echo "  Link against:   NTS (process mode only)"
fi
echo "  Rust profile:   $([ -n "$CARGO_PROFILE" ] && echo "release" || echo "debug")"
echo "  Output:         ${BINARY_DIR}/turbine"

# Show selected extensions
SELECTED_EXTS=""
$INSTALL_PHALCON && SELECTED_EXTS="${SELECTED_EXTS} phalcon"
$INSTALL_REDIS   && SELECTED_EXTS="${SELECTED_EXTS} redis"
$INSTALL_IMAGICK && SELECTED_EXTS="${SELECTED_EXTS} imagick"
$INSTALL_APCU    && SELECTED_EXTS="${SELECTED_EXTS} apcu"
$INSTALL_XDEBUG  && SELECTED_EXTS="${SELECTED_EXTS} xdebug"
if [ -n "$SELECTED_EXTS" ]; then
    echo "  PECL extensions:${SELECTED_EXTS}"
else
    echo "  PECL extensions: (none)"
fi
echo ""

echo -n "Proceed? [Y/n] "
read -r CONFIRM
if [ "$(echo "$CONFIRM" | tr "[:upper:]" "[:lower:]")" = "n" ]; then
    echo "Aborted."
    exit 0
fi

# ── Install macOS build dependencies for PHP ────────────────────────────────

if ($BUILD_NTS || $BUILD_ZTS) && [ "$OS" = "macos" ] && command -v brew &>/dev/null; then
    header "Installing PHP build dependencies"
    brew install --quiet \
        autoconf automake bison re2c pkg-config \
        libxml2 sqlite openssl@3 zlib curl libpng \
        oniguruma libzip icu4c libiconv \
        libsodium gmp libffi \
        libjpeg-turbo webp freetype 2>/dev/null || true
    ok "PHP build dependencies ready"
fi

# ── Build PHP NTS ───────────────────────────────────────────────────────────

build_php() {
    local ZTS_FLAG="$1"
    local LABEL="$2"
    local INSTALL="$3"

    header "Building PHP ${PHP_VERSION} (${LABEL})"

    if [ -f "${INSTALL}/bin/php-config" ]; then
        EXISTING_VER=$("${INSTALL}/bin/php-config" --version 2>/dev/null || echo "")
        if [ "$EXISTING_VER" = "$PHP_VERSION" ]; then
            ok "PHP ${PHP_VERSION} ${LABEL} already built — skipping"
            echo -n "  Rebuild anyway? [y/N] "
            read -r REBUILD
            if [ "$(echo "$REBUILD" | tr "[:upper:]" "[:lower:]")" != "y" ]; then
                return 0
            fi
        fi
    fi

    local BUILD_DIR="$PROJECT_ROOT/vendor/php-build"
    mkdir -p "$BUILD_DIR"
    cd "$BUILD_DIR"

    # Download
    local TARBALL="php-${PHP_VERSION}.tar.xz"
    if [ ! -f "${TARBALL}" ]; then
        info "Downloading PHP ${PHP_VERSION}..."
        curl -fSL "https://www.php.net/distributions/${TARBALL}" -o "${TARBALL}"
    fi

    # Extract
    if [ ! -d "php-${PHP_VERSION}" ]; then
        info "Extracting..."
        tar xf "${TARBALL}"
    fi

    cd "php-${PHP_VERSION}"

    # Clean previous build if configure was run with different flags
    if [ -f Makefile ]; then
        make distclean 2>/dev/null || true
    fi

    # Detect library paths
    if [ "$OS" = "macos" ]; then
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
        BISON_DIR=$(brew --prefix bison 2>/dev/null || echo "${BREW_PREFIX}/opt/bison")

        export PATH="${BISON_DIR}/bin:${PATH}"
        export PKG_CONFIG_PATH="${OPENSSL_DIR}/lib/pkgconfig:${ICU_DIR}/lib/pkgconfig:${LIBXML2_DIR}/lib/pkgconfig:${LIBZIP_DIR}/lib/pkgconfig:${ONIG_DIR}/lib/pkgconfig:${LIBICONV_DIR}/lib/pkgconfig:${SODIUM_DIR}/lib/pkgconfig:${GMP_DIR}/lib/pkgconfig:${FFI_DIR}/lib/pkgconfig:${JPEG_DIR}/lib/pkgconfig:${WEBP_DIR}/lib/pkgconfig:${FREETYPE_DIR}/lib/pkgconfig:${PKG_CONFIG_PATH:-}"

        EXTRA_CONFIGURE=(
            "--with-iconv=${LIBICONV_DIR}"
            "--with-openssl=${OPENSSL_DIR}"
            "--with-zip=${LIBZIP_DIR}"
            "--with-gmp=${GMP_DIR}"
        )
    else
        EXTRA_CONFIGURE=(
            "--with-iconv"
            "--with-openssl"
            "--with-zip"
            "--with-gmp"
        )
    fi

    local ZTS_CONFIGURE=()
    if [ "$ZTS_FLAG" = "1" ]; then
        ZTS_CONFIGURE=("--enable-zts")
    fi

    info "Configuring PHP ${PHP_VERSION} (${LABEL})..."
    ./configure \
        --prefix="${INSTALL}" \
        --enable-embed=shared \
        --enable-opcache \
        --enable-mbstring \
        --enable-intl \
        --enable-bcmath \
        --enable-sockets \
        --enable-pcntl \
        --enable-gd \
        --enable-ftp=no \
        ${ZTS_CONFIGURE[@]+"${ZTS_CONFIGURE[@]}"} \
        ${EXTRA_CONFIGURE[@]+"${EXTRA_CONFIGURE[@]}"} \
        --with-zlib \
        --with-curl \
        --with-pdo-mysql=mysqlnd \
        --with-mysqli=mysqlnd \
        --with-pdo-sqlite \
        --with-sqlite3 \
        --with-libxml \
        --with-sodium \
        --with-ffi \
        --with-jpeg \
        --with-webp \
        --with-freetype \
        --with-config-file-path="${INSTALL}/etc" \
        --disable-cgi \
        --disable-phpdbg \
        2>&1 | tail -3

    info "Compiling (${NPROC} parallel jobs)..."
    make -j"${NPROC}" 2>&1 | tail -3

    info "Installing to ${INSTALL}..."
    make install 2>&1 | tail -3

    ok "PHP ${PHP_VERSION} ${LABEL} built successfully"
    cd "$PROJECT_ROOT"
}

if $BUILD_NTS; then
    build_php "0" "NTS" "$PROJECT_ROOT/vendor/php-embed"
fi

if $BUILD_ZTS; then
    build_php "1" "ZTS" "$PROJECT_ROOT/vendor/php-embed-zts"
fi

# ── Install PECL extensions ─────────────────────────────────────────────────

install_pecl_ext() {
    local EXT_NAME="$1"
    local PECL_PKG="$2"
    local PHP_DIR="$3"
    local LABEL="$4"

    local PECL_BIN="${PHP_DIR}/bin/pecl"
    local PHPIZE_BIN="${PHP_DIR}/bin/phpize"
    local PHP_BIN="${PHP_DIR}/bin/php"
    local EXT_DIR
    EXT_DIR=$("${PHP_DIR}/bin/php-config" --extension-dir 2>/dev/null)

    # Check if already installed
    if [ -f "${EXT_DIR}/${EXT_NAME}.so" ]; then
        ok "${EXT_NAME}.so already exists in ${LABEL} — skipping"
        return 0
    fi

    info "Installing ${EXT_NAME} for ${LABEL}..."

    if [ "$EXT_NAME" = "phalcon" ]; then
        # Phalcon uses its own build process via pecl or git
        echo "" | "${PECL_BIN}" install phalcon 2>&1 | tail -5 || {
            warn "pecl install phalcon failed — trying from source..."
            local PHALCON_BUILD_DIR="${PROJECT_ROOT}/vendor/php-build/cphalcon"
            if [ ! -d "$PHALCON_BUILD_DIR" ]; then
                info "Cloning Phalcon source..."
                git clone --depth 1 https://github.com/phalcon/cphalcon.git "$PHALCON_BUILD_DIR" 2>&1 | tail -3
            fi
            cd "${PHALCON_BUILD_DIR}/build"
            export PATH="${PHP_DIR}/bin:${PATH}"
            ./install 2>&1 | tail -5 || {
                fail "Failed to build Phalcon for ${LABEL}"
                cd "$PROJECT_ROOT"
                return 1
            }
            cd "$PROJECT_ROOT"
        }
    else
        # Standard PECL install (redis, imagick, apcu, xdebug)
        local PECL_INPUT=""
        # imagick needs no prompts; redis has optional options
        echo "" | "${PECL_BIN}" install "${PECL_PKG}" 2>&1 | tail -5 || {
            fail "Failed to install ${EXT_NAME} for ${LABEL}"
            return 1
        }
    fi

    if [ -f "${EXT_DIR}/${EXT_NAME}.so" ]; then
        ok "${EXT_NAME} installed for ${LABEL}"
    else
        warn "${EXT_NAME}.so not found after install — check logs"
    fi
}

HAS_PECL_EXTS=false
($INSTALL_PHALCON || $INSTALL_REDIS || $INSTALL_IMAGICK || $INSTALL_APCU || $INSTALL_XDEBUG) && HAS_PECL_EXTS=true

if $HAS_PECL_EXTS; then
    header "Installing PECL extensions"

    # Determine which PHP dirs to install into
    PECL_TARGETS=()
    if $BUILD_NTS && [ -f "$PROJECT_ROOT/vendor/php-embed/bin/pecl" ]; then
        PECL_TARGETS+=("$PROJECT_ROOT/vendor/php-embed|NTS")
    fi
    if $BUILD_ZTS && [ -f "$PROJECT_ROOT/vendor/php-embed-zts/bin/pecl" ]; then
        PECL_TARGETS+=("$PROJECT_ROOT/vendor/php-embed-zts|ZTS")
    fi
    # If we didn't build PHP this run, check existing installs
    if [ ${#PECL_TARGETS[@]} -eq 0 ]; then
        if $LINK_ZTS && [ -f "$PROJECT_ROOT/vendor/php-embed-zts/bin/pecl" ]; then
            PECL_TARGETS+=("$PROJECT_ROOT/vendor/php-embed-zts|ZTS")
        elif [ -f "$PROJECT_ROOT/vendor/php-embed/bin/pecl" ]; then
            PECL_TARGETS+=("$PROJECT_ROOT/vendor/php-embed|NTS")
        fi
    fi

    if [ ${#PECL_TARGETS[@]} -eq 0 ]; then
        warn "No PHP installation with pecl found — skipping PECL extensions"
    else
        for target_entry in "${PECL_TARGETS[@]}"; do
            PHP_DIR="${target_entry%%|*}"
            TARGET_LABEL="${target_entry##*|}"

            $INSTALL_PHALCON && install_pecl_ext "phalcon" "phalcon"  "$PHP_DIR" "$TARGET_LABEL"
            $INSTALL_REDIS   && install_pecl_ext "redis"   "redis"    "$PHP_DIR" "$TARGET_LABEL"
            $INSTALL_IMAGICK && install_pecl_ext "imagick" "imagick"  "$PHP_DIR" "$TARGET_LABEL"
            $INSTALL_APCU    && install_pecl_ext "apcu"    "apcu"     "$PHP_DIR" "$TARGET_LABEL"
            $INSTALL_XDEBUG  && install_pecl_ext "xdebug"  "xdebug"  "$PHP_DIR" "$TARGET_LABEL"
        done
    fi
fi

# ── Build Turbine ───────────────────────────────────────────────────────────

header "Building Turbine"

if $LINK_ZTS; then
    PHP_CONFIG_PATH="$PROJECT_ROOT/vendor/php-embed-zts/bin/php-config"
    LIB_PATH="$PROJECT_ROOT/vendor/php-embed-zts/lib"
else
    PHP_CONFIG_PATH="$PROJECT_ROOT/vendor/php-embed/bin/php-config"
    LIB_PATH="$PROJECT_ROOT/vendor/php-embed/lib"
fi

if [ ! -f "$PHP_CONFIG_PATH" ]; then
    fail "php-config not found at ${PHP_CONFIG_PATH}"
    fail "PHP embed library must be built first."
    exit 1
fi

info "Linking against: ${PHP_CONFIG_PATH}"
info "Profile: $([ -n "$CARGO_PROFILE" ] && echo "release" || echo "debug")"

export PHP_CONFIG="$PHP_CONFIG_PATH"

cargo build ${CARGO_PROFILE} 2>&1

BINARY="${BINARY_DIR}/turbine"

if [ ! -f "$BINARY" ]; then
    fail "Build failed — binary not found at ${BINARY}"
    exit 1
fi

SIZE=$(du -h "$BINARY" | cut -f1 | tr -d ' ')

# ── Done ────────────────────────────────────────────────────────────────────

header "Build complete"

echo ""
echo -e "  ${GREEN}Binary:${NC}  ${BINARY} (${SIZE})"
echo ""
echo "  Run with:"
echo ""
if [ "$OS" = "macos" ]; then
    echo -e "    ${CYAN}export DYLD_LIBRARY_PATH=\"${LIB_PATH}\"${NC}"
else
    echo -e "    ${CYAN}export LD_LIBRARY_PATH=\"${LIB_PATH}\"${NC}"
fi
echo -e "    ${CYAN}${BINARY} serve --root /path/to/your/app${NC}"
echo ""
if $LINK_ZTS; then
    echo "  Worker modes available:"
    echo "    worker_mode = \"process\"  (fork-based, default)"
    echo "    worker_mode = \"thread\"   (in-memory channels)"
else
    echo "  Worker mode: process (fork-based)"
    echo "  For thread mode, rebuild with option 2 or 3."
fi

if [ -n "$SELECTED_EXTS" ]; then
    echo ""
    echo "  PECL extensions installed:${SELECTED_EXTS}"
    echo "  Load them in turbine.toml:"
    echo ""
    echo "    [php]"
    for ext in $SELECTED_EXTS; do
        if [ "$ext" = "xdebug" ]; then
            echo "    # zend_extensions = [\"${ext}.so\"]"
        else
            echo "    # extensions = [\"${ext}.so\"]"
        fi
    done
fi
echo ""
