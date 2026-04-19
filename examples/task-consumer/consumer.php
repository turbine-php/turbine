<?php
/**
 * Turbine task consumer — run one or more copies next to `turbine serve`.
 *
 * This is a long-running CLI that pops jobs off the named queue and
 * processes them.  It uses the same `turbine_task_*()` helpers that are
 * auto-injected into the embed SAPI worker, because we include them
 * manually here for CLI use.
 *
 * Usage:
 *   TURBINE_TABLE_URL=http://127.0.0.1:8080 \
 *   TURBINE_TOKEN=your-dashboard-token    \
 *     php consumer.php emails
 */

$channel = $argv[1] ?? 'default';
$wait_ms = 10_000;

fwrite(STDERR, "[consumer] channel={$channel} wait_ms={$wait_ms}\n");

// Copy of the helpers the embed SAPI injects automatically.  Keeping a
// CLI-local copy lets consumers run under plain `php` without depending
// on the server having injected them.
function turbine_task_request(string $method, string $path, ?string $body = null, int $timeout_ms = 2000): array {
    static $base = null, $token = null;
    if ($base === null) {
        $base  = getenv('TURBINE_TABLE_URL') ?: 'http://127.0.0.1:8080';
        $token = getenv('TURBINE_TOKEN') ?: '';
    }
    $url = rtrim($base, '/') . $path;
    $headers = ['Expect:', 'Content-Type: application/octet-stream'];
    if ($token !== '') $headers[] = 'Authorization: Bearer ' . $token;
    $ch = curl_init($url);
    curl_setopt_array($ch, [
        CURLOPT_CUSTOMREQUEST     => $method,
        CURLOPT_RETURNTRANSFER    => true,
        CURLOPT_HEADER            => true,
        CURLOPT_TIMEOUT_MS        => $timeout_ms,
        CURLOPT_CONNECTTIMEOUT_MS => 1000,
        CURLOPT_HTTPHEADER        => $headers,
        CURLOPT_TCP_KEEPALIVE     => 1,
    ]);
    if ($body !== null) curl_setopt($ch, CURLOPT_POSTFIELDS, $body);
    $resp = curl_exec($ch);
    $code = curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
    $hlen = curl_getinfo($ch, CURLINFO_HEADER_SIZE);
    curl_close($ch);
    if ($resp === false) return [0, '', ''];
    return [(int)$code, substr((string)$resp, (int)$hlen), substr((string)$resp, 0, (int)$hlen)];
}

function turbine_task_pop(string $channel, int $wait_ms = 0): ?array {
    $q = '/_/task/pop?channel=' . rawurlencode($channel) . '&wait_ms=' . $wait_ms;
    [$code, $body, $headers] = turbine_task_request('POST', $q, null, $wait_ms + 2000);
    if ($code !== 200) return null;
    $id = 0;
    foreach (explode("\r\n", $headers) as $h) {
        if (stripos($h, 'X-Task-Id:') === 0) {
            $id = (int)trim(substr($h, 10));
            break;
        }
    }
    return ['id' => $id, 'payload' => $body];
}

// Graceful shutdown on Ctrl-C / SIGTERM.
$running = true;
if (function_exists('pcntl_signal')) {
    pcntl_async_signals(true);
    pcntl_signal(SIGINT,  function () use (&$running) { $running = false; });
    pcntl_signal(SIGTERM, function () use (&$running) { $running = false; });
}

while ($running) {
    $job = turbine_task_pop($channel, $wait_ms);
    if ($job === null) continue;

    // Replace this with your job-handling logic.  Payload is a raw string.
    $data = json_decode($job['payload'], true) ?: $job['payload'];
    try {
        fprintf(STDOUT, "[consumer] id=%d data=%s\n", $job['id'], is_string($data) ? $data : json_encode($data));
        // ... do work ...
    } catch (\Throwable $e) {
        fprintf(STDERR, "[consumer] id=%d failed: %s\n", $job['id'], $e->getMessage());
    }
}
fwrite(STDERR, "[consumer] shutting down\n");
