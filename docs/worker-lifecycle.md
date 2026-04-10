# Worker Lifecycle (Lightweight Boot)

> **The biggest performance win in Turbine.** When enabled, framework boot
> happens **once per worker** instead of once per request — giving up to
> 8–12× throughput improvement over PHP-FPM.

## Overview

By default, persistent workers (`persistent_workers = true`) run a full
`php_request_startup` / `php_request_shutdown` cycle for every request.
This is safe and correct, but it re-initializes extensions and rebuilds
PHP superglobals from scratch each time.

When you add **`worker_boot`** and **`worker_handler`** to your config,
Turbine switches to a *lightweight lifecycle*:

1. **Boot phase** (once per worker): executes `worker_boot` to load the
   autoloader, boot the framework, and store the app in `$GLOBALS`.
2. **Request phase** (per request): rearms `$_GET`, `$_POST`, `$_SERVER`,
   `$_COOKIE` from the incoming HTTP request, then includes `worker_handler`
   to dispatch the request through the already-booted application.
3. **Cleanup**: calls `gc_collect_cycles()` at your discretion (in the handler).

This skips extension RINIT/RSHUTDOWN on every request, preserving database
connections, the Composer class map, compiled routes, and service container
bindings across thousands of requests.

## Configuration

```toml
[server]
persistent_workers = true
worker_boot = "turbine-boot.php"       # path to boot script (relative to app root)
worker_handler = "turbine-handler.php"  # path to per-request handler
worker_max_requests = 10000            # recycle workers periodically
```

| Field | Required | Description |
|-------|----------|-------------|
| `persistent_workers` | Yes | Must be `true` to enable persistent mode |
| `worker_boot` | No* | PHP script executed **once** per worker at startup |
| `worker_handler` | No* | PHP script included on **every request** |
| `worker_max_requests` | Recommended | Recycle workers after N requests (prevents state accumulation) |

\* Both `worker_boot` and `worker_handler` must be set together.
If only one is set, the lightweight lifecycle is not activated.

### Path Resolution

- **Relative paths** are resolved from the application root (`-r` / `--root` flag).
- **Absolute paths** are used as-is.

```toml
# These are equivalent when app root is /var/www/myapp
worker_boot = "turbine-boot.php"
worker_boot = "/var/www/myapp/turbine-boot.php"

# You can also put scripts in a subdirectory
worker_boot = "config/turbine/boot.php"
worker_handler = "config/turbine/handler.php"
```

## How It Works

```
┌─────────────────────────────────────────────────────────┐
│                      Worker Process                     │
│                                                         │
│  BOOT (once):                                           │
│    turbine_worker_boot()        ← init PHP + extensions │
│    require 'worker_boot'        ← load app into $GLOBALS│
│    turbine_worker_request_shutdown() ← preserve state   │
│                                                         │
│  REQUEST LOOP:                                          │
│    ┌─────────────────────────────────────────────┐      │
│    │ turbine_worker_request_startup()            │      │
│    │   → rearm $_GET, $_POST, $_SERVER, $_COOKIE │      │
│    │ include 'worker_handler'                    │      │
│    │   → dispatch request through booted app     │      │
│    │ turbine_worker_request_shutdown()            │      │
│    │   → clean up request state, keep app alive  │      │
│    └─────────────────────────────────────────────┘      │
│    ... repeat for worker_max_requests ...               │
│                                                         │
│  SHUTDOWN:                                              │
│    turbine_worker_shutdown()    ← full cleanup          │
└─────────────────────────────────────────────────────────┘
```

### Comparison with Full Lifecycle

| Aspect | Full Lifecycle | Lightweight Lifecycle |
|--------|---------------|----------------------|
| Boot per request | `php_request_startup()` (all extensions) | `turbine_worker_request_startup()` (superglobals only) |
| Framework init | Every request | Once per worker |
| Extension RINIT/RSHUTDOWN | Every request | Skipped |
| DB connections | Reopened each request | Preserved |
| Class table | Rebuilt each request | Preserved |
| Autoloader | Re-registered each request | Loaded once |

## Writing Boot Scripts

The boot script runs **once** when the worker starts. It should:

1. Load the autoloader
2. Boot the framework / application
3. Store the app instance in `$GLOBALS` for the handler to use

### Laravel

```php
<?php
// turbine-boot.php

define('LARAVEL_START', microtime(true));

require __DIR__.'/vendor/autoload.php';

$GLOBALS['__turbine_app'] = require_once __DIR__.'/bootstrap/app.php';

$GLOBALS['__turbine_kernel'] = $GLOBALS['__turbine_app']
    ->make(\Illuminate\Contracts\Http\Kernel::class);
```

### Phalcon

```php
<?php
// turbine-boot.php

use Phalcon\Di\FactoryDefault;
use Phalcon\Mvc\Micro;

$di  = new FactoryDefault();
$app = new Micro($di);

// Register all routes...
$app->get('/hello/{name}', function (string $name) {
    return new \Phalcon\Http\Response(json_encode([
        'message' => "Hello, {$name}!",
    ]), 200, 'OK');
});

$GLOBALS['__turbine_app'] = $app;
```

