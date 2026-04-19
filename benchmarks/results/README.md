# Turbine Benchmark Results

| | |
|---|---|
| **Version** | v0.4.0 |
| **Date** | 2026-04-19 |
| **PHP** | 8.4 |
| **Server** | Hetzner CCX33 (8 vCPU dedicated / 32 GB RAM / NVMe) |
| **Tool** | [wrk](https://github.com/wg/wrk) |
| **Parameters** | 30s · 256 connections |
| **Workers** | 4w and 8w variants (Turbine + FPM) |
| **Memory limit** | 256 MB per worker |
| **Max req/worker** | 50,000 |
| **Turbine NTS image** | `katisuhara/turbine-php:latest-php8.4-nts` |
| **Turbine ZTS image** | `katisuhara/turbine-php:latest-php8.4-zts` |

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
| Turbine NTS · 4w | 23,063 | 0.8× | 11.0 ms | 13.6 ms | 300.3% | 58 MiB | 0 | ✅ |
| Turbine NTS · 8w | 28,735 | 1.1× | 8.8 ms | 11.8 ms | 350.0% | 57 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 25,582 | 0.9× | 10.0 ms | 11.7 ms | 290.6% | 48 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 32,007 | 1.2× | 7.9 ms | 10.5 ms | 335.7% | 55 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 32,461 | 1.2× | 7.8 ms | 11.2 ms | 398.5% | 62 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 36,160 | 1.3× | 6.9 ms | 10.9 ms | 436.0% | 77 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 21,915 | 0.8× | 11.6 ms | 13.8 ms | 350.7% | 59 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 25,788 | 1.0× | 9.8 ms | 12.3 ms | 391.4% | 69 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 22,642 | 0.8× | 10.8 ms | 32.8 ms | 473.6% | 63 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 22,555 | 0.8× | 10.8 ms | 33.8 ms | 472.6% | 56 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 21,849 | 0.8× | 11.1 ms | 34.5 ms | 473.1% | 62 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 22,297 | 0.8× | 10.9 ms | 34.4 ms | 471.6% | 56 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 23,703 | 0.9× | 10.7 ms | 13.3 ms | 397.7% | 32 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 27,135 | baseline | 9.3 ms | 11.6 ms | 426.3% | 36 MiB | 0 | ✅ |

## Laravel

_Laravel 13 — mixed JSON routes: GET /, GET /user/:id, POST /user (stateless, no database)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 151 | 0.1× | 1685.2 ms | 1708.8 ms | 403.0% | 93 MiB | 0 | ✅ |
| Turbine NTS · 8w | 748 | 0.3× | 341.9 ms | 357.2 ms | 783.7% | 102 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 4,710 | 1.7× | 54.2 ms | 57.8 ms | 484.0% | 107 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 5,586 | 2.0× | 45.5 ms | 52.5 ms | 661.7% | 284 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 139 | 0.0× | 1826.9 ms | 1874.2 ms | 402.7% | 103 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 730 | 0.3× | 350.6 ms | 370.1 ms | 783.4% | 116 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 4,372 | 1.6× | 58.4 ms | 61.7 ms | 481.0% | 117 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 4,976 | 1.8× | 51.1 ms | 57.9 ms | 634.6% | 280 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 1,436 | 0.5× | 177.3 ms | 193.8 ms | 766.8% | 91 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 2,858 | 1.0× | 89.0 ms | 103.8 ms | 747.1% | 85 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 1,473 | 0.5× | 173.0 ms | 189.7 ms | 766.3% | 90 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 2,879 | 1.0× | 88.7 ms | 103.7 ms | 747.0% | 86 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 1,792 | 0.6× | 142.3 ms | 155.3 ms | 426.2% | 68 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 2,814 | baseline | 90.3 ms | 99.8 ms | 713.0% | 76 MiB | 0 | ✅ |

## Symfony

_Symfony 7 — mixed JSON routes: GET /, GET /user/:id, POST /user (prod env, cached routes/config)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 481 | 0.1× | 533.7 ms | 557.8 ms | 411.2% | 77 MiB | 0 | ✅ |
| Turbine NTS · 8w | 767 | 0.2× | 334.2 ms | 347.8 ms | 782.9% | 83 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 15,524 | 4.0× | 16.4 ms | 20.2 ms | 430.2% | 72 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 15,594 | 4.0× | 16.3 ms | 19.6 ms | 451.5% | 77 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 446 | 0.1× | 574.7 ms | 591.2 ms | 407.9% | 87 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 752 | 0.2× | 338.4 ms | 358.1 ms | 783.3% | 102 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 11,808 | 3.0× | 21.5 ms | 24.9 ms | 437.8% | 79 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 14,309 | 3.7× | 17.8 ms | 21.3 ms | 473.4% | 92 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 4,744 | 1.2× | 53.5 ms | 70.6 ms | 725.5% | 70 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 4,688 | 1.2× | 54.1 ms | 71.2 ms | 723.1% | 73 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 4,740 | 1.2× | 53.5 ms | 70.6 ms | 724.9% | 72 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 4,612 | 1.2× | 55.0 ms | 73.1 ms | 724.2% | 71 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 2,646 | 0.7× | 96.5 ms | 103.2 ms | 432.8% | 44 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 3,879 | baseline | 65.6 ms | 74.0 ms | 677.4% | 50 MiB | 0 | ✅ |

## Phalcon

_Phalcon Micro — mixed JSON routes: GET /, GET /user/:id, POST /user_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 14,827 | 0.8× | 17.1 ms | 20.2 ms | 384.8% | 61 MiB | 0 | ✅ |
| Turbine NTS · 8w | 18,916 | 1.0× | 13.4 ms | 16.7 ms | 434.0% | 68 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 18,619 | 1.0× | 13.7 ms | 16.5 ms | 392.4% | 57 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 19,638 | 1.1× | 12.9 ms | 15.8 ms | 422.6% | 60 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 19,218 | 1.1× | 13.2 ms | 16.7 ms | 459.6% | 71 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 20,855 | 1.1× | 12.1 ms | 16.1 ms | 501.7% | 77 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 15,007 | 0.8× | 16.9 ms | 20.1 ms | 424.6% | 65 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 16,740 | 0.9× | 15.2 ms | 18.3 ms | 460.8% | 75 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 16,193 | 0.9× | 15.7 ms | 18.7 ms | 452.3% | 32 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 18,212 | baseline | 13.9 ms | 16.8 ms | 491.0% | 37 MiB | 0 | ✅ |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 17,815 | 1.4× | 14.2 ms | 17.7 ms | 343.2% | 63 MiB | 0 | ✅ |
| Turbine NTS · 8w | 20,202 | 1.6× | 12.5 ms | 16.2 ms | 374.2% | 66 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 18,508 | 1.5× | 13.7 ms | 16.9 ms | 329.2% | 64 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 20,641 | 1.7× | 12.2 ms | 15.6 ms | 360.7% | 60 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 24,354 | 2.0× | 10.3 ms | 14.1 ms | 391.6% | 78 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 26,151 | 2.1× | 9.5 ms | 14.9 ms | 431.0% | 100 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 15,250 | 1.2× | 16.7 ms | 20.0 ms | 367.2% | 71 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 18,134 | 1.5× | 14.0 ms | 17.4 ms | 398.7% | 89 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 12,598 | 1.0× | 19.6 ms | 49.5 ms | 400.4% | 64 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 12,504 | 1.0× | 19.8 ms | 49.3 ms | 404.1% | 63 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 12,462 | 1.0× | 19.9 ms | 49.2 ms | 402.4% | 59 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 12,656 | 1.0× | 19.6 ms | 49.4 ms | 397.9% | 56 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 10,907 | 0.9× | 23.3 ms | 28.1 ms | 337.8% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 12,461 | baseline | 20.4 ms | 24.9 ms | 377.5% | 37 MiB | 0 | ✅ |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 17,670 | 1.4× | 14.3 ms | 17.9 ms | 341.6% | 62 MiB | 0 | ✅ |
| Turbine NTS · 8w | 20,724 | 1.7× | 12.2 ms | 16.0 ms | 387.8% | 63 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 17,903 | 1.5× | 14.2 ms | 17.4 ms | 322.3% | 63 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 20,779 | 1.7× | 12.1 ms | 15.7 ms | 361.3% | 56 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 23,788 | 1.9× | 10.6 ms | 14.1 ms | 389.2% | 74 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 26,528 | 2.2× | 9.4 ms | 14.7 ms | 432.6% | 94 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 17,404 | 1.4× | 14.6 ms | 18.2 ms | 382.7% | 67 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 18,248 | 1.5× | 13.9 ms | 17.3 ms | 403.3% | 91 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 12,442 | 1.0× | 19.9 ms | 49.2 ms | 400.8% | 64 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 12,423 | 1.0× | 19.9 ms | 49.6 ms | 401.6% | 63 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 12,368 | 1.0× | 19.9 ms | 49.9 ms | 399.7% | 59 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 12,385 | 1.0× | 19.9 ms | 50.2 ms | 399.5% | 57 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 10,752 | 0.9× | 23.6 ms | 27.9 ms | 336.8% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 12,315 | baseline | 20.4 ms | 29.2 ms | 379.1% | 37 MiB | 0 | ✅ |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 16,597 | 1.5× | 15.2 ms | 19.1 ms | 394.1% | 61 MiB | 0 | ✅ |
| Turbine NTS · 8w | 17,051 | 1.6× | 14.8 ms | 18.8 ms | 419.7% | 65 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 10,581 | 1.0× | 24.1 ms | 26.5 ms | 239.4% | 62 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 17,334 | 1.6× | 14.6 ms | 18.2 ms | 408.5% | 59 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 18,021 | 1.7× | 14.1 ms | 17.5 ms | 436.9% | 74 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 19,119 | 1.8× | 13.2 ms | 17.6 ms | 472.7% | 91 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 14,672 | 1.4× | 17.3 ms | 20.6 ms | 411.2% | 63 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 14,921 | 1.4× | 16.9 ms | 21.0 ms | 454.4% | 76 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 10,741 | 1.0× | 22.8 ms | 56.8 ms | 422.1% | 66 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 10,561 | 1.0× | 23.1 ms | 58.5 ms | 418.7% | 64 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 10,706 | 1.0× | 22.9 ms | 57.0 ms | 415.1% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 10,541 | 1.0× | 23.2 ms | 57.1 ms | 418.0% | 60 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 9,459 | 0.9× | 26.9 ms | 31.6 ms | 363.1% | 34 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 10,795 | baseline | 23.5 ms | 28.3 ms | 403.7% | 38 MiB | 0 | ✅ |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
