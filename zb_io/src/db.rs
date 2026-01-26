use std::path::Path;

use rusqlite::{Connection, Transaction, params};

use zb_core::Error;

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct InstalledKeg {
    pub name: String,
    pub version: String,
    pub store_key: String,
    pub installed_at: i64,
    pub pinned: bool,
    /// Whether this package was explicitly installed by the user (true) or as a dependency (false)
    pub explicit: bool,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to open database: {e}"),
        })?;

        Self::init_schema(&conn)?;

        Ok(Self { conn })
    }

    pub fn in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory().map_err(|e| Error::StoreCorruption {
            message: format!("failed to open in-memory database: {e}"),
        })?;

        Self::init_schema(&conn)?;

        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), Error> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS installed_kegs (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                store_key TEXT NOT NULL,
                installed_at INTEGER NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                explicit INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS store_refs (
                store_key TEXT PRIMARY KEY,
                refcount INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS keg_files (
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                linked_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                PRIMARY KEY (name, linked_path)
            );
            ",
        )
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to initialize schema: {e}"),
        })?;

        // Migration: add pinned column if it doesn't exist (for existing databases)
        Self::migrate_add_pinned_column(conn)?;

        // Migration: add explicit column if it doesn't exist (for existing databases)
        Self::migrate_add_explicit_column(conn)?;

        Ok(())
    }

    fn migrate_add_pinned_column(conn: &Connection) -> Result<(), Error> {
        // Check if pinned column exists
        let has_pinned: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('installed_kegs') WHERE name = 'pinned'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_pinned {
            conn.execute(
                "ALTER TABLE installed_kegs ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to add pinned column: {e}"),
            })?;
        }

        Ok(())
    }

    fn migrate_add_explicit_column(conn: &Connection) -> Result<(), Error> {
        // Check if explicit column exists
        let has_explicit: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('installed_kegs') WHERE name = 'explicit'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_explicit {
            // Default to 1 (explicit) for existing packages - safe assumption
            conn.execute(
                "ALTER TABLE installed_kegs ADD COLUMN explicit INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to add explicit column: {e}"),
            })?;
        }

        Ok(())
    }

    pub fn transaction(&mut self) -> Result<InstallTransaction<'_>, Error> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to start transaction: {e}"),
            })?;

        Ok(InstallTransaction { tx })
    }

    pub fn get_installed(&self, name: &str) -> Option<InstalledKeg> {
        self.conn
            .query_row(
                "SELECT name, version, store_key, installed_at, pinned, explicit FROM installed_kegs WHERE name = ?1",
                params![name],
                |row| {
                    Ok(InstalledKeg {
                        name: row.get(0)?,
                        version: row.get(1)?,
                        store_key: row.get(2)?,
                        installed_at: row.get(3)?,
                        pinned: row.get::<_, i64>(4)? != 0,
                        explicit: row.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .ok()
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledKeg>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, version, store_key, installed_at, pinned, explicit FROM installed_kegs ORDER BY name",
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let kegs = stmt
            .query_map([], |row| {
                Ok(InstalledKeg {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    store_key: row.get(2)?,
                    installed_at: row.get(3)?,
                    pinned: row.get::<_, i64>(4)? != 0,
                    explicit: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query installed kegs: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(kegs)
    }

    /// List only pinned packages
    pub fn list_pinned(&self) -> Result<Vec<InstalledKeg>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, version, store_key, installed_at, pinned, explicit FROM installed_kegs WHERE pinned = 1 ORDER BY name",
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let kegs = stmt
            .query_map([], |row| {
                Ok(InstalledKeg {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    store_key: row.get(2)?,
                    installed_at: row.get(3)?,
                    pinned: row.get::<_, i64>(4)? != 0,
                    explicit: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query pinned kegs: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(kegs)
    }

    /// List only packages installed as dependencies (not explicitly)
    pub fn list_dependencies(&self) -> Result<Vec<InstalledKeg>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, version, store_key, installed_at, pinned, explicit FROM installed_kegs WHERE explicit = 0 ORDER BY name",
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let kegs = stmt
            .query_map([], |row| {
                Ok(InstalledKeg {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    store_key: row.get(2)?,
                    installed_at: row.get(3)?,
                    pinned: row.get::<_, i64>(4)? != 0,
                    explicit: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query dependency kegs: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(kegs)
    }

    /// Mark a package as explicitly installed (not as a dependency)
    pub fn mark_explicit(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute(
                "UPDATE installed_kegs SET explicit = 1 WHERE name = ?1",
                params![name],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to mark package as explicit: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Mark a package as a dependency (not explicitly installed)
    pub fn mark_dependency(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute(
                "UPDATE installed_kegs SET explicit = 0 WHERE name = ?1",
                params![name],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to mark package as dependency: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Check if a package was explicitly installed
    pub fn is_explicit(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT explicit FROM installed_kegs WHERE name = ?1",
                params![name],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Pin a package to prevent upgrades
    pub fn pin(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute(
                "UPDATE installed_kegs SET pinned = 1 WHERE name = ?1",
                params![name],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to pin package: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Unpin a package to allow upgrades
    pub fn unpin(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute(
                "UPDATE installed_kegs SET pinned = 0 WHERE name = ?1",
                params![name],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to unpin package: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Check if a package is pinned
    pub fn is_pinned(&self, name: &str) -> bool {
        self.conn
            .query_row(
                "SELECT pinned FROM installed_kegs WHERE name = ?1",
                params![name],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    pub fn get_store_refcount(&self, store_key: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT refcount FROM store_refs WHERE store_key = ?1",
                params![store_key],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    pub fn get_unreferenced_store_keys(&self) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT store_key FROM store_refs WHERE refcount <= 0")
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let keys = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query unreferenced keys: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(keys)
    }

    /// Get all linked files for a package
    pub fn get_linked_files(&self, name: &str) -> Result<Vec<(String, String)>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT linked_path, target_path FROM keg_files WHERE name = ?1 ORDER BY linked_path",
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let files = stmt
            .query_map(params![name], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query linked files: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(files)
    }
}

pub struct InstallTransaction<'a> {
    tx: Transaction<'a>,
}

impl<'a> InstallTransaction<'a> {
    /// Record a package installation.
    ///
    /// # Arguments
    /// * `name` - Package name
    /// * `version` - Package version
    /// * `store_key` - Content-addressable store key
    /// * `explicit` - Whether this was explicitly installed by user (true) or as a dependency (false)
    pub fn record_install(
        &self,
        name: &str,
        version: &str,
        store_key: &str,
        explicit: bool,
    ) -> Result<(), Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let explicit_int: i64 = if explicit { 1 } else { 0 };

        self.tx
            .execute(
                "INSERT OR REPLACE INTO installed_kegs (name, version, store_key, installed_at, explicit)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, version, store_key, now, explicit_int],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to record install: {e}"),
            })?;

        // Increment store ref
        self.tx
            .execute(
                "INSERT INTO store_refs (store_key, refcount) VALUES (?1, 1)
                 ON CONFLICT(store_key) DO UPDATE SET refcount = refcount + 1",
                params![store_key],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to increment store ref: {e}"),
            })?;

        Ok(())
    }

    pub fn record_linked_file(
        &self,
        name: &str,
        version: &str,
        linked_path: &str,
        target_path: &str,
    ) -> Result<(), Error> {
        self.tx
            .execute(
                "INSERT OR REPLACE INTO keg_files (name, version, linked_path, target_path)
                 VALUES (?1, ?2, ?3, ?4)",
                params![name, version, linked_path, target_path],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to record linked file: {e}"),
            })?;

        Ok(())
    }

    pub fn record_uninstall(&self, name: &str) -> Result<Option<String>, Error> {
        // Get the store_key before removing
        let store_key: Option<String> = self
            .tx
            .query_row(
                "SELECT store_key FROM installed_kegs WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .ok();

        // Remove installed keg record
        self.tx
            .execute("DELETE FROM installed_kegs WHERE name = ?1", params![name])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove install record: {e}"),
            })?;

        // Remove linked files records
        self.tx
            .execute("DELETE FROM keg_files WHERE name = ?1", params![name])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove keg files records: {e}"),
            })?;

        // Decrement store ref if we had one
        if let Some(ref key) = store_key {
            self.tx
                .execute(
                    "UPDATE store_refs SET refcount = refcount - 1 WHERE store_key = ?1",
                    params![key],
                )
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to decrement store ref: {e}"),
                })?;
        }

        Ok(store_key)
    }

    pub fn commit(self) -> Result<(), Error> {
        self.tx.commit().map_err(|e| Error::StoreCorruption {
            message: format!("failed to commit transaction: {e}"),
        })
    }

    // Transaction is rolled back automatically when dropped without commit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_list() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("foo", "1.0.0", "abc123", true).unwrap();
            tx.commit().unwrap();
        }

        let installed = db.list_installed().unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "foo");
        assert_eq!(installed[0].version, "1.0.0");
        assert_eq!(installed[0].store_key, "abc123");
        assert!(!installed[0].pinned);
        assert!(installed[0].explicit);
    }

    #[test]
    fn pin_and_unpin_package() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("pinnable", "1.0.0", "abc123", true).unwrap();
            tx.commit().unwrap();
        }

        // Initially not pinned
        assert!(!db.is_pinned("pinnable"));
        let keg = db.get_installed("pinnable").unwrap();
        assert!(!keg.pinned);

        // Pin the package
        let result = db.pin("pinnable").unwrap();
        assert!(result); // Should return true (rows affected)
        assert!(db.is_pinned("pinnable"));

        // Verify via get_installed
        let keg = db.get_installed("pinnable").unwrap();
        assert!(keg.pinned);

        // Unpin the package
        let result = db.unpin("pinnable").unwrap();
        assert!(result);
        assert!(!db.is_pinned("pinnable"));

        // Verify via get_installed
        let keg = db.get_installed("pinnable").unwrap();
        assert!(!keg.pinned);
    }

    #[test]
    fn pin_nonexistent_package_returns_false() {
        let db = Database::in_memory().unwrap();

        // Pinning a non-existent package should return false (no rows affected)
        let result = db.pin("doesnotexist").unwrap();
        assert!(!result);

        // is_pinned should return false for non-existent packages
        assert!(!db.is_pinned("doesnotexist"));
    }

    #[test]
    fn list_pinned_only_returns_pinned_packages() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("pinned1", "1.0.0", "abc123", true).unwrap();
            tx.record_install("unpinned", "1.0.0", "def456", true).unwrap();
            tx.record_install("pinned2", "2.0.0", "ghi789", true).unwrap();
            tx.commit().unwrap();
        }

        // Pin two packages
        db.pin("pinned1").unwrap();
        db.pin("pinned2").unwrap();

        // list_installed should return all 3
        let all = db.list_installed().unwrap();
        assert_eq!(all.len(), 3);

        // list_pinned should return only 2
        let pinned = db.list_pinned().unwrap();
        assert_eq!(pinned.len(), 2);
        assert!(pinned.iter().all(|k| k.pinned));
        assert!(pinned.iter().any(|k| k.name == "pinned1"));
        assert!(pinned.iter().any(|k| k.name == "pinned2"));
        assert!(!pinned.iter().any(|k| k.name == "unpinned"));
    }

    #[test]
    fn rollback_leaves_no_partial_state() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("foo", "1.0.0", "abc123", true).unwrap();
            // Don't commit - transaction will be rolled back when dropped
        }

        let installed = db.list_installed().unwrap();
        assert!(installed.is_empty());

        // Store ref should also not exist
        assert_eq!(db.get_store_refcount("abc123"), 0);
    }

    #[test]
    fn uninstall_decrements_refcount() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("foo", "1.0.0", "shared123", true).unwrap();
            tx.record_install("bar", "2.0.0", "shared123", true).unwrap();
            tx.commit().unwrap();
        }

        assert_eq!(db.get_store_refcount("shared123"), 2);

        {
            let tx = db.transaction().unwrap();
            tx.record_uninstall("foo").unwrap();
            tx.commit().unwrap();
        }

        assert_eq!(db.get_store_refcount("shared123"), 1);
        assert!(db.get_installed("foo").is_none());
        assert!(db.get_installed("bar").is_some());
    }

    #[test]
    fn get_unreferenced_store_keys() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("foo", "1.0.0", "key1", true).unwrap();
            tx.record_install("bar", "2.0.0", "key2", true).unwrap();
            tx.commit().unwrap();
        }

        // Uninstall both
        {
            let tx = db.transaction().unwrap();
            tx.record_uninstall("foo").unwrap();
            tx.record_uninstall("bar").unwrap();
            tx.commit().unwrap();
        }

        let unreferenced = db.get_unreferenced_store_keys().unwrap();
        assert_eq!(unreferenced.len(), 2);
        assert!(unreferenced.contains(&"key1".to_string()));
        assert!(unreferenced.contains(&"key2".to_string()));
    }

    #[test]
    fn linked_files_are_recorded() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("foo", "1.0.0", "abc123", true).unwrap();
            tx.record_linked_file(
                "foo",
                "1.0.0",
                "/opt/homebrew/bin/foo",
                "/opt/zerobrew/cellar/foo/1.0.0/bin/foo",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // Verify via uninstall that removes records
        {
            let tx = db.transaction().unwrap();
            tx.record_uninstall("foo").unwrap();
            tx.commit().unwrap();
        }

        assert!(db.get_installed("foo").is_none());
    }

    #[test]
    fn explicit_vs_dependency_tracking() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            // Explicit install
            tx.record_install("user-requested", "1.0.0", "key1", true).unwrap();
            // Dependency installs
            tx.record_install("dep1", "1.0.0", "key2", false).unwrap();
            tx.record_install("dep2", "2.0.0", "key3", false).unwrap();
            tx.commit().unwrap();
        }

        // Verify explicit flag
        let user_pkg = db.get_installed("user-requested").unwrap();
        assert!(user_pkg.explicit);

        let dep1 = db.get_installed("dep1").unwrap();
        assert!(!dep1.explicit);

        let dep2 = db.get_installed("dep2").unwrap();
        assert!(!dep2.explicit);

        // is_explicit method
        assert!(db.is_explicit("user-requested"));
        assert!(!db.is_explicit("dep1"));
        assert!(!db.is_explicit("dep2"));
        assert!(!db.is_explicit("nonexistent")); // Not installed

        // list_dependencies should only return deps
        let deps = db.list_dependencies().unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().all(|k| !k.explicit));
        assert!(deps.iter().any(|k| k.name == "dep1"));
        assert!(deps.iter().any(|k| k.name == "dep2"));
    }

    #[test]
    fn mark_explicit_and_dependency() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("pkg", "1.0.0", "key1", false).unwrap();
            tx.commit().unwrap();
        }

        // Initially a dependency
        assert!(!db.is_explicit("pkg"));

        // Mark as explicit (user explicitly installs a previously auto-installed dep)
        let result = db.mark_explicit("pkg").unwrap();
        assert!(result);
        assert!(db.is_explicit("pkg"));

        // Mark back as dependency
        let result = db.mark_dependency("pkg").unwrap();
        assert!(result);
        assert!(!db.is_explicit("pkg"));

        // Non-existent package
        let result = db.mark_explicit("nonexistent").unwrap();
        assert!(!result);
    }

    #[test]
    fn get_linked_files_returns_linked_files_for_package() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("test-pkg", "1.0.0", "key1", true).unwrap();
            tx.record_linked_file(
                "test-pkg",
                "1.0.0",
                "/opt/zerobrew/prefix/bin/test",
                "/opt/zerobrew/cellar/test-pkg/1.0.0/bin/test",
            )
            .unwrap();
            tx.record_linked_file(
                "test-pkg",
                "1.0.0",
                "/opt/zerobrew/prefix/lib/libtest.so",
                "/opt/zerobrew/cellar/test-pkg/1.0.0/lib/libtest.so",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        let files = db.get_linked_files("test-pkg").unwrap();
        assert_eq!(files.len(), 2);

        // Files should be sorted by link path
        assert_eq!(files[0].0, "/opt/zerobrew/prefix/bin/test");
        assert_eq!(files[1].0, "/opt/zerobrew/prefix/lib/libtest.so");
    }

    #[test]
    fn get_linked_files_returns_empty_for_nonexistent_package() {
        let db = Database::in_memory().unwrap();
        let files = db.get_linked_files("nonexistent").unwrap();
        assert!(files.is_empty());
    }
}
