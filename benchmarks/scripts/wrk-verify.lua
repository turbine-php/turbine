-- wrk-verify.lua — Validates responses are unique (no caching) with per-thread tracking.
--
-- wrk runs each thread in its own Lua state. The response() callback tracks
-- duplicates per-thread. done() is called once globally and reports aggregated stats.
--
-- Usage: wrk -c N -d Xs -t T -s wrk-verify.lua <url>

local total      = 0
local duplicates = 0
local val_errors = 0
local status_ok  = 0
local status_err = 0

-- Track unique values
local unique_c   = {}
local unique_ts  = {}
local pids       = {}

-- For large-body tests, compare first 128 bytes as fingerprint
local seen_prefix = {}

local req_counter = 0

init = function(args)
    -- nothing needed, ensures response() is called
end

request = function()
    req_counter = req_counter + 1
    wrk.headers["X-Request-Token"] = string.format("r%d", req_counter)
    return wrk.format(nil, nil, nil, nil)
end

response = function(status, headers, body)
    total = total + 1

    if status >= 200 and status < 300 then
        status_ok = status_ok + 1
    else
        status_err = status_err + 1
        return
    end

    -- Check for duplicate response (use first 128 bytes as fingerprint)
    local prefix = body:sub(1, 128)
    if seen_prefix[prefix] then
        duplicates = duplicates + 1
    else
        seen_prefix[prefix] = true
    end

    -- Parse JSON fields if present
    local c_val   = body:match('"c"%s*:%s*(%d+)')
    local ts_val  = body:match('"ts"%s*:%s*(%d+)')
    local pid_val = body:match('"pid"%s*:%s*(%d+)')

    if c_val  then unique_c[c_val]   = true end
    if ts_val then unique_ts[ts_val] = true end
    if pid_val then pids[pid_val]    = true end

    -- Validate SHA-256 hash format (64 hex chars)
    local hash_val = body:match('"hash"%s*:%s*"([^"]+)"')
    if hash_val then
        if #hash_val ~= 64 or not hash_val:match("^[0-9a-f]+$") then
            val_errors = val_errors + 1
        end
    end
end

local function tcount(t)
    local n = 0
    for _ in pairs(t) do n = n + 1 end
    return n
end

done = function(summary, latency, requests)
    local req_errors = summary.errors.connect + summary.errors.timeout
    local rps = math.floor(summary.requests / (summary.duration / 1e6))
    local p50 = latency:percentile(50) / 1000
    local p99 = latency:percentile(99) / 1000
    local pmax = latency.max / 1000

    print(string.format(
        '{"rps":%d,"latency_p50_ms":%.2f,"latency_p99_ms":%.2f,"latency_max_ms":%.2f,' ..
        '"req_2xx":%d,"req_errors":%d,' ..
        '"total_responses":%d,"duplicates":%d,"unique_counters":%d,' ..
        '"unique_timestamps":%d,"unique_pids":%d,"validation_errors":%d,' ..
        '"status_ok":%d,"status_err":%d}',
        rps, p50, p99, pmax,
        summary.requests - req_errors, req_errors,
        total, duplicates, tcount(unique_c),
        tcount(unique_ts), tcount(pids), val_errors,
        status_ok, status_err
    ))
end
