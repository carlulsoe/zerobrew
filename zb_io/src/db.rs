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

/// Information about an installed tap
#[derive(Debug, Clone)]
pub struct InstalledTap {
    /// Tap name in "user/repo" format
    pub name: String,
    /// GitHub URL for the tap
    pub url: String,
    /// Unix timestamp when the tap was added
    pub added_at: i64,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to open database: {e}"),
        })?;

        // Enable foreign key enforcement
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to enable foreign keys: {e}"),
            })?;

        Self::init_schema(&conn)?;

        Ok(Self { conn })
    }

    pub fn in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory().map_err(|e| Error::StoreCorruption {
            message: format!("failed to open in-memory database: {e}"),
        })?;

        // Enable foreign key enforcement
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to enable foreign keys: {e}"),
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
                refcount INTEGER NOT NULL DEFAULT 1 CHECK(refcount >= 0)
            );

            CREATE TABLE IF NOT EXISTS keg_files (
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                linked_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                PRIMARY KEY (name, linked_path)
            );

            CREATE TABLE IF NOT EXISTS taps (
                name TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                added_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS services (
                name TEXT PRIMARY KEY,
                formula TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'stopped',
                pid INTEGER,
                started_at INTEGER,
                config TEXT,
                FOREIGN KEY (formula) REFERENCES installed_kegs(name) ON DELETE CASCADE
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

        // Migration: create services table if it doesn't exist (for existing databases)
        Self::migrate_add_services_table(conn)?;

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

    fn migrate_add_services_table(conn: &Connection) -> Result<(), Error> {
        // Check if services table exists
        let has_services: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='services'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_services {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS services (
                    name TEXT PRIMARY KEY,
                    formula TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'stopped',
                    pid INTEGER,
                    started_at INTEGER,
                    config TEXT,
                    FOREIGN KEY (formula) REFERENCES installed_kegs(name) ON DELETE CASCADE
                )",
                [],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to create services table: {e}"),
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

    /// Clear all linked files records for a package (used when unlinking)
    pub fn clear_linked_files(&self, name: &str) -> Result<usize, Error> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM keg_files WHERE name = ?1", params![name])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to clear linked files: {e}"),
            })?;

        Ok(rows_affected)
    }

    /// Record a linked file for a package (non-transactional version)
    pub fn record_linked_file(
        &self,
        name: &str,
        version: &str,
        linked_path: &str,
        target_path: &str,
    ) -> Result<(), Error> {
        self.conn
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

    // ========== Tap Operations ==========

    /// Add a tap to the database
    pub fn add_tap(&self, name: &str, url: &str) -> Result<(), Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO taps (name, url, added_at) VALUES (?1, ?2, ?3)",
                params![name, url, now],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to add tap: {e}"),
            })?;

        Ok(())
    }

    /// Remove a tap from the database
    pub fn remove_tap(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM taps WHERE name = ?1", params![name])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove tap: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Check if a tap is installed
    pub fn is_tapped(&self, name: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM taps WHERE name = ?1", params![name], |_| {
                Ok(())
            })
            .is_ok()
    }

    /// Get information about a specific tap
    pub fn get_tap(&self, name: &str) -> Option<InstalledTap> {
        self.conn
            .query_row(
                "SELECT name, url, added_at FROM taps WHERE name = ?1",
                params![name],
                |row| {
                    Ok(InstalledTap {
                        name: row.get(0)?,
                        url: row.get(1)?,
                        added_at: row.get(2)?,
                    })
                },
            )
            .ok()
    }

    /// List all installed taps
    pub fn list_taps(&self) -> Result<Vec<InstalledTap>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, url, added_at FROM taps ORDER BY name")
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let taps = stmt
            .query_map([], |row| {
                Ok(InstalledTap {
                    name: row.get(0)?,
                    url: row.get(1)?,
                    added_at: row.get(2)?,
                })
            })
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query taps: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(taps)
    }

    // ========== Service Operations ==========

    /// Record a service for a formula
    pub fn record_service(
        &self,
        name: &str,
        formula: &str,
        config: Option<&str>,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO services (name, formula, status, config)
                 VALUES (?1, ?2, 'stopped', ?3)",
                params![name, formula, config],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to record service: {e}"),
            })?;

        Ok(())
    }

    /// Update service status
    pub fn update_service_status(
        &self,
        name: &str,
        status: &str,
        pid: Option<u32>,
    ) -> Result<bool, Error> {
        let started_at = if status == "running" {
            Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            )
        } else {
            None
        };

        let rows_affected = self
            .conn
            .execute(
                "UPDATE services SET status = ?1, pid = ?2, started_at = ?3 WHERE name = ?4",
                params![status, pid, started_at, name],
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to update service status: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Get service by name
    pub fn get_service(&self, name: &str) -> Option<ServiceRecord> {
        self.conn
            .query_row(
                "SELECT name, formula, status, pid, started_at, config FROM services WHERE name = ?1",
                params![name],
                |row| {
                    Ok(ServiceRecord {
                        name: row.get(0)?,
                        formula: row.get(1)?,
                        status: row.get(2)?,
                        pid: row.get(3)?,
                        started_at: row.get(4)?,
                        config: row.get(5)?,
                    })
                },
            )
            .ok()
    }

    /// Get service for a formula
    pub fn get_service_for_formula(&self, formula: &str) -> Option<ServiceRecord> {
        self.conn
            .query_row(
                "SELECT name, formula, status, pid, started_at, config FROM services WHERE formula = ?1",
                params![formula],
                |row| {
                    Ok(ServiceRecord {
                        name: row.get(0)?,
                        formula: row.get(1)?,
                        status: row.get(2)?,
                        pid: row.get(3)?,
                        started_at: row.get(4)?,
                        config: row.get(5)?,
                    })
                },
            )
            .ok()
    }

    /// List all services
    pub fn list_services(&self) -> Result<Vec<ServiceRecord>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, formula, status, pid, started_at, config FROM services ORDER BY name",
            )
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to prepare statement: {e}"),
            })?;

        let services = stmt
            .query_map([], |row| {
                Ok(ServiceRecord {
                    name: row.get(0)?,
                    formula: row.get(1)?,
                    status: row.get(2)?,
                    pid: row.get(3)?,
                    started_at: row.get(4)?,
                    config: row.get(5)?,
                })
            })
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to query services: {e}"),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to collect results: {e}"),
            })?;

        Ok(services)
    }

    /// Remove a service
    pub fn remove_service(&self, name: &str) -> Result<bool, Error> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM services WHERE name = ?1", params![name])
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove service: {e}"),
            })?;

        Ok(rows_affected > 0)
    }

    /// Check if formula has a service
    pub fn has_service(&self, formula: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM services WHERE formula = ?1",
                params![formula],
                |_| Ok(()),
            )
            .is_ok()
    }
}

