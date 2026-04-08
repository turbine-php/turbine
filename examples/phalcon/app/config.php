<?php

declare(strict_types=1);

/**
 * Phalcon application configuration.
 */

return [
    'database' => [
        'adapter' => 'sqlite',
        'dbname' => __DIR__ . '/../data/app.sqlite',
    ],
    'application' => [
        'modelsDir' => __DIR__ . '/models/',
        'viewsDir' => __DIR__ . '/views/',
        'cacheDir' => __DIR__ . '/../cache/',
    ],
];
