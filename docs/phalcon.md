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

If your Phalcon app uses `public/index.php` as a front controller, Turbine detects that pattern automatically:

```
$ turbine serve --root /path/to/phalcon-app
[INFO] Detected front-controller application (public/index.php)
```

## Persistent Workers

Phalcon apps run in **persistent worker mode** when `persistent_workers = true`. You provide a `turbine-worker.php` bootstrap file in your project root; Turbine executes it once per worker at startup and then calls the returned application handler for each request.

### How It Works

1. **Bootstrap (once per worker)**:
   - Turbine loads `vendor/autoload.php` (Composer autoloader, if present)
   - Turbine executes `turbine-worker.php` and keeps the returned application in memory
2. **Per request**:
   - Superglobals (`$_SERVER`, `$_GET`, `$_POST`, etc.) are rebuilt from the HTTP request
   - The application processes the request and returns the response

There is **no automatic Phalcon-specific bootstrap**. Turbine does not scan for `config/services.php`, `config/routes.php`, or any other Phalcon config paths. All bootstrapping is done inside your `turbine-worker.php`.

## Bootstrap File

Create a `turbine-worker.php` file in your project root. It must set up the Phalcon DI and application and return the application instance:

```php
<?php
// turbine-worker.php

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

require __DIR__ . '/config/routes.php';
require __DIR__ . '/config/services.php';

$application = new Application($di);

return $application;
```

### Phalcon Micro Apps

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
├── composer.json
├── turbine-worker.php     ← Turbine bootstrap (required for persistent workers)
└── turbine.toml
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
- PHP 8.1+

## Comparison with Traditional Setup

| Feature | Nginx + PHP-FPM | Turbine |
|---------|-----------------|---------|
| DI bootstrap | Every request | Once per worker |
| DB connection | Per request (pool optional) | Persistent per worker |
| Service loading | Every request | Once per worker |
| Memory per request | Full PHP process | Shared worker state |
| Config parsing | Every request | Once per worker |
