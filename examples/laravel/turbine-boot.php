<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Boot Script (Laravel)
 *
 * Executed ONCE per worker process. Loads the autoloader, boots the
 * Laravel application, and stores the HTTP kernel in $GLOBALS for reuse.
 *
 * See: docs/worker-lifecycle.md
 */

define('LARAVEL_START', microtime(true));

require __DIR__.'/vendor/autoload.php';

$GLOBALS['__turbine_app'] = require_once __DIR__.'/bootstrap/app.php';

$GLOBALS['__turbine_kernel'] = $GLOBALS['__turbine_app']
    ->make(\Illuminate\Contracts\Http\Kernel::class);
