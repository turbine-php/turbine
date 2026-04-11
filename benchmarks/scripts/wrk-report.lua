-- wrk-report.lua — JSON output for wrk benchmark results
-- Usage: wrk -c N -d Xs -t T -s /root/bench/wrk-report.lua <url>
--
-- Outputs a single JSON line on completion (parsed by parse_wrk in run.sh).
-- Latency values are in microseconds internally; we convert to milliseconds.

done = function(summary, latency, requests)
    local errors = summary.errors.connect
                 + summary.errors.read
                 + summary.errors.write
                 + summary.errors.timeout
                 + (summary.errors.status or 0)

    -- summary.duration is in microseconds; convert to seconds for rps
    local rps = math.floor(summary.requests / (summary.duration / 1e6))

    -- latency percentiles are in microseconds; convert to ms
    local p50 = latency:percentile(50) / 1000
    local p99 = latency:percentile(99) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_max_ms":%.2f,"req_2xx":%d,"req_errors":%d}',
        rps, p50, p99, pmax, summary["2xx"], errors
    ))
end
