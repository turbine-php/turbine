//! PHP response post-processing: envelope parsing, cache-control
//! inspection, X-Sendfile rewrite, Early Hints, and structured-log
//! extraction.
//!
//! The PHP worker emits a wire format like:
//!
//! ```text
//! __TURBINE_STATUS__\t200
//! __TURBINE_HEADER__\tContent-Type\ttext/html
//! __TURBINE_BODY__
//! <actual body bytes>
//! ```
//!
//! [`parse_turbine_response_envelope`] turns that back into a
//! `(status, headers, body)` triple; [`postprocess_php_response`]
//! applies feature hooks before the bytes go on the wire.

use tracing::debug;

use crate::ServerState;
use crate::features;

/// Envelope markers emitted by the PHP SAPI layer.  Kept here so the
/// parser and the emitter live next to each other.
pub const TURBINE_STATUS_MARKER: &str = "__TURBINE_STATUS__\t";
pub const TURBINE_HEADER_MARKER: &str = "__TURBINE_HEADER__\t";
pub const TURBINE_BODY_MARKER: &str = "__TURBINE_BODY__\n";

/// Returns true when the PHP response headers indicate the response must not
/// be stored in a shared cache (Cache-Control: no-store / no-cache / private).
pub fn response_prevents_caching(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("Cache-Control")
            && (v.contains("no-store") || v.contains("no-cache") || v.contains("private"))
    })
}

/// Apply feature hooks (structured logs, Early Hints, X-Sendfile) to
/// the decoded PHP response.  Mutates in place.
pub fn postprocess_php_response(
    state: &ServerState,
    body: &mut Vec<u8>,
    status_code: &mut u16,
    content_type: &mut String,
    headers: &mut Vec<(String, String)>,
) {
    // 1. Structured logging: extract __TURBINE_LOG__ markers from body
    if state.structured_logging_enabled {
        let (cleaned, entries) = features::extract_structured_logs(body);
        if !entries.is_empty() {
            *body = cleaned;
            for entry in &entries {
                features::emit_log_entry(entry);
            }
        }
    }

    // 2. Early Hints: extract Link headers and include in final response
    if state.early_hints_enabled {
        let hints = features::extract_early_hints(headers);
        // Link headers are already present in the headers vec — they'll be
        // forwarded as-is. Nothing extra to do for HTTP/1.1.
        // For HTTP/2, we'd send 103 frames here.
        if !hints.is_empty() {
            debug!(hints = ?hints, "Early Hints detected (Link headers preserved)");
        }
    }

    // 3. X-Sendfile / X-Accel-Redirect: replace body with file contents
    if state.x_sendfile_enabled {
        if let Some(sendfile_path) = features::check_x_sendfile(headers) {
            if let Some(ref root) = state.x_sendfile_root {
                if let Some(resolved) = features::resolve_sendfile_path(&sendfile_path, root) {
                    if let Some((file_ct, file_body)) = features::serve_sendfile(&resolved) {
                        *body = file_body;
                        *content_type = file_ct;
                        *status_code = 200;
                        // Remove X-Accel-Redirect / X-Sendfile headers from response
                        headers.retain(|(k, _)| {
                            !k.eq_ignore_ascii_case("X-Accel-Redirect")
                                && !k.eq_ignore_ascii_case("X-Sendfile")
                        });
                    }
                }
            }
        }
    }
}

/// Parse a worker-emitted `__TURBINE_*__` envelope back into
/// `(status_code, headers, body)`.  Returns `None` when the status
/// marker is missing (e.g. the PHP script died before emitting
/// anything).
#[allow(clippy::type_complexity)]
pub fn parse_turbine_response_envelope(
    body: &[u8],
) -> Option<(u16, Vec<(String, String)>, Vec<u8>)> {
    let status_marker = TURBINE_STATUS_MARKER.as_bytes();
    let body_marker = TURBINE_BODY_MARKER.as_bytes();

    // Scan for status marker - may not be at position 0 if PHP emitted warnings/notices first
    let envelope_start = body
        .windows(status_marker.len())
        .position(|w| w == status_marker)
        .or_else(|| {
            // Debug: log first 80 bytes when marker not found
            let preview = &body[..body.len().min(80)];
            debug!(preview = ?String::from_utf8_lossy(preview), "Turbine envelope marker not found");
            None
        })?;

    let envelope = &body[envelope_start..];
    let body_marker_pos = envelope
        .windows(body_marker.len())
        .position(|w| w == body_marker)?;

    let meta = std::str::from_utf8(&envelope[..body_marker_pos]).ok()?;
    let payload = envelope[body_marker_pos + body_marker.len()..].to_vec();

    let mut status_code = 200u16;
    let mut headers = Vec::new();

    for line in meta.lines() {
        if let Some(rest) = line.strip_prefix(TURBINE_STATUS_MARKER) {
            status_code = rest.trim().parse().unwrap_or(200);
            continue;
        }

        if let Some(rest) = line.strip_prefix(TURBINE_HEADER_MARKER) {
            let mut parts = rest.splitn(2, '\t');
            let name = match parts.next() {
                Some(n) => n.trim(),
                None => continue,
            };
            let value = match parts.next() {
                Some(v) => v.trim(),
                None => continue,
            };
            headers.push((name.to_string(), value.to_string()));
        }
    }

    Some((status_code, headers, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let wire = b"__TURBINE_STATUS__\t201\n\
                     __TURBINE_HEADER__\tContent-Type\tapplication/json\n\
                     __TURBINE_HEADER__\tX-Custom\tyes\n\
                     __TURBINE_BODY__\n\
                     {\"ok\":true}";
        let (status, headers, body) = parse_turbine_response_envelope(wire).unwrap();
        assert_eq!(status, 201);
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("Content-Type".into(), "application/json".into()));
        assert_eq!(body, b"{\"ok\":true}");
    }

    #[test]
    fn envelope_missing_marker_returns_none() {
        assert!(parse_turbine_response_envelope(b"plain output").is_none());
    }

    #[test]
    fn envelope_tolerates_leading_noise() {
        let wire = b"PHP Warning: something\n\
                     __TURBINE_STATUS__\t500\n\
                     __TURBINE_BODY__\n\
                     oops";
        let (status, _h, body) = parse_turbine_response_envelope(wire).unwrap();
        assert_eq!(status, 500);
        assert_eq!(body, b"oops");
    }

    #[test]
    fn cache_control_detection() {
        let h = vec![("Cache-Control".into(), "no-store, max-age=0".into())];
        assert!(response_prevents_caching(&h));
        let h = vec![("cache-control".into(), "private".into())];
        assert!(response_prevents_caching(&h));
        let h = vec![("Cache-Control".into(), "public, max-age=60".into())];
        assert!(!response_prevents_caching(&h));
    }
}
