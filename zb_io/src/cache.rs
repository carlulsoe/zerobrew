use rusqlite::{Connection, params};
use std::path::Path;

pub struct ApiCache {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: String,
}

impl ApiCache {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    pub fn in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS api_cache (
                url TEXT PRIMARY KEY,
                etag TEXT,
                last_modified TEXT,
                body TEXT NOT NULL,
                cached_at INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    pub fn get(&self, url: &str) -> Option<CacheEntry> {
        self.conn
            .query_row(
                "SELECT etag, last_modified, body FROM api_cache WHERE url = ?1",
                params![url],
                |row| {
                    Ok(CacheEntry {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                        body: row.get(2)?,
                    })
                },
            )
            .ok()
    }

    pub fn put(&self, url: &str, entry: &CacheEntry) -> Result<(), rusqlite::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn.execute(
            "INSERT OR REPLACE INTO api_cache (url, etag, last_modified, body, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![url, entry.etag, entry.last_modified, entry.body, now],
        )?;
        Ok(())
    }

    /// Remove all cache entries older than the specified number of days
    /// Returns the number of entries removed
    pub fn cleanup_older_than(&self, days: u32) -> Result<usize, rusqlite::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let cutoff = now - (days as i64 * 24 * 60 * 60);

        let rows_affected = self.conn.execute(
            "DELETE FROM api_cache WHERE cached_at < ?1",
            params![cutoff],
        )?;

