# Compression

Turbine compresses HTTP responses automatically using Brotli, Zstd, or Gzip, based on the client's `Accept-Encoding` header.

## Configuration

```toml
[compression]
enabled = true
# Minimum response size to compress (bytes)
min_size = 1024
# Compression level (1-9, higher = smaller but slower)
level = 6
# Algorithm preference order
algorithms = ["br", "zstd", "gzip"]
```

## Algorithms

| Algorithm | `Accept-Encoding` | Ratio | Speed | Browser Support |
|-----------|-------------------|:-----:|:-----:|:---------------:|
| Brotli | `br` | Best | Fast | All modern browsers |
| Zstd | `zstd` | Good | Fastest | Chrome 123+, Firefox 126+ |
| Gzip | `gzip` | Good | Fast | Universal |

Turbine selects the best algorithm the client supports, using the `algorithms` priority order.

## How It Works

1. Client sends `Accept-Encoding: br, gzip, zstd`
2. Turbine generates the PHP response (uncompressed)
3. If response size >= `min_size`, Turbine compresses using the highest-priority algorithm the client supports
4. Response includes `Content-Encoding: br` (or `gzip` / `zstd`)

## What Gets Compressed

Turbine compresses text-based content types:
- `text/html`, `text/css`, `text/javascript`, `text/plain`
- `application/json`, `application/xml`, `application/javascript`

Binary content (images, PDFs, ZIP files) is not compressed — it's already compressed.

## Disabling Compression

```toml
[compression]
enabled = false
```

Or set a very high `min_size` to effectively disable:

```toml
[compression]
min_size = 999999999
```

## Performance Impact

Compression adds CPU overhead but reduces bandwidth:

| Level | Overhead | Size Reduction |
|:-----:|:--------:|:--------------:|
| 1 | ~0.1ms | ~60% |
| 6 | ~0.5ms | ~75% |
| 9 | ~2ms | ~80% |

Level 6 (default) is the recommended balance for most workloads.
