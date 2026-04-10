//! Feature implementations for Turbine:
//! - Early Hints (103)
//! - X-Sendfile / X-Accel-Redirect
//! - Structured Logging from PHP

use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ── Early Hints (103) ─────────────────────────────────────────────────────
//
// PHP sends: header('Link: </style.css>; rel=preload; as=style');
//            headers_send(103);
//
// In the native SAPI response, headers are available. We look for Link headers
// and detect a 103 status hint. For the envelope-based protocol, we
// use a custom marker: __TURBINE_EARLY_HINT__\t<header-value>\n
//
// Since hyper HTTP/1.1 doesn't support sending 103 as an intermediate response
// on the same connection (it's only supported in HTTP/2), we collect the hints
// and include them as Link headers in the final response. This still allows
// HTTP/2 clients and proxies to benefit.

/// Marker used by PHP envelope code to signal Early Hints.
#[allow(dead_code)]
pub const TURBINE_EARLY_HINT_MARKER: &str = "__TURBINE_EARLY_HINT__\t";

/// Extract Early Hints (Link headers) from response headers and return them.
/// In HTTP/1.1, these are included as Link headers in the final response.
/// In HTTP/2, they could be sent as 103 informational frames.
pub fn extract_early_hints(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("Link"))
        .map(|(_, v)| v.clone())
        .collect()
}

/// Parse early hint markers from the envelope-based response body (before the
/// body marker). Returns extracted hint values and the cleaned metadata.
#[allow(dead_code)]
pub fn parse_early_hint_markers(meta_lines: &str) -> Vec<String> {
    meta_lines
        .lines()
        .filter_map(|line| line.strip_prefix(TURBINE_EARLY_HINT_MARKER))
        .map(|v| v.to_string())
        .collect()
}

// ── X-Sendfile / X-Accel-Redirect ─────────────────────────────────────────
//
// PHP sends: header('X-Accel-Redirect: /files/report.pdf');
// or:        header('X-Sendfile: /var/data/report.pdf');
//
// Turbine intercepts this header, strips it from the response, reads the file
// from disk, and sends it as the response body. This avoids PHP buffering
// large files in memory.

/// Check response headers for X-Sendfile or X-Accel-Redirect.
/// Returns the file path if found, and whether to remove the header.
pub fn check_x_sendfile(headers: &[(String, String)]) -> Option<String> {
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("X-Accel-Redirect") || k.eq_ignore_ascii_case("X-Sendfile") {
            return Some(v.clone());
        }
    }
    None
}

/// Resolve the X-Sendfile path, ensuring it's within the allowed root.
/// Returns the absolute path if valid, None if path traversal detected.
pub fn resolve_sendfile_path(file_path: &str, sendfile_root: &Path) -> Option<PathBuf> {
    // The path from the header may be:
    //   Absolute:  /private-files/report.pdf  (mapped to sendfile_root)
    //   Relative:  report.pdf                 (relative to sendfile_root)
    let target = if file_path.starts_with('/') {
        // Strip leading slash and join with root
        let relative = file_path.trim_start_matches('/');
        sendfile_root.join(relative)
    } else {
        sendfile_root.join(file_path)
    };

    // Canonicalize both to prevent path traversal
    let resolved = match target.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            warn!(path = %file_path, error = %e, "X-Sendfile: file not found or inaccessible");
            return None;
        }
    };

    let root_canonical = match sendfile_root.canonicalize() {
        Ok(p) => p,
        Err(_) => return None,
    };

    if resolved.starts_with(&root_canonical) {
        Some(resolved)
    } else {
        warn!(
            path = %file_path,
            resolved = %resolved.display(),
            root = %root_canonical.display(),
            "X-Sendfile: path traversal attempt blocked"
        );
        None
    }
}

/// Serve a file for X-Sendfile, returning (content_type, body).
pub fn serve_sendfile(path: &Path) -> Option<(String, Vec<u8>)> {
    match std::fs::read(path) {
        Ok(body) => {
            let content_type = mime_for_sendfile(path);
            debug!(
                path = %path.display(),
                size = body.len(),
                content_type = %content_type,
                "X-Sendfile: serving file"
            );
            Some((content_type, body))
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "X-Sendfile: failed to read file");
            None
        }
    }
}

fn mime_for_sendfile(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "csv" => "text/csv",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ── Structured Logging ────────────────────────────────────────────────────
//
// PHP can emit structured log entries using a special marker in output:
//   __TURBINE_LOG__\t{"level":"warn","msg":"Memory high","current_usage":10485760}\n
//
// Turbine intercepts these markers, parses them, and emits them via tracing.
// This gives PHP apps structured logging with severity levels, compatible with
// Datadog, Grafana Loki, Elastic, etc.

/// Marker for structured log messages from PHP.
pub const TURBINE_LOG_MARKER: &str = "__TURBINE_LOG__\t";

/// A structured log entry from PHP.
#[derive(Debug)]
pub struct PhpLogEntry {
    pub level: String,
    pub message: String,
    pub context: Vec<(String, String)>,
}

/// Extract and process structured log markers from the response body.
/// Returns the cleaned body (with markers removed) and the extracted log entries.
pub fn extract_structured_logs(body: &[u8]) -> (Vec<u8>, Vec<PhpLogEntry>) {
    let text = match std::str::from_utf8(body) {
        Ok(t) => t,
        Err(_) => return (body.to_vec(), Vec::new()),
    };

    if !text.contains(TURBINE_LOG_MARKER) {
        return (body.to_vec(), Vec::new());
    }

    let mut cleaned = Vec::with_capacity(body.len());
    let mut entries = Vec::new();

    for line in text.split('\n') {
        if let Some(json_str) = line.strip_prefix(TURBINE_LOG_MARKER) {
            if let Some(entry) = parse_log_json(json_str.trim()) {
                entries.push(entry);
            }
        } else if !cleaned.is_empty() || !line.is_empty() {
            if !cleaned.is_empty() {
                cleaned.push(b'\n');
            }
            cleaned.extend_from_slice(line.as_bytes());
        }
    }

    (cleaned, entries)
}

fn parse_log_json(json_str: &str) -> Option<PhpLogEntry> {
    // Minimal JSON parsing without pulling in serde_json — we just need level, msg, context.
    // Format: {"level":"warn","msg":"text","key1":"val1","key2":"val2"}
    let trimmed = json_str
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}');

    let mut level = "info".to_string();
    let mut message = String::new();
    let mut context = Vec::new();

    for part in split_json_fields(trimmed) {
        let (key, value) = match part.split_once(':') {
            Some((k, v)) => (k.trim().trim_matches('"'), v.trim().trim_matches('"')),
            None => continue,
        };
        match key {
            "level" => level = value.to_string(),
            "msg" | "message" => message = value.to_string(),
            _ => context.push((key.to_string(), value.to_string())),
        }
    }

    if message.is_empty() {
        return None;
    }

    Some(PhpLogEntry {
        level,
        message,
        context,
    })
}

