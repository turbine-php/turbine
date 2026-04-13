<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Cleanup Script (Laravel)
 *
 * Executed AFTER every request to reset application state between requests.
 * Prevents auth, session, and scoped service leaks across workers.
 *
 * See: docs/worker-lifecycle.md
 */

$app = $GLOBALS['__turbine_app'];

if (method_exists($app, 'resetScope')) { $app->resetScope(); }
if (method_exists($app, 'forgetScopedInstances')) { $app->forgetScopedInstances(); }

if ($app->resolved('session')) {
    try {
        $session = $app->make('session')->driver();
        $session->flush();
        $session->regenerate();
    } catch (\Throwable $e) {}
}

$app->forgetInstance('session.store');

if ($app->resolved('auth.driver')) { $app->forgetInstance('auth.driver'); }
if ($app->resolved('auth')) { $app->make('auth')->forgetGuards(); }

\Illuminate\Support\Facades\Facade::clearResolvedInstances();