/// Information about a service stored in the database
#[derive(Debug, Clone)]
pub struct ServiceRecord {
    /// Service name (e.g., "zerobrew.redis")
    pub name: String,
    /// Formula this service is for
    pub formula: String,
    /// Last known status
    pub status: String,
    /// Last known PID
    pub pid: Option<u32>,
    /// Unix timestamp when last started
    pub started_at: Option<i64>,
    /// JSON configuration (optional)
    pub config: Option<String>,
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

        // Decrement store ref if we had one (clamped to 0 to prevent negative values)
        if let Some(ref key) = store_key {
            self.tx
                .execute(
                    "UPDATE store_refs SET refcount = MAX(refcount - 1, 0) WHERE store_key = ?1",
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
            tx.record_install("pinnable", "1.0.0", "abc123", true)
                .unwrap();
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
            tx.record_install("pinned1", "1.0.0", "abc123", true)
                .unwrap();
            tx.record_install("unpinned", "1.0.0", "def456", true)
                .unwrap();
            tx.record_install("pinned2", "2.0.0", "ghi789", true)
                .unwrap();
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
            tx.record_install("foo", "1.0.0", "shared123", true)
                .unwrap();
            tx.record_install("bar", "2.0.0", "shared123", true)
                .unwrap();
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
            tx.record_install("user-requested", "1.0.0", "key1", true)
                .unwrap();
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
            tx.record_install("test-pkg", "1.0.0", "key1", true)
                .unwrap();
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

    // ========== Tap Tests ==========

    #[test]
    fn add_and_list_taps() {
        let db = Database::in_memory().unwrap();

        // Initially no taps
        let taps = db.list_taps().unwrap();
        assert!(taps.is_empty());

        // Add a tap
        db.add_tap("user/repo", "https://github.com/user/homebrew-repo")
            .unwrap();

        // Should be listed
        let taps = db.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].name, "user/repo");
        assert_eq!(taps[0].url, "https://github.com/user/homebrew-repo");
        assert!(taps[0].added_at > 0);
    }

    #[test]
    fn is_tapped_returns_correct_status() {
        let db = Database::in_memory().unwrap();

        assert!(!db.is_tapped("user/repo"));

        db.add_tap("user/repo", "https://github.com/user/homebrew-repo")
            .unwrap();

        assert!(db.is_tapped("user/repo"));
        assert!(!db.is_tapped("other/tap"));
    }

