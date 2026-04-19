//! Pure HTTP helpers: response building, CORS, static-file serving,
//! MIME detection, query-string parsing, access logging.
//!
//! Extracted from `main.rs` to keep the request-handler hot path
//! readable.  Everything here is either a pure function or takes
//! plain references to config/metrics; no `ServerState` coupling
//! beyond the thin `write_access_log` wrapper.

use std::time::Instant;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use tracing::info;
use turbine_metrics::MetricsCollector;

use crate::ServerState;
use crate::config;

pub type HyperResponse = Response<Full<Bytes>>;

/// Check if a request origin is allowed by the CORS config.
pub fn cors_origin_allowed(cors: &config::CorsConfig, origin: &str) -> bool {
    cors.allow_origins.iter().any(|o| o == "*" || o == origin)
}

/// Apply CORS headers to a response.
pub fn apply_cors_headers(
    headers: &mut hyper::header::HeaderMap,
    cors: &config::CorsConfig,
    origin: &str,
) {
    use hyper::header::HeaderValue;

    let origin_value = if cors.allow_origins.iter().any(|o| o == "*") && !cors.allow_credentials {
        "*"
    } else {
        origin
    };
    if let Ok(val) = HeaderValue::from_str(origin_value) {
        headers.insert("Access-Control-Allow-Origin", val);
    }

    if cors.allow_credentials {
        headers.insert(
            "Access-Control-Allow-Credentials",
            HeaderValue::from_static("true"),
        );
    }

    let methods = cors.allow_methods.join(", ");
    if let Ok(val) = HeaderValue::from_str(&methods) {
        headers.insert("Access-Control-Allow-Methods", val);
    }

    let allow_headers = cors.allow_headers.join(", ");
    if let Ok(val) = HeaderValue::from_str(&allow_headers) {
        headers.insert("Access-Control-Allow-Headers", val);
    }

    if !cors.expose_headers.is_empty() {
        let expose = cors.expose_headers.join(", ");
        if let Ok(val) = HeaderValue::from_str(&expose) {
            headers.insert("Access-Control-Expose-Headers", val);
        }
    }

    if cors.max_age > 0 {
        headers.insert("Access-Control-Max-Age", cors.max_age.into());
    }
}

/// Parse PHP-style size strings like `"64M"`, `"2G"`, `"128K"`, `"1024"`
/// into bytes.  Returns `None` for `"0"`, empty, or unparseable input
/// (= no limit).
pub fn parse_php_size(s: &str) -> Option<usize> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "0" {
        return None;
    }
    let (num_part, mult): (&str, usize) = match trimmed.chars().last() {
        Some('K') | Some('k') => (&trimmed[..trimmed.len() - 1], 1024),
        Some('M') | Some('m') => (&trimmed[..trimmed.len() - 1], 1024 * 1024),
        Some('G') | Some('g') => (&trimmed[..trimmed.len() - 1], 1024 * 1024 * 1024),
        _ => (trimmed, 1),
    };
    num_part
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
}

/// Extract the first raw value of `name` from a `k=v&k=v`-style query
/// string.  No percent-decoding is performed — callers on the PHP side
/// either restrict keys to `[A-Za-z0-9_.-]` or base64-url-encode them.
pub fn query_param<'a>(qs: &'a str, name: &str) -> Option<&'a str> {
    for pair in qs.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next()? == name {
            return it.next();
        }
    }
    None
}

/// Build a `hyper::Response` with security headers pre-applied.
pub fn build_response(
    status: u16,
    content_type: &str,
    body: impl Into<Bytes>,
    extra_headers: &[(&str, &str)],
) -> HyperResponse {
    let body: Bytes = body.into();
    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let content_length = body.len();
    let mut builder = Response::builder()
        .status(status_code)
        .header("Content-Type", content_type)
        .header("Content-Length", content_length)
        .header("Server", format!("Turbine/{}", env!("CARGO_PKG_VERSION")))
        // Security headers
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "SAMEORIGIN")
        .header("X-XSS-Protection", "0")
        .header("Referrer-Policy", "strict-origin-when-cross-origin")
        .header(
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=()",
        );

    for (name, value) in extra_headers {
        // Skip invalid header names/values to prevent panics from PHP code
        // that emit pseudo-headers like "Status: 200 OK"
        if hyper::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
            && hyper::header::HeaderValue::from_str(value).is_ok()
        {
            builder = builder.header(*name, *value);
        }
    }

    builder.body(Full::new(body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Internal response build error")))
            .unwrap()
    })
}

/// Write an access log entry in Combined Log Format.
pub fn write_access_log(
    state: &ServerState,
    method: &str,
    path: &str,
    status: u16,
    request_start: Instant,
    client_ip: &str,
) {
    if let Some(ref log_mutex) = state.access_log {
        use std::io::Write;
        let elapsed_ms = request_start.elapsed().as_millis();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let line = format!("{client_ip} - - [{now}] \"{method} {path}\" {status} {elapsed_ms}ms\n");

        if let Ok(mut writer) = log_mutex.lock() {
            let _ = writer.write_all(line.as_bytes());
            let _ = writer.flush();
        }
    }
}

