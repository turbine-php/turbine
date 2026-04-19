<?php
/**
 * Turbine Swoole-style demo — combines SharedTable, TaskQueue,
 * WebSocket and AsyncIO in a single request handler.
 *
 * All `turbine_*` helpers are auto-injected by the embed SAPI, so
 * this file runs with zero setup beyond `turbine serve`.
 */

header('Content-Type: application/json');

$ip   = $_SERVER['REMOTE_ADDR'] ?? '0.0.0.0';
$now  = microtime(true);
$out  = ['ip' => $ip, 'ts' => $now];

// ── 1. Rate limit via SharedTable ───────────────────────────────────
// Counter expires 60 s after first hit from this IP.
$rate_key = "rate:$ip";
$count    = turbine_table_incr($rate_key, 1);
if ($count === 1) {
    // First hit in the window — set TTL.
    turbine_table_set($rate_key, (string) $count, 60_000);
}
$out['rate_count'] = $count;

if ($count > 100) {
    http_response_code(429);
    echo json_encode(['error' => 'rate limit exceeded', 'retry_after_s' => 60]);
    return;
}

// ── 2. Feature flag warm cache ──────────────────────────────────────
$flag = turbine_table_get('feature:new_ui');
if ($flag === null) {
    // Simulate a "slow" lookup; cached for 10 minutes.
    $flag = random_int(0, 1) ? 'on' : 'off';
    turbine_table_set('feature:new_ui', $flag, 600_000);
}
$out['new_ui'] = $flag;

// ── 3. Fire-and-forget email job via TaskQueue ──────────────────────
$job = json_encode(['to' => 'user@example.com', 'template' => 'welcome', 'ip' => $ip]);
$ok  = turbine_task_push('emails', $job);
$out['email_queued'] = $ok;

// ── 4. Broadcast activity on WebSocket channel ──────────────────────
$event = json_encode(['type' => 'hit', 'ip' => $ip, 'rate' => $count, 'ts' => $now]);
$subs  = turbine_ws_publish('events', $event);
$out['ws_subscribers'] = $subs;

// ── 5. Parallel non-blocking file reads via AsyncIO ─────────────────
// Works even if data/ doesn't exist — returns null for missing files.
$configs = turbine_async_parallel([
    ['read', './examples/swoole-demo/data/site.json'],
    ['read', './examples/swoole-demo/data/features.json'],
]);
$out['config_files'] = array_map(
    static fn($body) => ['ok' => $body !== null, 'bytes' => $body !== null ? strlen($body) : 0],
    $configs
);

// ── 6. Schedule a retry timer that fires a follow-up job ────────────
// 5 s from now, a task lands on the `emails` channel.
$timer_ok = turbine_async_timer(
    'emails',
    json_encode(['to' => 'user@example.com', 'template' => 'followup', 'ip' => $ip]),
    5_000
);
$out['retry_timer_ok'] = $timer_ok;

echo json_encode($out, JSON_PRETTY_PRINT);
