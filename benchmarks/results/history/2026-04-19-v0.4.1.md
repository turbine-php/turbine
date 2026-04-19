# Turbine Benchmark Results

| | |
|---|---|
| **Version** | v0.4.1 |
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
| Turbine NTS · 4w | 30,145 | 0.9× | 8.5 ms | 10.6 ms | 287.4% | 64 MiB | 0 | ✅ |
| Turbine NTS · 8w | 36,786 | 1.0× | 6.8 ms | 9.6 ms | 342.3% | 57 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 34,355 | 1.0× | 7.4 ms | 8.8 ms | 286.6% | 50 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 39,807 | 1.1× | 6.4 ms | 8.4 ms | 330.2% | 50 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 41,129 | 1.2× | 6.2 ms | 9.4 ms | 392.8% | 67 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 46,887 | 1.3× | 5.3 ms | 8.5 ms | 434.1% | 75 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 28,765 | 0.8× | 8.8 ms | 11.0 ms | 363.2% | 61 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 35,165 | 1.0× | 7.2 ms | 9.7 ms | 415.6% | 70 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 30,102 | 0.9× | 8.0 ms | 27.8 ms | 473.4% | 55 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 30,657 | 0.9× | 7.9 ms | 27.4 ms | 475.0% | 55 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 30,146 | 0.9× | 8.0 ms | 27.8 ms | 473.3% | 57 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 34,424 | 1.0× | 7.1 ms | 21.9 ms | 451.3% | 56 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 31,161 | 0.9× | 8.1 ms | 10.4 ms | 417.3% | 32 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 35,094 | baseline | 7.2 ms | 9.2 ms | 442.3% | 37 MiB | 0 | ✅ |

## Laravel

_Laravel 13 — mixed JSON routes: GET /, GET /user/:id, POST /user (stateless, no database)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 581 | 0.2× | 438.1 ms | 460.2 ms | 410.8% | 95 MiB | 0 | ✅ |
| Turbine NTS · 8w | 929 | 0.3× | 274.5 ms | 292.2 ms | 782.6% | 105 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 4,635 | 1.6× | 54.8 ms | 59.4 ms | 469.2% | 233 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 6,482 | 2.2× | 39.1 ms | 46.7 ms | 672.5% | 308 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 565 | 0.2× | 450.9 ms | 472.2 ms | 408.6% | 101 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 910 | 0.3× | 278.3 ms | 330.9 ms | 783.0% | 118 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 4,378 | 1.5× | 58.1 ms | 62.9 ms | 468.4% | 234 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 6,083 | 2.0× | 41.7 ms | 48.9 ms | 673.4% | 308 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 3,296 | 1.1× | 77.2 ms | 92.4 ms | 751.5% | 88 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 3,184 | 1.1× | 79.9 ms | 95.8 ms | 750.3% | 90 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 3,298 | 1.1× | 77.1 ms | 90.9 ms | 751.0% | 86 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 3,272 | 1.1× | 77.7 ms | 91.9 ms | 750.4% | 86 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 2,013 | 0.7× | 126.7 ms | 135.5 ms | 425.3% | 68 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 2,973 | baseline | 84.7 ms | 123.4 ms | 709.8% | 75 MiB | 0 | ✅ |

## Symfony

_Symfony 7 — mixed JSON routes: GET /, GET /user/:id, POST /user (prod env, cached routes/config)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 587 | 0.1× | 434.3 ms | 449.8 ms | 409.8% | 78 MiB | 0 | ✅ |
| Turbine NTS · 8w | 968 | 0.2× | 262.6 ms | 277.3 ms | 783.9% | 85 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 15,551 | 3.6× | 16.3 ms | 19.5 ms | 421.2% | 74 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 19,947 | 4.7× | 12.8 ms | 15.8 ms | 476.1% | 80 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 564 | 0.1× | 452.5 ms | 469.0 ms | 406.9% | 84 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 928 | 0.2× | 273.7 ms | 289.0 ms | 783.2% | 99 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 13,715 | 3.2× | 18.5 ms | 21.7 ms | 456.1% | 82 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 18,391 | 4.3× | 13.8 ms | 17.3 ms | 508.2% | 94 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 5,280 | 1.2× | 48.0 ms | 65.0 ms | 731.3% | 73 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 5,261 | 1.2× | 48.2 ms | 65.1 ms | 731.0% | 67 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 5,233 | 1.2× | 48.5 ms | 65.3 ms | 731.3% | 71 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 5,209 | 1.2× | 48.6 ms | 65.5 ms | 729.2% | 74 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 2,883 | 0.7× | 88.3 ms | 94.7 ms | 430.7% | 44 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 4,268 | baseline | 59.6 ms | 66.3 ms | 679.7% | 50 MiB | 0 | ✅ |

## Phalcon

