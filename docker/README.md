# Turbine — Docker

This directory contains the multi-stage Dockerfile and default configuration for running a PHP application with Turbine inside a container.

## Files

| File | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage build: compiles PHP embed SAPI + extensions + Turbine binary |
| `turbine.toml` | Turbine runtime config mounted into the container as `/var/www/html/turbine.toml` |
| `app.env.docker.ini` | Optional app-level ini override (database hosts, etc.) — remove if not needed |

## Quick Start

### 1. Build the image

```bash
docker build -t turbine:latest -f docker/Dockerfile .
```

To change the exposed port (must match `[server] listen` in `turbine.toml`):

```bash
docker build -t turbine:latest -f docker/Dockerfile --build-arg PORT=8080 .
```

### 2. Run with your PHP application

```bash
docker run -d \
  -p 80:80 \
  -v ./my-app:/var/www/html \
  -v ./docker/turbine.toml:/var/www/html/turbine.toml:ro \
  turbine:latest
```

### 3. Use Docker Compose

```bash
# Edit docker-compose.turbine.yml to point the app volume to your PHP project
docker compose -f docker-compose.turbine.yml build
docker compose -f docker-compose.turbine.yml up -d
```

## Configuration

All runtime behaviour is driven by `turbine.toml`. The most important settings:

### Port

The port is configured in **`turbine.toml`** and mapped at runtime — the Dockerfile is not involved:

```toml
# docker/turbine.toml
[server]
listen = "0.0.0.0:8080"   # port Turbine listens on inside the container
```

```bash
# match the left side to the host port you want, right side to turbine.toml
docker run -p 8080:8080 ...
```

Or in `docker-compose.turbine.yml`:

```yaml
ports:
  - "8080:8080"
```

`EXPOSE` in the Dockerfile is just documentation — change it if you change the default port, but it has no runtime effect.

### Workers

```toml
[server]
workers = 8              # set to CPU core count of the host
worker_mode = "process"  # "process" (NTS PHP) or "thread" (ZTS PHP)
persistent_workers = true
worker_max_requests = 50000
```

### PHP Extensions

```toml
[php]
extension_dir = "/opt/php-embed/lib/php/extensions/no-debug-non-zts-20240924"
extensions = ["redis.so", "phalcon.so"]
```

The extension directory path corresponds to the PHP version compiled in the `builder` stage. If you change `PHP_VERSION` in the Dockerfile, update this path accordingly.

Extensions available in the default image: **Phalcon**, **Redis**. To add more, extend the `builder` stage in the Dockerfile following the same pattern as the Redis and Phalcon build steps.

### App Root

The app root is `/var/www/html` by default (set via the `-r` flag in `CMD`). Mount your PHP project there:

```bash
-v /path/to/my-app:/var/www/html
```

### Changing PHP Versions

Edit the `ARG` at the top of the Dockerfile:

```dockerfile
ENV PHP_VERSION=8.4.6
```

After changing the PHP version, update `extension_dir` in `turbine.toml` to match the new extension directory path (`/opt/php-embed/lib/php/extensions/<dir>`).

## Build Arguments

Nenhum `--build-arg` é necessário. O Dockerfile compila Turbine + PHP + extensões sem depender de configuração de runtime.

## Health Check

The image includes a built-in health check:

```
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:80/ || exit 1
```

Adjust the port in the Dockerfile if using a non-default port.
