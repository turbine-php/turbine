# TLS & ACME Auto-TLS

Turbine supports HTTPS via manual TLS certificates or automatic Let's Encrypt provisioning.

## Manual TLS

Provide your own certificate and key:

```toml
[server.tls]
enabled = true
cert_file = "/etc/ssl/certs/mydomain.pem"
key_file = "/etc/ssl/private/mydomain.key"
```

Or via CLI:

```bash
turbine serve --tls-cert cert.pem --tls-key key.pem
```

Turbine uses **rustls** (a pure-Rust TLS implementation) — no OpenSSL dependency for the server itself.

### Supported Formats

- PEM-encoded certificate chain (cert + intermediate CAs)
- PEM-encoded private key (RSA or ECDSA)

## ACME Auto-TLS (Let's Encrypt)

Turbine can automatically provision and renew TLS certificates from Let's Encrypt.

### Single-site setup

For a server hosting one application, list your domains in `[acme].domains`:

```toml
[acme]
enabled = true
domains = ["example.com", "www.example.com"]
email = "admin@example.com"
cache_dir = "/var/lib/turbine/acme"
```

### With virtual hosting

When using `[[virtual_hosts]]`, you do **not** need `[acme].domains`. Turbine auto-collects `domain` + `aliases` from every virtual host:

```toml
[acme]
enabled = true
email = "admin@example.com"
# No 'domains' needed — auto-collected from [[virtual_hosts]]

[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
```

This requests a single certificate covering `xpto.com`, `www.xpto.com`, and `outro.com`.

> **Rule of thumb:** Use `[acme].domains` for single-site. Use `[[virtual_hosts]]` for multi-site — domains are auto-collected, no duplication needed. If you set both, `turbine check` will warn about redundancy.

Virtual hosts with their own `tls_cert`/`tls_key` are **excluded** from ACME — their manual certificates take priority. See [Virtual Hosting](virtual-hosts.md) for per-host SNI configuration.

> **Note:** Requires the `acme` feature flag: `cargo build --release --features acme`

### How It Works

1. On startup, Turbine checks for cached certificates in `cache_dir`
2. If no valid certificate exists, it starts the ACME provisioning flow:
   - Creates/loads an ACME account
   - Requests certificates for the configured domains
   - Handles HTTP-01 challenges automatically (serves `/.well-known/acme-challenge/{token}`)
   - Downloads and saves the certificate chain
3. A background task checks for renewal every 12 hours
4. Certificates are renewed when older than 60 days (Let's Encrypt certs expire at 90 days)

### Requirements

- Port 80 must be accessible from the internet (for HTTP-01 challenges)
- DNS must point to your server for all configured domains
- Turbine needs write access to `cache_dir`

### Testing with Staging

Always test with `staging = true` first — Let's Encrypt has strict rate limits on the production API:

```toml
[acme]
enabled = true
domains = ["test.example.com"]
staging = true  # Uses staging CA, no rate limits
```

### Certificate Storage

Certificates are stored in `cache_dir`:

```
/var/lib/turbine/acme/
├── account.json     # ACME account credentials (reused)
├── cert.pem         # Certificate chain
└── key.pem          # Private key (600 permissions)
```

## Session Security with TLS

When TLS is enabled, Turbine automatically sets `cookie_secure = true` for session cookies:

```toml
[session]
cookie_secure = false  # Auto-enabled when TLS is active
cookie_samesite = "Lax"
```

## HTTP/2

HTTP/2 is automatically available when TLS is enabled. No additional configuration needed. HTTP/2 enables:
- Multiplexed requests over a single connection
- Header compression (HPACK)
- True Early Hints (103 informational frame)

## Per-Domain Certificates (SNI)

When using [virtual hosting](virtual-hosts.md), each domain can have its own TLS certificate. Turbine uses SNI (Server Name Indication) to serve the correct certificate based on the client's hostname:

```toml
[server.tls]
enabled = true
cert_file = "/etc/ssl/default.pem"    # fallback
key_file = "/etc/ssl/default-key.pem"

[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
tls_cert = "/etc/ssl/xpto.com.pem"
tls_key = "/etc/ssl/xpto.com-key.pem"
```

See [Virtual Hosting — TLS with Virtual Hosts](virtual-hosts.md#tls-with-virtual-hosts) for full examples.

## TLS Decision Matrix

| Scenario | Config needed |
|----------|--------------|
| Single site, manual cert | `[server.tls]` with `cert_file` + `key_file` |
| Single site, auto cert | `[acme]` with `domains` |
| Multi-site, shared cert | `[server.tls]` + `[[virtual_hosts]]` (no per-host certs) |
| Multi-site, per-site certs | `[server.tls]` + `[[virtual_hosts]]` with `tls_cert`/`tls_key` (SNI) |
| Multi-site, auto certs | `[acme]` + `[[virtual_hosts]]` (domains auto-collected) |
