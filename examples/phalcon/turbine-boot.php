<?php

declare(strict_types=1);

/**
 * Turbine Persistent Worker — Boot Script (Phalcon Micro)
 *
 * Executed ONCE per worker process. Sets up the Phalcon application,
 * registers routes, and stores the app in $GLOBALS for reuse.
 */

use Phalcon\Di\FactoryDefault;
use Phalcon\Mvc\Micro;
use Phalcon\Http\Response;

// ─── Bootstrap ──────────────────────────────────────────────────
$di  = new FactoryDefault();
$app = new Micro($di);

// ─── Middleware ──────────────────────────────────────────────────
$app->before(function () use ($app) {
    $app->response->setContentType('application/json', 'UTF-8');
});

// ─── Routes ─────────────────────────────────────────────────────

$app->get('/', function () {
    $version = 'unknown';
    if (class_exists('\\Phalcon\\Version')) {
        try { $version = \Phalcon\Version::get(); } catch (\Throwable $e) { $version = 'error: '.$e->getMessage(); }
    }
    return jsonResponse([
        'name'            => 'Turbine + Phalcon Micro (persistent)',
        'phalcon_version' => $version,
        'php_version'     => PHP_VERSION,
        'worker_pid'      => getmypid(),
        'endpoints'       => [
            'GET /'              => 'This page',
            'GET /hello/{name}'  => 'Hello greeting',
            'GET /todos'         => 'List todos',
            'GET /todos/{id}'    => 'Get a todo',
            'POST /todos'        => 'Create a todo',
            'GET /health'        => 'Health check',
            'GET /counter'       => 'Request counter (persistent state test)',
        ],
    ]);
});

$app->get('/hello/{name}', function (string $name) {
    return jsonResponse([
        'message' => "Hello, {$name}!",
        'time'    => date('Y-m-d H:i:s'),
        'pid'     => getmypid(),
    ]);
});

$todos = [
    1 => ['id' => 1, 'title' => 'Install Phalcon',           'done' => true],
    2 => ['id' => 2, 'title' => 'Configure Turbine',         'done' => true],
    3 => ['id' => 3, 'title' => 'Build something awesome',   'done' => false],
];

$app->get('/todos', function () use ($todos) {
    return jsonResponse(array_values($todos));
});

$app->get('/todos/{id:[0-9]+}', function (int $id) use ($todos) {
    if (!isset($todos[$id])) {
        return jsonResponse(['error' => 'Todo not found'], 404);
    }
    return jsonResponse($todos[$id]);
});

$app->post('/todos', function () use ($app, $todos) {
    $body = $app->request->getJsonRawBody(true);
    if (empty($body['title'])) {
        return jsonResponse(['error' => 'Title is required'], 422);
    }
    $newId = empty($todos) ? 1 : max(array_keys($todos)) + 1;
    return jsonResponse([
        'id'    => $newId,
        'title' => $body['title'],
        'done'  => $body['done'] ?? false,
    ], 201);
});

$app->get('/health', function () {
    $version = 'unknown';
    if (class_exists('\\Phalcon\\Version')) {
        try { $version = \Phalcon\Version::get(); } catch (\Throwable $e) { $version = 'error: '.$e->getMessage(); }
    }
    return jsonResponse([
        'status'    => 'ok',
        'phalcon'   => $version,
        'timestamp' => time(),
        'memory'    => round(memory_get_usage(true) / 1024 / 1024, 2) . ' MB',
        'pid'       => getmypid(),
    ]);
});

// Counter — proves state persists across requests in the same worker
$app->get('/counter', function () {
    if (!isset($GLOBALS['__turbine_counter'])) {
        $GLOBALS['__turbine_counter'] = 0;
    }
    $GLOBALS['__turbine_counter']++;
    return jsonResponse([
        'count' => $GLOBALS['__turbine_counter'],
        'pid'   => getmypid(),
    ]);
});

$app->notFound(function () {
    return jsonResponse(['error' => 'Not found'], 404);
});

function jsonResponse(mixed $data, int $code = 200): Response
{
    $response = new Response();
    $response->setStatusCode($code);
    $response->setContentType('application/json', 'UTF-8');
    $response->setJsonContent($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE);
    return $response;
}

// ─── Store for persistent reuse ─────────────────────────────────
$GLOBALS['__turbine_app'] = $app;
