use rusqlite::{Connection, params};
use std::path::Path;

pub struct ApiCache {
    conn: Connection,
}

/// Cached formula metadata stored in SQLite
#[derive(Debug, Clone)]
pub struct CachedFormula {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub aliases: Vec<String>,
    pub deprecated: bool,
    pub disabled: bool,
}

/// Cache metadata for conditional requests
#[derive(Debug, Clone)]
pub struct FormulaCacheMeta {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub cached_at: i64,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: String,
    pub cached_at: i64,
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

        // Formula storage for fast search (Phase 2)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS formulas (
                name TEXT PRIMARY KEY,
                full_name TEXT NOT NULL,
                description TEXT,
                version TEXT,
                aliases TEXT,
                deprecated INTEGER NOT NULL DEFAULT 0,
                disabled INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS formula_cache_meta (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                etag TEXT,
                last_modified TEXT,
                cached_at INTEGER NOT NULL
            )",
            [],
        )?;

        // FTS5 full-text search index (Phase 4)
        // Uses external content table to avoid data duplication
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS formula_fts USING fts5(
                name,
                description,
                aliases,
                content='formulas',
                content_rowid='rowid'
            )",
            [],
        )?;

        Ok(())
    }

    pub fn get(&self, url: &str) -> Option<CacheEntry> {
        self.conn
            .query_row(
                "SELECT etag, last_modified, body, cached_at FROM api_cache WHERE url = ?1",
                params![url],
                |row| {
                    Ok(CacheEntry {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                        body: row.get(2)?,
                        cached_at: row.get(3)?,
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

    // ========================================================================
    // Formula cache methods (Phase 2)
    // ========================================================================

    /// Get formula cache metadata for conditional requests
    pub fn get_formula_cache_meta(&self) -> Option<FormulaCacheMeta> {
        self.conn
            .query_row(
                "SELECT etag, last_modified, cached_at FROM formula_cache_meta WHERE id = 1",
                [],
                |row| {
                    Ok(FormulaCacheMeta {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                        cached_at: row.get(2)?,
                    })
                },
            )
            .ok()
    }

    /// Get all cached formulas from SQLite
    pub fn get_formulas(&self) -> Result<Vec<CachedFormula>, rusqlite::Error> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, full_name, description, version, aliases, deprecated, disabled FROM formulas",
        )?;

        let rows = stmt.query_map([], |row| {
            let aliases_json: Option<String> = row.get(4)?;
            let aliases: Vec<String> = aliases_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok(CachedFormula {
                name: row.get(0)?,
                full_name: row.get(1)?,
                description: row.get(2)?,
                version: row.get(3)?,
                aliases,
                deprecated: row.get::<_, i64>(5)? != 0,
                disabled: row.get::<_, i64>(6)? != 0,
            })
        })?;

        rows.collect()
    }

    /// Store formulas in SQLite with cache metadata
    pub fn put_formulas(
        &self,
        formulas: &[CachedFormula],
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Use a transaction for atomicity
        self.conn.execute("BEGIN TRANSACTION", [])?;

        // Clear existing formulas and FTS index
        self.conn.execute("DELETE FROM formulas", [])?;
        self.conn.execute("DELETE FROM formula_fts", [])?;

        // Insert new formulas
        {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO formulas (name, full_name, description, version, aliases, deprecated, disabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;

            let mut fts_stmt = self.conn.prepare_cached(
                "INSERT INTO formula_fts (rowid, name, description, aliases)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;

            for (idx, f) in formulas.iter().enumerate() {
                let aliases_json = serde_json::to_string(&f.aliases).unwrap_or_else(|_| "[]".to_string());
                // Join aliases with spaces for FTS searchability
                let aliases_text = f.aliases.join(" ");

                stmt.execute(params![
                    &f.name,
                    &f.full_name,
                    &f.description,
                    &f.version,
                    &aliases_json,
                    f.deprecated as i64,
                    f.disabled as i64,
                ])?;

                // Insert into FTS index (rowid is 1-based in SQLite)
                fts_stmt.execute(params![
                    (idx + 1) as i64,
                    &f.name,
                    &f.description,
                    &aliases_text,
                ])?;
            }
        }

        // Update cache metadata
        self.conn.execute(
            "INSERT OR REPLACE INTO formula_cache_meta (id, etag, last_modified, cached_at)
             VALUES (1, ?1, ?2, ?3)",
            params![etag, last_modified, now],
        )?;

        self.conn.execute("COMMIT", [])?;

        Ok(())
    }

    /// Check if formula cache is fresh (within TTL)
    pub fn is_formula_cache_fresh(&self, ttl_secs: i64) -> bool {
        self.get_formula_cache_meta()
            .map(|meta| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                now - meta.cached_at < ttl_secs
            })
            .unwrap_or(false)
    }

    /// Get formula count in cache
    pub fn formula_count(&self) -> Result<usize, rusqlite::Error> {
        self.conn.query_row("SELECT COUNT(*) FROM formulas", [], |row| {
            row.get::<_, i64>(0).map(|n| n as usize)
        })
    }

    // ========================================================================
    // FTS5 search methods (Phase 4)
    // ========================================================================

    /// Search formulas using FTS5 full-text search
    ///
    /// Supports prefix matching with '*' suffix (e.g., "pyth*" matches "python")
    /// Returns formula names that match the query.
    pub fn search_fts(&self, query: &str) -> Result<Vec<String>, rusqlite::Error> {
        // Escape special FTS5 characters and prepare query for prefix matching
        let fts_query = Self::prepare_fts_query(query);

        let mut stmt = self.conn.prepare_cached(
            "SELECT f.name FROM formulas f
             JOIN formula_fts fts ON f.rowid = fts.rowid
             WHERE formula_fts MATCH ?1
             AND f.deprecated = 0 AND f.disabled = 0",
        )?;

        let rows = stmt.query_map(params![fts_query], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    /// Search formulas using FTS5 and return full formula info
    pub fn search_fts_full(&self, query: &str) -> Result<Vec<CachedFormula>, rusqlite::Error> {
        let fts_query = Self::prepare_fts_query(query);

        let mut stmt = self.conn.prepare_cached(
            "SELECT f.name, f.full_name, f.description, f.version, f.aliases, f.deprecated, f.disabled
             FROM formulas f
             JOIN formula_fts fts ON f.rowid = fts.rowid
             WHERE formula_fts MATCH ?1
             AND f.deprecated = 0 AND f.disabled = 0",
        )?;

        let rows = stmt.query_map(params![fts_query], |row| {
            let aliases_json: Option<String> = row.get(4)?;
            let aliases: Vec<String> = aliases_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            Ok(CachedFormula {
                name: row.get(0)?,
                full_name: row.get(1)?,
                description: row.get(2)?,
                version: row.get(3)?,
                aliases,
                deprecated: row.get::<_, i64>(5)? != 0,
                disabled: row.get::<_, i64>(6)? != 0,
            })
        })?;

        rows.collect()
    }

    /// Prepare a search query for FTS5
    /// - Escapes special characters
    /// - Adds prefix matching with '*'
    fn prepare_fts_query(query: &str) -> String {
        // FTS5 special characters that need escaping: " * ( ) : ^
        let escaped: String = query
            .chars()
            .map(|c| match c {
                '"' | '*' | '(' | ')' | ':' | '^' => format!("\"{}\"", c),
                _ => c.to_string(),
            })
            .collect();

        // Add prefix matching: "python" -> "python*"
        // This allows partial word matching
        if escaped.is_empty() {
            escaped
        } else {
            format!("{}*", escaped)
        }
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
            cached_at: 0, // Will be overwritten by put()
        };

        cache.put("https://example.com/foo.json", &entry).unwrap();
        let retrieved = cache.get("https://example.com/foo.json").unwrap();

        assert_eq!(retrieved.etag, Some("abc123".to_string()));
        assert_eq!(retrieved.body, r#"{"name":"foo"}"#);
        assert!(retrieved.cached_at > 0);
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
            cached_at: 0,
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
            cached_at: 0,
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
            cached_at: 0,
        };
        let entry2 = CacheEntry {
            etag: None,
            last_modified: None,
            body: "world!".to_string(), // 6 bytes
            cached_at: 0,
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
            cached_at: 0,
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
            cached_at: 0,
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
            cached_at: 0,
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

    // ========================================================================
    // Formula cache and FTS tests
    // ========================================================================

    #[test]
    fn stores_and_retrieves_formulas() {
        let cache = ApiCache::in_memory().unwrap();

        let formulas = vec![
            CachedFormula {
                name: "python".to_string(),
                full_name: "homebrew/core/python".to_string(),
                description: Some("Interpreted programming language".to_string()),
                version: Some("3.12.0".to_string()),
                aliases: vec!["python3".to_string()],
                deprecated: false,
                disabled: false,
            },
            CachedFormula {
                name: "node".to_string(),
                full_name: "homebrew/core/node".to_string(),
                description: Some("JavaScript runtime".to_string()),
                version: Some("20.0.0".to_string()),
                aliases: vec!["nodejs".to_string()],
                deprecated: false,
                disabled: false,
            },
        ];

        cache
            .put_formulas(&formulas, Some("etag123"), Some("last-modified"))
            .unwrap();

        let retrieved = cache.get_formulas().unwrap();
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved.iter().any(|f| f.name == "python"));
        assert!(retrieved.iter().any(|f| f.name == "node"));

        let meta = cache.get_formula_cache_meta().unwrap();
        assert_eq!(meta.etag, Some("etag123".to_string()));
    }

    #[test]
    fn fts_search_finds_formulas() {
        let cache = ApiCache::in_memory().unwrap();

        let formulas = vec![
            CachedFormula {
                name: "python".to_string(),
                full_name: "homebrew/core/python".to_string(),
                description: Some("Interpreted programming language".to_string()),
                version: Some("3.12.0".to_string()),
                aliases: vec!["python3".to_string()],
                deprecated: false,
                disabled: false,
            },
            CachedFormula {
                name: "pyenv".to_string(),
                full_name: "homebrew/core/pyenv".to_string(),
                description: Some("Python version management".to_string()),
                version: Some("2.0.0".to_string()),
                aliases: vec![],
                deprecated: false,
                disabled: false,
            },
            CachedFormula {
                name: "node".to_string(),
                full_name: "homebrew/core/node".to_string(),
                description: Some("JavaScript runtime".to_string()),
                version: Some("20.0.0".to_string()),
                aliases: vec![],
                deprecated: false,
                disabled: false,
            },
        ];

        cache.put_formulas(&formulas, None, None).unwrap();

        // Search for "pyth" should find python via prefix matching
        let results = cache.search_fts("pyth").unwrap();
        assert!(results.contains(&"python".to_string()));

        // Search for "python" in description should find pyenv
        let results = cache.search_fts("python").unwrap();
        assert!(results.contains(&"python".to_string()));
        assert!(results.contains(&"pyenv".to_string())); // Has "Python" in description
    }

    #[test]
    fn fts_search_excludes_deprecated_and_disabled() {
        let cache = ApiCache::in_memory().unwrap();

        let formulas = vec![
            CachedFormula {
                name: "good-pkg".to_string(),
                full_name: "homebrew/core/good-pkg".to_string(),
                description: Some("A good package".to_string()),
                version: Some("1.0.0".to_string()),
                aliases: vec![],
                deprecated: false,
                disabled: false,
            },
            CachedFormula {
                name: "old-pkg".to_string(),
                full_name: "homebrew/core/old-pkg".to_string(),
                description: Some("An old package".to_string()),
                version: Some("1.0.0".to_string()),
                aliases: vec![],
                deprecated: true,
                disabled: false,
            },
            CachedFormula {
                name: "broken-pkg".to_string(),
                full_name: "homebrew/core/broken-pkg".to_string(),
                description: Some("A broken package".to_string()),
                version: Some("1.0.0".to_string()),
                aliases: vec![],
                deprecated: false,
                disabled: true,
            },
        ];

        cache.put_formulas(&formulas, None, None).unwrap();

        // Search for "pkg" should only find good-pkg
        let results = cache.search_fts("pkg").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "good-pkg");
    }

    #[test]
    fn formula_cache_freshness() {
        let cache = ApiCache::in_memory().unwrap();

        // No cache metadata yet
        assert!(!cache.is_formula_cache_fresh(300));

        let formulas = vec![CachedFormula {
            name: "test".to_string(),
            full_name: "test".to_string(),
            description: None,
            version: None,
            aliases: vec![],
            deprecated: false,
            disabled: false,
        }];

        cache.put_formulas(&formulas, None, None).unwrap();

        // Just cached, should be fresh
        assert!(cache.is_formula_cache_fresh(300));

        // With 0 TTL, should not be fresh
        assert!(!cache.is_formula_cache_fresh(0));
    }
}
