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

---

## Raw PHP

_Single PHP file returning plain-text Hello World_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 93,828 | 3.7× | 2.4 ms | 8.1 ms | 209.0% | 30 MiB | 0 |
| Turbine NTS · 8w | 92,252 | 3.7× | 2.5 ms | 8.2 ms | 211.4% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 89,523 | 3.6× | 2.5 ms | 8.3 ms | 211.1% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 92,545 | 3.7× | 2.5 ms | 8.1 ms | 210.4% | 32 MiB | 0 |
| Turbine ZTS · 4w | 93,547 | 3.7× | 2.4 ms | 8.0 ms | 208.0% | 31 MiB | 0 |
| Turbine ZTS · 8w | 94,799 | 3.8× | 2.4 ms | 7.9 ms | 209.9% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 23,794 | 0.9× | 10.3 ms | 30.0 ms | 450.5% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 21,533 | 0.9× | 11.2 ms | 34.9 ms | 473.3% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 21,359 | 0.8× | 11.3 ms | 35.0 ms | 474.9% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 21,018 | 0.8× | 11.5 ms | 35.4 ms | 472.6% | 60 MiB | 0 |
| Nginx + FPM · 4w | 22,775 | 0.9× | 11.1 ms | 13.8 ms | 402.4% | 32 MiB | 0 |
| Nginx + FPM · 8w | 25,208 | baseline | 10.1 ms | 12.7 ms | 427.8% | 36 MiB | 0 |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 95,939 | — | 2.4 ms | 8.6 ms | 210.8% | 76 MiB | 0 |
| Turbine NTS · 8w | 95,458 | — | 2.4 ms | 7.9 ms | 211.0% | 85 MiB | 0 |
| Turbine NTS · 4w · persistent | 96,132 | — | 2.4 ms | 8.0 ms | 208.8% | 95 MiB | 0 |
| Turbine NTS · 8w · persistent | 94,508 | — | 2.4 ms | 8.1 ms | 208.3% | 120 MiB | 0 |
| Turbine ZTS · 4w | 90,532 | — | 2.4 ms | 620.2 ms | 212.5% | 71 MiB | 54 |
| Turbine ZTS · 8w | 93,955 | — | 2.4 ms | 7.9 ms | 210.5% | 79 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 278 | — | 884.0 ms | 1550.1 ms | 218.6% | 96 MiB | 10 |
| FrankenPHP (ZTS) · 8w | 288 | — | 855.3 ms | 1453.4 ms | 230.9% | 95 MiB | 21 |
| FrankenPHP (ZTS) · 4w · worker | 286 | — | 860.7 ms | 1423.6 ms | 217.3% | 96 MiB | 13 |
| FrankenPHP (ZTS) · 8w · worker | 291 | — | 842.9 ms | 1405.1 ms | 219.8% | 96 MiB | 17 |
| Nginx + FPM · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Nginx + FPM · 8w | 0 | baseline | 0.0 ms | 0.0 ms | — | — | 0 |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 97,077 | 5.3× | 2.3 ms | 7.8 ms | 209.5% | 35 MiB | 0 |
| Turbine NTS · 8w | 94,370 | 5.1× | 2.4 ms | 8.1 ms | 209.2% | 38 MiB | 0 |
| Turbine NTS · 4w · persistent | 97,545 | 5.3× | 2.3 ms | 7.9 ms | 210.5% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 94,464 | 5.1× | 2.4 ms | 7.9 ms | 210.0% | 39 MiB | 0 |
| Turbine ZTS · 4w | 97,833 | 5.3× | 2.3 ms | 7.8 ms | 209.2% | 34 MiB | 0 |
| Turbine ZTS · 8w | 94,511 | 5.1× | 2.4 ms | 8.1 ms | 210.3% | 39 MiB | 0 |
| Nginx + FPM · 4w | 16,284 | 0.9× | 15.6 ms | 19.1 ms | 454.8% | 32 MiB | 0 |
| Nginx + FPM · 8w | 18,367 | baseline | 13.8 ms | 16.8 ms | 488.2% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: Hello World, 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### Hello World

