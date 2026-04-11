# Turbine Benchmark Results

| | |
|---|---|
| **Version** | 0.1.0 |
| **Date** | 2026-04-11 |
| **Server** | Hetzner CCX33 (8 vCPU dedicated / 32 GB RAM / NVMe) |
| **Tool** | [bombardier v1.2.6](https://github.com/codesenberg/bombardier) |
| **Parameters** | 30s · 100 connections |
| **Workers** | 4w and 8w variants (Turbine + FPM) |
| **Memory limit** | 256 MB per worker |
| **Max req/worker** | 50,000 |
| **Turbine NTS image** | `katisuhara/turbine-php:latest-php8.4-nts` |
| **Turbine ZTS image** | `katisuhara/turbine-php:latest-php8.4-zts` |

> **Baseline**: Nginx + PHP-FPM · 8 workers.
> **Persistent**: PHP worker process stays alive across requests (same as FrankenPHP worker mode).
> **FrankenPHP** uses ZTS PHP internally and does **not** support Phalcon.
> CPU and memory metrics are collected via `docker stats` during the benchmark run.
> Nginx + PHP-FPM runs natively (no docker stats).

---

## Raw PHP

_Single PHP file returning plain-text Hello World_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 107,017 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 23 MiB |
| Turbine NTS · 8w | 106,207 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 23 MiB |
| Turbine NTS · 4w · persistent | 107,042 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 24 MiB |
| Turbine NTS · 8w · persistent | 106,567 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 27 MiB |
| Turbine ZTS · 4w | 106,935 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 24 MiB |
| Turbine ZTS · 8w | 105,979 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 26 MiB |
| FrankenPHP (ZTS) · 4w | 21,262 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w | 20,807 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 4w · worker | 15,490 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w · worker | 15,590 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| Nginx + FPM · 4w | 201,393 | 1.0× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 197,902 | baseline | 0.0 ms | 0.0 ms | — | — |

## Laravel

_Laravel framework, single JSON route, no database_

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 4w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| FrankenPHP (ZTS) · 4w | 21,096 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 40 MiB |
| FrankenPHP (ZTS) · 8w | 21,284 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 4w · worker | 16,374 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w · worker | 16,586 | 0.1× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| Nginx + FPM · 4w | 192,715 | 1.0× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 194,361 | baseline | 0.0 ms | 0.0 ms | — | — |

## Phalcon

_Phalcon micro application, single JSON route_

> FrankenPHP excluded — Phalcon is incompatible with FrankenPHP (ZTS threading)

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 107,009 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 28 MiB |
| Turbine NTS · 8w | 106,828 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 29 MiB |
| Turbine NTS · 4w · persistent | 107,041 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 29 MiB |
| Turbine NTS · 8w · persistent | 106,959 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 33 MiB |
| Turbine ZTS · 4w | 107,176 | 0.6× | 0.0 ms | 0.0 ms | 0.0% | 29 MiB |
| Turbine ZTS · 8w | 107,004 | 0.5× | 0.0 ms | 0.0 ms | 0.0% | 31 MiB |
| Nginx + FPM · 4w | 198,642 | 1.0× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 194,762 | baseline | 0.0 ms | 0.0 ms | — | — |

## PHP Scripts

_Individual scripts: Hello World, 50 KB HTML, 50 KB PDF binary, 50 KB random (incompressible)_

### Hello World

_Minimal `echo 'Hello World!'` response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 4w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| FrankenPHP (ZTS) · 4w | 21,037 | 0.4× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w | 21,218 | 0.4× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 4w · worker | 17,489 | 0.3× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 8w · worker | 17,718 | 0.3× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| Nginx + FPM · 4w | 44,504 | 0.9× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 52,125 | baseline | 0.0 ms | 0.0 ms | — | — |

### HTML 50 KB

_50 KB HTML response — SSR page simulation._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 4w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| FrankenPHP (ZTS) · 4w | 15,577 | 0.8× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 8w | 16,280 | 0.8× | 0.0 ms | 0.0 ms | 0.0% | 40 MiB |
| FrankenPHP (ZTS) · 4w · worker | 19,951 | 1.0× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w · worker | 20,005 | 1.0× | 0.0 ms | 0.0 ms | 0.0% | 41 MiB |
| Nginx + FPM · 4w | 17,911 | 0.9× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 20,497 | baseline | 0.0 ms | 0.0 ms | — | — |

### PDF Binary 50 KB

_50 KB `application/pdf` binary response._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 4w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| FrankenPHP (ZTS) · 4w | 18,617 | 0.9× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 8w | 19,123 | 0.9× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 4w · worker | 15,499 | 0.8× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| FrankenPHP (ZTS) · 8w · worker | 16,363 | 0.8× | 0.0 ms | 0.0 ms | 0.0% | 38 MiB |
| Nginx + FPM · 4w | 17,967 | 0.9× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 20,463 | baseline | 0.0 ms | 0.0 ms | — | — |

### Random 50 KB

_50 KB incompressible random data — stress-tests compression bypass._

| Server | Req/s | vs baseline | p50 | p99 | Avg CPU | Peak Mem |
|--------|------:|:-----------:|----:|----:|:-------:|---------:|
| Turbine NTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 4w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine NTS · 8w · persistent | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 4w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| Turbine ZTS · 8w | 0 | 0.0× | 0.0 ms | 0.0 ms | — | — |
| FrankenPHP (ZTS) · 4w | 18,990 | 1.1× | 0.0 ms | 0.0 ms | 0.0% | 40 MiB |
| FrankenPHP (ZTS) · 8w | 18,660 | 1.1× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 4w · worker | 19,910 | 1.2× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| FrankenPHP (ZTS) · 8w · worker | 20,220 | 1.2× | 0.0 ms | 0.0 ms | 0.0% | 39 MiB |
| Nginx + FPM · 4w | 14,911 | 0.9× | 0.0 ms | 0.0 ms | — | — |
| Nginx + FPM · 8w | 16,528 | baseline | 0.0 ms | 0.0 ms | — | — |

---

*Generated automatically — [benchmark workflow](/.github/workflows/benchmark.yml)*  
*[View history](history/)*