_Phalcon Micro — mixed JSON routes: GET /, GET /user/:id, POST /user_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 17,659 | 0.8× | 14.4 ms | 17.2 ms | 388.7% | 60 MiB | 0 | ✅ |
| Turbine NTS · 8w | 22,090 | 1.0× | 11.5 ms | 14.4 ms | 450.1% | 64 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 19,485 | 0.9× | 13.0 ms | 15.4 ms | 393.4% | 55 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 23,414 | 1.1× | 10.9 ms | 13.5 ms | 440.8% | 61 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 24,412 | 1.1× | 10.4 ms | 13.4 ms | 486.1% | 71 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 25,593 | 1.2× | 9.8 ms | 13.3 ms | 516.9% | 79 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 20,613 | 1.0× | 12.3 ms | 15.2 ms | 471.0% | 64 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 20,325 | 0.9× | 12.5 ms | 15.4 ms | 485.5% | 74 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 19,328 | 0.9× | 13.1 ms | 16.2 ms | 476.4% | 34 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 21,564 | baseline | 11.8 ms | 14.2 ms | 513.4% | 51 MiB | 0 | ✅ |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 20,924 | 1.4× | 12.1 ms | 15.2 ms | 329.0% | 66 MiB | 0 | ✅ |
| Turbine NTS · 8w | 25,146 | 1.6× | 10.0 ms | 13.5 ms | 370.8% | 68 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 21,762 | 1.4× | 11.7 ms | 14.4 ms | 317.3% | 59 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 26,406 | 1.7× | 9.5 ms | 12.7 ms | 358.3% | 65 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 30,749 | 2.0× | 8.2 ms | 11.6 ms | 389.1% | 77 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 33,973 | 2.2× | 7.3 ms | 12.1 ms | 424.6% | 99 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 19,126 | 1.2× | 13.3 ms | 16.1 ms | 362.9% | 71 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 22,330 | 1.4× | 11.3 ms | 14.3 ms | 400.9% | 92 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 15,278 | 1.0× | 16.2 ms | 41.2 ms | 398.9% | 62 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 15,575 | 1.0× | 15.9 ms | 40.7 ms | 397.6% | 62 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 15,524 | 1.0× | 16.1 ms | 40.3 ms | 400.4% | 59 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 15,612 | 1.0× | 15.9 ms | 40.6 ms | 399.1% | 60 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 13,930 | 0.9× | 18.2 ms | 22.6 ms | 345.2% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 15,460 | baseline | 16.4 ms | 20.4 ms | 378.5% | 37 MiB | 0 | ✅ |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 20,950 | 1.4× | 12.1 ms | 15.3 ms | 329.6% | 57 MiB | 0 | ✅ |
| Turbine NTS · 8w | 25,174 | 1.6× | 10.0 ms | 13.4 ms | 370.1% | 63 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 23,877 | 1.5× | 10.6 ms | 13.7 ms | 338.4% | 56 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 26,365 | 1.7× | 9.5 ms | 12.8 ms | 360.9% | 60 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 30,826 | 2.0× | 8.1 ms | 11.5 ms | 387.7% | 74 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 34,003 | 2.2× | 7.3 ms | 11.9 ms | 427.2% | 94 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 21,800 | 1.4× | 11.6 ms | 15.0 ms | 402.0% | 67 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 22,400 | 1.5× | 11.3 ms | 14.3 ms | 404.7% | 88 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 15,362 | 1.0× | 16.2 ms | 41.1 ms | 397.6% | 61 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 15,577 | 1.0× | 16.0 ms | 40.8 ms | 398.4% | 62 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 15,572 | 1.0× | 16.0 ms | 41.2 ms | 401.0% | 59 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 15,666 | 1.0× | 15.9 ms | 40.7 ms | 400.8% | 63 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 13,891 | 0.9× | 18.2 ms | 22.2 ms | 343.5% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 15,430 | baseline | 16.4 ms | 20.1 ms | 378.8% | 37 MiB | 0 | ✅ |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors | Status |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|:-------|
| Turbine NTS · 4w | 16,679 | 1.3× | 15.2 ms | 18.1 ms | 360.3% | 58 MiB | 0 | ✅ |
| Turbine NTS · 8w | 21,248 | 1.6× | 11.8 ms | 15.5 ms | 435.9% | 67 MiB | 0 | ✅ |
| Turbine NTS · 4w · persistent | 20,029 | 1.5× | 12.7 ms | 16.2 ms | 394.7% | 55 MiB | 0 | ✅ |
| Turbine NTS · 8w · persistent | 20,594 | 1.6× | 12.3 ms | 15.2 ms | 407.5% | 59 MiB | 0 | ✅ |
| Turbine ZTS · 4w | 22,444 | 1.7× | 11.2 ms | 14.6 ms | 434.0% | 74 MiB | 0 | ✅ |
| Turbine ZTS · 8w | 24,429 | 1.9× | 10.3 ms | 14.2 ms | 470.8% | 92 MiB | 0 | ✅ |
| Turbine ZTS · 4w · persistent | 18,503 | 1.4× | 13.7 ms | 17.3 ms | 438.7% | 62 MiB | 0 | ✅ |
| Turbine ZTS · 8w · persistent | 18,186 | 1.4× | 13.9 ms | 17.2 ms | 435.3% | 75 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w | 12,736 | 1.0× | 19.4 ms | 49.0 ms | 422.7% | 61 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w | 12,612 | 1.0× | 19.6 ms | 47.9 ms | 411.9% | 61 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 4w · worker | 12,671 | 1.0× | 19.5 ms | 49.4 ms | 418.6% | 60 MiB | 0 | ✅ |
| FrankenPHP (ZTS) · 8w · worker | 12,916 | 1.0× | 19.2 ms | 47.9 ms | 421.5% | 61 MiB | 0 | ✅ |
| Nginx + FPM · 4w | 11,667 | 0.9× | 21.7 ms | 25.9 ms | 376.3% | 33 MiB | 0 | ✅ |
| Nginx + FPM · 8w | 12,939 | baseline | 19.6 ms | 23.6 ms | 411.9% | 38 MiB | 0 | ✅ |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
