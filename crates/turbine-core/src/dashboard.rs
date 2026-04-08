//! Embedded HTML dashboard for `/_/dashboard`.
//!
//! Single self-contained HTML page with inline CSS and JS.
//! Auto-refreshes metrics from `/_/status` every 2 seconds.
//! When `auth_required` is true a login overlay is shown; the user types
//! their Bearer token, which is stored in sessionStorage and sent on every
//! internal-API request. The actual token value is never embedded in the HTML.

/// Returns the full HTML dashboard page as a static string.
pub fn dashboard_html(listen: &str, auth_required: bool) -> String {
    format!(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Turbine Dashboard</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         background: #0f172a; color: #e2e8f0; padding: 20px; }}
  /* ── Login overlay ─────────────────────────────────────────────── */
  #login-overlay {{
    position: fixed; inset: 0; background: rgba(15,23,42,0.98);
    display: flex; align-items: center; justify-content: center; z-index: 1000;
  }}
  .login-card {{
    background: #1e293b; border-radius: 16px; padding: 40px 36px;
    border: 1px solid #334155; width: 100%; max-width: 380px;
    display: flex; flex-direction: column; gap: 18px;
  }}
  .login-logo {{ text-align: center; }}
  .login-logo h1 {{ font-size: 28px; color: #f97316; }}
  .login-logo p {{ font-size: 13px; color: #64748b; margin-top: 4px; }}
  .login-input {{
    width: 100%; padding: 10px 14px; border-radius: 8px;
    border: 1px solid #334155; background: #0f172a; color: #e2e8f0;
    font-size: 14px; outline: none; transition: border-color 0.2s;
  }}
  .login-input:focus {{ border-color: #f97316; }}
  .login-btn {{
    padding: 10px; border-radius: 8px; border: none;
    background: #f97316; color: #fff; font-size: 15px;
    font-weight: 600; cursor: pointer; width: 100%; transition: background 0.2s;
  }}
  .login-btn:hover {{ background: #ea6f10; }}
  #login-error {{
    color: #ef4444; font-size: 13px; text-align: center; display: none;
  }}
  /* ── Main content ───────────────────────────────────────────────── */
  .header {{ display: flex; align-items: center; gap: 16px; margin-bottom: 24px; }}
  .header h1 {{ font-size: 24px; color: #f97316; }}
  .header .ver {{ color: #64748b; font-size: 14px; }}
  .header .uptime {{ margin-left: auto; color: #94a3b8; font-size: 14px; }}
  .logout-btn {{
    padding: 4px 12px; border-radius: 6px; border: 1px solid #334155;
    background: transparent; color: #64748b; font-size: 12px; cursor: pointer;
    transition: border-color 0.2s, color 0.2s; display: none;
  }}
  .logout-btn:hover {{ border-color: #ef4444; color: #ef4444; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; margin-bottom: 24px; }}
  .card {{ background: #1e293b; border-radius: 12px; padding: 20px; border: 1px solid #334155; }}
  .card .label {{ font-size: 12px; color: #64748b; text-transform: uppercase; letter-spacing: 1px; }}
  .card .value {{ font-size: 32px; font-weight: 700; margin-top: 4px; }}
  .card .sub {{ font-size: 13px; color: #94a3b8; margin-top: 4px; }}
  .green {{ color: #22c55e; }}
  .orange {{ color: #f97316; }}
  .red {{ color: #ef4444; }}
  .blue {{ color: #3b82f6; }}
  .purple {{ color: #a855f7; }}
  .section {{ background: #1e293b; border-radius: 12px; padding: 20px; border: 1px solid #334155; margin-bottom: 24px; }}
  .section h2 {{ font-size: 16px; color: #f97316; margin-bottom: 16px; }}
  table {{ width: 100%; border-collapse: collapse; }}
  th {{ text-align: left; font-size: 12px; color: #64748b; text-transform: uppercase; letter-spacing: 1px; padding: 8px; border-bottom: 1px solid #334155; }}
  td {{ padding: 8px; font-size: 14px; border-bottom: 1px solid #1e293b; }}
  .bar-bg {{ width: 100%; height: 8px; background: #334155; border-radius: 4px; overflow: hidden; }}
  .bar-fill {{ height: 100%; border-radius: 4px; transition: width 0.5s; }}
  .status-dot {{ display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }}
  .status-dot.ok {{ background: #22c55e; }}
  .pill {{ display: inline-block; padding: 2px 8px; border-radius: 10px; font-size: 11px; font-weight: 600; }}
  .pill.s2 {{ background: #22c55e22; color: #22c55e; }}
  .pill.s4 {{ background: #f9731622; color: #f97316; }}
  .pill.s5 {{ background: #ef444422; color: #ef4444; }}
  #error-banner {{ display: none; background: #ef444422; color: #ef4444; padding: 10px 16px; border-radius: 8px; margin-bottom: 16px; font-size: 14px; }}
  .btn-unblock {{ padding: 4px 12px; border-radius: 6px; border: none; background: #ef444422; color: #ef4444; font-size: 12px; cursor: pointer; font-weight: 600; }}
  .btn-unblock:hover {{ background: #ef444444; }}
  .empty-state {{ color: #64748b; font-size: 14px; padding: 8px 0; }}
</style>
</head>
<body>

<!-- Login overlay (shown when auth is required and no token in sessionStorage) -->
<div id="login-overlay" style="display:none">
  <div class="login-card">
    <div class="login-logo">
      <h1>Turbine</h1>
      <p>Dashboard access is protected. Enter the Bearer token to continue.</p>
    </div>
    <input type="password" id="token-input" class="login-input"
           placeholder="Bearer token" autocomplete="current-password"
           onkeydown="if(event.key==='Enter')doLogin()" />
    <div id="login-error"></div>
    <button class="login-btn" onclick="doLogin()">Sign in</button>
  </div>
</div>

<!-- Dashboard -->
<div id="main-content" style="display:none">
<div class="header">
  <h1>Turbine</h1>
  <span class="ver" id="version"></span>
  <span class="uptime" id="uptime"></span>
  <button class="logout-btn" id="logout-btn" onclick="doLogout()">Sign out</button>
</div>
<div id="error-banner"></div>
<div class="grid">
  <div class="card">
    <div class="label">Requests</div>
    <div class="value green" id="total-reqs">-</div>
    <div class="sub" id="rps">- req/s</div>
  </div>
  <div class="card">
    <div class="label">Latency (mean)</div>
    <div class="value blue" id="latency-mean">-</div>
    <div class="sub" id="latency-detail">p50: - / p99: -</div>
  </div>
  <div class="card">
    <div class="label">Cache Hit Ratio</div>
    <div class="value orange" id="cache-ratio">-</div>
    <div class="sub" id="cache-detail">hits: - / misses: -</div>
  </div>
  <div class="card">
    <div class="label">Security Blocks</div>
    <div class="value red" id="sec-blocks">0</div>
    <div class="sub" id="sec-detail">OWASP guards active</div>
  </div>
  <div class="card">
    <div class="label">Workers</div>
    <div class="value purple" id="workers">-</div>
    <div class="sub"><span class="status-dot ok"></span>All healthy</div>
  </div>
  <div class="card">
    <div class="label">Bytes Out</div>
    <div class="value" style="color:#38bdf8" id="bytes-out">-</div>
    <div class="sub" id="status-codes">2xx: - / 4xx: - / 5xx: -</div>
  </div>
</div>
<div class="section">
  <h2>Endpoints</h2>
  <table>
    <thead><tr><th>Path</th><th>Requests</th><th>Errors</th><th>Mean (ms)</th><th>P99 (ms)</th><th>Load</th></tr></thead>
    <tbody id="endpoints-body"><tr><td colspan="6" style="color:#64748b">Loading...</td></tr></tbody>
  </table>
</div>
<div class="section">
  <h2>Status Codes</h2>
  <div style="display:flex;gap:12px;flex-wrap:wrap">
    <span class="pill s2" id="pill-2xx">2xx: -</span>
    <span class="pill s4" id="pill-4xx">4xx: -</span>
    <span class="pill s5" id="pill-5xx">5xx: -</span>
  </div>
</div>
<div class="section">
  <h2>Blocked IPs</h2>
  <div id="blocked-content"><div class="empty-state">Loading...</div></div>
</div>
</div><!-- /#main-content -->

<script>
const AUTH_REQUIRED = {auth_required};
const STATUS_URL  = 'http://{listen}/_/status';
const BLOCKED_URL = 'http://{listen}/_/security/blocked';
const UNBLOCK_URL = 'http://{listen}/_/security/unblock';

let _refreshTimer = null;
let _blockedTimer = null;

function getToken() {{ return sessionStorage.getItem('turbine_token'); }}

function authHeaders(extra) {{
  const h = Object.assign({{}}, extra);
  const t = getToken();
  if (t) h['Authorization'] = 'Bearer ' + t;
  return h;
}}

function showLogin(err) {{
  clearTimers();
  document.getElementById('login-overlay').style.display = 'flex';
  document.getElementById('main-content').style.display = 'none';
  const errEl = document.getElementById('login-error');
  if (err) {{ errEl.textContent = err; errEl.style.display = 'block'; }}
  else {{ errEl.style.display = 'none'; }}
  setTimeout(() => document.getElementById('token-input').focus(), 50);
}}

function doLogin() {{
  const t = document.getElementById('token-input').value.trim();
  if (!t) return;
  sessionStorage.setItem('turbine_token', t);
  document.getElementById('login-overlay').style.display = 'none';
  document.getElementById('main-content').style.display = 'block';
  if (AUTH_REQUIRED) document.getElementById('logout-btn').style.display = 'inline-block';
  startRefresh();
}}

function doLogout() {{
  sessionStorage.removeItem('turbine_token');
  showLogin();
}}

function clearTimers() {{
  if (_refreshTimer) {{ clearInterval(_refreshTimer); _refreshTimer = null; }}
  if (_blockedTimer) {{ clearInterval(_blockedTimer); _blockedTimer = null; }}
}}

function startRefresh() {{
  refresh();
  refreshBlocked();
  _refreshTimer = setInterval(refresh, 2000);
  _blockedTimer = setInterval(refreshBlocked, 5000);
}}

function handleUnauthorized() {{
  sessionStorage.removeItem('turbine_token');
  showLogin('Invalid token — please try again.');
}}

function fmt(n) {{ if (n >= 1e6) return (n/1e6).toFixed(1)+'M'; if (n >= 1e3) return (n/1e3).toFixed(1)+'K'; return n.toString(); }}
function fmtBytes(b) {{ if (b >= 1073741824) return (b/1073741824).toFixed(1)+' GB'; if (b >= 1048576) return (b/1048576).toFixed(1)+' MB'; if (b >= 1024) return (b/1024).toFixed(1)+' KB'; return b+' B'; }}
function fmtUptime(s) {{ const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), sec=s%60; return (h?h+'h ':'')+(m?m+'m ':'')+(sec+'s'); }}

async function refresh() {{
  try {{
    const r = await fetch(STATUS_URL, {{headers: authHeaders({{}})}});
    if (r.status === 401) {{ handleUnauthorized(); return; }}
    const d = await r.json();
    document.getElementById('error-banner').style.display = 'none';
    document.getElementById('version').textContent = 'Runtime';
    document.getElementById('uptime').textContent = 'Uptime: ' + fmtUptime(d.uptime_seconds);
    document.getElementById('total-reqs').textContent = fmt(d.total_requests);
    document.getElementById('rps').textContent = d.requests_per_second.toFixed(1) + ' req/s';
    document.getElementById('latency-mean').textContent = d.latency_ms.mean.toFixed(2) + ' ms';
    document.getElementById('latency-detail').textContent = 'p50: ' + d.latency_ms.p50.toFixed(2) + 'ms / p99: ' + d.latency_ms.p99.toFixed(2) + 'ms';
    document.getElementById('cache-ratio').textContent = (d.cache.hit_ratio * 100).toFixed(1) + '%';
    document.getElementById('cache-detail').textContent = 'hits: ' + fmt(d.cache.hits) + ' / misses: ' + fmt(d.cache.misses);
    document.getElementById('sec-blocks').textContent = fmt(d.security.blocks);
    document.getElementById('workers').textContent = d.workers;
    document.getElementById('bytes-out').textContent = fmtBytes(d.bytes_out);
    document.getElementById('status-codes').textContent = '2xx: '+fmt(d.status_codes['2xx'])+' / 4xx: '+fmt(d.status_codes['4xx'])+' / 5xx: '+fmt(d.status_codes['5xx']);
    document.getElementById('pill-2xx').textContent = '2xx: '+fmt(d.status_codes['2xx']);
    document.getElementById('pill-4xx').textContent = '4xx: '+fmt(d.status_codes['4xx']);
    document.getElementById('pill-5xx').textContent = '5xx: '+fmt(d.status_codes['5xx']);
    const tbody = document.getElementById('endpoints-body');
    if (d.endpoints && d.endpoints.length) {{
      const maxReqs = Math.max(...d.endpoints.map(e => e.requests));
      tbody.innerHTML = d.endpoints.map(e => `
        <tr>
          <td style="font-family:monospace">${{e.path}}</td>
          <td>${{fmt(e.requests)}}</td>
          <td style="color:${{e.errors>0?'#ef4444':'#64748b'}}">${{e.errors}}</td>
          <td>${{e.mean_ms.toFixed(2)}}</td>
          <td>${{e.p99_ms.toFixed(2)}}</td>
          <td><div class="bar-bg"><div class="bar-fill" style="width:${{(e.requests/maxReqs*100).toFixed(0)}}%;background:#f97316"></div></div></td>
        </tr>
      `).join('');
    }}
  }} catch (e) {{
    document.getElementById('error-banner').style.display = 'block';
    document.getElementById('error-banner').textContent = 'Cannot connect to server: ' + e.message;
  }}
}}

async function refreshBlocked() {{
  const el = document.getElementById('blocked-content');
  try {{
    const r = await fetch(BLOCKED_URL, {{headers: authHeaders({{}})}});
    if (r.status === 401) {{ handleUnauthorized(); return; }}
    if (!r.ok) {{ el.innerHTML = '<div class="empty-state">Security data unavailable (' + r.status + ')</div>'; return; }}
    const d = await r.json();
    if (!d.blocked || d.blocked.length === 0) {{
      el.innerHTML = '<div class="empty-state">No blocked IPs</div>';
    }} else {{
      el.innerHTML = '<table><thead><tr><th>IP Address</th><th>Expires In</th><th>Action</th></tr></thead><tbody>' +
        d.blocked.map(b => '<tr>' +
          '<td style="font-family:monospace">' + b.ip + '</td>' +
          '<td style="color:#ef4444">' + (b.expires_in_secs != null ? b.expires_in_secs + 's' : 'permanent') + '</td>' +
          '<td><button class="btn-unblock" onclick="unblockIp(\'' + b.ip + '\')">Unblock</button></td>' +
          '</tr>'
        ).join('') + '</tbody></table>';
    }}
  }} catch (e) {{
    el.innerHTML = '<div class="empty-state">Error loading blocked IPs</div>';
  }}
}}

async function unblockIp(ip) {{
  try {{
    const r = await fetch(UNBLOCK_URL, {{
      method: 'POST',
      headers: authHeaders({{'Content-Type': 'application/json'}}),
      body: JSON.stringify({{ip}})
    }});
    if (r.status === 401) {{ handleUnauthorized(); return; }}
  }} catch (_) {{}}
  refreshBlocked();
}}

// ── Bootstrap ──────────────────────────────────────────────────────
if (AUTH_REQUIRED) {{
  const stored = getToken();
  if (stored) {{
    document.getElementById('login-overlay').style.display = 'none';
    document.getElementById('main-content').style.display = 'block';
    document.getElementById('logout-btn').style.display = 'inline-block';
    startRefresh();
  }} else {{
    showLogin();
  }}
}} else {{
  document.getElementById('login-overlay').style.display = 'none';
  document.getElementById('main-content').style.display = 'block';
  startRefresh();
}}
</script>
</body>
</html>"##, listen = listen, auth_required = auth_required)
}

    format!(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Turbine Dashboard</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         background: #0f172a; color: #e2e8f0; padding: 20px; }}
  .header {{ display: flex; align-items: center; gap: 16px; margin-bottom: 24px; }}
  .header h1 {{ font-size: 24px; color: #f97316; }}
  .header .ver {{ color: #64748b; font-size: 14px; }}
  .header .uptime {{ margin-left: auto; color: #94a3b8; font-size: 14px; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; margin-bottom: 24px; }}
  .card {{ background: #1e293b; border-radius: 12px; padding: 20px; border: 1px solid #334155; }}
  .card .label {{ font-size: 12px; color: #64748b; text-transform: uppercase; letter-spacing: 1px; }}
  .card .value {{ font-size: 32px; font-weight: 700; margin-top: 4px; }}
  .card .sub {{ font-size: 13px; color: #94a3b8; margin-top: 4px; }}
  .green {{ color: #22c55e; }}
  .orange {{ color: #f97316; }}
  .red {{ color: #ef4444; }}
  .blue {{ color: #3b82f6; }}
  .purple {{ color: #a855f7; }}
  .section {{ background: #1e293b; border-radius: 12px; padding: 20px; border: 1px solid #334155; margin-bottom: 24px; }}
  .section h2 {{ font-size: 16px; color: #f97316; margin-bottom: 16px; }}
  table {{ width: 100%; border-collapse: collapse; }}
  th {{ text-align: left; font-size: 12px; color: #64748b; text-transform: uppercase; letter-spacing: 1px; padding: 8px; border-bottom: 1px solid #334155; }}
  td {{ padding: 8px; font-size: 14px; border-bottom: 1px solid #1e293b; }}
  .bar-bg {{ width: 100%; height: 8px; background: #334155; border-radius: 4px; overflow: hidden; }}
  .bar-fill {{ height: 100%; border-radius: 4px; transition: width 0.5s; }}
  .status-dot {{ display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }}
  .status-dot.ok {{ background: #22c55e; }}
  .pill {{ display: inline-block; padding: 2px 8px; border-radius: 10px; font-size: 11px; font-weight: 600; }}
  .pill.s2 {{ background: #22c55e22; color: #22c55e; }}
  .pill.s4 {{ background: #f9731622; color: #f97316; }}
  .pill.s5 {{ background: #ef444422; color: #ef4444; }}
  #error-banner {{ display: none; background: #ef444422; color: #ef4444; padding: 10px 16px; border-radius: 8px; margin-bottom: 16px; font-size: 14px; }}
  .btn-unblock {{ padding: 4px 12px; border-radius: 6px; border: none; background: #ef444422; color: #ef4444; font-size: 12px; cursor: pointer; font-weight: 600; }}
  .btn-unblock:hover {{ background: #ef444444; }}
  .empty-state {{ color: #64748b; font-size: 14px; padding: 8px 0; }}
</style>
</head>
<body>
<div class="header">
  <h1>Turbine</h1>
  <span class="ver" id="version"></span>
  <span class="uptime" id="uptime"></span>
</div>
<div id="error-banner"></div>
<div class="grid">
  <div class="card">
    <div class="label">Requests</div>
    <div class="value green" id="total-reqs">-</div>
    <div class="sub" id="rps">- req/s</div>
  </div>
  <div class="card">
    <div class="label">Latency (mean)</div>
    <div class="value blue" id="latency-mean">-</div>
    <div class="sub" id="latency-detail">p50: - / p99: -</div>
  </div>
  <div class="card">
    <div class="label">Cache Hit Ratio</div>
    <div class="value orange" id="cache-ratio">-</div>
    <div class="sub" id="cache-detail">hits: - / misses: -</div>
  </div>
  <div class="card">
    <div class="label">Security Blocks</div>
    <div class="value red" id="sec-blocks">0</div>
    <div class="sub" id="sec-detail">OWASP guards active</div>
  </div>
  <div class="card">
    <div class="label">Workers</div>
    <div class="value purple" id="workers">-</div>
    <div class="sub"><span class="status-dot ok"></span>All healthy</div>
  </div>
  <div class="card">
    <div class="label">Bytes Out</div>
    <div class="value" style="color:#38bdf8" id="bytes-out">-</div>
    <div class="sub" id="status-codes">2xx: - / 4xx: - / 5xx: -</div>
  </div>
</div>
<div class="section">
  <h2>Endpoints</h2>
  <table>
    <thead><tr><th>Path</th><th>Requests</th><th>Errors</th><th>Mean (ms)</th><th>P99 (ms)</th><th>Load</th></tr></thead>
    <tbody id="endpoints-body"><tr><td colspan="6" style="color:#64748b">Loading...</td></tr></tbody>
  </table>
</div>
<div class="section">
  <h2>Status Codes</h2>
  <div style="display:flex;gap:12px;flex-wrap:wrap">
    <span class="pill s2" id="pill-2xx">2xx: -</span>
    <span class="pill s4" id="pill-4xx">4xx: -</span>
    <span class="pill s5" id="pill-5xx">5xx: -</span>
  </div>
</div>
<div class="section">
  <h2>Blocked IPs</h2>
  <div id="blocked-content"><div class="empty-state">Loading...</div></div>
</div>
<script>
const STATUS_URL = 'http://{listen}/_/status';
const BLOCKED_URL = 'http://{listen}/_/security/blocked';
const UNBLOCK_URL = 'http://{listen}/_/security/unblock';
const TOKEN = {token};
function authHeaders(extra) {{
  const h = Object.assign({{}}, extra);
  if (TOKEN) h['Authorization'] = 'Bearer ' + TOKEN;
  return h;
}}
function fmt(n) {{ if (n >= 1e6) return (n/1e6).toFixed(1)+'M'; if (n >= 1e3) return (n/1e3).toFixed(1)+'K'; return n.toString(); }}
function fmtBytes(b) {{ if (b >= 1073741824) return (b/1073741824).toFixed(1)+' GB'; if (b >= 1048576) return (b/1048576).toFixed(1)+' MB'; if (b >= 1024) return (b/1024).toFixed(1)+' KB'; return b+' B'; }}
function fmtUptime(s) {{ const h=Math.floor(s/3600), m=Math.floor((s%3600)/60), sec=s%60; return (h?h+'h ':'')+(m?m+'m ':'')+(sec+'s'); }}

async function refresh() {{
  try {{
    const r = await fetch(STATUS_URL, {{headers: authHeaders({{}})}});
    const d = await r.json();
    document.getElementById('error-banner').style.display = 'none';
    document.getElementById('version').textContent = 'Runtime';
    document.getElementById('uptime').textContent = 'Uptime: ' + fmtUptime(d.uptime_seconds);
    document.getElementById('total-reqs').textContent = fmt(d.total_requests);
    document.getElementById('rps').textContent = d.requests_per_second.toFixed(1) + ' req/s';
    document.getElementById('latency-mean').textContent = d.latency_ms.mean.toFixed(2) + ' ms';
    document.getElementById('latency-detail').textContent = 'p50: ' + d.latency_ms.p50.toFixed(2) + 'ms / p99: ' + d.latency_ms.p99.toFixed(2) + 'ms';
    document.getElementById('cache-ratio').textContent = (d.cache.hit_ratio * 100).toFixed(1) + '%';
    document.getElementById('cache-detail').textContent = 'hits: ' + fmt(d.cache.hits) + ' / misses: ' + fmt(d.cache.misses);
    document.getElementById('sec-blocks').textContent = fmt(d.security.blocks);
    document.getElementById('workers').textContent = d.workers;
    document.getElementById('bytes-out').textContent = fmtBytes(d.bytes_out);
    document.getElementById('status-codes').textContent = '2xx: '+fmt(d.status_codes['2xx'])+' / 4xx: '+fmt(d.status_codes['4xx'])+' / 5xx: '+fmt(d.status_codes['5xx']);
    document.getElementById('pill-2xx').textContent = '2xx: '+fmt(d.status_codes['2xx']);
    document.getElementById('pill-4xx').textContent = '4xx: '+fmt(d.status_codes['4xx']);
    document.getElementById('pill-5xx').textContent = '5xx: '+fmt(d.status_codes['5xx']);
    // Endpoints table
    const tbody = document.getElementById('endpoints-body');
    if (d.endpoints && d.endpoints.length) {{
      const maxReqs = Math.max(...d.endpoints.map(e => e.requests));
      tbody.innerHTML = d.endpoints.map(e => `
        <tr>
          <td style="font-family:monospace">${{e.path}}</td>
          <td>${{fmt(e.requests)}}</td>
          <td style="color:${{e.errors>0?'#ef4444':'#64748b'}}">${{e.errors}}</td>
          <td>${{e.mean_ms.toFixed(2)}}</td>
          <td>${{e.p99_ms.toFixed(2)}}</td>
          <td><div class="bar-bg"><div class="bar-fill" style="width:${{(e.requests/maxReqs*100).toFixed(0)}}%;background:#f97316"></div></div></td>
        </tr>
      `).join('');
    }}
  }} catch (e) {{
    document.getElementById('error-banner').style.display = 'block';
    document.getElementById('error-banner').textContent = 'Cannot connect to server: ' + e.message;
  }}
}}

async function refreshBlocked() {{
  const el = document.getElementById('blocked-content');
  try {{
    const r = await fetch(BLOCKED_URL, {{headers: authHeaders({{}})}});
    if (!r.ok) {{ el.innerHTML = '<div class="empty-state">Security data unavailable (' + r.status + ')</div>'; return; }}
    const d = await r.json();
    if (!d.blocked || d.blocked.length === 0) {{
      el.innerHTML = '<div class="empty-state">No blocked IPs</div>';
    }} else {{
      el.innerHTML = '<table><thead><tr><th>IP Address</th><th>Expires In</th><th>Action</th></tr></thead><tbody>' +
        d.blocked.map(b => '<tr>' +
          '<td style="font-family:monospace">' + b.ip + '</td>' +
          '<td style="color:#ef4444">' + (b.expires_in_secs != null ? b.expires_in_secs + 's' : 'permanent') + '</td>' +
          '<td><button class="btn-unblock" onclick="unblockIp(\'' + b.ip + '\')">Unblock</button></td>' +
          '</tr>'
        ).join('') + '</tbody></table>';
    }}
  }} catch (e) {{
    el.innerHTML = '<div class="empty-state">Error loading blocked IPs</div>';
  }}
}}

async function unblockIp(ip) {{
  try {{
    await fetch(UNBLOCK_URL, {{
      method: 'POST',
      headers: authHeaders({{'Content-Type': 'application/json'}}),
      body: JSON.stringify({{ip}})
    }});
  }} catch (_) {{}}
  refreshBlocked();
}}

refresh();
refreshBlocked();
setInterval(refresh, 2000);
setInterval(refreshBlocked, 5000);
</script>
</body>
</html>"##, listen = listen, token = token_js)
}
