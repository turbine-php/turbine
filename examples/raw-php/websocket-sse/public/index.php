<?php

declare(strict_types=1);

/**
 * Turbine — Server-Sent Events (SSE) Example
 *
 * Demonstrates real-time streaming via SSE. The /events endpoint
 * pushes a timestamp every second. The HTML page connects via EventSource.
 */

$uri = trim(parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH), '/');

// SSE endpoint
if ($uri === 'events') {
    header('Content-Type: text/event-stream');
    header('Cache-Control: no-cache');
    header('Connection: keep-alive');
    header('X-Accel-Buffering: no');

    $start = time();
    $count = 0;

    while (true) {
        $count++;
        $data = json_encode([
            'time' => date('H:i:s'),
            'message' => "Event #{$count}",
            'uptime' => time() - $start,
            'memory' => round(memory_get_usage(true) / 1024 / 1024, 2) . ' MB',
        ]);

        echo "id: {$count}\n";
        echo "data: {$data}\n\n";

        if (ob_get_level()) {
            ob_flush();
        }
        flush();

        // Stop after 5 minutes to prevent resource leaks
        if ($count >= 300) {
            break;
        }

        sleep(1);
    }
    exit;
}

// HTML page
header('Content-Type: text/html; charset=utf-8');
?>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Turbine — Server-Sent Events</title>
    <style>
        body { font-family: system-ui, sans-serif; max-width: 600px; margin: 60px auto; }
        #status { padding: 6px 12px; border-radius: 4px; display: inline-block; margin-bottom: 16px; }
        .connected { background: #e8f5e9; color: #2e7d32; }
        .disconnected { background: #ffebee; color: #c62828; }
        #events { border: 1px solid #ddd; border-radius: 8px; padding: 16px; max-height: 400px; overflow-y: auto; }
        .event { padding: 8px 0; border-bottom: 1px solid #f0f0f0; font-family: monospace; font-size: 14px; }
        button { background: #e44d26; color: #fff; border: none; padding: 8px 20px; border-radius: 4px; cursor: pointer; margin: 8px 4px 8px 0; }
    </style>
</head>
<body>
    <h1>Server-Sent Events</h1>
    <div id="status" class="disconnected">Disconnected</div>
    <br>
    <button onclick="connect()">Connect</button>
    <button onclick="disconnect()">Disconnect</button>
    <button onclick="clearEvents()">Clear</button>

    <div id="events"></div>

    <script>
        let evtSource = null;

        function connect() {
            if (evtSource) evtSource.close();
            evtSource = new EventSource('/events');
            document.getElementById('status').textContent = 'Connected';
            document.getElementById('status').className = 'connected';

            evtSource.onmessage = function(e) {
                const data = JSON.parse(e.data);
                const div = document.createElement('div');
                div.className = 'event';
                div.textContent = `[${data.time}] ${data.message} — Memory: ${data.memory}`;
                const events = document.getElementById('events');
                events.prepend(div);
                // Keep last 100 events
                while (events.children.length > 100) events.removeChild(events.lastChild);
            };

            evtSource.onerror = function() {
                document.getElementById('status').textContent = 'Disconnected';
                document.getElementById('status').className = 'disconnected';
            };
        }

        function disconnect() {
            if (evtSource) { evtSource.close(); evtSource = null; }
            document.getElementById('status').textContent = 'Disconnected';
            document.getElementById('status').className = 'disconnected';
        }

        function clearEvents() {
            document.getElementById('events').innerHTML = '';
        }

        connect();
    </script>
</body>
</html>
