# Turbine Benchmark Results

| | |
|---|---|
| **Version** | 0.1.0 |
| **Date** | 2026-04-12 |
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
| Turbine NTS · 4w | 99,670 | 3.7× | 2.3 ms | 7.6 ms | 209.8% | 31 MiB | 0 |
| Turbine NTS · 8w | 99,114 | 3.6× | 2.3 ms | 7.7 ms | 212.2% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 99,333 | 3.6× | 2.3 ms | 7.8 ms | 208.5% | 32 MiB | 0 |
| Turbine NTS · 8w · persistent | 99,419 | 3.6× | 2.3 ms | 7.7 ms | 212.3% | 32 MiB | 0 |
| Turbine ZTS · 4w | 99,023 | 3.6× | 2.3 ms | 7.7 ms | 208.3% | 30 MiB | 0 |
| Turbine ZTS · 8w | 99,678 | 3.7× | 2.3 ms | 7.5 ms | 209.6% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 22,656 | 0.8× | 10.7 ms | 34.4 ms | 477.7% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 22,532 | 0.8× | 10.8 ms | 33.6 ms | 475.4% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 22,662 | 0.8× | 10.7 ms | 34.5 ms | 476.8% | 55 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 25,668 | 0.9× | 9.5 ms | 28.1 ms | 453.8% | 54 MiB | 0 |
| Nginx + FPM · 4w | 24,459 | 0.9× | 10.4 ms | 13.0 ms | 403.9% | 32 MiB | 0 |
| Nginx + FPM · 8w | 27,241 | baseline | 9.3 ms | 11.7 ms | 430.6% | 36 MiB | 0 |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Turbine NTS · 8w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Turbine NTS · 4w · persistent | 100,396 | — | 2.3 ms | 7.5 ms | 209.1% | 87 MiB | 0 |
| Turbine NTS · 8w · persistent | 101,081 | — | 2.2 ms | 7.5 ms | 206.2% | 112 MiB | 0 |
| Turbine ZTS · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Turbine ZTS · 8w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| FrankenPHP (ZTS) · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| FrankenPHP (ZTS) · 8w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| FrankenPHP (ZTS) · 4w · worker | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| FrankenPHP (ZTS) · 8w · worker | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Nginx + FPM · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Nginx + FPM · 8w | 0 | baseline | 0.0 ms | 0.0 ms | — | — | 0 |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 103,209 | 5.2× | 2.2 ms | 7.4 ms | 208.6% | 36 MiB | 0 |
| Turbine NTS · 8w | 103,933 | 5.3× | 2.2 ms | 7.3 ms | 208.3% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 103,587 | 5.2× | 2.2 ms | 7.3 ms | 207.9% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 103,589 | 5.2× | 2.2 ms | 7.5 ms | 211.0% | 39 MiB | 0 |
| Turbine ZTS · 4w | 103,685 | 5.3× | 2.2 ms | 7.3 ms | 207.2% | 35 MiB | 0 |
| Turbine ZTS · 8w | 103,858 | 5.3× | 2.2 ms | 7.5 ms | 209.7% | 38 MiB | 0 |
| Nginx + FPM · 4w | 17,729 | 0.9× | 14.3 ms | 17.6 ms | 454.7% | 34 MiB | 0 |
| Nginx + FPM · 8w | 19,745 | baseline | 12.9 ms | 15.7 ms | 492.6% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 92,033 | 7.1× | 2.5 ms | 8.0 ms | 243.7% | 31 MiB | 0 |
| Turbine NTS · 8w | 93,483 | 7.2× | 2.4 ms | 8.1 ms | 242.6% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 91,009 | 7.1× | 2.5 ms | 8.2 ms | 245.0% | 31 MiB | 0 |
| Turbine NTS · 8w · persistent | 91,702 | 7.1× | 2.5 ms | 8.2 ms | 244.4% | 34 MiB | 0 |
| Turbine ZTS · 4w | 92,300 | 7.2× | 2.5 ms | 8.0 ms | 243.3% | 31 MiB | 0 |
| Turbine ZTS · 8w | 91,371 | 7.1× | 2.5 ms | 8.1 ms | 240.4% | 33 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 12,750 | 1.0× | 19.5 ms | 49.3 ms | 406.7% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 12,970 | 1.0× | 19.1 ms | 48.2 ms | 409.1% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 12,995 | 1.0× | 19.0 ms | 48.4 ms | 403.5% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 12,942 | 1.0× | 19.1 ms | 48.5 ms | 405.4% | 57 MiB | 0 |
| Nginx + FPM · 4w | 11,390 | 0.9× | 22.3 ms | 27.6 ms | 343.6% | 34 MiB | 0 |
| Nginx + FPM · 8w | 12,900 | baseline | 19.7 ms | 24.4 ms | 382.6% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 91,932 | 7.1× | 2.5 ms | 8.1 ms | 242.0% | 33 MiB | 0 |
| Turbine NTS · 8w | 92,951 | 7.1× | 2.5 ms | 7.9 ms | 242.0% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 91,987 | 7.1× | 2.5 ms | 8.0 ms | 241.8% | 32 MiB | 0 |
| Turbine NTS · 8w · persistent | 92,734 | 7.1× | 2.5 ms | 8.0 ms | 243.4% | 38 MiB | 0 |
| Turbine ZTS · 4w | 91,522 | 7.0× | 2.5 ms | 7.9 ms | 243.3% | 33 MiB | 0 |
| Turbine ZTS · 8w | 92,015 | 7.1× | 2.5 ms | 8.0 ms | 243.3% | 34 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 13,028 | 1.0× | 19.1 ms | 47.8 ms | 405.8% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 12,989 | 1.0× | 19.1 ms | 48.4 ms | 404.0% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 12,987 | 1.0× | 19.1 ms | 48.8 ms | 406.4% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 12,895 | 1.0× | 19.2 ms | 47.9 ms | 404.5% | 60 MiB | 0 |
| Nginx + FPM · 4w | 11,163 | 0.9× | 22.8 ms | 27.7 ms | 338.0% | 33 MiB | 0 |
| Nginx + FPM · 8w | 13,036 | baseline | 19.5 ms | 23.7 ms | 380.9% | 37 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 91,988 | 8.1× | 2.5 ms | 8.0 ms | 241.9% | 35 MiB | 0 |
| Turbine NTS · 8w | 90,611 | 8.0× | 2.5 ms | 9.0 ms | 244.6% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 92,340 | 8.1× | 2.5 ms | 8.0 ms | 239.6% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 92,904 | 8.2× | 2.5 ms | 8.0 ms | 244.1% | 39 MiB | 0 |
| Turbine ZTS · 4w | 90,944 | 8.0× | 2.5 ms | 8.1 ms | 241.2% | 34 MiB | 0 |
| Turbine ZTS · 8w | 92,504 | 8.2× | 2.5 ms | 8.0 ms | 242.4% | 37 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 11,049 | 1.0× | 22.2 ms | 54.6 ms | 428.2% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 10,813 | 1.0× | 22.8 ms | 56.2 ms | 428.3% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 10,945 | 1.0× | 22.4 ms | 55.8 ms | 424.2% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 10,692 | 0.9× | 22.8 ms | 58.2 ms | 430.1% | 61 MiB | 0 |
| Nginx + FPM · 4w | 10,215 | 0.9× | 24.8 ms | 30.5 ms | 378.5% | 33 MiB | 0 |
| Nginx + FPM · 8w | 11,333 | baseline | 22.4 ms | 27.0 ms | 417.1% | 38 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
