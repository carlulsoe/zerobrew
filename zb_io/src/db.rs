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
                pinned INTEGER NOT NULL DEFAULT 0
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
                "SELECT name, version, store_key, installed_at, pinned FROM installed_kegs WHERE name = ?1",
                params![name],
                |row| {
                    Ok(InstalledKeg {
                        name: row.get(0)?,
                        version: row.get(1)?,
                        store_key: row.get(2)?,
                        installed_at: row.get(3)?,
                        pinned: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .ok()
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledKeg>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, version, store_key, installed_at, pinned FROM installed_kegs ORDER BY name",
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
                "SELECT name, version, store_key, installed_at, pinned FROM installed_kegs WHERE pinned = 1 ORDER BY name",
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
}

pub struct InstallTransaction<'a> {
    tx: Transaction<'a>,
}

impl<'a> InstallTransaction<'a> {
    pub fn record_install(&self, name: &str, version: &str, store_key: &str) -> Result<(), Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.tx
            .execute(
                "INSERT OR REPLACE INTO installed_kegs (name, version, store_key, installed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![name, version, store_key, now],
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
            tx.record_install("foo", "1.0.0", "abc123").unwrap();
            tx.commit().unwrap();
        }

        let installed = db.list_installed().unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "foo");
        assert_eq!(installed[0].version, "1.0.0");
        assert_eq!(installed[0].store_key, "abc123");
        assert!(!installed[0].pinned);
    }

    #[test]
    fn pin_and_unpin_package() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("pinnable", "1.0.0", "abc123").unwrap();
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
            tx.record_install("pinned1", "1.0.0", "abc123").unwrap();
            tx.record_install("unpinned", "1.0.0", "def456").unwrap();
            tx.record_install("pinned2", "2.0.0", "ghi789").unwrap();
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
            tx.record_install("foo", "1.0.0", "abc123").unwrap();
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
            tx.record_install("foo", "1.0.0", "shared123").unwrap();
            tx.record_install("bar", "2.0.0", "shared123").unwrap();
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
            tx.record_install("foo", "1.0.0", "key1").unwrap();
            tx.record_install("bar", "2.0.0", "key2").unwrap();
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
            tx.record_install("foo", "1.0.0", "abc123").unwrap();
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
}
