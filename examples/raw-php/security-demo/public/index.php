<?php

declare(strict_types=1);

/**
 * Turbine — Security Demo (form edition)
 */

header('Content-Type: text/html; charset=utf-8');
header('X-Served-By: Turbine-PHP');

$presets = [
    'Safe inputs' => [
        ['label' => 'Normal text',                   'method' => 'GET',  'param' => 'q', 'value' => 'Hello Turbine!'],
        ['label' => 'Email address',                  'method' => 'GET',  'param' => 'q', 'value' => 'user@example.com'],
        ['label' => 'Integer',                        'method' => 'GET',  'param' => 'q', 'value' => '42'],
        ['label' => 'SELECT without injection',       'method' => 'GET',  'param' => 'q', 'value' => 'SELECT products'],
        ['label' => 'Safe POST JSON body',            'method' => 'POST', 'param' => 'q', 'value' => 'perfectly safe payload'],
    ],
    'SQL Injection' => [
        ['label' => 'UNION SELECT',                  'method' => 'GET',  'param' => 'q', 'value' => "1 UNION SELECT * FROM users"],
        ['label' => "'; DROP TABLE",                 'method' => 'GET',  'param' => 'q', 'value' => "'; DROP TABLE users;--"],
        ['label' => 'SLEEP() blind',                 'method' => 'GET',  'param' => 'q', 'value' => "1 AND SLEEP(5)"],
        ['label' => 'WAITFOR DELAY (MSSQL)',         'method' => 'GET',  'param' => 'q', 'value' => "1'; WAITFOR DELAY '0:0:5'--"],
        ['label' => 'information_schema scan',       'method' => 'GET',  'param' => 'q', 'value' => "UNION SELECT table_name FROM information_schema.tables"],
        ['label' => "/**/ comment bypass",           'method' => 'GET',  'param' => 'q', 'value' => "1/**/UNION/**/SELECT/**/1,2,3"],
        ['label' => 'LOAD_FILE exfiltration',        'method' => 'GET',  'param' => 'q', 'value' => "LOAD_FILE('/etc/passwd')"],
        ['label' => 'EXTRACTVALUE error-based',      'method' => 'GET',  'param' => 'q', 'value' => "EXTRACTVALUE(1,CONCAT(0x7e,version()))"],
        ['label' => 'GROUP_CONCAT dump',             'method' => 'GET',  'param' => 'q', 'value' => "GROUP_CONCAT(username,password)"],
        ['label' => 'BENCHMARK CPU-burn',            'method' => 'GET',  'param' => 'q', 'value' => "BENCHMARK(10000000,MD5('x'))"],
        ['label' => 'INTO OUTFILE webshell',         'method' => 'GET',  'param' => 'q', 'value' => "1 INTO OUTFILE '/var/www/shell.php'"],
        ['label' => 'SQL injection via POST JSON',   'method' => 'POST', 'param' => 'q', 'value' => "1 UNION SELECT * FROM users"],
    ],
    'Code Injection' => [
        ['label' => "eval() direct",                            'method' => 'GET',  'param' => 'q', 'value' => 'eval(\'system("id")\')'],
        ['label' => "eval+base64_decode obfuscation",           'method' => 'GET',  'param' => 'q', 'value' => "eval(base64_decode('c3lzdGVtKCdpZCcp'))"],
        ['label' => "eval+gzinflate+base64 (3-layer)",          'method' => 'GET',  'param' => 'q', 'value' => "eval(gzinflate(base64_decode('encoded')))"],
        ['label' => "assert+base64_decode obfuscation",         'method' => 'GET',  'param' => 'q', 'value' => "assert(base64_decode('c3lzdGVtKCdpZCcp'))"],
        ['label' => "preg_replace /e modifier",                 'method' => 'GET',  'param' => 'q', 'value' => 'preg_replace("/.*/e",$input,$x)'],
        ['label' => "system() OS execution",                    'method' => 'GET',  'param' => 'q', 'value' => "system('cat /etc/shadow')"],
        ['label' => "shell_exec()",                             'method' => 'GET',  'param' => 'q', 'value' => "shell_exec('id')"],
        ['label' => "backtick operator",                        'method' => 'GET',  'param' => 'q', 'value' => '`whoami`'],
        ['label' => "call_user_func()",                         'method' => 'GET',  'param' => 'q', 'value' => "call_user_func('system','id')"],
        ['label' => "create_function()",                        'method' => 'GET',  'param' => 'q', 'value' => 'create_function("","system(\"id\")")'],
        ['label' => "ReflectionFunction dynamic call",          'method' => 'GET',  'param' => 'q', 'value' => "new ReflectionFunction('system')"],
        ['label' => "\$\$var variable variables",               'method' => 'GET',  'param' => 'q', 'value' => '$$func("id")'],
        ['label' => "proc_open()",                              'method' => 'GET',  'param' => 'q', 'value' => "proc_open('cmd',[],'pipes')"],
        ['label' => "popen()",                                  'method' => 'GET',  'param' => 'q', 'value' => "popen('ls -la','r')"],
        ['label' => "Code injection via POST JSON",             'method' => 'POST', 'param' => 'q', 'value' => "eval(gzinflate(base64_decode('encoded')))"],
    ],
    'Behaviour Guard' => [
        ['label' => 'SQLi attempt #1 (starts accumulation)',    'method' => 'GET',  'param' => 'q', 'value' => "1 UNION SELECT 1"],
        ['label' => 'SQLi attempt #2',                          'method' => 'GET',  'param' => 'q', 'value' => "1 UNION SELECT 2"],
        ['label' => 'SQLi attempt #3',                          'method' => 'GET',  'param' => 'q', 'value' => "1 UNION SELECT 3"],
        ['label' => 'Clean request (safe after SQLi attempts)', 'method' => 'GET',  'param' => 'q', 'value' => "completely innocent"],
    ],
];
?>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Turbine — Security Demo</title>
    <style>
        *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: system-ui, -apple-system, sans-serif;
            background: #0f1117;
            color: #e2e8f0;
            min-height: 100vh;
            display: grid;
            grid-template-columns: 360px 1fr;
            grid-template-rows: auto 1fr;
        }
        /* ── Header ── */
        header {
            grid-column: 1 / -1;
            background: #161b27;
            border-bottom: 1px solid #2d3748;
            padding: 16px 28px;
            display: flex;
            align-items: center;
            gap: 20px;
        }
        header h1 { font-size: 1.25rem; color: #e44d26; }
        header p  { font-size: 0.82rem; color: #64748b; }
        .badge-pill {
            display: inline-flex; align-items: center; gap: 6px;
            background: #1e293b; border: 1px solid #334155;
            border-radius: 999px; padding: 3px 10px; font-size: 0.75rem;
            color: #94a3b8;
        }
        .dot { width: 8px; height: 8px; border-radius: 50%; background: #22c55e; }

        /* ── Left panel ── */
        .panel {
            background: #161b27;
            border-right: 1px solid #2d3748;
            padding: 20px;
            overflow-y: auto;
            display: flex;
            flex-direction: column;
            gap: 20px;
        }
        .panel label { display: block; font-size: 0.78rem; color: #64748b; margin-bottom: 4px; font-weight: 600; text-transform: uppercase; letter-spacing: .05em; }
        select, textarea, input[type=text] {
            width: 100%;
            background: #0f1117;
            border: 1px solid #334155;
            border-radius: 6px;
            color: #e2e8f0;
            padding: 8px 10px;
            font-size: 0.85rem;
            font-family: inherit;
            outline: none;
            transition: border-color 0.15s;
        }
        select:focus, textarea:focus, input:focus { border-color: #3b82f6; }
        select option { background: #161b27; }
        optgroup { color: #64748b; }
        textarea { resize: vertical; min-height: 90px; font-family: 'Fira Mono', 'Consolas', monospace; font-size: 0.78rem; }
        .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
        .method-group { display: flex; gap: 0; border: 1px solid #334155; border-radius: 6px; overflow: hidden; }
        .method-group label {
            flex: 1; text-align: center; padding: 8px 0;
            cursor: pointer; font-size: 0.85rem; color: #94a3b8;
            font-weight: 400; text-transform: none; letter-spacing: 0;
        }
        .method-group input[type=radio] { display: none; }
        .method-group input[type=radio]:checked + label {
            background: #3b82f6; color: #fff; font-weight: 600;
        }
        .method-group label:hover { background: #1e293b; }
        button.run-btn {
            width: 100%; padding: 11px;
            background: #3b82f6; color: #fff;
            border: none; border-radius: 6px;
            font-size: 0.9rem; font-weight: 700;
            cursor: pointer; transition: background 0.15s;
        }
        button.run-btn:hover { background: #2563eb; }
        button.run-btn:disabled { background: #334155; cursor: not-allowed; }
        .hint { font-size: 0.75rem; color: #475569; line-height: 1.5; }
        .hint code { background: #1e293b; padding: 1px 4px; border-radius: 3px; color: #93c5fd; }

        /* ── Right panel ── */
        .output {
            padding: 24px 28px;
            overflow-y: auto;
            display: flex;
            flex-direction: column;
            gap: 16px;
        }
        .result-card {
            background: #161b27;
            border: 1px solid #334155;
            border-radius: 10px;
            overflow: hidden;
        }
        .result-header {
            display: flex; align-items: center; gap: 10px;
            padding: 12px 16px;
            border-bottom: 1px solid #1e293b;
        }
        .status-badge {
            font-weight: 700; font-size: 0.78rem; padding: 2px 10px;
            border-radius: 999px; white-space: nowrap;
        }
        .s200 { background: #052e16; color: #4ade80; }
        .s403 { background: #450a0a; color: #f87171; }
        .s4xx { background: #422006; color: #fb923c; }
        .result-meta { font-size: 0.78rem; color: #64748b; }
        .result-label { font-size: 0.85rem; color: #cbd5e1; flex: 1; font-weight: 500; }
        .result-body {
            padding: 14px 16px;
            font-family: 'Fira Mono', 'Consolas', monospace;
            font-size: 0.78rem;
            white-space: pre-wrap;
            word-break: break-all;
            color: #94a3b8;
            line-height: 1.6;
        }
        .result-body.blocked { color: #f87171; }
        .result-body.allowed { color: #4ade80; }
        .empty-state {
            flex: 1; display: flex; flex-direction: column;
            align-items: center; justify-content: center;
            color: #334155; text-align: center; gap: 8px;
        }
        .empty-state svg { opacity: 0.3; }
        .empty-state p { font-size: 0.85rem; }
        .clear-btn {
            align-self: flex-end;
            background: transparent; color: #475569;
            border: 1px solid #334155; border-radius: 6px;
            padding: 5px 12px; font-size: 0.78rem; cursor: pointer;
        }
        .clear-btn:hover { background: #1e293b; color: #94a3b8; }
        /* ── Category colour coding ── */
        .cat-safe     .result-header { border-left: 3px solid #22c55e; }
        .cat-sql      .result-header { border-left: 3px solid #f59e0b; }
        .cat-code     .result-header { border-left: 3px solid #a78bfa; }
        .cat-behaviour .result-header { border-left: 3px solid #38bdf8; }
        .cat-unknown  .result-header { border-left: 3px solid #334155; }
    </style>
</head>
<body>

<header>
    <div>
        <h1>Turbine Security Demo</h1>
        <p>Requests pass through Rust security guards before PHP runs</p>
    </div>
    <div style="margin-left:auto; display:flex; gap:8px; flex-wrap:wrap; align-items:center">
        <span class="badge-pill"><span class="dot"></span>PHP <?= PHP_VERSION ?></span>
        <span class="badge-pill">SQL guard ✓</span>
        <span class="badge-pill">Code guard ✓</span>
        <span class="badge-pill">Behaviour guard ✓</span>
    </div>
</header>

<!-- ── Left panel: form ─────────────────────────────────────────────── -->
<aside class="panel">

    <div>
        <label for="preset">Preset payload</label>
        <select id="preset" onchange="applyPreset()">
            <option value="">— pick a test case —</option>
            <?php foreach ($presets as $group => $items): ?>
                <optgroup label="<?= htmlspecialchars($group) ?>">
                    <?php foreach ($items as $i => $p): ?>
                        <option value="<?= htmlspecialchars(json_encode($p), ENT_QUOTES) ?>">
                            <?= htmlspecialchars($p['label']) ?>
                        </option>
                    <?php endforeach; ?>
                </optgroup>
            <?php endforeach; ?>
        </select>
    </div>

    <div class="row2">
        <div>
            <label>HTTP method</label>
            <div class="method-group">
                <input type="radio" name="method" id="m-get"  value="GET"  checked>
                <label for="m-get">GET</label>
                <input type="radio" name="method" id="m-post" value="POST">
                <label for="m-post">POST</label>
            </div>
        </div>
        <div>
            <label for="param-name">Param name</label>
            <input id="param-name" type="text" value="q" placeholder="q">
        </div>
    </div>

    <div>
        <label for="payload">Payload value</label>
        <textarea id="payload" placeholder="Type or select a preset above…"></textarea>
    </div>

    <button class="run-btn" id="run-btn" onclick="runTest()">▶ Send request</button>

    <div class="hint">
        <strong style="color:#94a3b8">How it works:</strong><br>
        GET sends <code>?{param}={value}</code>.<br>
        POST sends <code>{"param":"value"}</code> as JSON.<br>
        Blocked payloads return <strong>HTTP 403</strong> instantly — PHP never runs.<br>
        Safe inputs reach <code>/api.php</code> and return <strong>HTTP 200</strong>.
    </div>

</aside>

<!-- ── Right panel: results ─────────────────────────────────────────── -->
<main class="output" id="output">
    <div class="empty-state" id="empty-state">
        <svg width="48" height="48" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
        </svg>
        <p>Select a preset or type a payload, then click <strong>Send request</strong>.</p>
    </div>
</main>

<script>
const PRESETS = <?= json_encode($presets, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES) ?>;

function applyPreset() {
    const sel = document.getElementById('preset');
    if (!sel.value) return;
    const p = JSON.parse(sel.value);
    document.getElementById('payload').value    = p.value;
    document.getElementById('param-name').value = p.param;
    document.querySelector(`input[name=method][value=${p.method}]`).checked = true;
}

// Map optgroup label → CSS category class
const CAT_CLASS = {
    'Safe inputs':       'cat-safe',
    'SQL Injection':     'cat-sql',
    'Code Injection':    'cat-code',
    'Behaviour Guard':   'cat-behaviour',
};

function currentCategory() {
    const sel = document.getElementById('preset');
    if (!sel.value) return 'cat-unknown';
    const opt = sel.selectedOptions[0];
    const group = opt.closest('optgroup')?.label ?? '';
    return CAT_CLASS[group] ?? 'cat-unknown';
}

async function runTest() {
    const payload   = document.getElementById('payload').value;
    const param     = document.getElementById('param-name').value || 'q';
    const method    = document.querySelector('input[name=method]:checked').value;
    const label     = document.getElementById('preset').selectedOptions[0]?.text ?? 'Custom';
    const btn       = document.getElementById('run-btn');
    const catClass  = currentCategory();

    btn.disabled    = true;
    btn.textContent = '⏳ Sending…';

    const t0 = performance.now();
    let status, body;
    try {
        let url = '/api.php';
        const fetchOpts = { method };
        if (method === 'GET') {
            url += '?' + new URLSearchParams({ [param]: payload });
        } else {
            fetchOpts.headers = { 'Content-Type': 'application/json' };
            fetchOpts.body    = JSON.stringify({ [param]: payload });
        }
        const resp = await fetch(url, fetchOpts);
        status = resp.status;
        body   = await resp.text();
    } catch (err) {
        status = 0;
        body   = `Network error: ${err.message}`;
    }
    const elapsed = (performance.now() - t0).toFixed(1);

    // Remove empty state
    document.getElementById('empty-state')?.remove();

    // Status badge
    const badgeClass = status === 200 ? 's200' : status === 403 ? 's403' : 's4xx';
    const bodyClass  = status === 200 ? 'allowed' : 'blocked';
    const statusText = status === 200 ? '✓ 200 Allowed' : status === 403 ? '✗ 403 Blocked' : `${status}`;

    // Format body
    let display = body;
    if (status === 200) {
        try {
            display = JSON.stringify(JSON.parse(body), null, 2);
        } catch {}
    }

    const card = document.createElement('div');
    card.className = `result-card ${catClass}`;
    card.innerHTML = `
        <div class="result-header">
            <span class="result-label">${escHtml(label)}</span>
            <span class="status-badge ${badgeClass}">${statusText}</span>
            <span class="result-meta">${method} · ${elapsed} ms</span>
        </div>
        <div class="result-body ${bodyClass}">${escHtml(display.slice(0, 1200))}${display.length > 1200 ? '\n… truncated' : ''}</div>
    `;

    const output = document.getElementById('output');
    // Insert newest at top
    output.prepend(card);

    // Show clear button if >1 result
    if (!document.getElementById('clear-btn')) {
        const clearBtn = document.createElement('button');
        clearBtn.className = 'clear-btn';
        clearBtn.id = 'clear-btn';
        clearBtn.textContent = 'Clear results';
        clearBtn.onclick = () => {
            document.querySelectorAll('.result-card').forEach(c => c.remove());
            clearBtn.remove();
            const empty = document.createElement('div');
            empty.className = 'empty-state';
            empty.id = 'empty-state';
            empty.innerHTML = `<p>Select a preset or type a payload, then click <strong>Send request</strong>.</p>`;
            document.getElementById('output').appendChild(empty);
        };
        output.prepend(clearBtn);
    }

    btn.textContent = status === 200 ? '▶ Send request' : '▶ Send request';
    btn.disabled = false;
}

function escHtml(s) {
    return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// Keyboard shortcut: Ctrl+Enter / Cmd+Enter in textarea
document.getElementById('payload').addEventListener('keydown', e => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) runTest();
});
</script>
</body>
</html>

 *
 * Interactive page that explains each security guard and lets the
 * browser exercise the /api.php endpoint with pre-built attack payloads.
 * Blocked requests never reach PHP; the browser sees the raw 403 from Turbine.
 */

header('Content-Type: text/html; charset=utf-8');
header('X-Served-By: Turbine-PHP');

?>
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Turbine — Security Demo</title>
    <style>
        *, *::before, *::after { box-sizing: border-box; }
        body {
            font-family: system-ui, -apple-system, sans-serif;
            background: #0f1117;
            color: #e2e8f0;
            margin: 0;
            padding: 24px;
        }
        h1 { color: #e44d26; margin: 0 0 4px; font-size: 1.8rem; }
        h2 { color: #94a3b8; font-size: 1rem; font-weight: 400; margin: 0 0 32px; }
        h3 { color: #60a5fa; margin: 0 0 12px; font-size: 1rem; }
        .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 16px; }
        .card {
            background: #1e2330;
            border: 1px solid #2d3748;
            border-radius: 10px;
            padding: 20px;
        }
        .badge {
            display: inline-block;
            padding: 2px 8px;
            border-radius: 999px;
            font-size: 0.7rem;
            font-weight: 700;
            text-transform: uppercase;
            margin-bottom: 8px;
        }
        .badge-sql      { background: #92400e; color: #fde68a; }
        .badge-code     { background: #4c1d95; color: #ddd6fe; }
        .badge-behavior { background: #065f46; color: #a7f3d0; }
        .badge-safe     { background: #1e3a5f; color: #93c5fd; }
        code {
            font-family: 'Fira Mono', 'Consolas', monospace;
            font-size: 0.78rem;
            background: #0f1117;
            border: 1px solid #2d3748;
            border-radius: 4px;
            padding: 2px 6px;
            color: #f9a8d4;
            word-break: break-all;
        }
        button {
            margin-top: 12px;
            padding: 8px 16px;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            font-size: 0.85rem;
            font-weight: 600;
            transition: opacity 0.15s;
        }
        button:hover { opacity: 0.85; }
        .btn-test   { background: #3b82f6; color: #fff; }
        .btn-safe   { background: #10b981; color: #fff; }
        .result {
            margin-top: 12px;
            padding: 10px 14px;
            border-radius: 6px;
            font-family: 'Fira Mono', monospace;
            font-size: 0.78rem;
            white-space: pre-wrap;
            word-break: break-all;
            display: none;
        }
        .result-ok  { background: #052e16; color: #4ade80; border: 1px solid #16a34a; }
        .result-err { background: #450a0a; color: #f87171; border: 1px solid #dc2626; }
        .desc { color: #94a3b8; font-size: 0.82rem; margin: 6px 0 10px; line-height: 1.5; }
        .sep { border: 0; border-top: 1px solid #2d3748; margin: 24px 0; }
        .info-bar {
            background: #1e2330;
            border: 1px solid #2d3748;
            border-radius: 10px;
            padding: 14px 20px;
            margin-bottom: 24px;
            font-size: 0.85rem;
            color: #94a3b8;
        }
        .info-bar strong { color: #e2e8f0; }
        .curl-box {
            background: #0f1117;
            border: 1px solid #2d3748;
            border-radius: 6px;
            padding: 12px 16px;
            font-family: monospace;
            font-size: 0.78rem;
            color: #a5f3fc;
            white-space: pre;
            overflow-x: auto;
        }
    </style>
</head>
<body>

<h1>Turbine Security Demo</h1>
<h2>All OWASP guards active — click any test to hit <code>/api.php</code> and observe the response</h2>

<div class="info-bar">
    <strong>How it works:</strong> Requests pass through Turbine's Rust security layer before PHP runs.
    Blocked payloads receive <strong>HTTP 403</strong> instantly — PHP never executes.
    Safe inputs reach <code>api.php</code> and return <strong>HTTP 200</strong> with the echoed input.
    Server: <strong>PHP <?= PHP_VERSION ?></strong> · Turbine worker #<?= $_SERVER['TURBINE_WORKER_ID'] ?? '0' ?>
</div>

<div class="grid">

    <!-- ── Safe input ─────────────────────────────────────────── -->
    <div class="card">
        <span class="badge badge-safe">Safe input</span>
        <h3>Normal query string</h3>
        <p class="desc">Ordinary text passes all guards and reaches PHP unchanged.</p>
        <code>?q=Hello+from+Turbine!</code>
        <br>
        <button class="btn-safe" onclick="probe(this, '/api.php?q=Hello+from+Turbine!', 'GET')">
            ▶ Run (expect 200)
        </button>
        <div class="result"></div>
    </div>

    <!-- ── SQL Injection ──────────────────────────────────────── -->
    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>UNION SELECT injection</h3>
        <p class="desc">Classic column enumeration via <code>UNION SELECT</code>. Matched by Aho-Corasick automaton in ~150 ns.</p>
        <code>?q=1+UNION+SELECT+*+FROM+users</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php?q=1+UNION+SELECT+*+FROM+users', 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>DROP TABLE (destructive)</h3>
        <p class="desc">Destructive DDL statement injected into a parameter.</p>
        <code>?q=%27%3B+DROP+TABLE+users%3B--</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q='; DROP TABLE users;--\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>Blind SLEEP injection</h3>
        <p class="desc">Time-based blind injection. Turbine blocks before execution — no 5-second delay.</p>
        <code>?q=1+AND+SLEEP(5)</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php?q=1+AND+SLEEP(5)', 'GET')">
            ▶ Run (expect 403, instant)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>WaitFor Delay (MSSQL)</h3>
        <p class="desc">Microsoft SQL Server variant of time-based blind injection.</p>
        <code>?q=1%27%3B+WAITFOR+DELAY+%270:0:5%27--</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=1'; WAITFOR DELAY '0:0:5'--\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>INFORMATION_SCHEMA enumeration</h3>
        <p class="desc">Schema-discovery attack to list all tables in the database.</p>
        <code>?q=UNION+SELECT+table_name+FROM+information_schema.tables</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php?q=UNION+SELECT+table_name+FROM+information_schema.tables', 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>Comment-bypass obfuscation (/**/ )</h3>
        <p class="desc">Attacker inserts inline SQL comments to break simple keyword matching. Turbine uses full-string Aho-Corasick — comments don&apos;t help.</p>
        <code>?q=1/**/UNION/**/SELECT/**/1,2,3</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php?q=1/**/UNION/**/SELECT/**/1,2,3', 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-sql">SQL Guard</span>
        <h3>LOAD_FILE — file exfiltration</h3>
        <p class="desc">MySQL function used to read server files inside SQL queries.</p>
        <code>?q=1+AND+LOAD_FILE('/etc/passwd')</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=1 AND LOAD_FILE('/etc/passwd')\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <!-- ── Code Injection ─────────────────────────────────────── -->
    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>eval() — direct PHP execution</h3>
        <p class="desc">The most direct PHP code injection vector.</p>
        <code>?q=eval('system("id")')</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=eval('system(id)')\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>eval(base64_decode()) — obfuscated payload</h3>
        <p class="desc">Classic obfuscation chain detected by the obfuscation-chain automaton (phase 1).</p>
        <code>?q=eval(base64_decode('c3lzdGVtKCdpZCcp'))</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=eval(base64_decode('c3lzdGVtKCdpZCcp'))\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>Multi-layer deobfuscation chain</h3>
        <p class="desc"><code>eval(gzinflate(base64_decode(...)))</code> — triple-layer obfuscation used by PHP webshells.</p>
        <code>?q=eval(gzinflate(base64_decode('encoded')))</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=eval(gzinflate(base64_decode('encoded')))\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>system() — OS command execution</h3>
        <p class="desc">Direct system call injection — matched by basic code pattern automaton.</p>
        <code>?q=system('cat+/etc/shadow')</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=system('cat /etc/shadow')\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>Backtick operator</h3>
        <p class="desc">PHP&apos;s backtick executes a shell command and returns its output.</p>
        <code>?q=`whoami`</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php?q=`whoami`', 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>ReflectionFunction — dynamic invocation</h3>
        <p class="desc">PHP Reflection API used to call dangerous functions indirectly.</p>
        <code>?q=$rf=new+ReflectionFunction('system')</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=$rf=new ReflectionFunction('system')\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-code">Code Guard</span>
        <h3>$$var — variable variables</h3>
        <p class="desc">Double-dollar lets attackers call arbitrary functions by name stored in a variable.</p>
        <code>?q=$$func('id')</code>
        <br>
        <button class="btn-test" onclick="probe(this, \"/api.php?q=$$func('id')\", 'GET')">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

    <!-- ── Behaviour Guard ────────────────────────────────────── -->
    <div class="card">
        <span class="badge badge-behavior">Behaviour Guard</span>
        <h3>SQLi accumulation block</h3>
        <p class="desc">
            Demo config sets <code>sqli_block_threshold = 3</code>. After 3 SQL injection attempts
            your IP is blocked for 10 minutes — even on clean requests.
            Click the SQL tests above 3 times then try the normal request.
        </p>
        <button class="btn-safe" onclick="probe(this, '/api.php?q=safe+request+after+attacks', 'GET')">
            ▶ Safe request (will be 403 if you triggered 3 SQLi blocks)
        </button>
        <div class="result"></div>
    </div>

    <div class="card">
        <span class="badge badge-behavior">Behaviour Guard</span>
        <h3>POST body injection</h3>
        <p class="desc">Security guards also scan JSON POST body parameters, not just GET query strings.</p>
        <code>POST /api.php  {"q": "1 UNION SELECT 1,2,3"}</code>
        <br>
        <button class="btn-test" onclick="probe(this, '/api.php', 'POST', {q: '1 UNION SELECT 1,2,3'})">
            ▶ Run (expect 403)
        </button>
        <div class="result"></div>
    </div>

</div>

<hr class="sep">
<h3 style="color:#94a3b8; font-size:0.9rem; margin-bottom:8px">▸ Equivalent curl commands</h3>
<div class="curl-box"><?php
$base = 'http://127.0.0.1:8083';
$examples = [
    ['GET', "$base/api.php?q=Hello+World",                            '200 — safe input'],
    ['GET', "$base/api.php?q=1+UNION+SELECT+*+FROM+users",            '403 — SQL injection'],
    ['GET', "$base/api.php?q=1+AND+SLEEP(5)",                         '403 — blind SQLi'],
    ['GET', "$base/api.php?q=eval(base64_decode('c3lzdGVtKCdpZCcp'))",'403 — obfuscated eval'],
    ['GET', "$base/api.php?q=system('id')",                           '403 — system()'],
    ['GET', "$base/api.php?q=1/**/UNION/**/SELECT/**/1,2,3",          '403 — comment bypass'],
    ['POST',"$base/api.php",                                          '403 — SQLi in POST body (see -d below)'],
];
foreach ($examples as [$method, $url, $note]) {
    if ($method === 'POST') {
        echo "curl -s -o /dev/null -w \"%{http_code}\" -X POST $base/api.php \\\n";
        echo "     -H 'Content-Type: application/json' \\\n";
        echo "     -d '{\"q\":\"1 UNION SELECT 1,2,3\"}'   # $note\n\n";
    } else {
        printf("curl -s -o /dev/null -w \"%%{http_code}\" \"%s\"   # %s\n", $url, $note);
    }
}
?></div>

<script>
async function probe(btn, url, method = 'GET', body = null) {
    const card = btn.closest('.card');
    const resultBox = card.querySelector('.result');
    btn.disabled = true;
    btn.textContent = '⏳ …';
    resultBox.style.display = 'none';

    try {
        const opts = { method };
        if (body) {
            opts.headers = { 'Content-Type': 'application/json' };
            opts.body = JSON.stringify(body);
        }

        const resp = await fetch(url, opts);
        const text = await resp.text();
        const ok   = resp.status === 200;

        resultBox.className = 'result ' + (ok ? 'result-ok' : 'result-err');
        resultBox.textContent =
            `HTTP ${resp.status} ${resp.statusText}\n\n` +
            (text.length > 600 ? text.slice(0, 600) + '\n… (truncated)' : text);
        resultBox.style.display = 'block';

        btn.textContent = ok ? '✓ 200 Allowed' : `✗ ${resp.status} Blocked`;
        btn.style.background = ok ? '#059669' : '#dc2626';
    } catch (err) {
        resultBox.className = 'result result-err';
        resultBox.textContent = `Network error: ${err.message}`;
        resultBox.style.display = 'block';
        btn.textContent = '⚠ Error';
        btn.style.background = '#b45309';
    } finally {
        btn.disabled = false;
    }
}
</script>
</body>
</html>
