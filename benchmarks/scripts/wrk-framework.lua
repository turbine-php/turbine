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

done = function(summary, latency, requests)
    local req_errors = summary.errors.connect + summary.errors.timeout
    local req_2xx = summary.requests - req_errors
    local rps = math.floor(summary.requests / (summary.duration / 1e6))
    local p50 = latency:percentile(50) / 1000
    local p99 = latency:percentile(99) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_max_ms":%.2f,"req_2xx":%d,"req_errors":%d}',
        rps, p50, p99, pmax, req_2xx, req_errors
    ))
end