    #[test]
    fn get_tap_returns_tap_info() {
        let db = Database::in_memory().unwrap();

        assert!(db.get_tap("user/repo").is_none());

        db.add_tap("user/repo", "https://github.com/user/homebrew-repo")
            .unwrap();

        let tap = db.get_tap("user/repo").unwrap();
        assert_eq!(tap.name, "user/repo");
        assert_eq!(tap.url, "https://github.com/user/homebrew-repo");
    }

    #[test]
    fn remove_tap_deletes_tap() {
        let db = Database::in_memory().unwrap();

        db.add_tap("user/repo", "https://github.com/user/homebrew-repo")
            .unwrap();
        assert!(db.is_tapped("user/repo"));

        let removed = db.remove_tap("user/repo").unwrap();
        assert!(removed);
        assert!(!db.is_tapped("user/repo"));
    }

    #[test]
    fn remove_tap_returns_false_for_nonexistent() {
        let db = Database::in_memory().unwrap();

        let removed = db.remove_tap("nonexistent/tap").unwrap();
        assert!(!removed);
    }

    #[test]
    fn add_tap_updates_existing() {
        let db = Database::in_memory().unwrap();

        db.add_tap("user/repo", "https://github.com/user/homebrew-repo")
            .unwrap();

        // Add again with different URL (should update)
        db.add_tap("user/repo", "https://github.com/user/homebrew-repo-new")
            .unwrap();

        let tap = db.get_tap("user/repo").unwrap();
        assert_eq!(tap.url, "https://github.com/user/homebrew-repo-new");

        // Should still be just one tap
        let taps = db.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
    }

    #[test]
    fn list_taps_sorted_by_name() {
        let db = Database::in_memory().unwrap();

        db.add_tap("zulu/tools", "https://github.com/zulu/homebrew-tools")
            .unwrap();
        db.add_tap("alpha/utils", "https://github.com/alpha/homebrew-utils")
            .unwrap();
        db.add_tap("mike/apps", "https://github.com/mike/homebrew-apps")
            .unwrap();

        let taps = db.list_taps().unwrap();
        assert_eq!(taps.len(), 3);
        assert_eq!(taps[0].name, "alpha/utils");
        assert_eq!(taps[1].name, "mike/apps");
        assert_eq!(taps[2].name, "zulu/tools");
    }

    #[test]
    fn clear_linked_files_removes_all_links_for_package() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("linktest", "1.0.0", "abc123", true)
                .unwrap();
            tx.record_linked_file(
                "linktest",
                "1.0.0",
                "/opt/homebrew/bin/linktest",
                "/opt/zerobrew/cellar/linktest/1.0.0/bin/linktest",
            )
            .unwrap();
            tx.record_linked_file(
                "linktest",
                "1.0.0",
                "/opt/homebrew/bin/linktest-tool",
                "/opt/zerobrew/cellar/linktest/1.0.0/bin/linktest-tool",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // Verify links recorded
        let links = db.get_linked_files("linktest").unwrap();
        assert_eq!(links.len(), 2);

        // Clear links
        let cleared = db.clear_linked_files("linktest").unwrap();
        assert_eq!(cleared, 2);

        // Verify links removed
        let links_after = db.get_linked_files("linktest").unwrap();
        assert!(links_after.is_empty());

        // Package still installed
        assert!(db.get_installed("linktest").is_some());
    }

    #[test]
    fn clear_linked_files_returns_zero_for_no_links() {
        let mut db = Database::in_memory().unwrap();

        {
            let tx = db.transaction().unwrap();
            tx.record_install("nolinks", "1.0.0", "def456", true)
                .unwrap();
            tx.commit().unwrap();
        }

        // No links to clear
        let cleared = db.clear_linked_files("nolinks").unwrap();
        assert_eq!(cleared, 0);
    }

