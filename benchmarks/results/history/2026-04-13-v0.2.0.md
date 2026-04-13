# Turbine Benchmark Results

| | |
|---|---|
| **Version** | v0.2.0 |
| **Date** | 2026-04-13 |
| **Server** | Hetzner CCX33 (8 vCPU dedicated / 32 GB RAM / NVMe) |
| **Tool** | [wrk](https://github.com/wg/wrk) |
| **Parameters** | 30s · 256 connections |
| **Workers** | 4w and 8w variants (Turbine + FPM) |
| **Memory limit** | 256 MB per worker |
| **Max req/worker** | 50,000 |
| **Turbine NTS image** | `katisuhara/turbine-php:latest-php8.4-nts` |
| **Turbine ZTS image** | `katisuhara/turbine-php:latest-php8.4-zts` |

> **Baseline**: Nginx + PHP-FPM · 8 workers.
> **Persistent**: PHP worker process stays alive across requests (same as FrankenPHP worker mode).
> **FrankenPHP** uses ZTS PHP internally and does **not** support Phalcon.
> All servers (including FPM) run inside Docker containers for equal overhead.
> CPU and memory metrics are collected via `docker stats --no-stream` during benchmark.

> **⚠️ Disclaimer:** These benchmarks are synthetic and may not reflect real-world performance. Results depend heavily on architecture, application design, dependencies, stack, and workload characteristics. The goal is **not** to declare any runtime better or worse — choosing the right tool depends on many factors beyond raw throughput.

---

## Raw PHP

_Single PHP file returning plain-text Hello World_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 149,402 | 4.1× | 1.5 ms | 3.9 ms | 227.8% | 30 MiB | 0 |
| Turbine NTS · 8w | 147,961 | 4.0× | 1.5 ms | 4.0 ms | 227.6% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 149,788 | 4.1× | 1.5 ms | 3.9 ms | 227.5% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 148,798 | 4.0× | 1.5 ms | 3.9 ms | 227.0% | 33 MiB | 0 |
| Turbine ZTS · 4w | 147,507 | 4.0× | 1.5 ms | 4.0 ms | 225.7% | 30 MiB | 0 |
| Turbine ZTS · 8w | 147,980 | 4.0× | 1.5 ms | 3.9 ms | 226.3% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 32,030 | 0.9× | 7.7 ms | 21.3 ms | 514.3% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 32,030 | 0.9× | 7.7 ms | 21.5 ms | 513.2% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 31,348 | 0.9× | 7.8 ms | 22.4 ms | 510.9% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 35,804 | 1.0× | 6.9 ms | 18.1 ms | 487.2% | 58 MiB | 0 |
| Nginx + FPM · 4w | 31,349 | 0.9× | 8.1 ms | 9.9 ms | 435.8% | 33 MiB | 0 |
| Nginx + FPM · 8w | 36,872 | baseline | 6.9 ms | 8.4 ms | 473.8% | 37 MiB | 0 |

## Laravel

_Laravel 13 — mixed routes: GET /, GET /user/:id, POST /user (no database)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 112,942 | 36.7× | 2.0 ms | 5.0 ms | 329.2% | 68 MiB | 0 |
| Turbine NTS · 8w | 113,874 | 37.0× | 2.0 ms | 5.0 ms | 330.7% | 74 MiB | 0 |
| Turbine NTS · 4w · persistent | 113,240 | 36.8× | 2.0 ms | 5.0 ms | 330.5% | 87 MiB | 0 |
| Turbine NTS · 8w · persistent | 113,268 | 36.8× | 2.0 ms | 5.0 ms | 330.8% | 110 MiB | 0 |
| Turbine ZTS · 4w | 113,151 | 36.7× | 2.0 ms | 5.1 ms | 331.0% | 67 MiB | 0 |
| Turbine ZTS · 8w | 113,163 | 36.7× | 2.0 ms | 5.0 ms | 330.7% | 69 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 1,643 | 0.5× | 155.0 ms | 169.3 ms | 772.2% | 92 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 3,319 | 1.1× | 76.6 ms | 92.1 ms | 753.3% | 86 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 1,634 | 0.5× | 155.9 ms | 170.6 ms | 771.8% | 92 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 3,332 | 1.1× | 76.3 ms | 91.8 ms | 753.1% | 88 MiB | 0 |
| Nginx + FPM · 4w | 1,988 | 0.6× | 128.1 ms | 140.9 ms | 427.7% | 67 MiB | 0 |
| Nginx + FPM · 8w | 3,080 | baseline | 82.8 ms | 89.6 ms | 732.8% | 75 MiB | 0 |

## Phalcon

_Phalcon Micro — mixed routes: GET /, GET /user/:id, POST /user_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 113,391 | 5.0× | 2.0 ms | 5.2 ms | 329.3% | 36 MiB | 0 |
| Turbine NTS · 8w | 113,161 | 5.0× | 2.0 ms | 5.1 ms | 328.9% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 113,752 | 5.0× | 2.0 ms | 5.0 ms | 330.7% | 36 MiB | 0 |
| Turbine NTS · 8w · persistent | 113,299 | 5.0× | 2.0 ms | 5.0 ms | 329.8% | 39 MiB | 0 |
| Turbine ZTS · 4w | 114,294 | 5.0× | 2.0 ms | 5.0 ms | 331.4% | 35 MiB | 0 |
| Turbine ZTS · 8w | 113,733 | 5.0× | 2.0 ms | 5.0 ms | 330.6% | 38 MiB | 0 |
| Nginx + FPM · 4w | 19,472 | 0.9× | 13.0 ms | 15.5 ms | 492.5% | 44 MiB | 0 |
| Nginx + FPM · 8w | 22,735 | baseline | 11.2 ms | 13.3 ms | 552.0% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 135,148 | 8.1× | 1.7 ms | 4.3 ms | 270.0% | 32 MiB | 0 |
| Turbine NTS · 8w | 135,063 | 8.1× | 1.7 ms | 4.3 ms | 269.2% | 34 MiB | 0 |
| Turbine NTS · 4w · persistent | 136,646 | 8.2× | 1.6 ms | 4.2 ms | 269.5% | 31 MiB | 0 |
| Turbine NTS · 8w · persistent | 133,947 | 8.0× | 1.7 ms | 4.4 ms | 267.1% | 34 MiB | 0 |
| Turbine ZTS · 4w | 135,079 | 8.1× | 1.7 ms | 4.4 ms | 267.3% | 30 MiB | 0 |
| Turbine ZTS · 8w | 135,490 | 8.1× | 1.7 ms | 4.3 ms | 270.1% | 34 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 17,374 | 1.0× | 14.5 ms | 33.9 ms | 452.5% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 17,666 | 1.1× | 14.3 ms | 33.0 ms | 458.0% | 69 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 17,338 | 1.0× | 14.5 ms | 34.2 ms | 453.2% | 56 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 17,569 | 1.1× | 14.4 ms | 33.5 ms | 457.7% | 62 MiB | 0 |
| Nginx + FPM · 4w | 14,481 | 0.9× | 17.6 ms | 20.7 ms | 375.1% | 33 MiB | 0 |
| Nginx + FPM · 8w | 16,647 | baseline | 15.3 ms | 17.9 ms | 411.6% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 135,696 | 8.1× | 1.7 ms | 4.2 ms | 268.4% | 33 MiB | 0 |
| Turbine NTS · 8w | 135,331 | 8.1× | 1.7 ms | 4.2 ms | 268.3% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 136,747 | 8.2× | 1.6 ms | 4.2 ms | 270.4% | 33 MiB | 0 |
| Turbine NTS · 8w · persistent | 134,326 | 8.0× | 1.7 ms | 4.3 ms | 266.8% | 38 MiB | 0 |
| Turbine ZTS · 4w | 135,405 | 8.1× | 1.7 ms | 4.3 ms | 268.2% | 32 MiB | 0 |
| Turbine ZTS · 8w | 135,566 | 8.1× | 1.7 ms | 4.3 ms | 268.1% | 36 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 17,185 | 1.0× | 14.7 ms | 34.6 ms | 450.0% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 17,391 | 1.0× | 14.5 ms | 33.9 ms | 455.3% | 71 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 17,330 | 1.0× | 14.6 ms | 34.5 ms | 452.6% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 17,475 | 1.0× | 14.4 ms | 33.8 ms | 455.3% | 63 MiB | 0 |
| Nginx + FPM · 4w | 14,368 | 0.9× | 17.7 ms | 21.1 ms | 374.2% | 33 MiB | 0 |
| Nginx + FPM · 8w | 16,750 | baseline | 15.2 ms | 17.9 ms | 415.7% | 37 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 136,197 | 9.5× | 1.6 ms | 4.3 ms | 269.2% | 34 MiB | 0 |
| Turbine NTS · 8w | 135,464 | 9.5× | 1.7 ms | 4.2 ms | 268.3% | 38 MiB | 0 |
| Turbine NTS · 4w · persistent | 136,233 | 9.5× | 1.6 ms | 4.3 ms | 267.6% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 135,090 | 9.5× | 1.7 ms | 4.3 ms | 268.7% | 38 MiB | 0 |
| Turbine ZTS · 4w | 135,688 | 9.5× | 1.7 ms | 4.2 ms | 268.0% | 32 MiB | 0 |
| Turbine ZTS · 8w | 135,534 | 9.5× | 1.7 ms | 4.3 ms | 267.7% | 38 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 14,221 | 1.0× | 17.7 ms | 40.2 ms | 473.1% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 14,420 | 1.0× | 17.5 ms | 39.5 ms | 480.1% | 73 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 14,323 | 1.0× | 17.6 ms | 40.9 ms | 476.1% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 14,121 | 1.0× | 17.8 ms | 41.1 ms | 471.2% | 62 MiB | 0 |
| Nginx + FPM · 4w | 12,381 | 0.9× | 20.6 ms | 23.8 ms | 413.5% | 34 MiB | 0 |
| Nginx + FPM · 8w | 14,266 | baseline | 17.8 ms | 20.5 ms | 463.1% | 38 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
