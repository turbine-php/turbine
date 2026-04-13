# Turbine Benchmark Results

| | |
|---|---|
| **Version** | v0.2.0 |
| **Date** | 2026-04-13 |
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

---

## Raw PHP

_Single PHP file returning plain-text Hello World_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 119,731 | 4.5× | 1.9 ms | 5.2 ms | 253.7% | 30 MiB | 0 |
| Turbine NTS · 8w | 120,205 | 4.5× | 1.9 ms | 5.1 ms | 255.0% | 32 MiB | 0 |
| Turbine NTS · 4w · persistent | 120,053 | 4.5× | 1.9 ms | 5.0 ms | 254.8% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 119,294 | 4.5× | 1.9 ms | 5.0 ms | 254.5% | 34 MiB | 0 |
| Turbine ZTS · 4w | 120,080 | 4.5× | 1.9 ms | 5.0 ms | 255.6% | 30 MiB | 0 |
| Turbine ZTS · 8w | 117,718 | 4.4× | 1.9 ms | 5.2 ms | 253.7% | 32 MiB | 0 |
| Turbine ZTS · 4w · persistent | 119,883 | 4.5× | 1.9 ms | 5.0 ms | 253.6% | 29 MiB | 0 |
| Turbine ZTS · 8w · persistent | 119,521 | 4.5× | 1.9 ms | 5.0 ms | 255.2% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 22,620 | 0.8× | 10.8 ms | 29.2 ms | 518.4% | 54 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 22,931 | 0.9× | 10.7 ms | 28.6 ms | 525.1% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 24,947 | 0.9× | 10.0 ms | 24.9 ms | 496.8% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 22,405 | 0.8× | 10.9 ms | 29.1 ms | 519.6% | 58 MiB | 0 |
| Nginx + FPM · 4w | 22,947 | 0.9× | 11.1 ms | 13.0 ms | 429.9% | 33 MiB | 0 |
| Nginx + FPM · 8w | 26,644 | baseline | 9.5 ms | 11.4 ms | 476.9% | 36 MiB | 0 |

## Laravel

_Laravel 13 — mixed JSON routes: GET /, GET /user/:id, POST /user (stateless, no database)_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 93,524 | 33.3× | 2.5 ms | 6.3 ms | 332.0% | 68 MiB | 0 |
| Turbine NTS · 8w | 93,094 | 33.1× | 2.5 ms | 6.4 ms | 331.9% | 73 MiB | 0 |
| Turbine NTS · 4w · persistent | 93,354 | 33.2× | 2.5 ms | 6.4 ms | 331.9% | 88 MiB | 0 |
| Turbine NTS · 8w · persistent | 93,860 | 33.4× | 2.5 ms | 6.4 ms | 331.9% | 110 MiB | 0 |
| Turbine ZTS · 4w | 92,502 | 32.9× | 2.5 ms | 6.5 ms | 331.6% | 67 MiB | 0 |
| Turbine ZTS · 8w | 94,023 | 33.4× | 2.4 ms | 6.3 ms | 333.1% | 71 MiB | 0 |
| Turbine ZTS · 4w · persistent | 92,782 | 33.0× | 2.5 ms | 6.4 ms | 330.9% | 88 MiB | 0 |
| Turbine ZTS · 8w · persistent | 94,374 | 33.6× | 2.4 ms | 6.3 ms | 333.9% | 110 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 1,421 | 0.5× | 179.3 ms | 195.0 ms | 770.1% | 92 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 2,871 | 1.0× | 88.6 ms | 103.5 ms | 750.5% | 91 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 1,426 | 0.5× | 178.5 ms | 194.0 ms | 771.3% | 90 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 2,886 | 1.0× | 88.1 ms | 103.8 ms | 750.1% | 88 MiB | 0 |
| Nginx + FPM · 4w | 1,822 | 0.6× | 139.3 ms | 156.5 ms | 422.5% | 68 MiB | 0 |
| Nginx + FPM · 8w | 2,811 | baseline | 90.6 ms | 97.8 ms | 734.1% | 75 MiB | 0 |

## Phalcon

