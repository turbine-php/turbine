# Hot Reload

Turbine includes a file watcher that automatically restarts workers when PHP files change. This is intended for **development only**.

## Configuration

```toml
[watcher]
enabled = true
# Directories to watch
paths = ["app/", "config/", "routes/", "src/", "public/"]
# File extensions to watch
extensions = ["php", "env"]
# Debounce delay to batch rapid changes (milliseconds)
debounce_ms = 500
```

## How It Works

1. Turbine monitors the specified directories using OS-level file notifications (inotify on Linux, FSEvents on macOS)
2. When a watched file changes, Turbine waits for `debounce_ms` to batch multiple saves
3. All PHP workers are gracefully restarted (current requests complete first)
4. OPcache is invalidated for changed files
5. Workers re-bootstrap with the updated code

## Laravel Development Setup

```toml
[watcher]
enabled = true
paths = ["app/", "config/", "routes/", "resources/views/", "database/"]
extensions = ["php", "env", "blade.php"]
debounce_ms = 300
```

## Production Warning

**Never enable the watcher in production.** File watching:
- Uses additional CPU and memory
- Causes brief service interruptions during restarts
- Is designed for development convenience, not production reliability

```toml
# Production
[watcher]
enabled = false
```

For production deployments, restart the server manually after deploying new code:

```bash
# Graceful restart (SIGHUP)
kill -HUP $(cat /var/run/turbine.pid)

# Or stop and start
kill $(cat /var/run/turbine.pid)
turbine serve --root /var/www/app
```
