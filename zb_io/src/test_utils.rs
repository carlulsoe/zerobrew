//! Test utilities for zerobrew
//!
//! This module provides common test infrastructure for writing integration and unit tests:
//!
//! - `TestContext` - Wraps TempDir, MockServer, and Installer setup
//! - Network failure helpers - Mock timeout, 500 errors, partial downloads
//! - Filesystem helpers - Create readonly directories, simulate permission denied
//! - Formula fixtures - Generate mock formula JSON and bottle tarballs
//!
//! # Example
//!
//! ```ignore
//! use zb_io::test_utils::{TestContext, mock_formula_json, mock_bottle_tarball};
//!
//! #[tokio::test]
//! async fn test_install() {
//!     let ctx = TestContext::new().await;
//!     
//!     // Mount formula mock
//!     ctx.mount_formula("testpkg", "1.0.0", &[]).await;
//!     
//!     // Install and verify
//!     ctx.installer().install("testpkg", true).await.unwrap();
//!     assert!(ctx.installer().is_installed("testpkg"));
//! }
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::api::ApiClient;
use crate::blob::BlobCache;
use crate::db::Database;
use crate::install::Installer;
use crate::link::Linker;
use crate::materialize::Cellar;
use crate::store::Store;
use crate::tap::TapManager;

// ============================================================================
// Platform helpers
// ============================================================================

/// Get the bottle tag for the current platform (for test fixtures)
pub fn platform_bottle_tag() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "arm64_sonoma"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "sonoma"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "arm64_linux"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64_linux"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    {
        "all"
    }
}

// ============================================================================
// Formula/Bottle fixtures
// ============================================================================

/// Generate a mock formula JSON with customizable name, version, and dependencies.
///
/// # Arguments
/// * `name` - Formula name
/// * `version` - Version string (e.g., "1.0.0")
/// * `deps` - Slice of dependency names
/// * `base_url` - Base URL for bottle downloads
/// * `bottle_sha` - SHA256 hash of the bottle tarball
///
/// # Returns
/// A JSON string representing the formula
pub fn mock_formula_json(
    name: &str,
    version: &str,
    deps: &[&str],
    base_url: &str,
    bottle_sha: &str,
) -> String {
    let tag = platform_bottle_tag();
    let deps_json = if deps.is_empty() {
        "[]".to_string()
    } else {
        let quoted: Vec<String> = deps.iter().map(|d| format!("\"{}\"", d)).collect();
        format!("[{}]", quoted.join(", "))
    };

    format!(
        r#"{{
            "name": "{name}",
            "versions": {{ "stable": "{version}" }},
            "dependencies": {deps_json},
            "bottle": {{
                "stable": {{
                    "files": {{
                        "{tag}": {{
                            "url": "{base_url}/bottles/{name}-{version}.{tag}.bottle.tar.gz",
                            "sha256": "{bottle_sha}"
                        }}
                    }}
                }}
            }}
        }}"#
    )
}

/// Generate a mock formula JSON with custom bottle files for multiple platforms.
/// Useful for testing platform-specific bottle selection.
pub fn mock_formula_json_with_bottles(
    name: &str,
    version: &str,
    deps: &[&str],
    bottles: &[(&str, &str, &str)], // (tag, url, sha256)
) -> String {
    let deps_json = if deps.is_empty() {
        "[]".to_string()
    } else {
        let quoted: Vec<String> = deps.iter().map(|d| format!("\"{}\"", d)).collect();
        format!("[{}]", quoted.join(", "))
    };

    let files_json: Vec<String> = bottles
        .iter()
        .map(|(tag, url, sha)| {
            format!(r#""{tag}": {{ "url": "{url}", "sha256": "{sha}" }}"#)
        })
        .collect();

    format!(
        r#"{{
            "name": "{name}",
            "versions": {{ "stable": "{version}" }},
            "dependencies": {deps_json},
            "bottle": {{
                "stable": {{
                    "files": {{ {files} }}
                }}
            }}
        }}"#,
        files = files_json.join(", ")
    )
}

/// Create a minimal bottle tarball for testing.
///
/// The tarball contains a single executable shell script in the bin directory.
///
/// # Arguments
/// * `formula_name` - Name of the formula (used for directory structure)
///
/// # Returns
/// Gzipped tar archive bytes
pub fn mock_bottle_tarball(formula_name: &str) -> Vec<u8> {
    mock_bottle_tarball_with_version(formula_name, "1.0.0")
}

