//! Patchelf binary management for Linux ELF patching.
//!
//! This module handles automatic download and caching of the patchelf binary,
//! which is required on Linux to patch ELF binaries (RPATH and interpreter).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use zb_core::Error;

/// Patchelf version to download
const PATCHELF_VERSION: &str = "0.18.0";

/// GitHub release URLs for patchelf binaries
const PATCHELF_URL_X86_64: &str =
    "https://github.com/NixOS/patchelf/releases/download/0.18.0/patchelf-0.18.0-x86_64.tar.gz";
const PATCHELF_URL_AARCH64: &str =
    "https://github.com/NixOS/patchelf/releases/download/0.18.0/patchelf-0.18.0-aarch64.tar.gz";

/// Cached path to patchelf binary (computed once per process)
static PATCHELF_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Get the path to patchelf, downloading it if necessary.
///
/// This function:
/// 1. Checks if patchelf is already in PATH
/// 2. Checks if we have a cached patchelf binary in the zerobrew directory
/// 3. Downloads patchelf from GitHub releases if not found
///
/// Returns `None` if patchelf cannot be found or downloaded.
pub fn get_patchelf_path(zerobrew_root: &Path) -> Option<PathBuf> {
    // Use cached result if available
    if let Some(cached) = PATCHELF_PATH.get() {
        return cached.clone();
    }

    let result = find_or_download_patchelf(zerobrew_root);

    // Cache the result (ignore if another thread beat us)
    let _ = PATCHELF_PATH.set(result.clone());

    result
}

/// Clear the cached patchelf path (useful for testing)
#[cfg(test)]
pub fn clear_cache() {
    // OnceLock doesn't support clearing, but tests can work around this
}

fn find_or_download_patchelf(zerobrew_root: &Path) -> Option<PathBuf> {
    // 1. Check if patchelf is in PATH
    if let Some(path) = find_patchelf_in_path() {
        return Some(path);
    }

    // 2. Check if we have a cached patchelf in zerobrew directory
    let cached_path = zerobrew_root.join("bin").join("patchelf");
    if cached_path.exists() && is_executable(&cached_path) {
        if verify_patchelf(&cached_path) {
            return Some(cached_path);
        }
        // Cached binary is broken, remove it
        let _ = fs::remove_file(&cached_path);
    }

    // 3. Download patchelf from GitHub releases
    match download_patchelf(zerobrew_root) {
        Ok(path) => Some(path),
        Err(e) => {
            eprintln!("    Warning: failed to download patchelf: {}", e);
            None
        }
    }
}

fn find_patchelf_in_path() -> Option<PathBuf> {
    // Try to run patchelf --version to check if it's available
    match Command::new("patchelf").arg("--version").output() {
        Ok(output) if output.status.success() => {
            // patchelf is in PATH, find its full path
            if let Ok(which_output) = Command::new("which").arg("patchelf").output() {
                if which_output.status.success() {
                    let path = String::from_utf8_lossy(&which_output.stdout)
                        .trim()
                        .to_string();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
            // Fallback: just use "patchelf" and let the system find it
            Some(PathBuf::from("patchelf"))
        }
        _ => None,
    }
}

fn verify_patchelf(path: &Path) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = path.metadata() {
        return meta.permissions().mode() & 0o111 != 0;
    }
    false
}

fn download_patchelf(zerobrew_root: &Path) -> Result<PathBuf, Error> {
    let arch = std::env::consts::ARCH;
    let url = match arch {
        "x86_64" => PATCHELF_URL_X86_64,
        "aarch64" => PATCHELF_URL_AARCH64,
        _ => {
            return Err(Error::StoreCorruption {
                message: format!("patchelf: unsupported architecture: {}", arch),
            });
        }
    };

    eprintln!("    Downloading patchelf {}...", PATCHELF_VERSION);

    // Create bin directory
    let bin_dir = zerobrew_root.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| Error::StoreCorruption {
        message: format!("patchelf: failed to create bin directory: {}", e),
    })?;

    // Download to a temp file
    let temp_path = bin_dir.join(".patchelf.tar.gz.tmp");
    download_file(url, &temp_path)?;

    // Extract patchelf binary from tarball
    let patchelf_path = extract_patchelf(&temp_path, &bin_dir)?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    // Verify the downloaded binary works
    if !verify_patchelf(&patchelf_path) {
        let _ = fs::remove_file(&patchelf_path);
        return Err(Error::StoreCorruption {
            message: "patchelf: downloaded binary is not functional".to_string(),
        });
    }

    eprintln!("    Installed patchelf to {}", patchelf_path.display());

    Ok(patchelf_path)
}

