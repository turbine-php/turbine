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
| Turbine NTS · 4w | 104,966 | 4.0× | 2.2 ms | 7.2 ms | 205.0% | 30 MiB | 0 |
| Turbine NTS · 8w | 101,602 | 3.9× | 2.2 ms | 7.4 ms | 207.8% | 32 MiB | 0 |
| Turbine NTS · 4w · persistent | 104,156 | 4.0× | 2.2 ms | 7.4 ms | 211.0% | 29 MiB | 0 |
| Turbine NTS · 8w · persistent | 100,816 | 3.8× | 2.2 ms | 7.5 ms | 206.8% | 33 MiB | 0 |
| Turbine ZTS · 4w | 104,876 | 4.0× | 2.1 ms | 7.3 ms | 207.3% | 29 MiB | 0 |
| Turbine ZTS · 8w | 100,177 | 3.8× | 2.3 ms | 7.6 ms | 208.8% | 31 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 24,087 | 0.9× | 10.1 ms | 33.2 ms | 480.2% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 23,345 | 0.9× | 10.4 ms | 33.6 ms | 474.8% | 58 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 23,970 | 0.9× | 10.1 ms | 32.8 ms | 478.8% | 61 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 22,503 | 0.9× | 10.7 ms | 36.0 ms | 475.5% | 57 MiB | 0 |
| Nginx + FPM · 4w | 25,242 | 1.0× | 10.1 ms | 12.7 ms | 407.4% | 33 MiB | 0 |
| Nginx + FPM · 8w | 26,253 | baseline | 9.7 ms | 12.2 ms | 431.6% | 36 MiB | 0 |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 98,454 | — | 2.3 ms | 314.1 ms | 211.0% | 76 MiB | 0 |
| Turbine NTS · 8w | 103,158 | — | 2.2 ms | 7.4 ms | 210.6% | 85 MiB | 0 |
| Turbine NTS · 4w · persistent | 100,979 | — | 2.2 ms | 7.4 ms | 208.4% | 94 MiB | 0 |
| Turbine NTS · 8w · persistent | 102,779 | — | 2.2 ms | 7.4 ms | 208.9% | 121 MiB | 0 |
| Turbine ZTS · 4w | 96,987 | — | 2.3 ms | 569.0 ms | 211.5% | 72 MiB | 25 |
| Turbine ZTS · 8w | 102,617 | — | 2.2 ms | 7.3 ms | 205.5% | 80 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 304 | — | 786.5 ms | 1401.3 ms | 201.6% | 97 MiB | 16 |
| FrankenPHP (ZTS) · 8w | 323 | — | 769.9 ms | 1342.2 ms | 206.9% | 93 MiB | 10 |
| FrankenPHP (ZTS) · 4w · worker | 325 | — | 762.7 ms | 1320.2 ms | 215.7% | 95 MiB | 9 |
| FrankenPHP (ZTS) · 8w · worker | 320 | — | 769.2 ms | 1318.0 ms | 213.0% | 96 MiB | 9 |
| Nginx + FPM · 4w | 0 | — | 0.0 ms | 0.0 ms | — | — | 0 |
| Nginx + FPM · 8w | 0 | baseline | 0.0 ms | 0.0 ms | — | — | 0 |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 102,302 | 5.2× | 2.2 ms | 7.4 ms | 207.2% | 35 MiB | 0 |
| Turbine NTS · 8w | 102,445 | 5.2× | 2.2 ms | 7.4 ms | 209.0% | 38 MiB | 0 |
| Turbine NTS · 4w · persistent | 100,188 | 5.1× | 2.3 ms | 7.5 ms | 210.0% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 102,833 | 5.3× | 2.2 ms | 7.5 ms | 208.9% | 38 MiB | 0 |
| Turbine ZTS · 4w | 100,684 | 5.1× | 2.3 ms | 7.6 ms | 211.4% | 35 MiB | 0 |
| Turbine ZTS · 8w | 102,613 | 5.2× | 2.2 ms | 7.3 ms | 206.7% | 37 MiB | 0 |
| Nginx + FPM · 4w | 17,176 | 0.9× | 14.8 ms | 18.2 ms | 459.8% | 32 MiB | 0 |
| Nginx + FPM · 8w | 19,583 | baseline | 13.0 ms | 15.8 ms | 496.9% | 37 MiB | 0 |

## PHP Scripts

_Individual scripts: Hello World, 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### Hello World

