# X-Sendfile / X-Accel-Redirect

Turbine supports `X-Sendfile` and `X-Accel-Redirect` headers for efficient file serving. Instead of PHP reading the file into memory and sending it, Turbine serves the file directly from disk.

## How It Works

```
Client ──→ PHP                     ──→ Turbine
           (checks permissions)         (reads file from disk)
           header('X-Sendfile: /path')  (serves directly)
```

PHP handles authentication and authorization. Turbine handles the file I/O, which is faster and uses less memory.

## Configuration

```toml
[x_sendfile]
enabled = true
# Base directory — acts as a security boundary. Paths sent via the
# X-Sendfile / X-Accel-Redirect headers are resolved relative to this
# directory and cannot escape it (no `..`, no absolute paths outside root).
# Relative values are resolved from the application root (`--root`).
root = "private-files/"
```

## Usage in PHP

```php
<?php
// Check user authentication
if (!$user->canDownload($fileId)) {
    http_response_code(403);
    exit;
}

$filePath = "private-files/reports/monthly-2026.pdf";

// Option 1: X-Sendfile (Apache-compatible)
header("X-Sendfile: $filePath");

// Option 2: X-Accel-Redirect (Nginx-compatible)
header("X-Accel-Redirect: $filePath");

// Set content type
header('Content-Type: application/pdf');
header('Content-Disposition: attachment; filename="report.pdf"');
```

Turbine intercepts the `X-Sendfile` or `X-Accel-Redirect` header, removes it from the response, and serves the file directly.

## Security

- The `root` setting restricts file access to a specific directory
- Path traversal is blocked (`../` is rejected)
- Files outside `root` cannot be served
- The header is stripped from the response (not sent to the client)

## Use Cases

- Protected file downloads (PDFs, ZIPs, media)
- Large file serving without PHP memory overhead
- Video streaming with range request support
