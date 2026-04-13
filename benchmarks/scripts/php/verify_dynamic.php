<?php
// Verification test: every response MUST be unique.
// Returns JSON with: unique ID, hrtime, pid, random nonce.
// If any cache is active, consecutive responses would be identical.

header('Content-Type: application/json');
header('Cache-Control: no-store');

echo json_encode([
    'id'   => bin2hex(random_bytes(16)),         // 32-char unique ID
    'ts'   => hrtime(true),                      // nanosecond timestamp
    'pid'  => getmypid(),                        // worker PID
    'rand' => bin2hex(random_bytes(16)),          // 32-char random nonce
], JSON_UNESCAPED_SLASHES);
