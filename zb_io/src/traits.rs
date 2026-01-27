//! Trait abstractions for I/O operations to enable mocking in tests.
//!
//! These traits abstract over HTTP and filesystem operations, allowing tests
//! to inject mock implementations that can simulate failures, timeouts, etc.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;

use zb_core::Error;

/// HTTP client trait for abstracting network operations.
///
/// This trait allows tests to inject mock HTTP clients that can simulate
/// network failures, timeouts, and specific response scenarios.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Perform a GET request and return the response body as bytes.
    async fn get(&self, url: &str) -> Result<Vec<u8>, Error>;

    /// Perform a GET request with a timeout.
    async fn get_with_timeout(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, Error>;
}

/// Filesystem trait for abstracting file operations.
///
/// This trait allows tests to inject mock filesystem implementations that
/// can simulate disk failures, permission errors, etc.
#[cfg_attr(test, automock)]
pub trait FileSystem: Send + Sync {
    /// Write data to a file, creating it if it doesn't exist.
    fn write(&self, path: &Path, data: &[u8]) -> Result<(), Error>;

    /// Read the entire contents of a file.
    fn read(&self, path: &Path) -> Result<Vec<u8>, Error>;

    /// Create a directory and all parent directories.
    fn create_dir_all(&self, path: &Path) -> Result<(), Error>;

    /// Remove a directory and all its contents.
    fn remove_dir_all(&self, path: &Path) -> Result<(), Error>;
}

/// Real HTTP client implementation using reqwest.
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("zerobrew/0.1")
                .pool_max_idle_per_host(10)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("HTTP {}", response.status()),
            });
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })
    }

    async fn get_with_timeout(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, Error> {
        let response = self
            .client
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("HTTP {}", response.status()),
            });
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| Error::NetworkFailure {
                message: e.to_string(),
            })
    }
}

/// Real filesystem implementation using std::fs.
pub struct StdFileSystem;