    #[test]
    fn record_linked_file_non_transactional() {
        let mut db = Database::in_memory().unwrap();

        // First install the package
        {
            let tx = db.transaction().unwrap();
            tx.record_install("nontx", "1.0.0", "ghi789", true).unwrap();
            tx.commit().unwrap();
        }

        // Use non-transactional record_linked_file
        db.record_linked_file(
            "nontx",
            "1.0.0",
            "/opt/homebrew/bin/nontx",
            "/opt/zerobrew/cellar/nontx/1.0.0/bin/nontx",
        )
        .unwrap();

        // Verify recorded
        let links = db.get_linked_files("nontx").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "/opt/homebrew/bin/nontx");
    }

    #[test]
    fn record_linked_file_non_transactional_replaces_existing() {
        let mut db = Database::in_memory().unwrap();

        // First install the package with a link
        {
            let tx = db.transaction().unwrap();
            tx.record_install("replace", "1.0.0", "jkl012", true)
                .unwrap();
            tx.record_linked_file(
                "replace",
                "1.0.0",
                "/opt/homebrew/bin/replace",
                "/opt/zerobrew/cellar/replace/1.0.0/bin/replace-old",
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // Use non-transactional to update the same link path
        db.record_linked_file(
            "replace",
            "1.0.0",
            "/opt/homebrew/bin/replace",
            "/opt/zerobrew/cellar/replace/1.0.0/bin/replace-new",
        )
        .unwrap();

        // Should have 1 link with new target
        let links = db.get_linked_files("replace").unwrap();
        assert_eq!(links.len(), 1);
        assert!(links[0].1.contains("replace-new"));
    }

    // =========================================================================
    // Property-based tests with proptest
    // =========================================================================

    mod proptest_tests {
        use super::*;
        use proptest::prelude::*;

        /// Strategy to generate valid package names
        fn package_name_strategy() -> impl Strategy<Value = String> {
            "[a-z][a-z0-9_-]{0,15}"
                .prop_filter("non-empty", |s| !s.is_empty())
        }

        /// Strategy to generate version strings
        fn version_strategy() -> impl Strategy<Value = String> {
            (1u32..20, 0u32..20, 0u32..20)
                .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
        }

        /// Strategy to generate store keys (hex-like strings)
        fn store_key_strategy() -> impl Strategy<Value = String> {
            "[a-f0-9]{16,64}"
        }

        proptest! {
            #[test]
            fn install_delete_roundtrip(
                name in package_name_strategy(),
                version in version_strategy(),
                store_key in store_key_strategy()
            ) {
                let mut db = Database::in_memory().unwrap();

                // Install
                {
                    let tx = db.transaction().unwrap();
                    tx.record_install(&name, &version, &store_key, true).unwrap();
                    tx.commit().unwrap();
                }

                // Verify installed
                let installed = db.get_installed(&name);
                prop_assert!(installed.is_some(), "Package should be installed");
                let pkg = installed.unwrap();
                prop_assert_eq!(&pkg.name, &name);
                prop_assert_eq!(&pkg.version, &version);
                prop_assert_eq!(&pkg.store_key, &store_key);

                // Uninstall
                {
                    let tx = db.transaction().unwrap();
                    let removed_key = tx.record_uninstall(&name).unwrap();
                    prop_assert_eq!(removed_key, Some(store_key.clone()));
                    tx.commit().unwrap();
                }

                // Verify removed
                let after = db.get_installed(&name);
                prop_assert!(after.is_none(), "Package should be removed after uninstall");
            }

            #[test]
            fn explicit_flag_persists(
                name in package_name_strategy(),
                version in version_strategy(),
                store_key in store_key_strategy(),
                explicit in prop::bool::ANY
            ) {
                let mut db = Database::in_memory().unwrap();

                {
                    let tx = db.transaction().unwrap();
                    tx.record_install(&name, &version, &store_key, explicit).unwrap();
                    tx.commit().unwrap();
                }

                let installed = db.get_installed(&name).unwrap();
                prop_assert_eq!(installed.explicit, explicit);
            }

            #[test]
            fn pin_unpin_roundtrip(
                name in package_name_strategy(),
                version in version_strategy(),
                store_key in store_key_strategy()
            ) {
                let mut db = Database::in_memory().unwrap();

                {
                    let tx = db.transaction().unwrap();
                    tx.record_install(&name, &version, &store_key, true).unwrap();
                    tx.commit().unwrap();
                }

                // Initially not pinned
                prop_assert!(!db.is_pinned(&name));

                // Pin
                db.pin(&name).unwrap();
                prop_assert!(db.is_pinned(&name));

                // Unpin
                db.unpin(&name).unwrap();
                prop_assert!(!db.is_pinned(&name));
            }

            #[test]
            fn store_refcount_increments_on_install(
                names in prop::collection::vec(package_name_strategy(), 1..5),
                version in version_strategy(),
                store_key in store_key_strategy()
            ) {
                let mut db = Database::in_memory().unwrap();

                // Deduplicate names
                let unique_names: std::collections::BTreeSet<_> = names.into_iter().collect();
                let unique_names: Vec<_> = unique_names.into_iter().collect();

                // Install multiple packages with same store_key
                for name in &unique_names {
                    let tx = db.transaction().unwrap();
                    tx.record_install(name, &version, &store_key, true).unwrap();
                    tx.commit().unwrap();
                }

                // Refcount should equal number of unique packages
                let refcount = db.get_store_refcount(&store_key);
                prop_assert_eq!(refcount, unique_names.len() as i64);
            }
        }
    }
}
