<?php
// Verification test: CPU-bound computation with unique input per request.
// Computes SHA-256 of (counter + hrtime + random), returns the hash.
// Proves real PHP execution — no shortcut can produce valid hashes.

header('Content-Type: application/json');
header('Cache-Control: no-store');

$input = hrtime(true) . ':' . bin2hex(random_bytes(8)) . ':' . getmypid();
$hash  = hash('sha256', $input);

echo json_encode([
    'input' => $input,
    'hash'  => $hash,
    'ts'    => hrtime(true),
    'pid'   => getmypid(),
], JSON_UNESCAPED_SLASHES);
