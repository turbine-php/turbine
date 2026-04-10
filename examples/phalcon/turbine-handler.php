<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Per-Request Handler (Phalcon Micro)
 *
 * Executed for EVERY request using the lightweight lifecycle.
 * The Phalcon Micro $app was already booted in turbine-boot.php.
 */

$app = $GLOBALS['__turbine_app'];

// Reset Phalcon response state for the new request
$app->response->resetHeaders();
$app->response->setContent('');
$app->response->setStatusCode(200);

// Handle the incoming request
$result = $app->handle($_SERVER['REQUEST_URI'] ?? '/');

// Send response
if ($result instanceof \Phalcon\Http\Response) {
    $result->send();
} elseif (is_string($result)) {
    echo $result;
}

gc_collect_cycles();
