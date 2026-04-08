<?php

declare(strict_types=1);

/**
 * Turbine — Security Demo
 *
 * A clean API endpoint used to probe Turbine's security layer.
 * If this file is reached by Turbine, the input was safe.
 * Malicious payloads are blocked BEFORE PHP runs and return HTTP 403.
 */

header('Content-Type: application/json; charset=utf-8');
header('X-Served-By: Turbine-PHP');

$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';

// Read the probe value from GET or POST body
$query = match ($method) {
    'POST'  => json_decode(file_get_contents('php://input') ?: '{}', true)['q'] ?? '',
    default => $_GET['q'] ?? '',
};

http_response_code(200);
echo json_encode([
    'status'  => 'allowed',
    'method'  => $method,
    'input'   => $query,
    'note'    => 'This response means Turbine found no attack pattern in your input.',
    'php'     => PHP_VERSION,
], JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
