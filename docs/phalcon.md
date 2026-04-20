# Phalcon PHP

Turbine supports **Phalcon applications** with persistent workers for maximum performance.

> **PHP build:** Phalcon is not thread-safe. Run Turbine in `worker_mode = "process"` (NTS) for Phalcon apps. Thread mode (ZTS) is not guaranteed to be safe even if the benchmark passes.

## Setup

Configure `turbine.toml` with persistent workers and explicit boot/handler scripts:

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
worker_mode = "process"        # Phalcon requires NTS
persistent_workers = true
worker_boot = "turbine-boot.php"
worker_handler = "turbine-handler.php"
worker_cleanup = "turbine-cleanup.php"
worker_max_requests = 10000

[php]
memory_limit = "256M"
extensions = ["phalcon"]      # if using dynamic Phalcon extension
```

See [Worker Lifecycle](worker-lifecycle.md) for the full boot/handler/cleanup model.

## Persistent Workers

In persistent mode Phalcon boots **once per worker** and serves thousands of requests against the already-booted app. Turbine does not auto-detect Phalcon; you wire the three scripts explicitly.

### How It Works

1. **Boot (once per worker)** — `turbine-boot.php` loads the autoloader, creates the Phalcon DI and app, and stores the app in `$GLOBALS`.
2. **Request (every request)** — `turbine-handler.php` retrieves the app from `$GLOBALS`, dispatches the URI, and sends the response.
3. **Cleanup (every request)** — `turbine-cleanup.php` resets response headers, session state, and any scoped services so state doesn't leak between requests.

There is **no automatic Phalcon-specific bootstrap**. Turbine does not scan for `config/services.php`, `config/routes.php`, or any other Phalcon config paths. All bootstrapping is done inside `turbine-boot.php`.

## Boot Script

Create `turbine-boot.php` in your project root:

```php
<?php
// turbine-boot.php

require __DIR__.'/vendor/autoload.php';

use Phalcon\Di\FactoryDefault;
use Phalcon\Mvc\Application;

$di = new FactoryDefault();

$di->setShared('config', function () {
    return new \Phalcon\Config\Adapter\Ini(__DIR__ . '/config/app.ini');
});

$di->setShared('db', function () use ($di) {
    $config = $di->getShared('config');
    return new \Phalcon\Db\Adapter\Pdo\Mysql([
        'host'     => $config->database->host,
        'dbname'   => $config->database->dbname,
        'username' => $config->database->username,
        'password' => $config->database->password,
    ]);
});

require __DIR__ . '/config/routes.php';
require __DIR__ . '/config/services.php';

$GLOBALS['__turbine_app'] = new Application($di);
```

### Phalcon Micro Apps

```php
<?php
// turbine-boot.php for Micro app
use Phalcon\Mvc\Micro;

$app = new Micro();

$app->get('/', function () {
    return 'Hello from Turbine + Phalcon!';
});

$app->get('/api/users', function () {
    $this->response->setJsonContent(['users' => []]);
    return $this->response;
});

$GLOBALS['__turbine_app'] = $app;
```

## Handler Script

```php
<?php
// turbine-handler.php

$app = $GLOBALS['__turbine_app'];

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

## Cleanup Script

```php
<?php
// turbine-cleanup.php

$app = $GLOBALS['__turbine_app'];

$app->response->resetHeaders();
$app->response->setContent('');
$app->response->setStatusCode(200);

if ($app->getDI()->has('session') && $app->getDI()->get('session')->isStarted()) {
    $app->getDI()->get('session')->destroy();
}
```

## Directory Structure

Standard Phalcon MVC app layout:

```
my-phalcon-app/
├── app/
│   ├── config/
│   │   ├── config.php
│   │   ├── loader.php
│   │   ├── routes.php
│   │   └── services.php
│   ├── controllers/
│   ├── models/
│   └── views/
├── public/
│   └── index.php          ← front controller
├── vendor/
│   └── autoload.php
├── composer.json
├── turbine-boot.php       ← boot once per worker
├── turbine-handler.php    ← per-request dispatch
├── turbine-cleanup.php    ← per-request cleanup
└── turbine.toml
```

## TOML Configuration

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
worker_mode = "process"
persistent_workers = true
worker_boot = "turbine-boot.php"
worker_handler = "turbine-handler.php"
worker_cleanup = "turbine-cleanup.php"
worker_max_requests = 10000

[php]
memory_limit = "256M"
extensions = ["phalcon"]

[php.ini]
phalcon.orm.events = "1"
phalcon.orm.virtual_foreign_keys = "1"
phalcon.orm.column_renaming = "1"
phalcon.orm.not_null_validations = "1"
```

## Requirements

- **Phalcon PHP extension** must be installed (compiled into PHP or loaded dynamically)
- Phalcon v5.x+ recommended
- PHP 8.1+

## Comparison with Traditional Setup

| Feature | Nginx + PHP-FPM | Turbine |
|---------|-----------------|---------|
| DI bootstrap | Every request | Once per worker |
| DB connection | Per request (pool optional) | Persistent per worker |
| Service loading | Every request | Once per worker |
| Memory per request | Full PHP process | Shared worker state |
| Config parsing | Every request | Once per worker |
