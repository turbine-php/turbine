# PHP Extensions

Turbine supports PHP extensions in two ways: **static** (compiled into `libphp`) and **dynamic** (loaded at runtime from `.so` files).

## Currently Included Extensions (45)

The default PHP embed build includes these extensions compiled statically:

| Extension | Category | Description |
|-----------|----------|-------------|
| Core | Core | PHP core functions |
| standard | Core | Standard library (arrays, strings, math) |
| SPL | Core | Standard PHP Library (iterators, data structures) |
| Reflection | Core | Runtime class/function introspection |
| date | Core | Date/time functions |
| pcre | Core | Regular expressions (PCRE2) |
| filter | Core | Input validation and sanitization |
| hash | Core | Hashing algorithms (60+ algorithms) |
| json | Core | JSON encode/decode |
| random | Core | Random number generation (PHP 8.2+) |
| ctype | Core | Character type checking |
| tokenizer | Core | PHP tokenizer |
| Phar | Core | PHP Archive support |
| Zend OPcache | Performance | Opcode cache + JIT compilation |
| mbstring | String | Multibyte string handling |
| iconv | String | Character encoding conversion |
| intl | i18n | ICU internationalization |
| openssl | Crypto | OpenSSL encryption/TLS |
| sodium | Crypto | Modern cryptography (libsodium) |
| bcmath | Math | Arbitrary precision math |
| gmp | Math | GNU Multiple Precision arithmetic |
| PDO | Database | PHP Data Objects abstraction |
| pdo_sqlite | Database | SQLite PDO driver |
| pdo_mysql | Database | MySQL PDO driver |
| sqlite3 | Database | SQLite3 native interface |
| mysqli | Database | MySQL native interface |
| mysqlnd | Database | MySQL Native Driver |
| curl | Network | HTTP client (cURL) |
| sockets | Network | Low-level socket operations |
| session | Web | Session management |
| dom | XML | DOM document manipulation |
| SimpleXML | XML | Simple XML parser |
| xml | XML | Expat XML parser |
| xmlreader | XML | XMLReader streaming parser |
| xmlwriter | XML | XMLWriter generator |
| libxml | XML | libxml2 base library |
| lexbor | HTML | HTML5 parser (PHP 8.4+) |
| uri | URL | URL parsing (PHP 8.5+) |
| gd | Image | Image processing (JPEG, PNG, WebP, FreeType) |
| FFI | System | Foreign Function Interface |
| pcntl | System | Process control |
| posix | System | POSIX functions |
| fileinfo | File | MIME type detection |
| zip | File | ZIP archive handling |
| zlib | Compression | Gzip compression |

## Adding Dynamic Extensions

For PECL or third-party extensions, configure them in `turbine.toml`:

```toml
[php]
extension_dir = "/opt/homebrew/lib/php/extensions/no-debug-non-zts-20240924"
extensions = ["redis.so", "imagick.so", "apcu.so"]
zend_extensions = ["xdebug.so"]
```

### Installing via PECL

Use the PHP binary from the embed build:

```bash
./vendor/php-embed/bin/pecl install redis
./vendor/php-embed/bin/pecl install imagick
./vendor/php-embed/bin/pecl install apcu
```

The `.so` files will be installed in the PHP extensions directory. Find the path with:

```bash
./vendor/php-embed/bin/php-config --extension-dir
```

### Configuring Extension Settings

Use `[php.ini]` for extension-specific directives:

```toml
[php]
extensions = ["redis.so"]

[php.ini]
"redis.session.locking_enabled" = "1"
"redis.session.lock_retries" = "10"
```

## Adding Static Extensions

To compile a new extension into `libphp`, modify the build script and recompile:

### 1. Add build dependency

```bash
brew install libsodium  # macOS example
```

### 2. Edit the build script

In `scripts/build-php-embed.sh`, add the configure flag:

```bash
./configure \
    ... \
    --with-sodium \
    ...
```

Available flags:

| Extension | Flag | System Dependency |
|-----------|------|-------------------|
| GD | `--enable-gd --with-jpeg --with-webp --with-freetype` | `libpng libjpeg-turbo webp freetype` |
| Sodium | `--with-sodium` | `libsodium` |
| GMP | `--with-gmp` | `gmp` |
| FFI | `--with-ffi` | `libffi` |
| PostgreSQL | `--with-pdo-pgsql --with-pgsql` | `libpq` |
| LDAP | `--with-ldap` | `openldap` |
| Readline | `--with-readline` | `readline` |
| Tidy | `--with-tidy` | `tidy-html5` |

### 3. Rebuild

```bash
# Rebuild PHP with new extension
./scripts/build-php-embed.sh 8.5.4

# Rebuild Turbine
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo build --release
```

## Verifying Extensions

Check loaded extensions at runtime:

```bash
# Via CLI
DYLD_LIBRARY_PATH="$PWD/vendor/php-embed/lib" ./vendor/php-embed/bin/php -m

# Via HTTP (create a test file)
echo '<?php phpinfo();' > test-app/info.php
```

## Static vs Dynamic

| | Static | Dynamic |
|---|--------|---------|
| **Performance** | Slightly faster (no dlopen) | Negligible difference |
| **Deployment** | Single `libphp` file | Requires `.so` files |
| **Recompile PHP** | Yes | No |
| **Recompile Turbine** | Yes | No |
| **Use case** | Core extensions, production | PECL packages, development tools |

**Recommendation:** Use static for extensions you always need (the 45 included by default). Use dynamic for optional extensions like Redis, Xdebug, or Imagick.