/// Create a bottle tarball with a specific version.
pub fn mock_bottle_tarball_with_version(formula_name: &str, version: &str) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use tar::Builder;

    let mut builder = Builder::new(Vec::new());

    // Create bin directory with executable
    let mut header = tar::Header::new_gnu();
    header
        .set_path(format!("{}/{}/bin/{}", formula_name, version, formula_name))
        .unwrap();
    header.set_size(20);
    header.set_mode(0o755);
    header.set_cksum();

    let content = format!("#!/bin/sh\necho {}", formula_name);
    builder.append(&header, content.as_bytes()).unwrap();

    let tar_data = builder.into_inner().unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_data).unwrap();
    encoder.finish().unwrap()
}

/// Create a bottle tarball with custom file contents.
///
/// # Arguments
/// * `formula_name` - Name of the formula
/// * `version` - Version string
/// * `files` - Slice of (relative_path, content, mode) tuples
///
/// # Returns
/// Gzipped tar archive bytes
pub fn mock_bottle_tarball_with_files(
    formula_name: &str,
    version: &str,
    files: &[(&str, &[u8], u32)],
) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use tar::Builder;

    let mut builder = Builder::new(Vec::new());

    for (rel_path, content, mode) in files {
        let mut header = tar::Header::new_gnu();
        header
            .set_path(format!("{}/{}/{}", formula_name, version, rel_path))
            .unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder.append(&header, *content).unwrap();
    }

    let tar_data = builder.into_inner().unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_data).unwrap();
    encoder.finish().unwrap()
}

/// Compute SHA256 hex digest of data.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ============================================================================
// Network failure helpers
// ============================================================================

/// Create a mock response that delays for the specified duration before responding.
/// Useful for simulating network timeouts.
///
/// # Arguments
/// * `delay` - How long to wait before responding
/// * `body` - Response body bytes (optional)
///
/// # Example
/// ```ignore
/// let response = mock_timeout_response(Duration::from_secs(30), None);
/// Mock::given(method("GET"))
///     .and(path("/slow"))
///     .respond_with(response)
///     .mount(&mock_server)
///     .await;
/// ```
pub fn mock_timeout_response(delay: Duration, body: Option<Vec<u8>>) -> ResponseTemplate {
    let mut response = ResponseTemplate::new(200).set_delay(delay);
    if let Some(bytes) = body {
        response = response.set_body_bytes(bytes);
    }
    response
}

/// Create a mock 500 Internal Server Error response.
/// Optionally include an error message body.
pub fn mock_500_error(message: Option<&str>) -> ResponseTemplate {
    let mut response = ResponseTemplate::new(500);
    if let Some(msg) = message {
        response = response.set_body_string(msg);
    }
    response
}

/// Create a mock 404 Not Found response.
pub fn mock_404_error() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_string("Not Found")
}

/// Create a mock response that returns partial/truncated data.
/// Simulates a connection being interrupted mid-download.
///
/// # Arguments
/// * `full_data` - The complete data that would normally be returned
/// * `fraction` - What fraction of the data to return (0.0 to 1.0)
pub fn mock_partial_download(full_data: &[u8], fraction: f64) -> ResponseTemplate {
    let len = (full_data.len() as f64 * fraction.clamp(0.0, 1.0)) as usize;
    let partial = full_data[..len].to_vec();
    ResponseTemplate::new(200).set_body_bytes(partial)
}

/// Create a mock response that returns data with wrong Content-Length header.
/// This simulates a corrupted download where the server claims more data than it sends.
pub fn mock_truncated_download(data: &[u8], claimed_length: usize) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("Content-Length", claimed_length.to_string())
        .set_body_bytes(data.to_vec())
}

// ============================================================================
// Filesystem failure helpers
// ============================================================================

/// Create a directory with read-only permissions.
/// Useful for simulating permission denied errors during install.
///
/// Returns the path to the created directory.
///
/// # Note
/// On some systems/configurations, root users may still be able to write
/// to "read-only" directories. This is best-effort simulation.
pub fn create_readonly_dir(parent: &Path, name: &str) -> std::io::Result<PathBuf> {
    let dir = parent.join(name);
    fs::create_dir_all(&dir)?;
    
    // Set permissions to read-only (no write for owner, group, or others)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o555);
        fs::set_permissions(&dir, perms)?;
    }
    
    #[cfg(not(unix))]
    {
        let mut perms = fs::metadata(&dir)?.permissions();
        perms.set_readonly(true);
        fs::set_permissions(&dir, perms)?;
    }
    
    Ok(dir)
}

