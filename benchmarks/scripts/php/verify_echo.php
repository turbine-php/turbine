<?php
// Verification test: echo back a request-specific token.
// wrk sends X-Request-Token header; PHP must echo it back.
// Proves the server processed THIS specific request.

header('Content-Type: application/json');
header('Cache-Control: no-store');

$token = $_SERVER['HTTP_X_REQUEST_TOKEN'] ?? 'missing';

echo json_encode([
    'echo'  => $token,
    'ts'    => hrtime(true),
    'pid'   => getmypid(),
], JSON_UNESCAPED_SLASHES);
