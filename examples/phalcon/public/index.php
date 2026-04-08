<?php

declare(strict_types=1);

/**
 * Turbine — Phalcon Micro Application Example
 *
 * A minimal Phalcon Micro application demonstrating routing,
 * JSON responses, and middleware on Turbine.
 */

use Phalcon\Di\FactoryDefault;
use Phalcon\Mvc\Micro;
use Phalcon\Http\Response;

// Bootstrap
$di = new FactoryDefault();
$app = new Micro($di);

// ----- Middleware: JSON Content-Type -----

$app->before(function () use ($app) {
    // Set default JSON response headers
    $app->response->setContentType('application/json', 'UTF-8');
});

// ----- Routes -----

// GET / — API info
$app->get('/', function () {
    return jsonResponse([
        'name' => 'Turbine + Phalcon Micro',
        'phalcon_version' => \Phalcon\Version::get(),
        'php_version' => PHP_VERSION,
        'endpoints' => [
            'GET /' => 'This page',
            'GET /hello/{name}' => 'Hello greeting',
            'GET /todos' => 'List todos',
            'GET /todos/{id}' => 'Get a todo',
            'POST /todos' => 'Create a todo',
            'GET /health' => 'Health check',
        ],
    ]);
});

// GET /hello/{name}
$app->get('/hello/{name}', function (string $name) {
    return jsonResponse([
        'message' => "Hello, {$name}!",
        'time' => date('Y-m-d H:i:s'),
    ]);
});

// In-memory data store
$todos = [
    1 => ['id' => 1, 'title' => 'Install Phalcon', 'done' => true],
    2 => ['id' => 2, 'title' => 'Configure Turbine', 'done' => true],
    3 => ['id' => 3, 'title' => 'Build something awesome', 'done' => false],
];

// GET /todos
$app->get('/todos', function () use ($todos) {
    return jsonResponse(array_values($todos));
});

// GET /todos/{id}
$app->get('/todos/{id:[0-9]+}', function (int $id) use ($todos) {
    if (!isset($todos[$id])) {
        return jsonResponse(['error' => 'Todo not found'], 404);
    }
    return jsonResponse($todos[$id]);
});

// POST /todos
$app->post('/todos', function () use ($app, $todos) {
    $body = $app->request->getJsonRawBody(true);
    if (empty($body['title'])) {
        return jsonResponse(['error' => 'Title is required'], 422);
    }
    $newId = empty($todos) ? 1 : max(array_keys($todos)) + 1;
    $todo = [
        'id' => $newId,
        'title' => $body['title'],
        'done' => $body['done'] ?? false,
    ];
    return jsonResponse($todo, 201);
});

// GET /health
$app->get('/health', function () {
    return jsonResponse([
        'status' => 'ok',
        'phalcon' => \Phalcon\Version::get(),
        'timestamp' => time(),
        'memory' => round(memory_get_usage(true) / 1024 / 1024, 2) . ' MB',
    ]);
});

// 404 handler
$app->notFound(function () {
    return jsonResponse(['error' => 'Not found'], 404);
});

/**
 * Create a JSON response.
 */
function jsonResponse(mixed $data, int $code = 200): Response
{
    $response = new Response();
    $response->setStatusCode($code);
    $response->setContentType('application/json', 'UTF-8');
    $response->setJsonContent($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE);
    return $response;
}

$app->handle($_SERVER['REQUEST_URI'] ?? '/');