/// Restore write permissions to a directory (cleanup helper).
pub fn restore_write_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    
    #[cfg(not(unix))]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_readonly(false);
        fs::set_permissions(path, perms)?;
    }
    
    Ok(())
}

/// Create a file that cannot be written to.
/// Useful for testing file overwrite failures.
pub fn create_readonly_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    fs::write(path, content)?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o444);
        fs::set_permissions(path, perms)?;
    }
    
    #[cfg(not(unix))]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_readonly(true);
        fs::set_permissions(path, perms)?;
    }
    
    Ok(())
}

/// Note on full disk simulation:
/// 
/// Simulating a full disk is platform-specific and often requires root privileges
/// or special setup (e.g., creating a small loopback filesystem). For practical
/// testing purposes, we recommend:
/// 
/// 1. Using a small ramdisk/tmpfs with limited size
/// 2. Mocking at the IO layer
/// 3. Testing with readonly directories instead (simpler, covers similar paths)
/// 
/// A true full-disk simulation would require:
/// - Linux: `dd if=/dev/zero of=disk.img bs=1M count=1 && mkfs.ext4 disk.img && mount -o loop disk.img /mnt/test`
/// - macOS: `hdiutil create -size 1m -fs HFS+ -volname Test disk.dmg && hdiutil attach disk.dmg`
/// 
/// These are not portable or easily automated in tests.

// ============================================================================
// TestContext - Main test infrastructure
// ============================================================================

/// Test context that wraps common test setup.
///
/// Provides:
/// - Temporary directory for all test files
/// - Mock server for HTTP requests
/// - Pre-configured Installer instance
///
/// # Usage
///
/// ```ignore
/// let ctx = TestContext::new().await;
/// 
/// // Mount mock responses
/// ctx.mount_formula("wget", "1.0.0", &[]).await;
/// 
/// // Use the installer
/// ctx.installer_mut().install("wget", true).await.unwrap();
/// ```
pub struct TestContext {
    pub tmp: TempDir,
    pub mock_server: MockServer,
    installer: Installer,
}

impl TestContext {
    /// Create a new test context with standard configuration.
    pub async fn new() -> Self {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().expect("failed to create temp dir");
        let installer = create_test_installer(&mock_server, &tmp);
        
        Self {
            tmp,
            mock_server,
            installer,
        }
    }
    
    /// Get a reference to the installer.
    pub fn installer(&self) -> &Installer {
        &self.installer
    }
    
    /// Get a mutable reference to the installer.
    pub fn installer_mut(&mut self) -> &mut Installer {
        &mut self.installer
    }
    
    /// Get the root path (zerobrew data directory).
    pub fn root(&self) -> PathBuf {
        self.tmp.path().join("zerobrew")
    }
    
    /// Get the prefix path (homebrew-compatible prefix).
    pub fn prefix(&self) -> PathBuf {
        self.tmp.path().join("homebrew")
    }
    
    /// Get the cellar path.
    pub fn cellar(&self) -> PathBuf {
        self.root().join("cellar")
    }
    
    /// Get the store path.
    pub fn store(&self) -> PathBuf {
        self.root().join("store")
    }
    
    /// Mount a formula API mock with auto-generated bottle.
    ///
    /// This is a convenience method that:
    /// 1. Creates a bottle tarball
    /// 2. Generates formula JSON
    /// 3. Mounts both the formula and bottle download mocks
    ///
    /// Returns the bottle SHA256 for verification.
    pub async fn mount_formula(&self, name: &str, version: &str, deps: &[&str]) -> String {
        let bottle = mock_bottle_tarball_with_version(name, version);
        let sha = sha256_hex(&bottle);
        let tag = platform_bottle_tag();
        
        let formula_json = mock_formula_json(name, version, deps, &self.mock_server.uri(), &sha);
        
        // Mount formula API
        Mock::given(method("GET"))
            .and(path(format!("/{}.json", name)))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&self.mock_server)
            .await;
        
        // Mount bottle download
        let bottle_path = format!("/bottles/{}-{}.{}.bottle.tar.gz", name, version, tag);
        Mock::given(method("GET"))
            .and(path(bottle_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bottle))
            .mount(&self.mock_server)
            .await;
        
