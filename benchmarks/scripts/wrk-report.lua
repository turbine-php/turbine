-- wrk-report.lua — JSON output for wrk benchmark results
-- Usage: wrk -c N -d Xs -t T -s /root/bench/wrk-report.lua <url>
--
-- Outputs a single JSON line on completion (parsed by parse_wrk in run.sh).
-- Latency values are in microseconds internally; we convert to milliseconds.

done = function(summary, latency, requests)
    -- Only count hard network failures as errors.
    -- summary.errors.read in wrk 4.1 counts EOF/close events on keep-alive cycling
    -- (incremented for every response read), NOT actual failures. Including it would
    -- make req_errors ≈ total_requests and req_2xx = 0 even on a healthy server.
    local req_errors = summary.errors.connect + summary.errors.timeout

    -- summary.requests = total HTTP requests that completed (all status codes)
    local req_2xx = summary.requests - req_errors

    -- summary.duration is in microseconds; convert to seconds for rps
    local rps = math.floor(summary.requests / (summary.duration / 1e6))

    -- latency percentiles are in microseconds; convert to ms
    local p50 = latency:percentile(50) / 1000
    local p99 = latency:percentile(99) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_max_ms":%.2f,"req_2xx":%d,"req_errors":%d}',
        rps, p50, p99, pmax, req_2xx, req_errors
    ))
end