_Phalcon Micro — mixed JSON routes: GET /, GET /user/:id, POST /user_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 94,524 | 5.2× | 2.4 ms | 6.3 ms | 332.6% | 36 MiB | 0 |
| Turbine NTS · 8w | 93,912 | 5.2× | 2.4 ms | 6.4 ms | 333.0% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 94,692 | 5.2× | 2.4 ms | 6.2 ms | 332.5% | 36 MiB | 0 |
| Turbine NTS · 8w · persistent | 94,273 | 5.2× | 2.4 ms | 6.3 ms | 332.7% | 40 MiB | 0 |
| Turbine ZTS · 4w | 94,479 | 5.2× | 2.4 ms | 6.3 ms | 333.9% | 35 MiB | 0 |
| Turbine ZTS · 8w | 94,029 | 5.2× | 2.4 ms | 6.3 ms | 332.5% | 38 MiB | 0 |
| Turbine ZTS · 4w · persistent | 93,881 | 5.2× | 2.4 ms | 6.4 ms | 333.5% | 35 MiB | 0 |
| Turbine ZTS · 8w · persistent | 94,101 | 5.2× | 2.4 ms | 6.4 ms | 333.7% | 38 MiB | 0 |
| Nginx + FPM · 4w | 15,111 | 0.8× | 16.8 ms | 19.0 ms | 482.3% | 33 MiB | 0 |
| Nginx + FPM · 8w | 18,087 | baseline | 14.1 ms | 16.3 ms | 544.8% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 105,867 | 8.1× | 2.2 ms | 5.9 ms | 290.3% | 30 MiB | 0 |
| Turbine NTS · 8w | 107,118 | 8.2× | 2.1 ms | 5.7 ms | 290.0% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 105,967 | 8.1× | 2.2 ms | 6.1 ms | 291.4% | 31 MiB | 0 |
| Turbine NTS · 8w · persistent | 106,897 | 8.2× | 2.1 ms | 5.7 ms | 291.1% | 34 MiB | 0 |
| Turbine ZTS · 4w | 106,587 | 8.1× | 2.1 ms | 5.8 ms | 287.5% | 30 MiB | 0 |
| Turbine ZTS · 8w | 107,411 | 8.2× | 2.1 ms | 5.7 ms | 292.1% | 33 MiB | 0 |
| Turbine ZTS · 4w · persistent | 106,560 | 8.1× | 2.1 ms | 5.8 ms | 291.8% | 30 MiB | 0 |
| Turbine ZTS · 8w · persistent | 107,660 | 8.2× | 2.1 ms | 5.8 ms | 285.6% | 34 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 13,947 | 1.1× | 18.0 ms | 41.6 ms | 477.0% | 54 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 14,065 | 1.1× | 17.8 ms | 42.1 ms | 473.1% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 13,945 | 1.1× | 18.0 ms | 42.0 ms | 471.2% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 14,151 | 1.1× | 17.8 ms | 41.4 ms | 474.3% | 59 MiB | 0 |
| Nginx + FPM · 4w | 10,746 | 0.8× | 23.7 ms | 26.7 ms | 376.4% | 33 MiB | 0 |
| Nginx + FPM · 8w | 13,090 | baseline | 19.4 ms | 22.4 ms | 428.3% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 106,584 | 8.1× | 2.1 ms | 5.8 ms | 289.9% | 33 MiB | 0 |
| Turbine NTS · 8w | 107,143 | 8.1× | 2.1 ms | 5.6 ms | 290.2% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 106,707 | 8.1× | 2.1 ms | 5.7 ms | 290.1% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 107,765 | 8.2× | 2.1 ms | 5.6 ms | 289.1% | 37 MiB | 0 |
| Turbine ZTS · 4w | 106,962 | 8.1× | 2.1 ms | 5.7 ms | 289.7% | 33 MiB | 0 |
| Turbine ZTS · 8w | 108,457 | 8.2× | 2.1 ms | 5.6 ms | 290.7% | 36 MiB | 0 |
| Turbine ZTS · 4w · persistent | 106,453 | 8.1× | 2.1 ms | 5.7 ms | 291.4% | 33 MiB | 0 |
| Turbine ZTS · 8w · persistent | 108,339 | 8.2× | 2.1 ms | 5.6 ms | 284.8% | 37 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 13,908 | 1.1× | 18.1 ms | 41.4 ms | 475.6% | 57 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 13,958 | 1.1× | 18.0 ms | 41.8 ms | 474.4% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 13,952 | 1.1× | 18.0 ms | 42.1 ms | 471.6% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 14,147 | 1.1× | 17.8 ms | 42.0 ms | 474.3% | 60 MiB | 0 |
| Nginx + FPM · 4w | 10,853 | 0.8× | 23.5 ms | 26.3 ms | 379.3% | 33 MiB | 0 |
| Nginx + FPM · 8w | 13,187 | baseline | 19.3 ms | 22.0 ms | 429.1% | 37 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 106,539 | 9.4× | 2.1 ms | 5.7 ms | 289.3% | 34 MiB | 0 |
| Turbine NTS · 8w | 107,089 | 9.5× | 2.1 ms | 5.6 ms | 289.7% | 38 MiB | 0 |
| Turbine NTS · 4w · persistent | 106,236 | 9.4× | 2.2 ms | 5.7 ms | 290.8% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 107,944 | 9.5× | 2.1 ms | 5.6 ms | 289.8% | 37 MiB | 0 |
| Turbine ZTS · 4w | 107,209 | 9.5× | 2.1 ms | 5.6 ms | 290.4% | 34 MiB | 0 |
| Turbine ZTS · 8w | 107,945 | 9.5× | 2.1 ms | 5.7 ms | 289.4% | 36 MiB | 0 |
| Turbine ZTS · 4w · persistent | 106,427 | 9.4× | 2.2 ms | 5.7 ms | 290.4% | 35 MiB | 0 |
| Turbine ZTS · 8w · persistent | 108,356 | 9.6× | 2.1 ms | 5.7 ms | 287.1% | 38 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 11,619 | 1.0× | 21.4 ms | 48.6 ms | 502.0% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 11,707 | 1.0× | 21.3 ms | 47.8 ms | 502.2% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 11,637 | 1.0× | 21.4 ms | 49.0 ms | 500.5% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 11,800 | 1.0× | 21.2 ms | 47.9 ms | 503.1% | 60 MiB | 0 |
| Nginx + FPM · 4w | 9,577 | 0.8× | 26.6 ms | 30.1 ms | 418.2% | 34 MiB | 0 |
| Nginx + FPM · 8w | 11,330 | baseline | 22.5 ms | 25.9 ms | 473.7% | 38 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
