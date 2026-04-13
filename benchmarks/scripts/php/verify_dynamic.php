<?php
// Verification test: every response MUST be unique.
// Returns JSON with: monotonic counter, hrtime, pid, random nonce.
// If any cache is active, consecutive responses would be identical.

header('Content-Type: application/json');
header('Cache-Control: no-store');

// File-based atomic counter — proves PHP executes on every request
$counterFile = __DIR__ . '/verify_counter.dat';
$fp = fopen($counterFile, 'c+');
flock($fp, LOCK_EX);
$counter = (int) fread($fp, 64);
$counter++;
fseek($fp, 0);
ftruncate($fp, 0);
fwrite($fp, (string) $counter);
flock($fp, LOCK_UN);
fclose($fp);

echo json_encode([
    'c'    => $counter,                         // monotonic counter
    'ts'   => hrtime(true),                     // nanosecond timestamp
    'pid'  => getmypid(),                       // worker PID
    'rand' => bin2hex(random_bytes(16)),         // 32-char random nonce
], JSON_UNESCAPED_SLASHES);