_Minimal `echo 'Hello World!'` response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 85,089 | 3.0× | 2.7 ms | 8.6 ms | 241.8% | 30 MiB | 0 |
| Turbine NTS · 8w | 86,896 | 3.1× | 2.6 ms | 8.6 ms | 240.1% | 32 MiB | 0 |
| Turbine NTS · 4w · persistent | 86,442 | 3.1× | 2.6 ms | 8.7 ms | 242.1% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 88,927 | 3.2× | 2.6 ms | 8.4 ms | 242.6% | 32 MiB | 0 |
| Turbine ZTS · 4w | 88,033 | 3.1× | 2.6 ms | 8.5 ms | 243.4% | 30 MiB | 0 |
| Turbine ZTS · 8w | 90,845 | 3.2× | 2.5 ms | 8.2 ms | 239.4% | 33 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 27,131 | 1.0× | 9.2 ms | 22.3 ms | 419.9% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 27,732 | 1.0× | 9.0 ms | 21.5 ms | 420.7% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 27,429 | 1.0× | 9.1 ms | 21.7 ms | 420.7% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 27,887 | 1.0× | 8.9 ms | 21.5 ms | 417.6% | 65 MiB | 0 |
| Nginx + FPM · 4w | 24,480 | 0.9× | 10.4 ms | 12.8 ms | 388.2% | 33 MiB | 0 |
| Nginx + FPM · 8w | 28,219 | baseline | 9.0 ms | 11.2 ms | 417.5% | 37 MiB | 0 |

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 84,510 | 6.7× | 2.7 ms | 8.7 ms | 241.9% | 34 MiB | 0 |
| Turbine NTS · 8w | 87,363 | 6.9× | 2.6 ms | 8.3 ms | 240.2% | 35 MiB | 0 |
| Turbine NTS · 4w · persistent | 85,772 | 6.8× | 2.7 ms | 8.7 ms | 241.8% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 91,191 | 7.2× | 2.5 ms | 8.3 ms | 241.1% | 37 MiB | 0 |
| Turbine ZTS · 4w | 88,395 | 7.0× | 2.6 ms | 8.3 ms | 241.0% | 32 MiB | 0 |
| Turbine ZTS · 8w | 89,928 | 7.1× | 2.5 ms | 8.1 ms | 239.2% | 35 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 27,457 | 2.2× | 9.1 ms | 21.7 ms | 419.2% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 27,942 | 2.2× | 8.9 ms | 21.2 ms | 420.7% | 66 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 27,455 | 2.2× | 9.1 ms | 22.1 ms | 420.2% | 59 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 27,954 | 2.2× | 8.9 ms | 21.7 ms | 418.5% | 65 MiB | 0 |
| Nginx + FPM · 4w | 10,693 | 0.8× | 23.7 ms | 28.0 ms | 347.4% | 33 MiB | 0 |
| Nginx + FPM · 8w | 12,633 | baseline | 20.1 ms | 24.6 ms | 386.7% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 84,868 | 6.7× | 2.7 ms | 9.7 ms | 241.1% | 35 MiB | 0 |
| Turbine NTS · 8w | 89,111 | 7.1× | 2.6 ms | 8.4 ms | 241.1% | 36 MiB | 0 |
| Turbine NTS · 4w · persistent | 87,945 | 7.0× | 2.6 ms | 8.5 ms | 242.1% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 88,908 | 7.0× | 2.6 ms | 8.4 ms | 240.6% | 38 MiB | 0 |
| Turbine ZTS · 4w | 85,358 | 6.8× | 2.7 ms | 8.6 ms | 241.0% | 35 MiB | 0 |
| Turbine ZTS · 8w | 90,234 | 7.2× | 2.5 ms | 8.2 ms | 240.9% | 37 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 27,799 | 2.2× | 8.9 ms | 21.8 ms | 418.7% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 27,686 | 2.2× | 9.0 ms | 21.8 ms | 419.6% | 66 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 27,789 | 2.2× | 9.0 ms | 21.9 ms | 420.0% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 27,933 | 2.2× | 8.9 ms | 21.4 ms | 419.7% | 65 MiB | 0 |
| Nginx + FPM · 4w | 10,617 | 0.8× | 23.9 ms | 29.0 ms | 341.8% | 33 MiB | 0 |
| Nginx + FPM · 8w | 12,613 | baseline | 20.1 ms | 24.4 ms | 384.2% | 37 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 86,799 | 7.9× | 2.6 ms | 8.6 ms | 240.2% | 37 MiB | 0 |
| Turbine NTS · 8w | 88,025 | 8.0× | 2.6 ms | 8.4 ms | 239.8% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 88,801 | 8.1× | 2.6 ms | 8.3 ms | 240.5% | 36 MiB | 0 |
| Turbine NTS · 8w · persistent | 89,700 | 8.2× | 2.5 ms | 8.2 ms | 238.3% | 40 MiB | 0 |
| Turbine ZTS · 4w | 85,797 | 7.8× | 2.7 ms | 8.6 ms | 241.4% | 34 MiB | 0 |
| Turbine ZTS · 8w | 90,778 | 8.3× | 2.5 ms | 8.2 ms | 241.4% | 39 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 27,834 | 2.5× | 8.9 ms | 21.6 ms | 420.1% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 27,752 | 2.5× | 9.0 ms | 21.3 ms | 419.0% | 65 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 27,329 | 2.5× | 9.1 ms | 21.9 ms | 420.2% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 27,772 | 2.5× | 9.0 ms | 21.5 ms | 419.9% | 66 MiB | 0 |
| Nginx + FPM · 4w | 9,525 | 0.9× | 26.6 ms | 31.6 ms | 377.9% | 34 MiB | 0 |
| Nginx + FPM · 8w | 10,996 | baseline | 23.1 ms | 27.9 ms | 419.4% | 38 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
