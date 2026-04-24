# Dockerfile.profile — Minimal Turbine image tailored for local CPU profiling
#
# Differences vs docker/Dockerfile:
#   - Builds NATIVELY (no --platform emulation on Apple Silicon).
#   - Skips Phalcon + Redis (unused by mandelbrot/pdf50k/helloworld workloads).
#   - Bakes samply into the runtime image so the same container can attach to
#     turbine's PID without a sidecar.
#   - Keeps libphp.so and the turbine binary at canonical paths so samply can
#     resolve their symbols.
#
# Build (NTS, default):
#   docker build -t turbine-profile:nts -f docker/Dockerfile.profile .
# Build (ZTS):
#   docker build -t turbine-profile:zts -f docker/Dockerfile.profile --build-arg ZTS=1 .
#
# Build ARGs:
#   PHP_VERSION     — PHP source tarball version (default 8.5.1)
#   ZTS             — 1 = thread-safe, 0 = NTS (default 0)
#   SAMPLY_VERSION  — samply release (default 0.13.1)

# ── Stage 1: builder ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive
ARG PHP_VERSION=8.5.1
ARG ZTS=0

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential autoconf automake bison re2c pkg-config \
    curl ca-certificates wget git \
    libxml2-dev libsqlite3-dev libssl-dev zlib1g-dev \
    libcurl4-openssl-dev libonig-dev libzip-dev \
    libicu-dev libsodium-dev libffi-dev xz-utils \
    libc++-dev libc++abi-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# ── PHP embed SAPI (minimal: opcache + jit + core extensions) ──────────────
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
        --with-openssl \
        --with-zlib \
        --with-curl \
        --with-pdo-sqlite \
        --with-sqlite3 \
        --with-zip \
        --with-libxml \
        --with-sodium \
        --with-ffi \
        --with-config-file-path=/opt/php-embed/etc \
        --disable-cgi \
        --disable-phpdbg \
        $([ "${ZTS}" = "1" ] && echo "--enable-zts" || true) \
    && make -j"$(nproc)" \
    && make install \
    && make install-headers

# ── Turbine ─────────────────────────────────────────────────────────────────
COPY Cargo.toml Cargo.lock* /build/turbine/
COPY crates /build/turbine/crates
WORKDIR /build/turbine
RUN PHP_CONFIG=/opt/php-embed/bin/php-config cargo build --release \
    && strip target/release/turbine

# ── samply (static musl binary) ─────────────────────────────────────────────
ARG SAMPLY_VERSION=0.13.1
RUN ARCH=$(uname -m) \
    && case "$ARCH" in \
         x86_64)  SAMPLY_ARCH="x86_64-unknown-linux-musl" ;; \
         aarch64) SAMPLY_ARCH="aarch64-unknown-linux-gnu"  ;; \
         *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;; \
       esac \
    && curl -fsSL -o /tmp/samply.tar.xz \
       "https://github.com/mstange/samply/releases/download/samply-v${SAMPLY_VERSION}/samply-${SAMPLY_ARCH}.tar.xz" \
    && tar -C /tmp -xf /tmp/samply.tar.xz \
    && install -m 0755 "$(find /tmp -name samply -type f | head -1)" /usr/local/bin/samply \
    && samply --version

# ── Stage 2: runtime ───────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    libxml2 libsqlite3-0 libssl3 zlib1g \
    libcurl4 libonig5 libzip4 \
    libicu72 libsodium23 libffi8 \
    libc++1 libc++abi1 \
    ca-certificates curl procps \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/php-embed       /opt/php-embed
COPY --from=builder /build/turbine/target/release/turbine /usr/local/bin/turbine
COPY --from=builder /usr/local/bin/samply /usr/local/bin/samply

# Default app with mandelbrot / pdf50k / helloworld / jitstatus so the image
# is self-contained for profiling.
COPY docker/profile-app /var/www/html

ENV LD_LIBRARY_PATH="/opt/php-embed/lib"
ENV PATH="/opt/php-embed/bin:${PATH}"

RUN mkdir -p /tmp/turbine-sessions /tmp/turbine-opcache
WORKDIR /var/www/html

EXPOSE 80

# No ENTRYPOINT — profile-docker.sh drives the container explicitly with
# the command it needs (`turbine serve -c /var/www/html/turbine.toml`).
CMD ["turbine", "serve", "-c", "/var/www/html/turbine.toml", "-r", "/var/www/html"]
