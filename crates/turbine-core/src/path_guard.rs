use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum PathGuardError {
    #[error("Path traversal detected: {path}")]
    PathTraversal { path: String },

    #[error("Null byte in path: {path}")]
    NullByte { path: String },

    #[error("Access to blocked system path: {path}")]
    BlockedPath { path: String },
}

/// Blocked system paths that should never be accessible.
const BLOCKED_PATHS: &[&str] = &[
    "/etc",
    "/var",
    "/home",
    "/root",
    "/proc",
    "/sys",
    "/dev",
    "/tmp",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

/// Request path guard — validates PHP file paths before execution.
///
/// Security features:
/// - Path traversal detection (../)
/// - Null byte injection prevention
/// - System path blocking (/etc, /var, /home, etc.)
/// - Execution whitelist enforcement
/// - Data directory protection (no PHP execution)
///
/// This does NOT read or store file contents. PHP handles file I/O
/// directly via the real filesystem, restricted by `open_basedir`.
pub struct RequestGuard {
    root: PathBuf,
}

impl RequestGuard {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        info!(root = %root.display(), "RequestGuard created");
        RequestGuard { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validate a request path for security issues.
    ///
    /// Checks for null bytes, path traversal, and blocked system paths.
    /// Returns the normalized relative path on success.
    pub fn validate(&self, path: &str) -> Result<PathBuf, PathGuardError> {
        if path.contains('\0') {
            warn!(path = path, "Null byte injection attempt");
            return Err(PathGuardError::NullByte {
                path: path.to_string(),
            });
        }

        let normalized = self.normalize_path(path)?;
        self.check_blocked_path(path)?;

        Ok(normalized)
    }

    /// Check if a file exists on disk (relative to root), after validation.
    pub fn exists(&self, path: &str) -> bool {
        if path.contains('\0') {
            return false;
        }
        if let Ok(normalized) = self.normalize_path(path) {
            self.root.join(&normalized).is_file()
        } else {
            false
        }
    }

    /// Check if a path is in a data directory (no PHP execution allowed).
    #[allow(dead_code)]
    pub fn is_data_directory(&self, path: &str, data_dirs: &[String]) -> bool {
        let lower = path.to_lowercase();
        data_dirs
            .iter()
            .any(|dir| lower.starts_with(&dir.to_lowercase()))
    }

    /// Check if a path is in the execution whitelist.
    #[allow(dead_code)]
    pub fn is_whitelisted(&self, path: &str, whitelist: &[String]) -> bool {
        whitelist.iter().any(|w| path == w || path.ends_with(w))
    }

    fn normalize_path(&self, path: &str) -> Result<PathBuf, PathGuardError> {
        let path_obj = Path::new(path);
        let mut normalized = PathBuf::new();

        for component in path_obj.components() {
            match component {
                Component::Normal(c) => normalized.push(c),
                Component::CurDir => {}
                Component::ParentDir => {
                    warn!(path = path, "Path traversal attempt detected");
                    return Err(PathGuardError::PathTraversal {
                        path: path.to_string(),
                    });
                }
                Component::RootDir => {}
                Component::Prefix(_) => {}
            }
        }

        Ok(normalized)
    }

    fn check_blocked_path(&self, path: &str) -> Result<(), PathGuardError> {
        let lower = path.to_lowercase();
        for blocked in BLOCKED_PATHS {
            if lower.starts_with(blocked) {
                warn!(path = path, blocked = blocked, "Blocked system path access");
                return Err(PathGuardError::BlockedPath {
                    path: path.to_string(),
                });
            }
        }

        if lower.contains("c:\\") || lower.contains("c:/") {
            return Err(PathGuardError::BlockedPath {
                path: path.to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> RequestGuard {
        RequestGuard::new("/app")
    }

    #[test]
    fn validate_normal_path() {
        let guard = test_guard();
        let result = guard.validate("index.php");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("index.php"));
    }

    #[test]
    fn validate_nested_path() {
        let guard = test_guard();
        let result = guard.validate("src/Controller.php");
        assert!(result.is_ok());
    }

    #[test]
    fn path_traversal_blocked() {
        let guard = test_guard();
        assert!(matches!(
            guard.validate("../../../etc/passwd"),
            Err(PathGuardError::PathTraversal { .. })
        ));
        assert!(matches!(
            guard.validate("src/../../etc/shadow"),
            Err(PathGuardError::PathTraversal { .. })
        ));
    }

    #[test]
    fn null_byte_blocked() {
        let guard = test_guard();
        assert!(matches!(
            guard.validate("index.php\0.jpg"),
            Err(PathGuardError::NullByte { .. })
        ));
    }

    #[test]
    fn system_path_blocked() {
        let guard = test_guard();
        assert!(matches!(
            guard.validate("/etc/passwd"),
            Err(PathGuardError::BlockedPath { .. })
        ));
        assert!(matches!(
            guard.validate("/var/log/syslog"),
            Err(PathGuardError::BlockedPath { .. })
        ));
    }

    #[test]
    fn exists_check() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.php"), b"<?php echo 1;").unwrap();
        let guard = RequestGuard::new(dir.path());
        assert!(guard.exists("index.php"));
        assert!(!guard.exists("missing.php"));
        assert!(!guard.exists("index.php\0evil"));
    }

    #[test]
    fn data_directory_check() {
        let guard = test_guard();
        let data_dirs = vec!["storage".to_string(), "uploads".to_string()];
        assert!(guard.is_data_directory("storage/logs/app.log", &data_dirs));
        assert!(guard.is_data_directory("uploads/image.jpg", &data_dirs));
        assert!(!guard.is_data_directory("src/Controller.php", &data_dirs));
    }

    #[test]
    fn whitelist_check() {
        let guard = test_guard();
        let whitelist = vec!["index.php".to_string(), "public/index.php".to_string()];
        assert!(guard.is_whitelisted("index.php", &whitelist));
        assert!(guard.is_whitelisted("public/index.php", &whitelist));
        assert!(!guard.is_whitelisted("evil.php", &whitelist));
    }
}
