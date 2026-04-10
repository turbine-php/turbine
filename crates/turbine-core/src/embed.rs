//! Embedded PHP application support.
//!
//! When built with `TURBINE_EMBED_DIR` set, the PHP application files are
//! packed into the binary at compile time. At runtime, they're extracted
//! to a temporary directory (or configured `extract_dir`) and served from there.
//!
//! ## Build
//!
//! ```sh
//! TURBINE_EMBED_DIR=./my-app cargo build --release --features embed
//! ```
//!
//! ## How it works
//!
//! 1. `build.rs` packs the app directory into a tar.gz archive
//! 2. The archive is embedded via `include_bytes!()`
//! 3. At runtime, `extract_embedded_app()` unpacks to disk
//! 4. The server uses the extracted directory as its root

use std::path::PathBuf;
use tracing::{error, info};

use crate::config::EmbedConfig;

/// Embedded app archive (populated by build script when `embed` feature is active).
/// When not building with embed feature, this is an empty slice.
#[cfg(feature = "embed")]
static EMBEDDED_APP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/embedded_app.tar.gz"));

#[cfg(not(feature = "embed"))]
static EMBEDDED_APP: &[u8] = &[];

/// Check if an embedded app is available in the binary.
pub fn has_embedded_app() -> bool {
    !EMBEDDED_APP.is_empty()
}

/// Extract the embedded PHP application to disk.
///
/// Returns the path to the extracted directory, or None if no app is embedded
/// or extraction fails.
pub fn extract_embedded_app(config: &EmbedConfig) -> Option<PathBuf> {
    if EMBEDDED_APP.is_empty() {
        return None;
    }

    let extract_dir = match &config.extract_dir {
        Some(dir) => PathBuf::from(dir),
        None => {
            let mut tmp = std::env::temp_dir();
            tmp.push("turbine-embedded-app");
            tmp
        }
    };

    // Create extraction directory
    if let Err(e) = std::fs::create_dir_all(&extract_dir) {
        error!(path = %extract_dir.display(), error = %e, "Failed to create embed extraction directory");
        return None;
    }

    // Check if already extracted (marker file with embedded hash)
    let hash = xxhash_rust::xxh3::xxh3_64(EMBEDDED_APP);
    let marker_path = extract_dir.join(".turbine-embed-hash");
    if marker_path.exists() {
        if let Ok(existing_hash) = std::fs::read_to_string(&marker_path) {
            if existing_hash.trim() == format!("{hash:x}") {
                info!(path = %extract_dir.display(), "Embedded app already extracted (hash match)");
                return Some(extract_dir);
            }
        }
    }

    // Extract tar.gz archive
    info!(
        size = EMBEDDED_APP.len(),
        path = %extract_dir.display(),
        "Extracting embedded PHP application"
    );

    match extract_tar_gz(EMBEDDED_APP, &extract_dir) {
        Ok(count) => {
            // Write hash marker
            let _ = std::fs::write(&marker_path, format!("{hash:x}"));
            info!(files = count, path = %extract_dir.display(), "Embedded app extracted successfully");
            Some(extract_dir)
        }
        Err(e) => {
            error!(error = %e, "Failed to extract embedded app");
            None
        }
    }
}

/// Extract a tar.gz archive to a directory. Returns the number of files extracted.
fn extract_tar_gz(data: &[u8], dest: &std::path::Path) -> Result<usize, String> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let decoder = GzDecoder::new(data);
    let mut archive = Vec::new();
    let mut decoder_reader = decoder;
    decoder_reader
        .read_to_end(&mut archive)
        .map_err(|e| format!("Gzip decompress failed: {e}"))?;

    // Simple tar extraction (POSIX tar format)
    // Each entry: 512-byte header + file data (padded to 512)
    let mut pos = 0;
    let mut count = 0;

    while pos + 512 <= archive.len() {
        let header = &archive[pos..pos + 512];

        // Check for end-of-archive (two zero blocks)
        if header.iter().all(|&b| b == 0) {
            break;
        }

        // Extract filename (0..100)
        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = String::from_utf8_lossy(&header[..name_end]).to_string();

        // Extract file size from octal field at offset 124..136
        let size_str = String::from_utf8_lossy(&header[124..136])
            .trim()
            .trim_matches('\0')
            .to_string();
        let file_size = usize::from_str_radix(&size_str, 8).unwrap_or(0);

        // Type flag at offset 156
        let type_flag = header[156];

        pos += 512; // Move past header

        if name.is_empty() || name == "." {
            pos += (file_size + 511) / 512 * 512;
            continue;
        }

        let target = dest.join(&name);

        // Security: prevent path traversal
        if let Ok(canonical_dest) = dest.canonicalize() {
            // For new files, check the parent
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(canonical_target) = target.canonicalize().or_else(|_| {
                // File doesn't exist yet — check parent
                target
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .map(|p| p.join(target.file_name().unwrap_or_default()))
                    .ok_or(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "no parent",
                    ))
            }) {
                if !canonical_target.starts_with(&canonical_dest) {
                    pos += (file_size + 511) / 512 * 512;
                    continue; // Skip path traversal attempts
                }
            }
        }

        match type_flag {
            b'5' | b'/' => {
                // Directory
                let _ = std::fs::create_dir_all(&target);
            }
            b'0' | b'\0' => {
                // Regular file
                if pos + file_size <= archive.len() {
                    if let Some(parent) = target.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::write(&target, &archive[pos..pos + file_size]).is_ok() {
                        count += 1;
                    }
                }
            }
            _ => {
                // Skip symlinks and other special entries for security
            }
        }

        // Advance past file data (padded to 512-byte boundary)
        pos += (file_size + 511) / 512 * 512;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_embedded_app() {
        // Without the embed feature, no app is embedded
        assert!(!has_embedded_app());
    }
}
