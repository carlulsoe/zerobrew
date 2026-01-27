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

        response.bytes().await.map(|b| b.to_vec()).map_err(|e| {
            Error::NetworkFailure {
                message: e.to_string(),
            }
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

        response.bytes().await.map(|b| b.to_vec()).map_err(|e| {
            Error::NetworkFailure {
                message: e.to_string(),
            }
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
        mock.expect_get()
            .returning(|_| Ok(b"hello world".to_vec()));

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
        mock.expect_create_dir_all()
            .times(1)
            .returning(|path| {
                assert!(path.to_string_lossy().contains("test_dir"));
                Ok(())
            });

        let result = mock.create_dir_all(Path::new("/tmp/test_dir/nested"));
        assert!(result.is_ok());
    }
}