/// Split JSON fields, respecting quoted strings.
fn split_json_fields(s: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_quotes = !in_quotes;
        } else if bytes[i] == b',' && !in_quotes {
            fields.push(&s[start..i]);
            start = i + 1;
        }
    }
    if start < s.len() {
        fields.push(&s[start..]);
    }
    fields
}

/// Emit a structured log entry via tracing.
pub fn emit_log_entry(entry: &PhpLogEntry) {
    let ctx_str: String = entry
        .context
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ");

    match entry.level.as_str() {
        "debug" | "-4" => debug!(source = "php", ctx = %ctx_str, "{}", entry.message),
        "info" | "0" => info!(source = "php", ctx = %ctx_str, "{}", entry.message),
        "warn" | "warning" | "4" => warn!(source = "php", ctx = %ctx_str, "{}", entry.message),
        "error" | "8" => tracing::error!(source = "php", ctx = %ctx_str, "{}", entry.message),
        _ => info!(source = "php", level = %entry.level, ctx = %ctx_str, "{}", entry.message),
    }
}

/// PHP helper code that defines the turbine_log() function.
/// This should be prepended to PHP code execution so the function is available.
pub fn php_turbine_log_function() -> &'static str {
    r#"if (!function_exists('turbine_log')) {
    function turbine_log(string $message, string $level = 'info', array $context = []): void {
        $entry = ['level' => $level, 'msg' => $message];
        foreach ($context as $k => $v) {
            $entry[$k] = is_scalar($v) ? (string)$v : json_encode($v);
        }
        echo "\x5f\x5fTURBINE_LOG__\t" . json_encode($entry, JSON_UNESCAPED_SLASHES) . "\n";
    }
}
"#
}

// ── Worker Pool Route Matching ────────────────────────────────────────────

/// Match a request path against a route pattern (supports trailing *).
pub fn matches_pool_route(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        path.starts_with(prefix)
    } else {
        path == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_early_hints() {
        let headers = vec![
            (
                "Link".to_string(),
                "</style.css>; rel=preload; as=style".to_string(),
            ),
            ("Content-Type".to_string(), "text/html".to_string()),
            (
                "Link".to_string(),
                "</app.js>; rel=preload; as=script".to_string(),
            ),
        ];
        let hints = extract_early_hints(&headers);
        assert_eq!(hints.len(), 2);
        assert!(hints[0].contains("style.css"));
        assert!(hints[1].contains("app.js"));
    }

    #[test]
    fn test_check_x_sendfile() {
        let headers = vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            (
                "X-Accel-Redirect".to_string(),
                "/files/report.pdf".to_string(),
            ),
        ];
        assert_eq!(
            check_x_sendfile(&headers),
            Some("/files/report.pdf".to_string())
        );

        let headers_sendfile = vec![("x-sendfile".to_string(), "/data/file.zip".to_string())];
        assert_eq!(
            check_x_sendfile(&headers_sendfile),
            Some("/data/file.zip".to_string())
        );

        let no_sendfile = vec![("Content-Type".to_string(), "text/html".to_string())];
        assert_eq!(check_x_sendfile(&no_sendfile), None);
    }

    #[test]
    fn test_structured_log_extract() {
        let body = b"Hello World\n__TURBINE_LOG__\t{\"level\":\"warn\",\"msg\":\"Memory high\",\"usage\":\"10485760\"}\nMore output";
        let (cleaned, entries) = extract_structured_logs(body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "warn");
        assert_eq!(entries[0].message, "Memory high");
        assert_eq!(entries[0].context.len(), 1);
        let cleaned_str = String::from_utf8(cleaned).unwrap();
        assert!(cleaned_str.contains("Hello World"));
        assert!(cleaned_str.contains("More output"));
        assert!(!cleaned_str.contains("TURBINE_LOG"));
    }

    #[test]
    fn test_matches_pool_route() {
        assert!(matches_pool_route("/api/slow/*", "/api/slow/endpoint"));
        assert!(matches_pool_route("/api/slow/*", "/api/slow/"));
        assert!(!matches_pool_route("/api/slow/*", "/api/fast/endpoint"));
        assert!(matches_pool_route("/webhook", "/webhook"));
        assert!(!matches_pool_route("/webhook", "/webhooks"));
    }

    #[test]
    fn test_php_log_function_syntax() {
        let code = php_turbine_log_function();
        assert!(code.contains("function turbine_log"));
        assert!(code.contains("TURBINE_LOG__"));
    }
}
