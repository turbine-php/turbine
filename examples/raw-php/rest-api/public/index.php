<?php

declare(strict_types=1);

/**
 * Turbine — REST API Example
 *
 * A simple JSON REST API with routing, CRUD operations, and proper
 * HTTP status codes. Uses an in-memory array as a data store.
 */

header('Content-Type: application/json; charset=utf-8');

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$uri = trim(parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH), '/');
$segments = $uri !== '' ? explode('/', $uri) : [];

// Simple in-memory data (in production, use a database)
$todos = [
    1 => ['id' => 1, 'title' => 'Learn Turbine', 'done' => false],
    2 => ['id' => 2, 'title' => 'Build an API', 'done' => true],
    3 => ['id' => 3, 'title' => 'Deploy to production', 'done' => false],
];

/**
 * Send a JSON response with the given status code.
 */
function jsonResponse(mixed $data, int $status = 200): never
{
    http_response_code($status);
    echo json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE);
    exit;
}

/**
 * Read the raw JSON request body.
 */
function getJsonBody(): array
{
    $body = file_get_contents('php://input');
    if ($body === false || $body === '') {
        jsonResponse(['error' => 'Request body is empty'], 400);
    }
    $data = json_decode($body, true);
    if (!is_array($data)) {
        jsonResponse(['error' => 'Invalid JSON'], 400);
    }
    return $data;
}

// --- Routing ---

// GET /
if ($method === 'GET' && count($segments) === 0) {
    jsonResponse([
        'name' => 'Turbine REST API Example',
        'version' => '1.0.0',
        'endpoints' => [
            'GET /todos' => 'List all todos',
            'GET /todos/{id}' => 'Get a todo by ID',
            'POST /todos' => 'Create a new todo',
            'PUT /todos/{id}' => 'Update a todo',
            'DELETE /todos/{id}' => 'Delete a todo',
        ],
    ]);
}

// GET /todos
if ($method === 'GET' && $segments[0] === 'todos' && !isset($segments[1])) {
    jsonResponse(array_values($todos));
}

// GET /todos/{id}
if ($method === 'GET' && ($segments[0] ?? '') === 'todos' && isset($segments[1])) {
    $id = (int) $segments[1];
    if (!isset($todos[$id])) {
        jsonResponse(['error' => 'Todo not found'], 404);
    }
    jsonResponse($todos[$id]);
}

// POST /todos
if ($method === 'POST' && ($segments[0] ?? '') === 'todos' && !isset($segments[1])) {
    $body = getJsonBody();
    if (empty($body['title'])) {
        jsonResponse(['error' => 'Title is required'], 422);
    }
    $newId = max(array_keys($todos)) + 1;
    $todo = [
        'id' => $newId,
        'title' => $body['title'],
        'done' => $body['done'] ?? false,
    ];
    jsonResponse($todo, 201);
}

// PUT /todos/{id}
if ($method === 'PUT' && ($segments[0] ?? '') === 'todos' && isset($segments[1])) {
    $id = (int) $segments[1];
    if (!isset($todos[$id])) {
        jsonResponse(['error' => 'Todo not found'], 404);
    }
    $body = getJsonBody();
    $todos[$id] = array_merge($todos[$id], array_intersect_key($body, ['title' => 1, 'done' => 1]));
    jsonResponse($todos[$id]);
}

// DELETE /todos/{id}
if ($method === 'DELETE' && ($segments[0] ?? '') === 'todos' && isset($segments[1])) {
    $id = (int) $segments[1];
    if (!isset($todos[$id])) {
        jsonResponse(['error' => 'Todo not found'], 404);
    }
    jsonResponse(['deleted' => true], 200);
}

// 404 fallback
jsonResponse(['error' => 'Not found', 'method' => $method, 'path' => '/' . $uri], 404);
