# Turbine Benchmark Results

| | |
|---|---|
| **Version** | v0.5.0 |
| **Date** | 2026-04-23 |
| **PHP** | 8.5 |
| **Server** | Hetzner CCX33 (8 vCPU dedicated / 32 GB RAM / NVMe) |
| **Tool** | [wrk](https://github.com/wg/wrk) |
| **Parameters** | 30s · 256 connections |
| **Workers** | 4w and 8w variants (Turbine + FPM) |
| **Memory limit** | 256 MB per worker |
| **Max req/worker** | 50,000 |
| **Turbine NTS image** | `katisuhara/turbine-php:latest-php8.5-nts` |
| **Turbine ZTS image** | `katisuhara/turbine-php:latest-php8.5-zts` |

> **Baseline**: Nginx + PHP-FPM · 8 workers.
> **NTS**: Non-thread-safe PHP — process mode (fork per worker).
> **ZTS**: Thread-safe PHP — thread mode (shared memory, lock-free dispatch).
> **Persistent**: PHP worker stays alive across requests (bootstrap once, handle many).
> **FrankenPHP** uses ZTS PHP internally and does **not** support Phalcon.
> All servers (including FPM) run inside Docker containers for equal overhead.
> CPU and memory metrics are collected via `docker stats --no-stream` during benchmark.

> **⚠️ Disclaimer:** These benchmarks are synthetic and may not reflect real-world performance. Results depend heavily on architecture, application design, dependencies, stack, and workload characteristics. The goal is **not** to declare any runtime better or worse — choosing the right tool depends on many factors beyond raw throughput.

> **Status column:** `✅` = preflight passed and all responses were 2xx. `⚠️` = the row returned non-2xx responses or failed preflight validation (tiny response body, wrong content-type, 404/5xx before the run). Req/s for flagged rows is **not comparable** to healthy rows — the server may be returning fast error pages.

---

## Raw PHP

_Single PHP file returning plain-text Hello World_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 26,869 | 0.9× | 9.5 ms | 10.9 ms | 365.1% | 45 MiB | 0 | ✅ |
| Turbine NTS · 8w | 34,597 | 1.2× | 7.4 ms | 8.6 ms | 406.1% | 50 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 27,261 | 0.9× | 9.3 ms | 10.7 ms | 358.2% | 40 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 35,169 | 1.2× | 7.2 ms | 8.6 ms | 390.6% | 43 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 39,485 | 1.3× | 6.4 ms | 7.8 ms | 442.0% | 54 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 44,578 | 1.5× | 5.7 ms | 7.7 ms | 475.1% | 56 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 22,420 | 0.7× | 11.4 ms | 12.8 ms | 401.6% | 42 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 29,365 | 1.0× | 8.7 ms | 10.2 ms | 448.9% | 47 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 24,794 | 0.8× | 9.9 ms | 27.5 ms | 512.4% | 56 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 25,121 | 0.8× | 9.8 ms | 26.5 ms | 510.7% | 64 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 27,458 | 0.9× | 9.1 ms | 22.9 ms | 490.7% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 25,076 | 0.8× | 9.8 ms | 27.2 ms | 516.3% | 54 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 24,672 | 0.8× | 10.3 ms | 12.1 ms | 426.4% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 29,995 | baseline | 8.5 ms | 10.0 ms | 473.6% | 42 MiB | 0 | ✅ |

## Laravel

_Laravel 13 — mixed JSON routes: GET /, GET /user/:id, POST /user (stateless, no database)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 653 | 0.2× | 390.0 ms | 398.5 ms | 404.6% | 83 MiB | 0 | ✅ |
| Turbine NTS · 8w | 972 | 0.3× | 261.9 ms | 273.0 ms | 779.7% | 91 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 2,589 | 0.9× | 98.9 ms | 103.0 ms | 258.3% | 171 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 5,690 | 2.0× | 44.8 ms | 49.6 ms | 617.8% | 277 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 621 | 0.2× | 406.6 ms | 448.5 ms | 406.5% | 87 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 962 | 0.3× | 265.1 ms | 277.6 ms | 780.0% | 96 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 3,595 | 1.2× | 70.9 ms | 77.7 ms | 361.4% | 198 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 6,286 | 2.2× | 40.4 ms | 46.9 ms | 703.0% | 295 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 2,943 | 1.0× | 86.4 ms | 101.1 ms | 751.3% | 86 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 2,942 | 1.0× | 86.5 ms | 102.5 ms | 753.3% | 88 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 2,944 | 1.0× | 86.3 ms | 101.5 ms | 750.2% | 85 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 2,965 | 1.0× | 85.8 ms | 100.9 ms | 752.9% | 85 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 1,902 | 0.7× | 134.2 ms | 145.3 ms | 423.3% | 67 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 2,893 | baseline | 88.1 ms | 94.8 ms | 735.6% | 75 MiB | 0 | ✅ |

## Symfony

_Symfony 7 — mixed JSON routes: GET /, GET /user/:id, POST /user (prod env, cached routes/config)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 717 | 0.2× | 355.2 ms | 373.7 ms | 408.0% | 69 MiB | 0 | ✅ |
| Turbine NTS · 8w | 1,114 | 0.3× | 228.8 ms | 237.9 ms | 780.3% | 72 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 18,885 | 4.3× | 13.5 ms | 15.6 ms | 525.1% | 63 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 17,801 | 4.0× | 14.3 ms | 16.3 ms | 529.6% | 67 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 679 | 0.2× | 375.2 ms | 407.8 ms | 405.9% | 68 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 1,072 | 0.2× | 237.6 ms | 247.4 ms | 780.7% | 77 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 14,949 | 3.4× | 17.1 ms | 19.0 ms | 512.8% | 64 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 16,809 | 3.8× | 15.2 ms | 17.5 ms | 552.0% | 70 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 4,625 | 1.0× | 54.9 ms | 71.9 ms | 737.0% | 71 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 4,639 | 1.0× | 54.7 ms | 72.3 ms | 737.2% | 74 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 4,671 | 1.1× | 54.3 ms | 71.7 ms | 736.4% | 68 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 4,661 | 1.1× | 54.5 ms | 72.2 ms | 736.8% | 68 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 2,889 | 0.7× | 87.9 ms | 95.7 ms | 430.5% | 44 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 4,423 | baseline | 57.6 ms | 63.4 ms | 712.9% | 50 MiB | 0 | ✅ |

## Phalcon

_Phalcon Micro — mixed JSON routes: GET /, GET /user/:id, POST /user_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 15,679 | 0.8× | 16.2 ms | 18.4 ms | 457.9% | 50 MiB | 0 | ✅ |
| Turbine NTS · 8w | 19,547 | 1.0× | 13.0 ms | 15.1 ms | 511.3% | 55 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 16,074 | 0.8× | 15.8 ms | 17.8 ms | 456.5% | 48 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 20,027 | 1.0× | 12.7 ms | 14.7 ms | 510.6% | 51 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 20,468 | 1.1× | 12.4 ms | 14.3 ms | 535.0% | 52 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 23,422 | 1.2× | 10.8 ms | 13.2 ms | 589.5% | 57 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 17,295 | 0.9× | 14.7 ms | 16.5 ms | 517.8% | 48 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 17,445 | 0.9× | 14.6 ms | 16.9 ms | 530.6% | 53 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 16,374 | 0.8× | 15.6 ms | 17.6 ms | 487.4% | 32 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 19,291 | baseline | 13.2 ms | 15.3 ms | 544.0% | 38 MiB | 0 | ✅ |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 20,310 | 1.4× | 12.5 ms | 14.4 ms | 396.4% | 57 MiB | 0 | ✅ |
| Turbine NTS · 8w | 23,683 | 1.6× | 10.7 ms | 12.8 ms | 415.6% | 70 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 8 | 0.0× | 11.9 ms | 12.0 ms | 0.5% | 70 MiB | 256 | ✅ |
| Turbine NTS · 8w · persistent | 13,660 | 0.9× | 18.6 ms | 21.6 ms | 538.1% | 80 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 30,070 | 2.0× | 8.4 ms | 10.1 ms | 446.6% | 72 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 33,433 | 2.3× | 7.5 ms | 10.3 ms | 485.6% | 81 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 12,322 | 0.8× | 20.6 ms | 23.9 ms | 541.7% | 64 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 12,901 | 0.9× | 19.7 ms | 22.8 ms | 564.0% | 74 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 15,449 | 1.0× | 16.3 ms | 38.0 ms | 469.8% | 66 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 15,365 | 1.0× | 16.3 ms | 39.1 ms | 468.5% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 15,460 | 1.0× | 16.2 ms | 38.8 ms | 467.9% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 15,559 | 1.0× | 16.2 ms | 37.8 ms | 469.4% | 64 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 12,401 | 0.8× | 20.5 ms | 23.5 ms | 368.0% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 14,837 | baseline | 17.1 ms | 19.7 ms | 424.2% | 37 MiB | 0 | ✅ |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 20,442 | 1.4× | 12.4 ms | 14.3 ms | 400.5% | 57 MiB | 0 | ✅ |
| Turbine NTS · 8w | 23,764 | 1.6× | 10.7 ms | 12.6 ms | 418.0% | 71 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 8 ⚠️ | 0.0× | 0.0 ms | 0.0 ms | 0.7% | 71 MiB | 256 | ⚠️ preflight failed |
| Turbine NTS · 8w · persistent | 6,109 | 0.4× | 42.0 ms | 44.6 ms | 277.1% | 81 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 29,884 | 2.0× | 8.5 ms | 10.2 ms | 440.8% | 76 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 34,062 | 2.3× | 7.4 ms | 10.3 ms | 487.6% | 84 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 8 | 0.0× | 2.1 ms | 2.1 ms | 0.5% | 69 MiB | 256 | ✅ |
| Turbine ZTS · 8w · persistent | 6 | 0.0× | 2.9 ms | 2.9 ms | 0.5% | 76 MiB | 192 | ✅ |
| FrankenPHP (ZTS) · 4w | 15,518 | 1.0× | 16.3 ms | 37.7 ms | 468.4% | 67 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 15,338 | 1.0× | 16.4 ms | 38.6 ms | 467.1% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 15,457 | 1.0× | 16.2 ms | 38.6 ms | 465.0% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 15,521 | 1.0× | 16.2 ms | 38.4 ms | 467.0% | 65 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 12,497 | 0.8× | 20.4 ms | 23.1 ms | 372.5% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 14,788 | baseline | 17.2 ms | 19.9 ms | 423.6% | 37 MiB | 0 | ✅ |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 17,889 | 1.4× | 14.2 ms | 16.5 ms | 479.2% | 58 MiB | 0 | ✅ |
| Turbine NTS · 8w | 17,228 | 1.4× | 14.8 ms | 17.0 ms | 481.8% | 70 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 6 ⚠️ | 0.0× | 0.0 ms | 0.0 ms | 0.7% | 71 MiB | 192 | ⚠️ preflight failed |
| Turbine NTS · 8w · persistent | 4 ⚠️ | 0.0× | 0.0 ms | 0.0 ms | 1.1% | 86 MiB | 128 | ⚠️ preflight failed |
| Turbine ZTS · 4w | 20,747 | 1.7× | 12.2 ms | 14.2 ms | 495.3% | 79 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 23,016 | 1.9× | 11.0 ms | 13.7 ms | 544.2% | 86 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 8 ⚠️ | 0.0× | 0.0 ms | 0.0 ms | 0.7% | 69 MiB | 256 | ⚠️ preflight failed |
| Turbine ZTS · 8w · persistent | 6 ⚠️ | 0.0× | 0.0 ms | 0.0 ms | 0.7% | 75 MiB | 192 | ⚠️ preflight failed |
| FrankenPHP (ZTS) · 4w | 12,581 | 1.0× | 20.0 ms | 44.3 ms | 493.3% | 67 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 12,564 | 1.0× | 20.0 ms | 45.4 ms | 496.1% | 59 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 12,424 | 1.0× | 20.2 ms | 45.8 ms | 493.8% | 61 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 12,665 | 1.0× | 19.8 ms | 44.5 ms | 498.1% | 64 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 10,612 | 0.9× | 24.0 ms | 27.4 ms | 409.0% | 34 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 12,359 | baseline | 20.6 ms | 23.3 ms | 466.0% | 38 MiB | 0 | ✅ |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