fn download_file(url: &str, dest: &Path) -> Result<(), Error> {
    // Use curl or wget if available
    // This is synchronous since patchelf download is a one-time bootstrap operation

    // Try curl first
    let curl_result = Command::new("curl")
        .args(["-fsSL", "-o", &dest.to_string_lossy(), url])
        .output();

    if let Ok(output) = curl_result {
        if output.status.success() {
            return Ok(());
        }
    }

    // Try wget as fallback
    let wget_result = Command::new("wget")
        .args(["-q", "-O", &dest.to_string_lossy(), url])
        .output();

    if let Ok(output) = wget_result {
        if output.status.success() {
            return Ok(());
        }
    }

    // Try Python as a last resort (usually available on Linux)
    let python_script = format!(
        r#"
import urllib.request
urllib.request.urlretrieve("{}", "{}")
"#,
        url,
        dest.to_string_lossy()
    );

    for python in ["python3", "python"] {
        let result = Command::new(python).args(["-c", &python_script]).output();

        if let Ok(output) = result {
            if output.status.success() {
                return Ok(());
            }
        }
    }

    Err(Error::StoreCorruption {
        message: "patchelf: no download tool available (tried curl, wget, python)".to_string(),
    })
}

fn extract_patchelf(tarball: &Path, dest_dir: &Path) -> Result<PathBuf, Error> {
    // The patchelf tarball contains: patchelf-VERSION-ARCH/bin/patchelf
    // We need to extract just the binary

    let output = Command::new("tar")
        .args([
            "-xzf",
            &tarball.to_string_lossy(),
            "-C",
            &dest_dir.to_string_lossy(),
            "--strip-components=2",
            "--wildcards",
            "*/bin/patchelf",
        ])
        .output()
        .map_err(|e| Error::StoreCorruption {
            message: format!("patchelf: failed to run tar: {}", e),
        })?;

    if !output.status.success() {
        // Try alternative extraction without --wildcards (some tar versions don't support it)
        let alt_output = Command::new("tar")
            .args([
                "-xzf",
                &tarball.to_string_lossy(),
                "-C",
                &dest_dir.to_string_lossy(),
            ])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("patchelf: failed to run tar: {}", e),
            })?;

        if !alt_output.status.success() {
            return Err(Error::StoreCorruption {
                message: format!(
                    "patchelf: failed to extract: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            });
        }

        // Find and move the patchelf binary
        let extracted_dir = find_extracted_patchelf_dir(dest_dir)?;
        let extracted_binary = extracted_dir.join("bin").join("patchelf");
        let final_path = dest_dir.join("patchelf");

        fs::rename(&extracted_binary, &final_path).map_err(|e| Error::StoreCorruption {
            message: format!("patchelf: failed to move binary: {}", e),
        })?;

        // Clean up extracted directory
        let _ = fs::remove_dir_all(&extracted_dir);

        return Ok(final_path);
    }

    let patchelf_path = dest_dir.join("patchelf");

    // Make executable
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&patchelf_path)
            .map_err(|e| Error::StoreCorruption {
                message: format!("patchelf: failed to get metadata: {}", e),
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&patchelf_path, perms).map_err(|e| Error::StoreCorruption {
            message: format!("patchelf: failed to make executable: {}", e),
        })?;
    }

    Ok(patchelf_path)
}

fn find_extracted_patchelf_dir(dest_dir: &Path) -> Result<PathBuf, Error> {
    for entry in fs::read_dir(dest_dir).map_err(|e| Error::StoreCorruption {
        message: format!("patchelf: failed to read directory: {}", e),
    })? {
        let entry = entry.map_err(|e| Error::StoreCorruption {
            message: format!("patchelf: failed to read directory entry: {}", e),
        })?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("patchelf-") {
                return Ok(path);
            }
        }
    }

    Err(Error::StoreCorruption {
        message: "patchelf: could not find extracted directory".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_find_patchelf_in_path() {
        // This test depends on the system having patchelf or not
        let result = find_patchelf_in_path();
        // Just verify it doesn't panic
        println!("patchelf in PATH: {:?}", result);
    }

    #[test]
    fn test_is_executable() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("test.sh");
        fs::write(&script, "#!/bin/sh\necho hello").unwrap();

        // Initially not executable
        assert!(!is_executable(&script));

        // Make executable
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }

        assert!(is_executable(&script));
    }

    #[test]
    fn test_verify_patchelf_with_nonexistent() {
        let path = PathBuf::from("/nonexistent/patchelf");
        assert!(!verify_patchelf(&path));
    }
}
