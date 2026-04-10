<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Per-Request Handler (Laravel)
 *
 * Executed for EVERY request using the lightweight lifecycle.
 * The Laravel kernel was already booted in turbine-boot.php.
 *
 * See: docs/worker-lifecycle.md
 */

$request  = \Illuminate\Http\Request::capture();
$response = $GLOBALS['__turbine_kernel']->handle($request);
$response->send();
$GLOBALS['__turbine_kernel']->terminate($request, $response);

gc_collect_cycles();