### Symfony

```php
<?php
// turbine-boot.php

require __DIR__.'/vendor/autoload.php';

$GLOBALS['__turbine_kernel'] = new App\Kernel(
    $_SERVER['APP_ENV'] ?? 'prod',
    (bool) ($_SERVER['APP_DEBUG'] ?? false)
);
$GLOBALS['__turbine_kernel']->boot();
```

### Generic PHP

```php
<?php
// turbine-boot.php

require __DIR__.'/vendor/autoload.php';

// Set up your application, router, DI container, etc.
$router = new MyRouter();
$router->loadRoutes(__DIR__.'/routes');

$GLOBALS['__turbine_router'] = $router;
$GLOBALS['__turbine_db'] = new PDO('mysql:host=localhost;dbname=myapp', 'user', 'pass', [
    PDO::ATTR_PERSISTENT => true,
]);
```

## Writing Handler Scripts

The handler script runs on **every request**. It should:

1. Retrieve the app instance from `$GLOBALS`
2. Dispatch the request
3. Send the response
4. Optionally call `gc_collect_cycles()`

### Laravel

```php
<?php
// turbine-handler.php

$request  = \Illuminate\Http\Request::capture();
$response = $GLOBALS['__turbine_kernel']->handle($request);
$response->send();
$GLOBALS['__turbine_kernel']->terminate($request, $response);

gc_collect_cycles();
```

### Phalcon

```php
<?php
// turbine-handler.php

$app = $GLOBALS['__turbine_app'];

// Reset response state
$app->response->resetHeaders();
$app->response->setContent('');
$app->response->setStatusCode(200);

$result = $app->handle($_SERVER['REQUEST_URI'] ?? '/');

if ($result instanceof \Phalcon\Http\Response) {
    $result->send();
} elseif (is_string($result)) {
    echo $result;
}

gc_collect_cycles();
```

### Symfony

```php
<?php
// turbine-handler.php

$request  = \Symfony\Component\HttpFoundation\Request::createFromGlobals();
$response = $GLOBALS['__turbine_kernel']->handle($request);
$response->send();
$GLOBALS['__turbine_kernel']->terminate($request, $response);

gc_collect_cycles();
```

### Generic PHP

```php
<?php
// turbine-handler.php

$router = $GLOBALS['__turbine_router'];
$db     = $GLOBALS['__turbine_db'];

$method = $_SERVER['REQUEST_METHOD'];
$uri    = $_SERVER['REQUEST_URI'];

$handler = $router->match($method, $uri);
$handler($db);

gc_collect_cycles();
```

## Important Considerations

### State Accumulation

Because the PHP process doesn't fully restart between requests, state can
leak across requests:

- **Static variables** persist across requests (intentional — this is what
  makes it fast, but be aware)
- **Global variables** persist (use `$GLOBALS['__turbine_*']` for intentional
  state, clean up everything else)
- **Memory**: long-lived objects accumulate; `gc_collect_cycles()` in your handler
  helps, and `worker_max_requests` provides the safety net

### worker_max_requests

Always set `worker_max_requests` to a reasonable value (10,000–50,000).
This ensures workers are periodically recycled, reclaiming any leaked memory
and resetting accumulated state.

```toml
worker_max_requests = 10000
```

### Extensions and Thread Safety

Some PHP extensions store per-request state that isn't reset by the lightweight
lifecycle. If you encounter unexplained behavior, test with the full lifecycle
first (`persistent_workers = true` without `worker_boot`/`worker_handler`).

### Without worker_boot / worker_handler

When `persistent_workers = true` but no boot/handler scripts are configured,
workers still bootstrap once (loading OPcache) but use the full
`php_request_startup` / `php_request_shutdown` cycle per request. This is
faster than per-request workers (OPcache stays warm) but doesn't get the
framework-boot-once benefit.

## Performance

Benchmarks on macOS M3 (Phalcon Micro, `wrk -t4 -c50 -d15s`):

| Configuration | GET /hello (req/s) | vs PHP-FPM |
|---|---:|---:|
| **Turbine Lightweight 8w** | **35,389** | **8.1×** |
| Turbine Full-lifecycle 8w | 27,333 | 6.2× |
| PHP-FPM+nginx 8w | 4,375 | 1.0× |

The lightweight lifecycle adds **~28–30%** throughput on top of the already-fast
full persistent lifecycle.

## Migration from Auto-Detection

If you were using the previous auto-detection behavior (Turbine automatically
detected Laravel by checking for `artisan` + `bootstrap/app.php`), follow these
steps:

1. Create `turbine-boot.php` (see [Laravel example](#laravel) above)
2. Create `turbine-handler.php` (see [Laravel example](#laravel-1) above)
3. Add to your `turbine.toml`:

```toml
[server]
persistent_workers = true
worker_boot = "turbine-boot.php"
worker_handler = "turbine-handler.php"
```

This gives you **the same performance** with explicit, configurable control
over the boot and handler logic — no magic filesystem detection.
