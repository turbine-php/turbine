use std::env;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();

    // Only pack embedded app when the `embed` feature is active
    #[cfg(feature = "embed")]
    {
        let embed_dir = match env::var("TURBINE_EMBED_DIR") {
            Ok(dir) => dir,
            Err(_) => {
                panic!(
                    "TURBINE_EMBED_DIR environment variable must be set when building with \
                     the 'embed' feature. Set it to the path of your PHP application directory."
                );
            }
        };

        let embed_path = Path::new(&embed_dir);
        if !embed_path.exists() || !embed_path.is_dir() {
            panic!(
                "TURBINE_EMBED_DIR='{}' does not exist or is not a directory",
                embed_dir
            );
        }

        let archive_path = Path::new(&out_dir).join("embedded_app.tar.gz");
        pack_tar_gz(embed_path, &archive_path);

        println!("cargo:rerun-if-changed={}", embed_dir);
        println!(
            "cargo:warning=Embedded app from '{}' ({} bytes)",
            embed_dir,
            std::fs::metadata(&archive_path)
                .map(|m| m.len())
                .unwrap_or(0)
        );
    }

    // When embed feature is not active, create an empty archive
    #[cfg(not(feature = "embed"))]
    {
        let archive_path = Path::new(&out_dir).join("embedded_app.tar.gz");
        if !archive_path.exists() {
            // Create a minimal valid gzip stream (empty)
            let _ = std::fs::write(
                &archive_path,
                [
                    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x03, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ],
            );
        }
    }
}

#[cfg(feature = "embed")]
fn pack_tar_gz(source_dir: &Path, archive_path: &Path) {
    use std::fs;
    use std::io::Write;

    let mut tar_data = Vec::new();
    pack_directory(source_dir, source_dir, &mut tar_data);

    // End-of-archive: two 512-byte zero blocks
    tar_data.extend_from_slice(&[0u8; 1024]);

    // Gzip compress
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar_data).expect("gzip write failed");
    let compressed = encoder.finish().expect("gzip finish failed");

    fs::write(archive_path, compressed).expect("Failed to write embedded app archive");
}

#[cfg(feature = "embed")]
fn pack_directory(root: &Path, current: &Path, tar_data: &mut Vec<u8>) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        // Skip hidden files and common non-essential directories
        if relative.starts_with('.') || relative.contains("/.") {
            continue;
        }
        if relative == "node_modules" || relative.starts_with("node_modules/") {
            continue;
        }
        if relative == ".git" || relative.starts_with(".git/") {
            continue;
        }

        if path.is_dir() {
            // Write directory header
            let dir_name = format!("{}/", relative);
            write_tar_header(tar_data, &dir_name, 0, b'5');
            pack_directory(root, &path, tar_data);
        } else if path.is_file() {
            if let Ok(content) = std::fs::read(&path) {
                write_tar_header(tar_data, &relative, content.len(), b'0');
                tar_data.extend_from_slice(&content);
                // Pad to 512-byte boundary
                let padding = (512 - (content.len() % 512)) % 512;
                tar_data.extend_from_slice(&vec![0u8; padding]);
            }
        }
    }
}

#[cfg(feature = "embed")]
fn write_tar_header(tar_data: &mut Vec<u8>, name: &str, size: usize, type_flag: u8) {
    let mut header = [0u8; 512];

    // Name (0..100)
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(100);
    header[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // Mode (100..108) — 0644 for files, 0755 for dirs
    let mode = if type_flag == b'5' {
        "0000755\0"
    } else {
        "0000644\0"
    };
    header[100..108].copy_from_slice(mode.as_bytes());

    // UID (108..116) and GID (116..124)
    header[108..116].copy_from_slice(b"0001000\0");
    header[116..124].copy_from_slice(b"0001000\0");

    // Size in octal (124..136)
    let size_str = format!("{:011o}\0", size);
    header[124..136].copy_from_slice(size_str.as_bytes());

    // Mtime (136..148)
    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mtime_str = format!("{:011o}\0", mtime);
    header[136..148].copy_from_slice(mtime_str.as_bytes());

    // Type flag (156)
    header[156] = type_flag;

    // Magic (257..265) — POSIX ustar
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");

    // Checksum (148..156) — sum of all bytes with checksum field as spaces
    header[148..156].copy_from_slice(b"        ");
    let checksum: u32 = header.iter().map(|&b| b as u32).sum();
    let cksum_str = format!("{:06o}\0 ", checksum);
    header[148..156].copy_from_slice(cksum_str.as_bytes());

    tar_data.extend_from_slice(&header);
}
