# Turbine Runtime — Docker image
# Multi-stage build: compile PHP embed + extensions + Turbine, then run
#
# Build (NTS):  docker build -t turbine:latest -f docker/Dockerfile .
# Build (ZTS):  docker build -t turbine:latest-zts -f docker/Dockerfile --build-arg ZTS=1 .
# Run:          docker run -e PORT=8080 -p 8080:8080 -v ./app:/var/www/html turbine:latest
#
# Build ARGs:
#   PHP_VERSION   — full PHP version (e.g. 8.4.6, 8.5.1)
#   ZTS           — 1 = thread-safe build, 0 = NTS (default)
#   PHALCON_VERSION, PHPREDIS_VERSION — extension versions
#   PORT          — default port exposed (default 80)
#
# PORT env var drives [server] listen in turbine.toml at startup.

# ── Stage 1: Build PHP embed SAPI + extensions + Turbine binary ──────────────
FROM debian:bookworm-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive

ARG PHP_VERSION=8.4.6
ARG PHALCON_VERSION=5.11.1
ARG PHPREDIS_VERSION=6.1.0
# ZTS=1 enables --enable-zts (thread-safe PHP)
ARG ZTS=0

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential autoconf automake bison re2c pkg-config \
    curl ca-certificates wget git \
    libxml2-dev libsqlite3-dev libssl-dev zlib1g-dev \
    libcurl4-openssl-dev libpng-dev libonig-dev libzip-dev \
    libicu-dev libsodium-dev libgmp-dev libffi-dev \
    libjpeg62-turbo-dev libwebp-dev libfreetype-dev \
    libc++-dev libc++abi-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# ── Build PHP embed SAPI ─────────────────────────────────────────────────────
RUN curl -fSL "https://www.php.net/distributions/php-${PHP_VERSION}.tar.xz" -o php.tar.xz \
    && tar xf php.tar.xz \
    && cd php-${PHP_VERSION} \
    && ./configure \
        --prefix=/opt/php-embed \
        --enable-embed=shared \
        --enable-opcache \
        --enable-mbstring \
        --enable-intl \
        --enable-bcmath \
        --enable-sockets \
        --enable-pcntl \
        --enable-gd \
        --with-openssl \
        --with-zlib \
        --with-curl \
        --with-pdo-mysql=mysqlnd \
        --with-mysqli=mysqlnd \
        --with-pdo-sqlite \
        --with-sqlite3 \
        --with-zip \
        --with-libxml \
        --with-sodium \
        --with-gmp \
        --with-ffi \
        --with-jpeg \
        --with-webp \
        --with-freetype \
        --with-config-file-path=/opt/php-embed/etc \
        --disable-cgi \
        --disable-phpdbg \
        $([ "${ZTS}" = "1" ] && echo "--enable-zts" || true) \
    && make -j"$(nproc)" \
    && make install \
    && make install-headers \
    && echo "PHP embed built OK (ZTS=${ZTS})"

# ── Build Phalcon extension ──────────────────────────────────────────────────
RUN EXT_DIR=$(/opt/php-embed/bin/php-config --extension-dir) \
    && mkdir -p "${EXT_DIR}" \
    && curl -fsSL "https://github.com/phalcon/cphalcon/archive/refs/tags/v${PHALCON_VERSION}.tar.gz" -o phalcon.tar.gz \
    && mkdir -p cphalcon && tar xf phalcon.tar.gz -C cphalcon --strip-components=1 \
    && cd cphalcon/ext \
    && /opt/php-embed/bin/phpize \
    && ./configure --with-php-config=/opt/php-embed/bin/php-config \
    && make -j"$(nproc)" \
    && cp modules/phalcon.so "${EXT_DIR}/" \
    && echo "Phalcon built OK"

# ── Build Redis extension ────────────────────────────────────────────────────
RUN EXT_DIR=$(/opt/php-embed/bin/php-config --extension-dir) \
    && mkdir -p "${EXT_DIR}" \
    && curl -fsSL "https://github.com/phpredis/phpredis/archive/refs/tags/${PHPREDIS_VERSION}.tar.gz" -o phpredis.tar.gz \
    && mkdir -p phpredis && tar xf phpredis.tar.gz -C phpredis --strip-components=1 \
    && cd phpredis \
    && /opt/php-embed/bin/phpize \
    && ./configure --with-php-config=/opt/php-embed/bin/php-config \
    && make -j"$(nproc)" \
    && cp modules/redis.so "${EXT_DIR}/" \
    && echo "Redis extension built OK"

# Bake the extension dir path into an env so runtime stage and entrypoint can use it
RUN /opt/php-embed/bin/php-config --extension-dir > /opt/php-embed/ext-dir.txt

# ── Build Turbine ────────────────────────────────────────────────────────────
COPY Cargo.toml Cargo.lock* /build/turbine/
COPY crates /build/turbine/crates
WORKDIR /build/turbine

RUN PHP_CONFIG=/opt/php-embed/bin/php-config cargo build --release \
    && strip target/release/turbine \
    && echo "Turbine built OK"

# ── Stage 2: Runtime image ───────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive

# Runtime libraries only (no dev packages)
RUN apt-get update && apt-get install -y --no-install-recommends \
    libxml2 libsqlite3-0 libssl3 zlib1g \
    libcurl4 libpng16-16 libonig5 libzip4 \
    libicu72 libsodium23 libgmp10 libffi8 \
    libjpeg62-turbo libwebp7 libfreetype6 \
    libc++1 libc++abi1 \
    ca-certificates curl gettext-base \
    && rm -rf /var/lib/apt/lists/*

# Copy PHP embed SAPI
COPY --from=builder /opt/php-embed /opt/php-embed

# Expose extension dir path to the entrypoint for turbine.toml substitution
RUN cat /opt/php-embed/ext-dir.txt

# Copy Turbine binary
COPY --from=builder /build/turbine/target/release/turbine /usr/local/bin/turbine

# Copy entrypoint and default config template
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
COPY docker/turbine.toml /etc/turbine/turbine.toml.tmpl

# Set library path for PHP embed
ENV LD_LIBRARY_PATH="/opt/php-embed/lib"
ENV PATH="/opt/php-embed/bin:${PATH}"

# Create app directory
RUN mkdir -p /var/www/html /tmp/turbine-sessions

WORKDIR /var/www/html

# PORT is set via env var at runtime (default 80) — drives turbine.toml listen
ARG PORT=80
ENV PORT=${PORT}
EXPOSE ${PORT}

# Health check
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:80/ || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD []
