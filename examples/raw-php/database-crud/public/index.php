<?php

declare(strict_types=1);

/**
 * Turbine — Database CRUD Example
 *
 * Simple SQLite CRUD application demonstrating PDO usage with Turbine.
 * Uses prepared statements for SQL injection protection (in addition
 * to Turbine's built-in sql_guard).
 */

header('Content-Type: application/json; charset=utf-8');

$dbPath = __DIR__ . '/../data/app.sqlite';
$dataDir = dirname($dbPath);

if (!is_dir($dataDir)) {
    mkdir($dataDir, 0755, true);
}

try {
    $db = new PDO('sqlite:' . $dbPath, null, null, [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
    ]);

    // Create table if not exists
    $db->exec('CREATE TABLE IF NOT EXISTS notes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        title TEXT NOT NULL,
        body TEXT DEFAULT "",
        created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
    )');
} catch (PDOException $e) {
    http_response_code(500);
    echo json_encode(['error' => 'Database error: ' . $e->getMessage()]);
    exit;
}

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$uri = trim(parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH), '/');
$segments = $uri !== '' ? explode('/', $uri) : [];

/**
 * Send JSON response and exit.
 */
function respond(mixed $data, int $status = 200): never
{
    http_response_code($status);
    echo json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE);
    exit;
}

// GET / — API info
if (count($segments) === 0 && $method === 'GET') {
    respond([
        'name' => 'Turbine Database CRUD Example',
        'database' => 'SQLite',
        'endpoints' => [
            'GET /notes' => 'List all notes',
            'GET /notes/{id}' => 'Get a note',
            'POST /notes' => 'Create a note (title, body)',
            'PUT /notes/{id}' => 'Update a note',
            'DELETE /notes/{id}' => 'Delete a note',
        ],
    ]);
}

// GET /notes
if ($method === 'GET' && ($segments[0] ?? '') === 'notes' && !isset($segments[1])) {
    $page = max(1, (int) ($_GET['page'] ?? 1));
    $limit = min(100, max(1, (int) ($_GET['limit'] ?? 20)));
    $offset = ($page - 1) * $limit;

    $total = (int) $db->query('SELECT COUNT(*) FROM notes')->fetchColumn();
    $stmt = $db->prepare('SELECT * FROM notes ORDER BY created_at DESC LIMIT :limit OFFSET :offset');
    $stmt->bindValue(':limit', $limit, PDO::PARAM_INT);
    $stmt->bindValue(':offset', $offset, PDO::PARAM_INT);
    $stmt->execute();

    respond([
        'data' => $stmt->fetchAll(),
        'meta' => ['page' => $page, 'limit' => $limit, 'total' => $total],
    ]);
}

// GET /notes/{id}
if ($method === 'GET' && ($segments[0] ?? '') === 'notes' && isset($segments[1])) {
    $stmt = $db->prepare('SELECT * FROM notes WHERE id = :id');
    $stmt->execute([':id' => (int) $segments[1]]);
    $note = $stmt->fetch();
    if (!$note) {
        respond(['error' => 'Note not found'], 404);
    }
    respond($note);
}

// POST /notes
if ($method === 'POST' && ($segments[0] ?? '') === 'notes') {
    $body = json_decode(file_get_contents('php://input'), true);
    if (empty($body['title'])) {
        respond(['error' => 'Title is required'], 422);
    }
    $stmt = $db->prepare('INSERT INTO notes (title, body) VALUES (:title, :body)');
    $stmt->execute([
        ':title' => $body['title'],
        ':body' => $body['body'] ?? '',
    ]);
    $id = (int) $db->lastInsertId();

    $stmt = $db->prepare('SELECT * FROM notes WHERE id = :id');
    $stmt->execute([':id' => $id]);
    respond($stmt->fetch(), 201);
}

// PUT /notes/{id}
if ($method === 'PUT' && ($segments[0] ?? '') === 'notes' && isset($segments[1])) {
    $id = (int) $segments[1];
    $body = json_decode(file_get_contents('php://input'), true);

    $stmt = $db->prepare('SELECT * FROM notes WHERE id = :id');
    $stmt->execute([':id' => $id]);
    if (!$stmt->fetch()) {
        respond(['error' => 'Note not found'], 404);
    }

    $stmt = $db->prepare('UPDATE notes SET title = :title, body = :body, updated_at = CURRENT_TIMESTAMP WHERE id = :id');
    $stmt->execute([
        ':id' => $id,
        ':title' => $body['title'] ?? '',
        ':body' => $body['body'] ?? '',
    ]);

    $stmt = $db->prepare('SELECT * FROM notes WHERE id = :id');
    $stmt->execute([':id' => $id]);
    respond($stmt->fetch());
}

// DELETE /notes/{id}
if ($method === 'DELETE' && ($segments[0] ?? '') === 'notes' && isset($segments[1])) {
    $id = (int) $segments[1];
    $stmt = $db->prepare('DELETE FROM notes WHERE id = :id');
    $stmt->execute([':id' => $id]);
    if ($stmt->rowCount() === 0) {
        respond(['error' => 'Note not found'], 404);
    }
    respond(['deleted' => true]);
}

respond(['error' => 'Not found'], 404);
