# Phalcon PHP

Turbine supports **Phalcon applications** with persistent workers for maximum performance.

## Setup

Configure `turbine.toml` with persistent workers enabled:

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
persistent_workers = true
worker_max_requests = 10000

[php]
memory_limit = "256M"
extensions = ["phalcon"]      # if using dynamic Phalcon extension
```

Turbine detects the `public/index.php` front controller pattern and routes all requests through it:

```
$ turbine serve --root /path/to/phalcon-app
[INFO] Detected front-controller application (public/index.php)
```

## Persistent Workers

Phalcon apps run in **persistent worker mode** when `persistent_workers = true` — the DI container and Application are bootstrapped once per worker and reused across requests. This eliminates the overhead of re-creating the DI, loading services, and connecting to databases on every request.

### How It Works

1. **Bootstrap (once per worker)**:
   - Loads `vendor/autoload.php`
   - Creates `\Phalcon\Di\FactoryDefault`
   - Loads config, services, loader, and routes from standard paths
   - Creates `\Phalcon\Mvc\Application` with registered modules
2. **Per request**:
   - Superglobals (`$_SERVER`, `$_GET`, `$_POST`, etc.) are rebuilt from the HTTP request
   - `$application->handle($uri)` is called
   - Response (status, headers, body) is extracted and sent back

### Auto-Bootstrap Config Paths

Turbine scans standard Phalcon directory layouts:

| File | Purpose |
|------|---------|
| `config/config.php` or `app/config/config.php` | Application config (`\Phalcon\Config\Config`) |
| `config/services.php` or `app/config/services.php` | DI service registration |
| `config/loader.php` or `app/config/loader.php` | Autoloader / namespace registration |
| `config/routes.php` or `app/config/routes.php` | Route definitions |
| `config/modules.php` or `app/config/modules.php` | Module registration (multi-module apps) |

## Custom Bootstrap

If your Phalcon app doesn't follow the standard layout, create a `turbine-worker.php` file in your project root:

```php
<?php
// turbine-worker.php
// Must return a \Phalcon\Mvc\Application or \Phalcon\Mvc\Micro instance

use Phalcon\Di\FactoryDefault;
use Phalcon\Mvc\Application;

$di = new FactoryDefault();

// Register your services
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

// Register routes, views, etc.
require __DIR__ . '/config/routes.php';
require __DIR__ . '/config/services.php';

$application = new Application($di);

return $application;
```

> **Important**: `turbine-worker.php` takes priority over auto-bootstrap when present.

## Phalcon Micro Apps

Phalcon Micro applications are also supported. If your app returns a string instead of a `\Phalcon\Http\ResponseInterface`, Turbine wraps it in a 200 OK response:

```php
<?php
// turbine-worker.php for Micro app
use Phalcon\Mvc\Micro;

$app = new Micro();

$app->get('/', function () {
    return 'Hello from Turbine + Phalcon!';
});

$app->get('/api/users', function () {
    $this->response->setJsonContent(['users' => []]);
    return $this->response;
});

return $app;
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
├── composer.json           ← contains "phalcon/*" dependency
└── turbine.toml            ← optional Turbine config
```

## TOML Configuration

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
persistent_workers = true
worker_max_requests = 10000

[php]
memory_limit = "256M"
extensions = ["phalcon"]      # if using dynamic Phalcon extension

[php.ini]
phalcon.orm.events = "1"
phalcon.orm.virtual_foreign_keys = "1"
phalcon.orm.column_renaming = "1"
phalcon.orm.not_null_validations = "1"
```

## Requirements

- **Phalcon PHP extension** must be installed (compiled into PHP or loaded dynamically)
- Phalcon v5.x+ recommended
- PHP 8.1+ (production-v5) or PHP 8.4+ (production-v6)

## Comparison with Traditional Setup

| Feature | Nginx + PHP-FPM | Turbine |
|---------|-----------------|---------|
| DI bootstrap | Every request | Once per worker |
| DB connection | Per request (pool optional) | Persistent per worker |
| Service loading | Every request | Once per worker |
| Memory per request | Full PHP process | Shared worker state |
| Config parsing | Every request | Once per worker |
