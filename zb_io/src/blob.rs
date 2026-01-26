use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zb_core::Error;

#[derive(Clone)]
pub struct BlobCache {
    blobs_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl BlobCache {
    pub fn new(cache_root: &Path) -> io::Result<Self> {
        let blobs_dir = cache_root.join("blobs");
        let tmp_dir = cache_root.join("tmp");

        fs::create_dir_all(&blobs_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        Ok(Self { blobs_dir, tmp_dir })
    }

    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        self.blobs_dir.join(format!("{sha256}.tar.gz"))
    }

    pub fn has_blob(&self, sha256: &str) -> bool {
        self.blob_path(sha256).exists()
    }

    /// Remove a blob from the cache (used when extraction fails due to corruption)
    pub fn remove_blob(&self, sha256: &str) -> io::Result<bool> {
        let path = self.blob_path(sha256);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn start_write(&self, sha256: &str) -> io::Result<BlobWriter> {
        let final_path = self.blob_path(sha256);
        // Use unique temp filename to avoid corruption from concurrent racing downloads
        let unique_id = std::process::id();
        let thread_id = std::thread::current().id();
        let tmp_path = self
            .tmp_dir
            .join(format!("{sha256}.{unique_id}.{thread_id:?}.tar.gz.part"));

        let file = fs::File::create(&tmp_path)?;

        Ok(BlobWriter {
            file,
            tmp_path,
            final_path,
            committed: false,
        })
    }

    /// List all blobs in the cache, returning (sha256, modified_time) pairs
    pub fn list_blobs(&self) -> io::Result<Vec<(String, std::time::SystemTime)>> {
        let mut blobs = Vec::new();

        for entry in fs::read_dir(&self.blobs_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.ends_with(".tar.gz")
            {
                let sha256 = name.trim_end_matches(".tar.gz").to_string();
                if let Ok(metadata) = entry.metadata()
                    && let Ok(mtime) = metadata.modified()
                {
                    blobs.push((sha256, mtime));
                }
            }
        }

        Ok(blobs)
    }

    /// Get the total size of all blobs in the cache
    pub fn total_size(&self) -> io::Result<u64> {
        let mut total = 0;

        for entry in fs::read_dir(&self.blobs_dir)? {
            let entry = entry?;
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
            {
                total += metadata.len();
            }
        }

        Ok(total)
    }

    /// Remove blobs older than the given duration
    /// Returns the list of removed sha256 hashes and the total bytes freed
    pub fn remove_blobs_older_than(
        &self,
        max_age: std::time::Duration,
    ) -> io::Result<(Vec<String>, u64)> {
        let now = std::time::SystemTime::now();
        let mut removed = Vec::new();
        let mut bytes_freed = 0;

        for entry in fs::read_dir(&self.blobs_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.ends_with(".tar.gz")
                && let Ok(metadata) = entry.metadata()
                && let Ok(mtime) = metadata.modified()
                && let Ok(age) = now.duration_since(mtime)
                && age > max_age
            {
                let size = metadata.len();
                if fs::remove_file(&path).is_ok() {
                    let sha256 = name.trim_end_matches(".tar.gz").to_string();
                    removed.push(sha256);
                    bytes_freed += size;
                }
            }
        }

        Ok((removed, bytes_freed))
    }

    /// Remove all blobs except those in the keep_set
    /// Returns the list of removed sha256 hashes and the total bytes freed
    pub fn remove_blobs_except(
        &self,
        keep_set: &std::collections::HashSet<String>,
    ) -> io::Result<(Vec<String>, u64)> {
        let mut removed = Vec::new();
        let mut bytes_freed = 0;

        for entry in fs::read_dir(&self.blobs_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.ends_with(".tar.gz")
            {
                let sha256 = name.trim_end_matches(".tar.gz").to_string();
                if !keep_set.contains(&sha256)
                    && let Ok(metadata) = entry.metadata()
                {
                    let size = metadata.len();
                    if fs::remove_file(&path).is_ok() {
                        removed.push(sha256);
                        bytes_freed += size;
                    }
                }
            }
        }

        Ok((removed, bytes_freed))
    }

    /// Clean up any stale temp files in the tmp directory
    /// Returns the number of files removed and bytes freed
    pub fn cleanup_temp_files(&self) -> io::Result<(usize, u64)> {
        let mut count = 0;
        let mut bytes_freed = 0;

        for entry in fs::read_dir(&self.tmp_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only remove .part files (incomplete downloads)
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.ends_with(".part")
                && let Ok(metadata) = entry.metadata()
            {
                let size = metadata.len();
                if fs::remove_file(&path).is_ok() {
                    count += 1;
                    bytes_freed += size;
                }
            }
        }

        Ok((count, bytes_freed))
    }
}

pub struct BlobWriter {
    file: fs::File,
    tmp_path: PathBuf,
    final_path: PathBuf,
    committed: bool,
}

impl BlobWriter {
    pub fn commit(mut self) -> Result<PathBuf, Error> {
        self.file.flush().map_err(|e| Error::NetworkFailure {
            message: format!("failed to flush blob: {e}"),
        })?;

        // Another racing download may have already created the final blob.
        // In that case, just clean up our temp file and return success.
        if self.final_path.exists() {
            let _ = fs::remove_file(&self.tmp_path);
            self.committed = true;
            return Ok(self.final_path.clone());
        }

        // Try to atomically rename. If it fails because the file already exists
        // (race with another download), that's fine - clean up and return success.
        match fs::rename(&self.tmp_path, &self.final_path) {
            Ok(()) => {}
            Err(_e) if self.final_path.exists() => {
                // Another download won the race, clean up our temp file
                let _ = fs::remove_file(&self.tmp_path);
            }
            Err(e) => {
                return Err(Error::NetworkFailure {
                    message: format!("failed to rename blob: {e}"),
                });
            }
        }

        self.committed = true;
        Ok(self.final_path.clone())
    }
}

impl Write for BlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for BlobWriter {
    fn drop(&mut self) {
        if !self.committed && self.tmp_path.exists() {
            let _ = fs::remove_file(&self.tmp_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn completed_write_produces_final_blob() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        let sha = "abc123";
        let mut writer = cache.start_write(sha).unwrap();
        writer.write_all(b"hello world").unwrap();

        let final_path = writer.commit().unwrap();

        assert!(final_path.exists());
        assert!(cache.has_blob(sha));
        assert_eq!(fs::read_to_string(&final_path).unwrap(), "hello world");
    }

    #[test]
    fn interrupted_write_leaves_no_final_blob() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        let sha = "def456";

        {
            let mut writer = cache.start_write(sha).unwrap();
            writer.write_all(b"partial data").unwrap();
            // writer is dropped without calling commit()
        }

        // Final blob should not exist
        assert!(!cache.has_blob(sha));

        // Temp file should be cleaned up (temp files now have unique suffixes)
        let tmp_dir = tmp.path().join("tmp");
        let has_temp_files = fs::read_dir(&tmp_dir)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(sha));
        assert!(!has_temp_files, "temp files for {sha} should be cleaned up");
    }

    #[test]
    fn blob_path_uses_sha256() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        let path = cache.blob_path("deadbeef");
        assert!(path.to_string_lossy().contains("deadbeef.tar.gz"));
    }

    #[test]
    fn remove_blob_deletes_existing_blob() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        let sha = "removeme";
        let mut writer = cache.start_write(sha).unwrap();
        writer.write_all(b"corrupt data").unwrap();
        writer.commit().unwrap();

        assert!(cache.has_blob(sha));

        let removed = cache.remove_blob(sha).unwrap();
        assert!(removed);
        assert!(!cache.has_blob(sha));
    }

