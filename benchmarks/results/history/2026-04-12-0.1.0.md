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
| Turbine NTS · 4w | 109,982 | 4.0× | 2.0 ms | 6.7 ms | 202.5% | 30 MiB | 0 |
| Turbine NTS · 8w | 104,607 | 3.8× | 2.2 ms | 7.0 ms | 204.9% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 109,096 | 4.0× | 2.1 ms | 6.8 ms | 205.7% | 29 MiB | 0 |
| Turbine NTS · 8w · persistent | 102,018 | 3.7× | 2.2 ms | 7.2 ms | 205.4% | 32 MiB | 0 |
| Turbine ZTS · 4w | 104,106 | 3.8× | 2.2 ms | 7.9 ms | 203.0% | 29 MiB | 0 |
| Turbine ZTS · 8w | 105,064 | 3.8× | 2.1 ms | 7.1 ms | 204.2% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 24,075 | 0.9× | 10.1 ms | 33.2 ms | 473.6% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 26,918 | 1.0× | 9.1 ms | 27.4 ms | 453.2% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 27,007 | 1.0× | 9.1 ms | 26.8 ms | 453.3% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 23,848 | 0.9× | 10.2 ms | 33.3 ms | 477.4% | 61 MiB | 0 |
| Nginx + FPM · 4w | 24,765 | 0.9× | 10.2 ms | 13.1 ms | 410.6% | 33 MiB | 0 |
| Nginx + FPM · 8w | 27,340 | baseline | 9.3 ms | 11.8 ms | 435.6% | 37 MiB | 0 |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 102,802 | — | 2.2 ms | 546.2 ms | 206.1% | 75 MiB | 1 |
| Turbine NTS · 8w | 113,740 | — | 2.0 ms | 6.5 ms | 202.4% | 84 MiB | 0 |
| Turbine NTS · 4w · persistent | 106,911 | — | 2.1 ms | 6.8 ms | 204.9% | 95 MiB | 0 |
| Turbine NTS · 8w · persistent | 114,931 | — | 2.0 ms | 6.4 ms | 201.6% | 121 MiB | 0 |
| Turbine ZTS · 4w | 102,859 | — | 2.1 ms | 585.4 ms | 205.2% | 72 MiB | 34 |
| Turbine ZTS · 8w | 113,620 | — | 2.0 ms | 6.5 ms | 201.7% | 78 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 280 | — | 880.7 ms | 1473.1 ms | 215.6% | 96 MiB | 20 |
| FrankenPHP (ZTS) · 8w | 312 | — | 788.5 ms | 1355.1 ms | 213.3% | 94 MiB | 14 |
| FrankenPHP (ZTS) · 4w · worker | 303 | — | 799.4 ms | 1432.8 ms | 204.3% | 94 MiB | 16 |
| FrankenPHP (ZTS) · 8w · worker | 309 | — | 786.9 ms | 1389.3 ms | 208.3% | 94 MiB | 16 |
| Nginx + FPM · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Nginx + FPM · 8w | 0 | baseline | 0.0 ms | 0.0 ms | — | — | 0 |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 111,300 | 5.5× | 2.0 ms | 6.6 ms | 201.7% | 35 MiB | 0 |
| Turbine NTS · 8w | 112,325 | 5.6× | 2.0 ms | 6.5 ms | 200.8% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 113,261 | 5.6× | 2.0 ms | 6.5 ms | 199.4% | 36 MiB | 0 |
| Turbine NTS · 8w · persistent | 113,315 | 5.6× | 2.0 ms | 6.6 ms | 201.4% | 39 MiB | 0 |
| Turbine ZTS · 4w | 111,936 | 5.6× | 2.0 ms | 6.5 ms | 202.5% | 35 MiB | 0 |
| Turbine ZTS · 8w | 113,463 | 5.6× | 2.0 ms | 6.5 ms | 201.7% | 38 MiB | 0 |
| Nginx + FPM · 4w | 17,685 | 0.9× | 14.3 ms | 17.6 ms | 463.4% | 32 MiB | 0 |
| Nginx + FPM · 8w | 20,098 | baseline | 12.6 ms | 15.8 ms | 500.3% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 95,251 | 7.1× | 2.4 ms | 7.6 ms | 239.4% | 30 MiB | 0 |
| Turbine NTS · 8w | 97,510 | 7.2× | 2.3 ms | 7.9 ms | 240.4% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 96,604 | 7.2× | 2.4 ms | 7.7 ms | 239.9% | 31 MiB | 0 |
| Turbine NTS · 8w · persistent | 97,379 | 7.2× | 2.3 ms | 7.5 ms | 240.4% | 33 MiB | 0 |
| Turbine ZTS · 4w | 96,919 | 7.2× | 2.4 ms | 7.4 ms | 240.3% | 30 MiB | 0 |
| Turbine ZTS · 8w | 97,823 | 7.2× | 2.3 ms | 7.4 ms | 240.3% | 33 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 13,264 | 1.0× | 18.7 ms | 48.7 ms | 413.7% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 13,333 | 1.0× | 18.6 ms | 47.8 ms | 411.7% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 13,205 | 1.0× | 18.8 ms | 47.7 ms | 408.7% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 13,154 | 1.0× | 18.8 ms | 48.4 ms | 407.7% | 57 MiB | 0 |
| Nginx + FPM · 4w | 11,723 | 0.9× | 21.6 ms | 26.8 ms | 348.5% | 34 MiB | 0 |
| Nginx + FPM · 8w | 13,502 | baseline | 18.8 ms | 23.3 ms | 389.0% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 95,677 | 7.2× | 2.4 ms | 7.6 ms | 240.0% | 32 MiB | 0 |
| Turbine NTS · 8w | 97,156 | 7.3× | 2.4 ms | 7.5 ms | 238.4% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 96,667 | 7.3× | 2.4 ms | 7.5 ms | 239.9% | 33 MiB | 0 |
| Turbine NTS · 8w · persistent | 97,474 | 7.3× | 2.3 ms | 7.4 ms | 238.2% | 36 MiB | 0 |
| Turbine ZTS · 4w | 96,673 | 7.3× | 2.4 ms | 7.4 ms | 238.7% | 32 MiB | 0 |
| Turbine ZTS · 8w | 98,212 | 7.4× | 2.3 ms | 7.3 ms | 238.0% | 35 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 13,059 | 1.0× | 19.0 ms | 47.6 ms | 406.8% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 13,281 | 1.0× | 18.7 ms | 46.6 ms | 412.0% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 13,120 | 1.0× | 18.9 ms | 47.9 ms | 411.4% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 13,267 | 1.0× | 18.7 ms | 47.5 ms | 411.3% | 59 MiB | 0 |
| Nginx + FPM · 4w | 11,839 | 0.9× | 21.4 ms | 26.3 ms | 348.1% | 33 MiB | 0 |
| Nginx + FPM · 8w | 13,293 | baseline | 19.1 ms | 23.6 ms | 389.1% | 38 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 96,173 | 8.4× | 2.4 ms | 7.5 ms | 238.9% | 33 MiB | 0 |
| Turbine NTS · 8w | 96,856 | 8.4× | 2.4 ms | 7.4 ms | 238.3% | 38 MiB | 0 |
| Turbine NTS · 4w · persistent | 97,216 | 8.5× | 2.3 ms | 7.5 ms | 238.7% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 95,928 | 8.4× | 2.4 ms | 7.3 ms | 239.2% | 37 MiB | 0 |
| Turbine ZTS · 4w | 96,678 | 8.4× | 2.4 ms | 7.4 ms | 238.4% | 33 MiB | 0 |
| Turbine ZTS · 8w | 100,003 | 8.7× | 2.3 ms | 7.2 ms | 238.1% | 37 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 11,136 | 1.0× | 22.0 ms | 56.2 ms | 437.4% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 11,038 | 1.0× | 22.3 ms | 55.6 ms | 433.7% | 65 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 11,026 | 1.0× | 22.2 ms | 55.6 ms | 433.3% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 11,160 | 1.0× | 22.0 ms | 56.2 ms | 438.8% | 60 MiB | 0 |
| Nginx + FPM · 4w | 10,055 | 0.9× | 25.3 ms | 30.4 ms | 381.6% | 34 MiB | 0 |
| Nginx + FPM · 8w | 11,464 | baseline | 22.1 ms | 26.9 ms | 426.8% | 38 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