_Minimal `echo 'Hello World!'` response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 92,091 | 3.1× | 2.5 ms | 8.1 ms | 239.8% | 31 MiB | 0 |
| Turbine NTS · 8w | 91,833 | 3.1× | 2.5 ms | 8.1 ms | 241.1% | 33 MiB | 0 |
| Turbine NTS · 4w · persistent | 92,905 | 3.1× | 2.5 ms | 7.9 ms | 241.1% | 30 MiB | 0 |
| Turbine NTS · 8w · persistent | 92,523 | 3.1× | 2.5 ms | 8.1 ms | 243.7% | 33 MiB | 0 |
| Turbine ZTS · 4w | 91,889 | 3.1× | 2.5 ms | 8.1 ms | 241.8% | 29 MiB | 0 |
| Turbine ZTS · 8w | 92,496 | 3.1× | 2.5 ms | 8.1 ms | 240.3% | 32 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 29,137 | 1.0× | 8.6 ms | 20.6 ms | 419.5% | 65 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 29,897 | 1.0× | 8.3 ms | 20.1 ms | 419.5% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 29,028 | 1.0× | 8.6 ms | 20.8 ms | 419.1% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 30,091 | 1.0× | 8.3 ms | 20.3 ms | 420.1% | 56 MiB | 0 |
| Nginx + FPM · 4w | 25,534 | 0.9× | 9.9 ms | 12.7 ms | 391.1% | 32 MiB | 0 |
| Nginx + FPM · 8w | 29,798 | baseline | 8.5 ms | 10.6 ms | 417.2% | 37 MiB | 0 |

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 90,958 | 6.9× | 2.5 ms | 8.1 ms | 240.9% | 34 MiB | 0 |
| Turbine NTS · 8w | 92,641 | 7.0× | 2.5 ms | 8.0 ms | 240.4% | 37 MiB | 0 |
| Turbine NTS · 4w · persistent | 93,001 | 7.0× | 2.5 ms | 8.1 ms | 240.8% | 33 MiB | 0 |
| Turbine NTS · 8w · persistent | 93,992 | 7.1× | 2.4 ms | 7.9 ms | 243.9% | 39 MiB | 0 |
| Turbine ZTS · 4w | 91,763 | 6.9× | 2.5 ms | 8.0 ms | 239.4% | 31 MiB | 0 |
| Turbine ZTS · 8w | 93,089 | 7.0× | 2.5 ms | 8.1 ms | 236.6% | 36 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 28,990 | 2.2× | 8.6 ms | 20.6 ms | 420.6% | 65 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 29,705 | 2.2× | 8.4 ms | 20.1 ms | 421.5% | 63 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 29,193 | 2.2× | 8.5 ms | 20.5 ms | 419.2% | 62 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 30,346 | 2.3× | 8.2 ms | 19.6 ms | 421.0% | 55 MiB | 0 |
| Nginx + FPM · 4w | 10,941 | 0.8× | 23.2 ms | 28.9 ms | 343.4% | 33 MiB | 0 |
| Nginx + FPM · 8w | 13,223 | baseline | 19.2 ms | 23.4 ms | 384.1% | 37 MiB | 0 |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 92,066 | 7.0× | 2.5 ms | 8.0 ms | 241.2% | 35 MiB | 0 |
| Turbine NTS · 8w | 92,453 | 7.0× | 2.5 ms | 7.9 ms | 238.7% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 93,459 | 7.1× | 2.4 ms | 7.9 ms | 237.9% | 34 MiB | 0 |
| Turbine NTS · 8w · persistent | 92,684 | 7.0× | 2.5 ms | 8.0 ms | 239.0% | 40 MiB | 0 |
| Turbine ZTS · 4w | 92,088 | 7.0× | 2.5 ms | 8.0 ms | 242.0% | 33 MiB | 0 |
| Turbine ZTS · 8w | 93,913 | 7.1× | 2.4 ms | 7.9 ms | 240.9% | 37 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 28,968 | 2.2× | 8.6 ms | 20.3 ms | 419.6% | 66 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 30,755 | 2.3× | 8.1 ms | 19.4 ms | 421.5% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 29,292 | 2.2× | 8.5 ms | 20.4 ms | 420.5% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 29,594 | 2.2× | 8.4 ms | 20.5 ms | 421.3% | 54 MiB | 0 |
| Nginx + FPM · 4w | 11,090 | 0.8× | 22.7 ms | 28.6 ms | 344.0% | 38 MiB | 0 |
| Nginx + FPM · 8w | 13,237 | baseline | 19.2 ms | 23.5 ms | 382.3% | 37 MiB | 0 |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem | Errors |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|-------:|
| Turbine NTS · 4w | 91,456 | 8.3× | 2.5 ms | 8.0 ms | 241.3% | 34 MiB | 0 |
| Turbine NTS · 8w | 92,144 | 8.3× | 2.5 ms | 8.0 ms | 243.5% | 39 MiB | 0 |
| Turbine NTS · 4w · persistent | 92,430 | 8.4× | 2.5 ms | 8.1 ms | 240.1% | 35 MiB | 0 |
| Turbine NTS · 8w · persistent | 93,146 | 8.4× | 2.5 ms | 7.9 ms | 239.1% | 41 MiB | 0 |
| Turbine ZTS · 4w | 91,556 | 8.3× | 2.5 ms | 8.0 ms | 238.6% | 33 MiB | 0 |
| Turbine ZTS · 8w | 94,043 | 8.5× | 2.4 ms | 7.9 ms | 239.7% | 38 MiB | 0 |
| FrankenPHP (ZTS) · 4w | 28,841 | 2.6× | 8.7 ms | 20.5 ms | 420.9% | 66 MiB | 0 |
| FrankenPHP (ZTS) · 8w | 30,240 | 2.7× | 8.3 ms | 19.9 ms | 422.2% | 64 MiB | 0 |
| FrankenPHP (ZTS) · 4w · worker | 29,086 | 2.6× | 8.6 ms | 20.8 ms | 420.0% | 60 MiB | 0 |
| FrankenPHP (ZTS) · 8w · worker | 29,936 | 2.7× | 8.3 ms | 20.1 ms | 421.8% | 54 MiB | 0 |
| Nginx + FPM · 4w | 9,791 | 0.9× | 25.9 ms | 31.2 ms | 377.0% | 34 MiB | 0 |
| Nginx + FPM · 8w | 11,068 | baseline | 22.9 ms | 28.1 ms | 408.2% | 40 MiB | 0 |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