impl StdFileSystem {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for StdFileSystem {
    fn write(&self, path: &Path, data: &[u8]) -> Result<(), Error> {
        std::fs::write(path, data).map_err(|e| Error::NetworkFailure {
            message: format!("failed to write to {}: {}", path.display(), e),
        })
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, Error> {
        std::fs::read(path).map_err(|e| Error::NetworkFailure {
            message: format!("failed to read {}: {}", path.display(), e),
        })
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), Error> {
        std::fs::create_dir_all(path).map_err(|e| Error::NetworkFailure {
            message: format!("failed to create directory {}: {}", path.display(), e),
        })
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), Error> {
        std::fs::remove_dir_all(path).map_err(|e| Error::NetworkFailure {
            message: format!("failed to remove directory {}: {}", path.display(), e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ========================================================================
    // Mock Implementation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_mock_http_client_returns_error() {
        let mut mock = MockHttpClient::new();
        mock.expect_get().returning(|_| {
            Err(Error::NetworkFailure {
                message: "connection timeout".into(),
            })
        });

        let result = mock.get("https://example.com/file").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NetworkFailure { message } => {
                assert!(message.contains("timeout"));
            }
            _ => panic!("expected NetworkFailure"),
        }
    }

    #[tokio::test]
    async fn test_mock_http_client_returns_data() {
        let mut mock = MockHttpClient::new();
        mock.expect_get().returning(|_| Ok(b"hello world".to_vec()));

        let result = mock.get("https://example.com/file").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"hello world");
    }

    #[tokio::test]
    async fn test_mock_http_client_with_timeout() {
        let mut mock = MockHttpClient::new();
        mock.expect_get_with_timeout().returning(|_, timeout| {
            // Verify timeout was passed
            assert_eq!(timeout, Duration::from_secs(30));
            Ok(vec![1, 2, 3])
        });

        let result = mock
            .get_with_timeout("https://example.com", Duration::from_secs(30))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_http_client_url_verification() {
        let mut mock = MockHttpClient::new();
        mock.expect_get()
            .withf(|url| url.starts_with("https://"))
            .times(1)
            .returning(|url| {
                assert_eq!(url, "https://secure.example.com/api");
                Ok(b"response".to_vec())
            });

        let result = mock.get("https://secure.example.com/api").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_http_client_multiple_calls() {
        let mut mock = MockHttpClient::new();
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_clone = call_count.clone();

        mock.expect_get().returning(move |_| {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(vec![])
        });

        mock.get("https://example.com/1").await.unwrap();
        mock.get("https://example.com/2").await.unwrap();
        mock.get("https://example.com/3").await.unwrap();

        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn test_mock_filesystem_write_error() {
        let mut mock = MockFileSystem::new();
        mock.expect_write().returning(|_, _| {
            Err(Error::NetworkFailure {
                message: "disk full".into(),
            })
        });

        let result = mock.write(Path::new("/tmp/test"), b"data");
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_filesystem_read_success() {
        let mut mock = MockFileSystem::new();
        mock.expect_read()
            .returning(|_| Ok(b"file contents".to_vec()));

        let result = mock.read(Path::new("/tmp/test"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"file contents");
    }

    #[test]
    fn test_mock_filesystem_create_dir() {
        let mut mock = MockFileSystem::new();
        mock.expect_create_dir_all().times(1).returning(|path| {
            assert!(path.to_string_lossy().contains("test_dir"));
            Ok(())
        });

        let result = mock.create_dir_all(Path::new("/tmp/test_dir/nested"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_mock_filesystem_remove_dir_success() {
        let mut mock = MockFileSystem::new();
        mock.expect_remove_dir_all().times(1).returning(|_| Ok(()));

        let result = mock.remove_dir_all(Path::new("/tmp/to_remove"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_mock_filesystem_remove_dir_error() {
        let mut mock = MockFileSystem::new();
        mock.expect_remove_dir_all().returning(|_| {
            Err(Error::NetworkFailure {
                message: "permission denied".into(),
            })
        });

        let result = mock.remove_dir_all(Path::new("/protected"));
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NetworkFailure { message } => {
                assert!(message.contains("permission denied"));
            }
            _ => panic!("expected NetworkFailure"),
        }
    }

    #[test]
    fn test_mock_filesystem_read_error() {
        let mut mock = MockFileSystem::new();
        mock.expect_read().returning(|_| {
            Err(Error::NetworkFailure {
                message: "file not found".into(),
            })
        });

        let result = mock.read(Path::new("/nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_filesystem_path_verification() {
        let mut mock = MockFileSystem::new();
        mock.expect_write()
            .withf(|path, data| path.to_string_lossy().ends_with(".txt") && !data.is_empty())
            .returning(|_, _| Ok(()));

        let result = mock.write(Path::new("/tmp/file.txt"), b"content");
        assert!(result.is_ok());
    }

    // ========================================================================
    // ReqwestHttpClient Tests (Real Implementation)
    // ========================================================================

    #[test]
    fn test_reqwest_http_client_new() {
        let client = ReqwestHttpClient::new();
        // Just verify it constructs without panic
        assert!(std::mem::size_of_val(&client) > 0);
    }

    #[test]
    fn test_reqwest_http_client_default() {
        let client = ReqwestHttpClient::default();
        // Verify Default trait works
        assert!(std::mem::size_of_val(&client) > 0);
    }

    #[test]
    fn test_reqwest_http_client_with_custom_client() {
        let custom = reqwest::Client::builder()
            .user_agent("custom-agent/1.0")
            .build()
            .unwrap();

        let client = ReqwestHttpClient::with_client(custom);
        assert!(std::mem::size_of_val(&client) > 0);
    }

    #[test]
    fn test_reqwest_http_client_new_and_default_equivalent() {
        // Both should create valid clients
        let client1 = ReqwestHttpClient::new();
        let client2 = ReqwestHttpClient::default();

        // They should both have clients (can't compare directly, but both should work)
        assert!(std::mem::size_of_val(&client1) == std::mem::size_of_val(&client2));
    }

    // ========================================================================
    // StdFileSystem Tests (Real Implementation)
    // ========================================================================

    #[test]
    fn test_std_filesystem_new() {
        let fs = StdFileSystem::new();
        assert!(std::mem::size_of_val(&fs) == 0); // Zero-sized type
    }

    #[test]
    fn test_std_filesystem_default() {
        let fs = StdFileSystem::default();
        assert!(std::mem::size_of_val(&fs) == 0);
    }

    #[test]
    fn test_std_filesystem_write_and_read() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("test_file.txt");

        // Write
        let write_result = fs.write(&path, b"hello world");
        assert!(write_result.is_ok());

        // Read back
        let read_result = fs.read(&path);
        assert!(read_result.is_ok());
        assert_eq!(read_result.unwrap(), b"hello world");
    }

    #[test]
    fn test_std_filesystem_write_binary_data() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("binary.bin");

        // Binary data with all byte values
        let binary_data: Vec<u8> = (0u8..=255u8).collect();

        let write_result = fs.write(&path, &binary_data);
        assert!(write_result.is_ok());

        let read_result = fs.read(&path);
        assert!(read_result.is_ok());
        assert_eq!(read_result.unwrap(), binary_data);
    }

    #[test]
    fn test_std_filesystem_write_empty_file() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("empty.txt");

        let write_result = fs.write(&path, b"");
        assert!(write_result.is_ok());

        let read_result = fs.read(&path);
        assert!(read_result.is_ok());
        assert!(read_result.unwrap().is_empty());
    }

    #[test]
    fn test_std_filesystem_write_overwrite() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("overwrite.txt");

        // Write initial content
        fs.write(&path, b"initial").unwrap();

        // Overwrite
        fs.write(&path, b"overwritten").unwrap();

        let content = fs.read(&path).unwrap();
        assert_eq!(content, b"overwritten");
    }

    #[test]
    fn test_std_filesystem_read_nonexistent() {
        let fs = StdFileSystem::new();
        let result = fs.read(Path::new("/nonexistent/path/file.txt"));

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NetworkFailure { message } => {
                assert!(message.contains("failed to read"));
            }
            _ => panic!("expected NetworkFailure"),
        }
    }

    #[test]
    fn test_std_filesystem_write_to_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("nonexistent_dir/file.txt");

        // Should fail because parent directory doesn't exist
        let result = fs.write(&path, b"data");
        assert!(result.is_err());
    }

    #[test]
    fn test_std_filesystem_create_dir_all() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("a/b/c/d/e");

        let result = fs.create_dir_all(&path);
        assert!(result.is_ok());
        assert!(path.exists());
        assert!(path.is_dir());
    }

    #[test]
    fn test_std_filesystem_create_dir_all_existing() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("existing");

        // Create once
        fs.create_dir_all(&path).unwrap();

        // Create again - should succeed (idempotent)
        let result = fs.create_dir_all(&path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_std_filesystem_remove_dir_all() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let dir_path = tmp.path().join("to_remove");

        // Create nested structure
        fs.create_dir_all(&dir_path.join("nested/deep")).unwrap();
        fs.write(&dir_path.join("file.txt"), b"content").unwrap();
        fs.write(&dir_path.join("nested/file.txt"), b"nested content")
            .unwrap();

        // Remove all
        let result = fs.remove_dir_all(&dir_path);
        assert!(result.is_ok());
        assert!(!dir_path.exists());
    }

    #[test]
    fn test_std_filesystem_remove_dir_all_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("does_not_exist");

        let result = fs.remove_dir_all(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::NetworkFailure { message } => {
                assert!(message.contains("failed to remove directory"));
            }
            _ => panic!("expected NetworkFailure"),
        }
    }

    #[test]
    fn test_std_filesystem_large_file() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("large.bin");

        // Create a 1MB file
        let large_data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();

        let write_result = fs.write(&path, &large_data);
        assert!(write_result.is_ok());

        let read_result = fs.read(&path);
        assert!(read_result.is_ok());
        assert_eq!(read_result.unwrap().len(), 1024 * 1024);
    }

    #[test]
    fn test_std_filesystem_unicode_path() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("日本語/файл/αρχείο.txt");

        fs.create_dir_all(path.parent().unwrap()).unwrap();
        let result = fs.write(&path, b"unicode path content");
        assert!(result.is_ok());

        let content = fs.read(&path).unwrap();
        assert_eq!(content, b"unicode path content");
    }

    #[test]
    fn test_std_filesystem_special_characters_in_content() {
        let tmp = TempDir::new().unwrap();
        let fs = StdFileSystem::new();
        let path = tmp.path().join("special.txt");

        let special_content = "null: \0, tab: \t, newline: \n, emoji: 🎉";
        let result = fs.write(&path, special_content.as_bytes());
        assert!(result.is_ok());

        let content = fs.read(&path).unwrap();
        assert_eq!(content, special_content.as_bytes());
    }

    // ========================================================================
    // Trait Object Tests (dyn Trait usage)
    // ========================================================================

    #[test]
    fn test_filesystem_as_trait_object() {
        let tmp = TempDir::new().unwrap();
        let fs: Box<dyn FileSystem> = Box::new(StdFileSystem::new());
        let path = tmp.path().join("trait_obj.txt");

        fs.write(&path, b"via trait object").unwrap();
        let content = fs.read(&path).unwrap();
        assert_eq!(content, b"via trait object");
    }

    #[test]
    fn test_mock_filesystem_as_trait_object() {
        let mut mock = MockFileSystem::new();
        mock.expect_read()
            .returning(|_| Ok(b"mocked content".to_vec()));

        let fs: Box<dyn FileSystem> = Box::new(mock);
        let content = fs.read(Path::new("/any/path")).unwrap();
        assert_eq!(content, b"mocked content");
    }

    // ========================================================================
    // Error Message Format Tests
    // ========================================================================

    #[test]
    fn test_write_error_message_format() {
        let fs = StdFileSystem::new();
        let result = fs.write(Path::new("/nonexistent_root/file.txt"), b"data");

        if let Err(Error::NetworkFailure { message }) = result {
            assert!(message.contains("failed to write to"));
            assert!(message.contains("/nonexistent_root/file.txt"));
        } else {
            panic!("expected NetworkFailure error");
        }
    }

    #[test]
    fn test_read_error_message_format() {
        let fs = StdFileSystem::new();
        let result = fs.read(Path::new("/nonexistent_root/file.txt"));

        if let Err(Error::NetworkFailure { message }) = result {
            assert!(message.contains("failed to read"));
            assert!(message.contains("/nonexistent_root/file.txt"));
        } else {
            panic!("expected NetworkFailure error");
        }
    }

    #[test]
    fn test_create_dir_error_message_format() {
        let fs = StdFileSystem::new();
        // Try to create a directory where a file already exists (on most systems)
        let result = fs.create_dir_all(Path::new("/dev/null/impossible"));

        if let Err(Error::NetworkFailure { message }) = result {
            assert!(message.contains("failed to create directory"));
        } else {
            panic!("expected NetworkFailure error");
        }
    }

    #[test]
    fn test_remove_dir_error_message_format() {
        let fs = StdFileSystem::new();
        let result = fs.remove_dir_all(Path::new("/nonexistent_path_12345"));

        if let Err(Error::NetworkFailure { message }) = result {
            assert!(message.contains("failed to remove directory"));
            assert!(message.contains("/nonexistent_path_12345"));
        } else {
            panic!("expected NetworkFailure error");
        }
    }

    // ========================================================================
    // Integration-style Mock Tests
    // ========================================================================

    #[test]
    fn test_mock_filesystem_full_workflow() {
        let mut mock = MockFileSystem::new();

        // Expect create_dir_all first
        mock.expect_create_dir_all().times(1).returning(|_| Ok(()));

        // Then expect write
        mock.expect_write().times(1).returning(|_, _| Ok(()));

        // Then expect read
        mock.expect_read()
            .times(1)
            .returning(|_| Ok(b"installed package".to_vec()));

        // Simulate installation workflow
        mock.create_dir_all(Path::new("/cellar/pkg/1.0.0")).unwrap();
        mock.write(Path::new("/cellar/pkg/1.0.0/binary"), b"ELF...")
            .unwrap();
        let content = mock.read(Path::new("/cellar/pkg/1.0.0/binary")).unwrap();
        assert_eq!(content, b"installed package");
    }

    #[tokio::test]
    async fn test_mock_http_simulates_download_flow() {
        let mut mock = MockHttpClient::new();

        // First call: get manifest
        mock.expect_get()
            .withf(|url| url.contains("manifest"))
            .times(1)
            .returning(|_| Ok(b"sha256:abc123".to_vec()));

        // Second call: get bottle
        mock.expect_get()
            .withf(|url| url.contains("bottle"))
            .times(1)
            .returning(|_| Ok(b"bottle content".to_vec()));

        let manifest = mock.get("https://example.com/manifest.json").await.unwrap();
        assert!(String::from_utf8_lossy(&manifest).contains("sha256"));

        let bottle = mock.get("https://example.com/bottle.tar.gz").await.unwrap();
        assert_eq!(bottle, b"bottle content");
    }

    // ========================================================================
    // Send + Sync Tests (trait bounds verification)
    // ========================================================================

    #[test]
    fn test_std_filesystem_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StdFileSystem>();
    }

    #[test]
    fn test_reqwest_http_client_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ReqwestHttpClient>();
    }

    #[test]
    fn test_mock_filesystem_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockFileSystem>();
    }

    #[test]
    fn test_mock_http_client_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockHttpClient>();
    }
}
