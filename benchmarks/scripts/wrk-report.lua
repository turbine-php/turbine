-- wrk-report.lua — JSON output for wrk benchmark results
-- Usage: wrk -c N -d Xs -t T -s /root/bench/wrk-report.lua <url>
--
-- Outputs a single JSON line on completion (parsed by parse_wrk in run.sh).
-- Latency values are in microseconds internally; we convert to milliseconds.
--
-- We hook `response(status, ...)` to classify every HTTP status returned by
-- the server.  `summary.errors` only tracks NETWORK-level failures
-- (connect/timeout/read/write); it does NOT catch 4xx/5xx, so a server that
-- returns 404 at ~10 µs each would look like a champion.  Without this
-- classifier, a mis-configured container silently inflates req/s.

local status_2xx = 0
local status_3xx = 0
local status_4xx = 0
local status_5xx = 0
local status_other = 0
local first_bad_status = 0

response = function(status, headers, body)
    if status >= 200 and status < 300 then
        status_2xx = status_2xx + 1
    elseif status >= 300 and status < 400 then
        status_3xx = status_3xx + 1
    elseif status >= 400 and status < 500 then
        status_4xx = status_4xx + 1
        if first_bad_status == 0 then first_bad_status = status end
    elseif status >= 500 and status < 600 then
        status_5xx = status_5xx + 1
        if first_bad_status == 0 then first_bad_status = status end
    else
        status_other = status_other + 1
        if first_bad_status == 0 then first_bad_status = status end
    end
end

done = function(summary, latency, requests)
    -- Network-level failures (connect/timeout). `read`/`write` are transport
    -- events (EOF on keep-alive cycling) and must NOT be counted as errors.
    local req_errors = summary.errors.connect + summary.errors.timeout

    -- HTTP-level non-2xx count (our response() hook).  This is what catches
    -- silent regressions where the server returns 404/502 super fast.
    local req_non_2xx = status_3xx + status_4xx + status_5xx + status_other

    -- Healthy 2xx count (from our hook).  Falls back to summary.requests
    -- if the hook somehow didn't run (older wrk builds).
    local req_2xx = status_2xx
    if req_2xx == 0 and req_non_2xx == 0 then
        req_2xx = summary.requests - req_errors
    end

    -- summary.duration is in microseconds; convert to seconds for rps
    local rps = math.floor(summary.requests / (summary.duration / 1e6))

    -- latency percentiles are in microseconds; convert to ms
    local p50  = latency:percentile(50)   / 1000
    local p99  = latency:percentile(99)   / 1000
    local p999 = latency:percentile(99.9) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_p999_ms":%.2f,"latency_max_ms":%.2f,' ..
        '"req_2xx":%d,"req_errors":%d,"req_non_2xx":%d,' ..
        '"status_2xx":%d,"status_3xx":%d,"status_4xx":%d,"status_5xx":%d,"status_other":%d,' ..
        '"first_bad_status":%d}',
        rps, p50, p99, p999, pmax,
        req_2xx, req_errors, req_non_2xx,
        status_2xx, status_3xx, status_4xx, status_5xx, status_other,
        first_bad_status
    ))
end
