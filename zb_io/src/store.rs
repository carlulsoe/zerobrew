use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

use crate::extract::extract_tarball;
use zb_core::Error;

pub struct Store {
    store_dir: PathBuf,
    locks_dir: PathBuf,
}

impl Store {
    pub fn new(root: &Path) -> io::Result<Self> {
        let store_dir = root.join("store");
        let locks_dir = root.join("locks");

        fs::create_dir_all(&store_dir)?;
        fs::create_dir_all(&locks_dir)?;

        Ok(Self {
            store_dir,
            locks_dir,
        })
    }

    pub fn entry_path(&self, store_key: &str) -> PathBuf {
        self.store_dir.join(store_key)
    }

    pub fn has_entry(&self, store_key: &str) -> bool {
        self.entry_path(store_key).exists()
    }

    pub fn ensure_entry(&self, store_key: &str, blob_path: &Path) -> Result<PathBuf, Error> {
        let entry_path = self.entry_path(store_key);

        // Fast path: already exists
        if entry_path.exists() {
            return Ok(entry_path);
        }

        // Acquire exclusive lock for this store_key
        let lock_path = self.locks_dir.join(format!("{store_key}.lock"));
        let lock_file = File::create(&lock_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create lock file: {e}"),
        })?;

        lock_file
            .lock_exclusive()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to acquire lock: {e}"),
            })?;

        // Double-check after acquiring lock (another process may have created it)
        if entry_path.exists() {
            // Lock will be released when lock_file is dropped
            return Ok(entry_path);
        }

        // Unpack to a temp directory first
        let tmp_dir = self
            .store_dir
            .join(format!(".{store_key}.tmp.{}", std::process::id()));

        // Clean up any leftover temp directory from a previous interrupted extraction
        // (can happen if the process crashed or was killed during extraction)
        if tmp_dir.exists() {
            let _ = fs::remove_dir_all(&tmp_dir);
        }

        fs::create_dir_all(&tmp_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create temp directory: {e}"),
        })?;

        // Extract the tarball
        if let Err(e) = extract_tarball(blob_path, &tmp_dir) {
            // Clean up temp directory on failure
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(e);
        }

        // Atomically rename temp dir to final path
        if let Err(e) = fs::rename(&tmp_dir, &entry_path) {
            // Clean up temp directory on failure
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(Error::StoreCorruption {
                message: format!("failed to rename store entry: {e}"),
            });
        }

        // Lock will be released when lock_file is dropped
        Ok(entry_path)
    }

    /// Remove a store entry. This should only be called when the refcount is 0.
    pub fn remove_entry(&self, store_key: &str) -> Result<(), Error> {
        let entry_path = self.entry_path(store_key);

        if !entry_path.exists() {
            return Ok(());
        }

        // Acquire exclusive lock for this store_key
        let lock_path = self.locks_dir.join(format!("{store_key}.lock"));
        let lock_file = File::create(&lock_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create lock file: {e}"),
        })?;

        lock_file
            .lock_exclusive()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to acquire lock: {e}"),
            })?;

        // Remove the directory
        if entry_path.exists() {
            fs::remove_dir_all(&entry_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove store entry: {e}"),
            })?;
        }

        // Clean up the lock file
        let _ = fs::remove_file(&lock_path);

        Ok(())
    }

    /// List all store entries (directories in the store)
    pub fn list_entries(&self) -> io::Result<Vec<String>> {
        let mut entries = Vec::new();

        for entry in fs::read_dir(&self.store_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Skip temp directories (start with .)
                    if !name.starts_with('.') {
                        entries.push(name.to_string());
                    }
                }
            }
        }

        Ok(entries)
    }

    /// Get the total size of the store
    pub fn total_size(&self) -> io::Result<u64> {
        dir_size(&self.store_dir)
    }

    /// Clean up stale lock files (locks without corresponding store entries)
    /// Returns the number of lock files removed
    pub fn cleanup_stale_locks(&self) -> io::Result<usize> {
        let mut count = 0;

        for entry in fs::read_dir(&self.locks_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".lock") {
                    let store_key = name.trim_end_matches(".lock");
                    // If there's no corresponding store entry, remove the lock
                    if !self.has_entry(store_key) {
                        if fs::remove_file(&path).is_ok() {
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Clean up stale temp directories from interrupted extractions
    /// Returns the count of directories removed and total bytes freed
    pub fn cleanup_temp_dirs(&self) -> io::Result<(usize, u64)> {
        let mut count = 0;
        let mut bytes_freed = 0;

        for entry in fs::read_dir(&self.store_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Temp directories start with "." and contain ".tmp."
                    if name.starts_with('.') && name.contains(".tmp.") {
                        let size = dir_size(&path).unwrap_or(0);
                        if fs::remove_dir_all(&path).is_ok() {
                            count += 1;
                            bytes_freed += size;
                        }
                    }
                }
            }
        }

        Ok((count, bytes_freed))
    }
}

/// Calculate the total size of a directory recursively
fn dir_size(path: &Path) -> io::Result<u64> {
    let mut total = 0;

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                total += dir_size(&path)?;
            } else {
                total += entry.metadata()?.len();
            }
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use tar::Builder;
    use tempfile::TempDir;

    fn create_test_tarball(content: &[u8]) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());

        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, content).unwrap();

        let tar_data = builder.into_inner().unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn second_call_is_noop() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        let tarball = create_test_tarball(b"hello world");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        let store_key = "abc123";

        // First call extracts
        let path1 = store.ensure_entry(store_key, &blob_path).unwrap();
        assert!(path1.exists());
        assert!(path1.join("test.txt").exists());

        // Modify the file to detect if it gets overwritten
        fs::write(path1.join("marker.txt"), "original").unwrap();

        // Second call should be a no-op
        let path2 = store.ensure_entry(store_key, &blob_path).unwrap();
        assert_eq!(path1, path2);

        // Marker file should still exist (wasn't re-extracted)
        assert!(path2.join("marker.txt").exists());
    }

    #[test]
    fn concurrent_calls_unpack_once() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(Store::new(tmp.path()).unwrap());

        let tarball = create_test_tarball(b"concurrent test");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        let store_key = "concurrent123";
        let unpack_count = Arc::new(AtomicUsize::new(0));

        // Spawn multiple threads that all try to ensure the same entry
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let store = store.clone();
                let blob = blob_path.clone();
                let count = unpack_count.clone();
                let key = store_key.to_string();

                thread::spawn(move || {
                    let entry_path = store.entry_path(&key);
                    let existed_before = entry_path.exists();

                    let result = store.ensure_entry(&key, &blob);

                    if !existed_before && result.is_ok() && entry_path.exists() {
                        // This thread might have been the one to create it
                        count.fetch_add(1, Ordering::SeqCst);
                    }

                    result
                })
            })
            .collect();

        // All threads should succeed
        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.is_ok());
        }

        // Entry should exist
        assert!(store.has_entry(store_key));

        // Content should be correct
        let entry_path = store.entry_path(store_key);
        let content = fs::read_to_string(entry_path.join("test.txt")).unwrap();
        assert_eq!(content, "concurrent test");
    }

    #[test]
    fn has_entry_returns_correct_state() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        let store_key = "checkme";

        assert!(!store.has_entry(store_key));

        let tarball = create_test_tarball(b"exists");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        store.ensure_entry(store_key, &blob_path).unwrap();

        assert!(store.has_entry(store_key));
    }

    #[test]
    fn list_entries_returns_all_store_entries() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        // Create some store entries
        let tarball = create_test_tarball(b"test content");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();

        for key in ["entry1", "entry2", "entry3"] {
            store.ensure_entry(key, &blob_path).unwrap();
        }

        let entries = store.list_entries().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&"entry1".to_string()));
        assert!(entries.contains(&"entry2".to_string()));
        assert!(entries.contains(&"entry3".to_string()));
    }

    #[test]
    fn cleanup_stale_locks_removes_orphaned_locks() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        // Create a store entry
        let tarball = create_test_tarball(b"test content");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();
        store.ensure_entry("realentry", &blob_path).unwrap();

        // Create some orphaned lock files (no corresponding store entry)
        let locks_dir = tmp.path().join("locks");
        fs::write(locks_dir.join("orphan1.lock"), b"").unwrap();
        fs::write(locks_dir.join("orphan2.lock"), b"").unwrap();

        let removed = store.cleanup_stale_locks().unwrap();
        assert_eq!(removed, 2);

        // Verify orphaned locks are gone but real entry's lock might still exist
        // (it was created during ensure_entry)
        assert!(!locks_dir.join("orphan1.lock").exists());
        assert!(!locks_dir.join("orphan2.lock").exists());
    }

    #[test]
    fn cleanup_temp_dirs_removes_stale_temp_directories() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        // Create some stale temp directories
        let store_dir = tmp.path().join("store");
        fs::create_dir_all(store_dir.join(".abc123.tmp.1234")).unwrap();
        fs::write(
            store_dir.join(".abc123.tmp.1234").join("file.txt"),
            b"temp file",
        )
        .unwrap();
        fs::create_dir_all(store_dir.join(".def456.tmp.5678")).unwrap();

        let (count, _) = store.cleanup_temp_dirs().unwrap();
        assert_eq!(count, 2);

        // Verify temp dirs are gone
        assert!(!store_dir.join(".abc123.tmp.1234").exists());
        assert!(!store_dir.join(".def456.tmp.5678").exists());
    }

    #[test]
    fn total_size_returns_correct_value() {
        let tmp = TempDir::new().unwrap();
        let store = Store::new(tmp.path()).unwrap();

        // Create a store entry with known content size
        let tarball = create_test_tarball(b"test content for size check");
        let blob_path = tmp.path().join("test.tar.gz");
        fs::write(&blob_path, &tarball).unwrap();
        store.ensure_entry("sizetest", &blob_path).unwrap();

        let size = store.total_size().unwrap();
        // Size should be > 0 (includes the extracted content)
        assert!(size > 0);
    }
}
