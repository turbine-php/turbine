<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Cleanup Script (Phalcon Micro)
 *
 * Executed AFTER every request to reset application state between requests.
 *
 * See: docs/worker-lifecycle.md
 */

$app = $GLOBALS['__turbine_app'];

// Reset response state
$app->response->resetHeaders();
$app->response->setContent('');
$app->response->setStatusCode(200);

// Clear session if active
if ($app->getDI()->has('session') && $app->getDI()->get('session')->isStarted()) {
    $app->getDI()->get('session')->destroy();
}
