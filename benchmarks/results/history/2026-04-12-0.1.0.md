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
| Turbine NTS · 4w | 125,568 | 4.1× | 1.8 ms | 5.8 ms | 197.6% | 30 MiB | 0 |
| Turbine NTS · 8w | 125,333 | 4.1× | 1.8 ms | 5.8 ms | 197.6% | 32 MiB | 0 |
| Turbine NTS · 4w · persistent | 125,357 | 4.1× | 1.8 ms | 5.9 ms | 197.1% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 124,860 | 4.1× | 1.8 ms | 6.3 ms | 197.2% | 32 MiB | 0 |
| Turbine ZTS · 4w | 125,915 | 4.1× | 1.8 ms | 6.0 ms | 197.4% | 30 MiB | 0 |
| Turbine ZTS · 8w | 125,867 | 4.1× | 1.8 ms | 5.8 ms | 198.0% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 30,238 | 1.0× | 8.0 ms | 26.3 ms | 477.3% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 28,493 | 0.9× | 8.5 ms | 28.8 ms | 477.0% | 54 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 30,099 | 1.0× | 8.0 ms | 27.6 ms | 477.0% | 56 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 29,980 | 1.0× | 8.1 ms | 25.2 ms | 453.8% | 56 MiB | 0 |
| Nginx + FPM · 4w | 30,933 | 1.0× | 8.2 ms | 10.4 ms | 419.7% | 33 MiB | 0 |
| Nginx + FPM · 8w | 30,702 | baseline | 8.2 ms | 10.8 ms | 439.7% | 37 MiB | 0 |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 114,288 | 73.0× | 2.0 ms | 6.5 ms | 203.1% | 68 MiB | 0 |
| Turbine NTS · 8w | 114,238 | 73.0× | 2.0 ms | 6.4 ms | 202.3% | 77 MiB | 0 |
| Turbine NTS · 4w · persistent | 116,003 | 74.1× | 1.9 ms | 6.3 ms | 200.7% | 89 MiB | 0 |
| Turbine NTS · 8w · persistent | 114,732 | 73.3× | 2.0 ms | 6.5 ms | 202.7% | 114 MiB | 0 |
| Turbine ZTS · 4w | 114,562 | 73.2× | 2.0 ms | 6.4 ms | 201.4% | 68 MiB | 0 |
| Turbine ZTS · 8w | 113,716 | 72.7× | 2.0 ms | 6.5 ms | 203.6% | 72 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 1,454 | 0.9× | 175.1 ms | 190.2 ms | 771.4% | 90 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 1,452 | 0.9× | 175.6 ms | 190.8 ms | 771.8% | 89 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 1,454 | 0.9× | 175.1 ms | 191.1 ms | 772.1% | 89 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 1,450 | 0.9× | 175.4 ms | 190.6 ms | 772.8% | 90 MiB | 0 |
| Nginx + FPM · 4w | 1,098 | 0.7× | 233.2 ms | 254.1 ms | 415.2% | 52 MiB | 0 |
| Nginx + FPM · 8w | 1,565 | baseline | 162.8 ms | 174.5 ms | 760.3% | 62 MiB | 0 |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 114,536 | 5.8× | 2.0 ms | 6.5 ms | 202.7% | 35 MiB | 0 |
| Turbine NTS · 8w | 112,277 | 5.6× | 2.0 ms | 6.6 ms | 199.8% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 114,231 | 5.7× | 2.0 ms | 6.4 ms | 201.2% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 111,994 | 5.6× | 2.0 ms | 6.6 ms | 202.5% | 38 MiB | 0 |
| Turbine ZTS · 4w | 109,750 | 5.5× | 2.1 ms | 6.8 ms | 204.8% | 34 MiB | 0 |
| Turbine ZTS · 8w | 112,082 | 5.6× | 2.0 ms | 6.6 ms | 202.0% | 39 MiB | 0 |
| Nginx + FPM · 4w | 17,969 | 0.9× | 14.1 ms | 17.7 ms | 462.8% | 32 MiB | 0 |
| Nginx + FPM · 8w | 19,908 | baseline | 12.7 ms | 15.6 ms | 503.2% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 106,873 | 7.5× | 2.1 ms | 6.8 ms | 237.3% | 31 MiB | 0 |
| Turbine NTS · 8w | 105,341 | 7.4× | 2.1 ms | 6.9 ms | 236.8% | 34 MiB | 0 |
| Turbine NTS · 4w · persistent | 105,183 | 7.4× | 2.2 ms | 6.9 ms | 237.8% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 105,538 | 7.4× | 2.1 ms | 7.0 ms | 239.3% | 34 MiB | 0 |
| Turbine ZTS · 4w | 105,277 | 7.4× | 2.2 ms | 7.0 ms | 239.2% | 30 MiB | 0 |
| Turbine ZTS · 8w | 105,427 | 7.4× | 2.1 ms | 6.9 ms | 237.3% | 33 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 14,118 | 1.0× | 17.6 ms | 45.9 ms | 408.4% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 14,124 | 1.0× | 17.6 ms | 45.2 ms | 407.7% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 14,053 | 1.0× | 17.6 ms | 45.4 ms | 407.0% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 14,122 | 1.0× | 17.6 ms | 45.6 ms | 410.6% | 62 MiB | 0 |
| Nginx + FPM · 4w | 12,440 | 0.9× | 20.4 ms | 24.6 ms | 345.7% | 33 MiB | 0 |
| Nginx + FPM · 8w | 14,230 | baseline | 17.9 ms | 21.9 ms | 388.8% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 108,628 | 7.7× | 2.1 ms | 6.7 ms | 236.3% | 34 MiB | 0 |
| Turbine NTS · 8w | 105,453 | 7.5× | 2.1 ms | 6.8 ms | 236.6% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 105,967 | 7.5× | 2.1 ms | 6.8 ms | 236.0% | 33 MiB | 0 |
| Turbine NTS · 8w · persistent | 105,928 | 7.5× | 2.1 ms | 6.8 ms | 236.4% | 36 MiB | 0 |
| Turbine ZTS · 4w | 105,082 | 7.5× | 2.2 ms | 6.8 ms | 236.6% | 34 MiB | 0 |
| Turbine ZTS · 8w | 106,256 | 7.5× | 2.1 ms | 6.8 ms | 236.5% | 35 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 14,120 | 1.0× | 17.6 ms | 45.0 ms | 408.1% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 14,086 | 1.0× | 17.6 ms | 45.5 ms | 408.9% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 14,031 | 1.0× | 17.7 ms | 44.9 ms | 403.5% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 14,040 | 1.0× | 17.7 ms | 44.5 ms | 408.0% | 63 MiB | 0 |
| Nginx + FPM · 4w | 12,544 | 0.9× | 20.2 ms | 25.0 ms | 349.4% | 33 MiB | 0 |
| Nginx + FPM · 8w | 14,086 | baseline | 18.1 ms | 22.2 ms | 385.8% | 38 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 106,110 | 8.8× | 2.1 ms | 6.8 ms | 237.5% | 36 MiB | 0 |
| Turbine NTS · 8w | 104,765 | 8.7× | 2.2 ms | 7.0 ms | 233.7% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 105,732 | 8.8× | 2.1 ms | 6.8 ms | 237.1% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 106,402 | 8.8× | 2.1 ms | 6.8 ms | 235.9% | 37 MiB | 0 |
| Turbine ZTS · 4w | 105,209 | 8.7× | 2.2 ms | 6.8 ms | 237.3% | 33 MiB | 0 |
| Turbine ZTS · 8w | 105,771 | 8.8× | 2.1 ms | 6.8 ms | 235.6% | 36 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 11,709 | 1.0× | 21.1 ms | 52.9 ms | 429.7% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 11,776 | 1.0× | 21.0 ms | 52.4 ms | 433.7% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 11,741 | 1.0× | 21.0 ms | 52.6 ms | 432.4% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 11,747 | 1.0× | 21.0 ms | 52.2 ms | 434.0% | 64 MiB | 0 |
| Nginx + FPM · 4w | 10,709 | 0.9× | 23.7 ms | 28.4 ms | 385.3% | 33 MiB | 0 |
| Nginx + FPM · 8w | 12,069 | baseline | 21.1 ms | 25.3 ms | 426.3% | 39 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
