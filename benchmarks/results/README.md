# Turbine Benchmark Results

No benchmarks have been run yet.

Results are generated automatically by the [benchmark workflow](/.github/workflows/benchmark.yml)
and updated here whenever a new release is published.

Each run provisions a fresh [Hetzner CPX41](https://www.hetzner.com/cloud) instance
(8 vCPU, 16 GB RAM, NVMe SSD), runs the full benchmark suite, and destroys the server.

## Scenarios

| Scenario | Description |
|----------|-------------|
| **Raw PHP** | Single PHP file returning a plain-text Hello World response |
| **Laravel** | Laravel application returning a JSON response (no database) |
| **Phalcon** | Phalcon micro application returning a JSON response |

## Servers compared

| Server | Notes |
|--------|-------|
| Turbine NTS (process) | `worker_mode = "process"`, 8 workers |
| Turbine ZTS (thread)  | `worker_mode = "thread"`, 8 workers |
| Nginx + PHP-FPM       | Static pool, 8 workers — baseline |

## History

Past benchmark runs are archived in [history/](history/).