        Ok(rows_affected)
    }

    /// Remove all cache entries
    /// Returns the number of entries removed
    pub fn clear(&self) -> Result<usize, rusqlite::Error> {
        let rows_affected = self.conn.execute("DELETE FROM api_cache", [])?;
        Ok(rows_affected)
    }

    /// Count the number of entries in the cache
    pub fn count(&self) -> Result<usize, rusqlite::Error> {
        self.conn
            .query_row("SELECT COUNT(*) FROM api_cache", [], |row| {
                row.get::<_, i64>(0).map(|n| n as usize)
            })
    }

    /// Count entries older than the specified number of days
    pub fn count_older_than(&self, days: u32) -> Result<usize, rusqlite::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let cutoff = now - (days as i64 * 24 * 60 * 60);

        self.conn.query_row(
            "SELECT COUNT(*) FROM api_cache WHERE cached_at < ?1",
            params![cutoff],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    /// Get the total size of all cached bodies in bytes
    pub fn total_body_size(&self) -> Result<u64, rusqlite::Error> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(LENGTH(body)), 0) FROM api_cache",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as u64),
        )
    }

    /// Get the total size of cached bodies older than the specified days
    pub fn body_size_older_than(&self, days: u32) -> Result<u64, rusqlite::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let cutoff = now - (days as i64 * 24 * 60 * 60);

        self.conn.query_row(
            "SELECT COALESCE(SUM(LENGTH(body)), 0) FROM api_cache WHERE cached_at < ?1",
            params![cutoff],
            |row| row.get::<_, i64>(0).map(|n| n as u64),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_retrieves_cache_entry() {
        let cache = ApiCache::in_memory().unwrap();

        let entry = CacheEntry {
            etag: Some("abc123".to_string()),
            last_modified: None,
            body: r#"{"name":"foo"}"#.to_string(),
        };

        cache.put("https://example.com/foo.json", &entry).unwrap();
        let retrieved = cache.get("https://example.com/foo.json").unwrap();

        assert_eq!(retrieved.etag, Some("abc123".to_string()));
        assert_eq!(retrieved.body, r#"{"name":"foo"}"#);
    }

    #[test]
    fn returns_none_for_missing_entry() {
        let cache = ApiCache::in_memory().unwrap();
        assert!(cache.get("https://example.com/nonexistent.json").is_none());
    }

    #[test]
    fn count_returns_number_of_entries() {
        let cache = ApiCache::in_memory().unwrap();
        assert_eq!(cache.count().unwrap(), 0);

        let entry = CacheEntry {
            etag: None,
            last_modified: None,
            body: "test".to_string(),
        };

        cache.put("https://example.com/1.json", &entry).unwrap();
        assert_eq!(cache.count().unwrap(), 1);

        cache.put("https://example.com/2.json", &entry).unwrap();
        assert_eq!(cache.count().unwrap(), 2);
    }

    #[test]
    fn clear_removes_all_entries() {
        let cache = ApiCache::in_memory().unwrap();

        let entry = CacheEntry {
            etag: None,
            last_modified: None,
            body: "test".to_string(),
        };

        cache.put("https://example.com/1.json", &entry).unwrap();
        cache.put("https://example.com/2.json", &entry).unwrap();
        assert_eq!(cache.count().unwrap(), 2);

        let removed = cache.clear().unwrap();
        assert_eq!(removed, 2);
        assert_eq!(cache.count().unwrap(), 0);
    }

    #[test]
    fn total_body_size_returns_sum_of_body_lengths() {
        let cache = ApiCache::in_memory().unwrap();
        assert_eq!(cache.total_body_size().unwrap(), 0);

        let entry1 = CacheEntry {
            etag: None,
            last_modified: None,
            body: "hello".to_string(), // 5 bytes
        };
        let entry2 = CacheEntry {
            etag: None,
            last_modified: None,
            body: "world!".to_string(), // 6 bytes
        };

        cache.put("https://example.com/1.json", &entry1).unwrap();
        cache.put("https://example.com/2.json", &entry2).unwrap();

        assert_eq!(cache.total_body_size().unwrap(), 11);
    }

    #[test]
    fn cleanup_older_than_removes_old_entries() {
        let cache = ApiCache::in_memory().unwrap();

        let entry = CacheEntry {
            etag: None,
            last_modified: None,
            body: "test".to_string(),
        };

        // Insert a recent entry
        cache
            .put("https://example.com/recent.json", &entry)
            .unwrap();

        // Insert an old entry by manipulating the database directly
        let old_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (10 * 24 * 60 * 60); // 10 days ago

        cache.conn.execute(
            "INSERT INTO api_cache (url, etag, last_modified, body, cached_at) VALUES (?1, NULL, NULL, ?2, ?3)",
            params!["https://example.com/old.json", "old data", old_time],
        ).unwrap();

        assert_eq!(cache.count().unwrap(), 2);

        // Cleanup entries older than 5 days
        let removed = cache.cleanup_older_than(5).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(cache.count().unwrap(), 1);

        // Recent entry should still exist
        assert!(cache.get("https://example.com/recent.json").is_some());
        // Old entry should be gone
        assert!(cache.get("https://example.com/old.json").is_none());
    }

    #[test]
    fn count_older_than_returns_count_of_old_entries() {
        let cache = ApiCache::in_memory().unwrap();

        let entry = CacheEntry {
            etag: None,
            last_modified: None,
            body: "test".to_string(),
        };

        // Insert a recent entry
        cache
            .put("https://example.com/recent.json", &entry)
            .unwrap();

        // Insert old entries
        let old_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (10 * 24 * 60 * 60); // 10 days ago

        cache.conn.execute(
            "INSERT INTO api_cache (url, etag, last_modified, body, cached_at) VALUES (?1, NULL, NULL, ?2, ?3)",
            params!["https://example.com/old1.json", "old", old_time],
        ).unwrap();
        cache.conn.execute(
            "INSERT INTO api_cache (url, etag, last_modified, body, cached_at) VALUES (?1, NULL, NULL, ?2, ?3)",
            params!["https://example.com/old2.json", "old", old_time],
        ).unwrap();

        assert_eq!(cache.count().unwrap(), 3);
        assert_eq!(cache.count_older_than(5).unwrap(), 2);
        assert_eq!(cache.count_older_than(15).unwrap(), 0); // Nothing older than 15 days
    }

    #[test]
    fn body_size_older_than_returns_size_of_old_entries() {
        let cache = ApiCache::in_memory().unwrap();

        let entry = CacheEntry {
            etag: None,
            last_modified: None,
            body: "recent".to_string(), // 6 bytes
        };

        // Insert a recent entry
        cache
            .put("https://example.com/recent.json", &entry)
            .unwrap();

        // Insert old entry with known size
        let old_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - (10 * 24 * 60 * 60); // 10 days ago

        cache.conn.execute(
            "INSERT INTO api_cache (url, etag, last_modified, body, cached_at) VALUES (?1, NULL, NULL, ?2, ?3)",
            params!["https://example.com/old.json", "old body here", old_time], // 13 bytes
        ).unwrap();

        assert_eq!(cache.total_body_size().unwrap(), 6 + 13);
        assert_eq!(cache.body_size_older_than(5).unwrap(), 13);
    }
}