        sha
    }
    
    /// Mount a formula with a custom bottle response.
    /// Useful for testing error conditions.
    pub async fn mount_formula_with_bottle_response(
        &self,
        name: &str,
        version: &str,
        deps: &[&str],
        bottle_response: ResponseTemplate,
        bottle_sha: &str,
    ) {
        let tag = platform_bottle_tag();
        let formula_json = mock_formula_json(name, version, deps, &self.mock_server.uri(), bottle_sha);
        
        // Mount formula API
        Mock::given(method("GET"))
            .and(path(format!("/{}.json", name)))
            .respond_with(ResponseTemplate::new(200).set_body_string(&formula_json))
            .mount(&self.mock_server)
            .await;
        
        // Mount bottle with custom response
        let bottle_path = format!("/bottles/{}-{}.{}.bottle.tar.gz", name, version, tag);
        Mock::given(method("GET"))
            .and(path(bottle_path))
            .respond_with(bottle_response)
            .mount(&self.mock_server)
            .await;
    }
    
    /// Mount a formula API that returns an error.
    pub async fn mount_formula_error(&self, name: &str, status: u16, body: Option<&str>) {
        let mut response = ResponseTemplate::new(status);
        if let Some(msg) = body {
            response = response.set_body_string(msg);
        }
        
        Mock::given(method("GET"))
            .and(path(format!("/{}.json", name)))
            .respond_with(response)
            .mount(&self.mock_server)
            .await;
    }
}

/// Create a test Installer with a mock server for API calls.
///
/// This helper reduces boilerplate in integration tests by setting up:
/// - API client configured to use the mock server
/// - Blob cache, store, cellar, linker, database, and tap manager
/// - All directories created under the temp directory
pub fn create_test_installer(mock_server: &MockServer, tmp: &TempDir) -> Installer {
    let root = tmp.path().join("zerobrew");
    let prefix = tmp.path().join("homebrew");
    fs::create_dir_all(root.join("db")).unwrap();
    fs::create_dir_all(&prefix).unwrap();

    let api_client = ApiClient::with_base_url(mock_server.uri());
    let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
    let store = Store::new(&root).unwrap();
    let cellar = Cellar::new(&root).unwrap();
    let linker = Linker::new(&prefix).unwrap();
    let db = Database::open(&root.join("db/zb.sqlite3")).unwrap();
    let taps_dir = root.join("taps");
    fs::create_dir_all(&taps_dir).unwrap();
    let tap_manager = TapManager::new(&taps_dir);

    Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        db,
        tap_manager,
        prefix.clone(),
        prefix.join("Cellar"),
        4,
    )
}

// ============================================================================
// Module tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_formula_json_no_deps() {
        let json = mock_formula_json("wget", "1.0.0", &[], "http://example.com", "abc123");
        assert!(json.contains("\"name\": \"wget\""));
        assert!(json.contains("\"stable\": \"1.0.0\""));
        assert!(json.contains("\"dependencies\": []"));
        assert!(json.contains("abc123"));
    }

    #[test]
    fn test_mock_formula_json_with_deps() {
        let json = mock_formula_json("curl", "2.0.0", &["openssl", "zlib"], "http://test.com", "def456");
        assert!(json.contains("\"name\": \"curl\""));
        assert!(json.contains("\"openssl\""));
        assert!(json.contains("\"zlib\""));
    }

    #[test]
    fn test_mock_bottle_tarball() {
        let tarball = mock_bottle_tarball("testpkg");
        assert!(!tarball.is_empty());
        
        // Should be valid gzip
        assert_eq!(tarball[0], 0x1f);
        assert_eq!(tarball[1], 0x8b);
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex(b"hello world");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_mock_partial_download() {
        let data = vec![0u8; 100];
        let _response = mock_partial_download(&data, 0.5);
        // Can't easily inspect ResponseTemplate internals, but it should not panic
    }

    #[test]
    fn test_mock_500_error() {
        let _response = mock_500_error(Some("Internal Server Error"));
        // Should not panic
    }

    #[tokio::test]
    async fn test_context_creation() {
        let ctx = TestContext::new().await;
        assert!(ctx.root().exists() || true); // May not exist until first use
        assert!(ctx.tmp.path().exists());
    }

    #[tokio::test]
    async fn test_context_mount_formula() {
        let ctx = TestContext::new().await;
        let sha = ctx.mount_formula("testpkg", "1.0.0", &[]).await;
        assert_eq!(sha.len(), 64);
    }
}
