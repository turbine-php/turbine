-- wrk-framework.lua — Multi-route benchmark for framework scenarios.
-- Rotates between: GET /, GET /user/<random_id>, POST /user
-- Combines request generation with JSON report output.
--
-- Usage: wrk -c N -d Xs -t T -s /root/bench/wrk-framework.lua <base_url>

local counter = 0

function request()
    counter = counter + 1
    local choice = counter % 3

    if choice == 0 then
        return wrk.format("GET", "/")
    elseif choice == 1 then
        return wrk.format("GET", "/user/" .. math.random(1, 100000))
    else
        return wrk.format("POST", "/user", nil, "")
    end
end

-- HTTP status classifier — see wrk-report.lua for rationale.
local status_2xx, status_3xx, status_4xx, status_5xx, status_other = 0, 0, 0, 0, 0
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
    local req_errors  = summary.errors.connect + summary.errors.timeout
    local req_non_2xx = status_3xx + status_4xx + status_5xx + status_other
    local req_2xx     = status_2xx
    if req_2xx == 0 and req_non_2xx == 0 then
        req_2xx = summary.requests - req_errors
    end
    local rps  = math.floor(summary.requests / (summary.duration / 1e6))
    local p50  = latency:percentile(50) / 1000
    local p99  = latency:percentile(99) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_max_ms":%.2f,' ..
        '"req_2xx":%d,"req_errors":%d,"req_non_2xx":%d,' ..
        '"status_2xx":%d,"status_3xx":%d,"status_4xx":%d,"status_5xx":%d,"status_other":%d,' ..
        '"first_bad_status":%d}',
        rps, p50, p99, pmax,
        req_2xx, req_errors, req_non_2xx,
        status_2xx, status_3xx, status_4xx, status_5xx, status_other,
        first_bad_status
    ))
end
