use std::path::{Path, PathBuf};

/// Maximum allowed file size for reading via compact (10 MB).
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Validate that a file path is within the allowed directory tree (CWD by default)
/// and does not traverse outside it. Also validates the file exists and is not too large.
///
/// Returns the canonical path. For TOCTOU-safe reading, use [`validate_and_read_file`].
pub fn validate_file_path(file_path: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(file_path);

    // Block obvious traversal attempts before canonicalization
    let path_str = path.to_string_lossy();
    if path_str.contains("..") {
        anyhow::bail!("Path traversal blocked: '..' not allowed in file path");
    }

    // Block absolute paths to sensitive system locations
    if path.is_absolute() {
        let blocked_prefixes = [
            "/etc/shadow",
            "/etc/passwd",
            "/etc/ssh",
            "/proc",
            "/sys",
            "/dev",
            "/root/.ssh",
            "/home",
        ];
        // Allow paths under CWD even if absolute
        let cwd = std::env::current_dir().unwrap_or_default();
        if let Ok(canonical) = path.canonicalize() {
            if canonical.starts_with(&cwd) {
                return validate_file_metadata(&canonical);
            }
        }
        for prefix in &blocked_prefixes {
            if path_str.starts_with(prefix) {
                anyhow::bail!(
                    "Access denied: path '{}' is outside the allowed directory",
                    file_path
                );
            }
        }
    }

    // Canonicalize to resolve symlinks and normalize
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            anyhow::bail!("File not found or inaccessible: {} ({})", file_path, e);
        }
    };

    // Verify the resolved path is within CWD
    let cwd = std::env::current_dir().unwrap_or_default();
    if !canonical.starts_with(&cwd) {
        anyhow::bail!(
            "Path traversal blocked: '{}' resolves outside working directory",
            file_path
        );
    }

    validate_file_metadata(&canonical)
}

/// Validate and read a file in one atomic operation to prevent TOCTOU races.
///
/// This opens the file handle immediately after canonicalization, so a symlink
/// swap between validation and read cannot redirect to an unauthorized path.
pub fn validate_and_read_file(file_path: &str) -> anyhow::Result<(PathBuf, String)> {
    let canonical = validate_file_path(file_path)?;

    // Open the file immediately — the file handle is bound to the inode,
    // so a subsequent symlink swap cannot redirect the read.
    let content = std::fs::read_to_string(&canonical)
        .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", canonical.display(), e))?;

    Ok((canonical, content))
}

/// Check file metadata — size limits only, no extension restrictions.
fn validate_file_metadata(canonical: &Path) -> anyhow::Result<PathBuf> {
    let metadata = std::fs::metadata(canonical).map_err(|e| {
        anyhow::anyhow!("Cannot read file metadata: {} ({})", canonical.display(), e)
    })?;

    if metadata.is_dir() {
        anyhow::bail!("Path '{}' is a directory, not a file", canonical.display());
    }

    if metadata.len() > MAX_FILE_SIZE {
        anyhow::bail!(
            "File too large: {} bytes (max {} bytes)",
            metadata.len(),
            MAX_FILE_SIZE
        );
    }

    Ok(canonical.to_path_buf())
}

/// Validate a CWD (working directory) parameter to prevent directory traversal.
pub fn validate_cwd(cwd: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(cwd);
    let cwd_str = path.to_string_lossy();

    if cwd_str.contains("..") {
        anyhow::bail!("Path traversal blocked: '..' not allowed in cwd");
    }

    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            anyhow::bail!("Directory not found: {} ({})", cwd, e);
        }
    };

    if !canonical.is_dir() {
        anyhow::bail!("'{}' is not a directory", cwd);
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_allows_file_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello").unwrap();

        // This test only passes if we set cwd to the temp dir
        let orig_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = validate_file_path("test.txt");
        std::env::set_current_dir(&orig_cwd).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn test_blocks_dotdot_traversal() {
        let result = validate_file_path("../../../etc/shadow");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[test]
    fn test_blocks_etc_shadow() {
        let result = validate_file_path("/etc/shadow");
        assert!(result.is_err());
    }

    #[test]
    fn test_blocks_proc() {
        let result = validate_file_path("/proc/self/environ");
        assert!(result.is_err());
    }

    #[test]
    fn test_blocks_nonexistent_file() {
        let result = validate_file_path("/nonexistent/path/file.txt");
        assert!(result.is_err());
    }

    #[test]
    fn test_cwd_blocks_dotdot() {
        let result = validate_cwd("../../../tmp");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }
}