/// Try to serve a static file.  Returns `Some(response)` when the file
/// was found and served (or a 304), `None` to let the caller fall
/// through to PHP dispatch.
pub fn try_serve_static(
    document_root: &std::path::Path,
    uri_path: &str,
    method: &hyper::Method,
    metrics: &MetricsCollector,
    request_start: Instant,
    if_none_match: Option<&str>,
) -> Option<HyperResponse> {
    let clean = uri_path.split('?').next().unwrap_or(uri_path);
    if clean.ends_with(".php") || clean == "/" {
        return None;
    }

    let relative = clean.trim_start_matches('/');
    if relative.is_empty() || relative.contains("..") {
        return None;
    }

    let file_path = document_root.join(relative);

    if let (Ok(resolved), Ok(root)) = (file_path.canonicalize(), document_root.canonicalize()) {
        if !resolved.starts_with(&root) {
            return None;
        }

        if resolved.is_file() {
            match std::fs::read(&resolved) {
                Ok(body) => {
                    let content_type = mime_type_for_extension(relative);

                    // ETag: xxh3 hash of file content
                    let hash = xxhash_rust::xxh3::xxh3_64(&body);
                    let etag = format!("\"{hash:x}\"");

                    // 304 Not Modified: check If-None-Match
                    if let Some(client_etag) = if_none_match {
                        if client_etag == etag || client_etag.trim() == etag {
                            let elapsed = request_start.elapsed();
                            let elapsed_us = elapsed.as_micros() as u64;
                            metrics.record_request(relative, 304, elapsed_us, 0);
                            info!(method = %method, path = uri_path, status = 304, elapsed_us = elapsed_us, "Not modified");
                            return Some(build_response(
                                304,
                                content_type,
                                Vec::new(),
                                &[("ETag", &etag)],
                            ));
                        }
                    }

                    let elapsed = request_start.elapsed();
                    let elapsed_us = elapsed.as_micros() as u64;
                    metrics.record_request(relative, 200, elapsed_us, body.len() as u64);

                    let cache_header = if relative.contains("/assets/") {
                        "public, max-age=31536000, immutable"
                    } else {
                        "public, max-age=3600"
                    };

                    info!(method = %method, path = uri_path, status = 200, elapsed_us = elapsed_us, bytes = body.len(), "Static file served");

                    Some(build_response(
                        200,
                        content_type,
                        body,
                        &[("Cache-Control", cache_header), ("ETag", &etag)],
                    ))
                }
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    }
}

/// Detect content type from the first 256 bytes of PHP output.
pub fn detect_content_type(output: &[u8]) -> &'static str {
    let prefix = &output[..output.len().min(256)];
    if prefix.starts_with(b"{") || prefix.starts_with(b"[") {
        "application/json"
    } else if prefix.windows(6).any(|w| w == b"<html>" || w == b"<HTML>")
        || prefix
            .windows(9)
            .any(|w| w == b"<!DOCTYPE" || w == b"<!doctype")
    {
        "text/html; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    }
}

/// Map file extension to MIME type for static file serving.
pub fn mime_type_for_extension(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "xml" => "application/xml",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_finds_value() {
        assert_eq!(query_param("a=1&b=2", "b"), Some("2"));
        assert_eq!(query_param("a=1&b=2", "c"), None);
        assert_eq!(query_param("", "a"), None);
    }

    #[test]
    fn parse_php_size_handles_suffixes() {
        assert_eq!(parse_php_size("1024"), Some(1024));
        assert_eq!(parse_php_size("2K"), Some(2048));
        assert_eq!(parse_php_size("4M"), Some(4 * 1024 * 1024));
        assert_eq!(parse_php_size("1g"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_php_size("0"), None);
        assert_eq!(parse_php_size(""), None);
        assert_eq!(parse_php_size("garbage"), None);
    }

    #[test]
    fn mime_type_mapping() {
        assert_eq!(mime_type_for_extension("foo.css"), "text/css; charset=utf-8");
        assert_eq!(mime_type_for_extension("a/b/c.JPG"), "image/jpeg");
        assert_eq!(mime_type_for_extension("unknown.xyz"), "application/octet-stream");
        assert_eq!(mime_type_for_extension("noext"), "application/octet-stream");
    }

    #[test]
    fn detect_content_type_basic() {
        assert_eq!(detect_content_type(b"{\"x\":1}"), "application/json");
        assert_eq!(detect_content_type(b"<!DOCTYPE html>..."), "text/html; charset=utf-8");
        assert_eq!(detect_content_type(b"hello world"), "text/plain; charset=utf-8");
    }
}