    #[test]
    fn remove_blob_returns_false_for_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        let removed = cache.remove_blob("nonexistent").unwrap();
        assert!(!removed);
    }

    #[test]
    fn list_blobs_returns_all_blobs() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        // Create some blobs
        for sha in ["sha1", "sha2", "sha3"] {
            let mut writer = cache.start_write(sha).unwrap();
            writer.write_all(b"test data").unwrap();
            writer.commit().unwrap();
        }

        let blobs = cache.list_blobs().unwrap();
        assert_eq!(blobs.len(), 3);

        let sha_list: Vec<&str> = blobs.iter().map(|(sha, _)| sha.as_str()).collect();
        assert!(sha_list.contains(&"sha1"));
        assert!(sha_list.contains(&"sha2"));
        assert!(sha_list.contains(&"sha3"));
    }

    #[test]
    fn total_size_returns_correct_size() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        // Create two blobs with known sizes
        let mut writer = cache.start_write("blob1").unwrap();
        writer.write_all(b"12345").unwrap(); // 5 bytes
        writer.commit().unwrap();

        let mut writer = cache.start_write("blob2").unwrap();
        writer.write_all(b"1234567890").unwrap(); // 10 bytes
        writer.commit().unwrap();

        let total = cache.total_size().unwrap();
        assert_eq!(total, 15);
    }

    #[test]
    fn remove_blobs_except_keeps_specified() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        // Create blobs
        for sha in ["keep1", "keep2", "remove1", "remove2"] {
            let mut writer = cache.start_write(sha).unwrap();
            writer.write_all(b"test data").unwrap();
            writer.commit().unwrap();
        }

        // Keep set
        let keep: std::collections::HashSet<String> =
            ["keep1", "keep2"].iter().map(|s| s.to_string()).collect();

        let (removed, _) = cache.remove_blobs_except(&keep).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"remove1".to_string()));
        assert!(removed.contains(&"remove2".to_string()));

        // Verify kept blobs still exist
        assert!(cache.has_blob("keep1"));
        assert!(cache.has_blob("keep2"));
        assert!(!cache.has_blob("remove1"));
        assert!(!cache.has_blob("remove2"));
    }

    #[test]
    fn cleanup_temp_files_removes_part_files() {
        let tmp = TempDir::new().unwrap();
        let cache = BlobCache::new(tmp.path()).unwrap();

        // Create some fake temp files
        let tmp_dir = tmp.path().join("tmp");
        fs::write(
            tmp_dir.join("abc123.1234.thread1.tar.gz.part"),
            b"incomplete",
        )
        .unwrap();
        fs::write(
            tmp_dir.join("def456.5678.thread2.tar.gz.part"),
            b"also incomplete",
        )
        .unwrap();

        let (count, _) = cache.cleanup_temp_files().unwrap();
        assert_eq!(count, 2);

        // Verify temp files are gone
        assert!(fs::read_dir(&tmp_dir).unwrap().count() == 0);
    }
}
