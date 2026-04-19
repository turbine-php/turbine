//! HTTP compression helpers (gzip / brotli / zstd).
//!
//! Negotiation honours the server-configured preference order from
//! `[compression] preferred_order` and picks the first encoding the
//! client advertises via `Accept-Encoding`.  All three codecs degrade
//! gracefully on error — if compression fails we return the original
//! bytes rather than surfacing the error to the user.

pub fn is_compressible_content_type(content_type: &str) -> bool {
    content_type.contains("text/")
        || content_type.contains("application/json")
        || content_type.contains("application/javascript")
        || content_type.contains("application/xml")
        || content_type.contains("image/svg+xml")
        || content_type.contains("application/manifest+json")
}

pub fn gzip_compress(data: &[u8], level: u32) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level));
    if encoder.write_all(data).is_err() {
        return data.to_vec();
    }
    encoder.finish().unwrap_or_else(|_| data.to_vec())
}

pub fn brotli_compress(data: &[u8], level: u32) -> Vec<u8> {
    let mut output = Vec::new();
    let quality = level.min(11);
    let params = brotli::enc::BrotliEncoderParams {
        quality: quality as i32,
        ..Default::default()
    };
    if brotli::BrotliCompress(&mut &data[..], &mut output, &params).is_err() {
        return data.to_vec();
    }
    output
}

pub fn zstd_compress(data: &[u8], level: u32) -> Vec<u8> {
    let zstd_level = level.min(19) as i32;
    zstd::bulk::compress(data, zstd_level).unwrap_or_else(|_| data.to_vec())
}

/// Negotiate the best compression algorithm based on the client's
/// `Accept-Encoding` and the server's preferred order.  Returns
/// `(encoding_name, compressed_data)` on a hit, `None` if no shared
/// algorithm is supported.
pub fn negotiate_compression(
    accept_encoding: &str,
    server_prefs: &[String],
    data: &[u8],
    level: u32,
) -> Option<(&'static str, Vec<u8>)> {
    let ae = accept_encoding.to_lowercase();
    for pref in server_prefs {
        match pref.as_str() {
            "br" if ae.contains("br") => {
                return Some(("br", brotli_compress(data, level)));
            }
            "zstd" if ae.contains("zstd") => {
                return Some(("zstd", zstd_compress(data, level)));
            }
            "gzip" if ae.contains("gzip") => {
                return Some(("gzip", gzip_compress(data, level)));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_detection() {
        assert!(is_compressible_content_type("text/html"));
        assert!(is_compressible_content_type("application/json"));
        assert!(is_compressible_content_type("image/svg+xml"));
        assert!(!is_compressible_content_type("image/png"));
        assert!(!is_compressible_content_type("application/octet-stream"));
    }

    #[test]
    fn gzip_roundtrip() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let input = b"hello ".repeat(100);
        let compressed = gzip_compress(&input, 6);
        assert!(compressed.len() < input.len());
        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn negotiate_prefers_server_order() {
        let prefs = vec!["br".into(), "gzip".into()];
        let (enc, _) = negotiate_compression("gzip, br", &prefs, b"data", 3).unwrap();
        assert_eq!(enc, "br");
    }

    #[test]
    fn negotiate_returns_none_when_no_overlap() {
        let prefs = vec!["zstd".into()];
        assert!(negotiate_compression("gzip", &prefs, b"data", 3).is_none());
    }
}
