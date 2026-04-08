# Virtual Hosting

Turbine supports virtual hosting — serving different PHP applications on different domains from a single server instance. All virtual hosts share the same worker pool for zero memory overhead.

## Quick Start

```toml
[server]
listen = "0.0.0.0:80"
workers = 8

[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
aliases = ["www.outro.com"]
```

Requests are matched by the `Host` header (O(1) HashMap lookup — zero overhead at scale). If no virtual host matches, the global `root` (from `--root` or `cwd`) is used as fallback.

## Configuration Reference

```toml
[[virtual_hosts]]
# Primary domain name (required)
domain = "xpto.com"
# Application root directory (required)
root = "/var/www/xpto"
# Alternative domain names (optional)
aliases = ["www.xpto.com", "xpto.net"]
# Override entry point (optional — auto-detected by default)
entry_point = "index.php"
# Per-host TLS certificate (optional — overrides global)
tls_cert = "/etc/ssl/xpto.com.pem"
tls_key = "/etc/ssl/xpto.com-key.pem"
```

| Field | Required | Description |
|-------|----------|-------------|
| `domain` | Yes | Primary domain (used for Host header matching) |
| `root` | Yes | Application root directory (absolute or relative to cwd) |
| `aliases` | No | Additional domains that route to this vhost |
| `entry_point` | No | PHP entry point (default: auto-detected from `public/index.php` or `index.php`) |
| `tls_cert` | No | PEM certificate file for this domain (SNI-based) |
| `tls_key` | No | PEM private key file for this domain (SNI-based) |

## How It Works

1. **Startup**: Turbine detects the `AppStructure` (document root, entry point) for each virtual host — the same auto-detection used for the main application (`public/index.php` → framework layout, etc.)
2. **Per-request**: The `Host` header is extracted and lowercased. A HashMap lookup resolves the matching virtual host. Static files and PHP paths are served from that vhost's document root.
3. **Fallback**: Requests with no `Host` header or unrecognized domains use the global application root.

### Performance

Virtual hosting adds zero measurable overhead per request:
- Host header extraction: already parsed by hyper
- Domain lookup: O(1) HashMap (pre-built at startup)
- No regex, no linear scan, no allocation

All virtual hosts share the same worker pool — there's no memory duplication.

## TLS with Virtual Hosts

### Shared Certificate (Recommended)

Use a single wildcard or SAN certificate that covers all domains:

```toml
[server]
listen = "0.0.0.0:443"

[server.tls]
enabled = true
cert_file = "/etc/ssl/wildcard.pem"
key_file = "/etc/ssl/wildcard-key.pem"

[[virtual_hosts]]
domain = "app1.example.com"
root = "/var/www/app1"

[[virtual_hosts]]
domain = "app2.example.com"
root = "/var/www/app2"
```

### Per-Host Certificates (SNI)

Each domain can have its own certificate. Turbine uses SNI (Server Name Indication) to serve the correct certificate based on the client's requested hostname:

```toml
[server]
listen = "0.0.0.0:443"

[server.tls]
enabled = true
# Default certificate (used when SNI doesn't match any vhost)
cert_file = "/etc/ssl/default.pem"
key_file = "/etc/ssl/default-key.pem"

[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]
tls_cert = "/etc/ssl/xpto.com.pem"
tls_key = "/etc/ssl/xpto.com-key.pem"

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
tls_cert = "/etc/ssl/outro.com.pem"
tls_key = "/etc/ssl/outro.com-key.pem"
```

### ACME Auto-TLS

When ACME is enabled, virtual host domains are **automatically collected** into the ACME certificate request. You do **not** need to list them in `[acme].domains` — Turbine collects `domain` + `aliases` from every `[[virtual_hosts]]` entry:

```toml
[acme]
enabled = true
email = "admin@example.com"
# No 'domains' needed — auto-collected from [[virtual_hosts]] below

[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
```

Turbine will request a single certificate covering `xpto.com`, `www.xpto.com`, and `outro.com` from Let's Encrypt.

> **When is `[acme].domains` needed?** Only for single-site setups without `[[virtual_hosts]]`. When virtual hosts are configured, `domains` is ignored (auto-collected).

Virtual hosts that have their own `tls_cert`/`tls_key` are **excluded** from ACME — their manual certificates take priority.

## Validation

Use `turbine check` to validate virtual host configuration before starting:

```bash
turbine check
```

The check validates:
- Domain names are not empty or duplicated
- Root directories exist
- Aliases don't conflict with other domains
- `tls_cert` and `tls_key` are both set (or both omitted)
- Certificate/key files exist on disk
- Warns when listening on `127.0.0.1` with virtual hosts configured

## Comparison with FrankenPHP/Caddy

| Feature | Turbine | FrankenPHP/Caddy |
|---------|---------|-----------------|
| Virtual host matching | O(1) HashMap | Caddy host matching |
| Worker pool | Shared (zero overhead) | Shared |
| Per-host TLS (SNI) | Yes | Yes |
| ACME auto-TLS | Yes (auto-collects vhost domains) | Yes |
| Wildcard domains | Not yet (planned) | Yes (`*.example.com`) |
| Per-host PHP config | No (shared PHP engine) | No |
| Config format | TOML (`[[virtual_hosts]]`) | Caddyfile blocks |

## Sandbox & Security

- **open_basedir**: Virtual host root directories are automatically added to PHP's `open_basedir` when `enforce_open_basedir = true`
- **Execution whitelist**: Applied globally (not per-vhost). In `strict` mode, all vhosts share the same whitelist.
- **Security guards**: SQL injection, code injection, and behaviour guards apply to all virtual hosts.

## Examples

### WordPress + Laravel on the same server

```toml
[server]
listen = "0.0.0.0:80"
workers = 8

[[virtual_hosts]]
domain = "blog.example.com"
root = "/var/www/wordpress"

[[virtual_hosts]]
domain = "app.example.com"
root = "/var/www/laravel"
# Laravel uses public/ — auto-detected
```

### Development with local domains

```bash
# /etc/hosts
127.0.0.1  myapp.local
127.0.0.1  api.local
```

```toml
[server]
listen = "0.0.0.0:8080"
workers = 4

[[virtual_hosts]]
domain = "myapp.local"
root = "/Users/dev/myapp"

[[virtual_hosts]]
domain = "api.local"
root = "/Users/dev/api"
```
