//! Health check diagnostics for zerobrew
//!
//! This module provides the `doctor` command functionality for checking
//! the health and integrity of a zerobrew installation.

use super::Installer;

/// Status level for a doctor check
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    /// Everything is fine
    Ok,
    /// A warning that may need attention
    Warning,
    /// A problem that should be fixed
    Error,
}

/// Result of a single doctor check
#[derive(Debug, Clone)]
pub struct DoctorCheck {
    /// Name of the check
    pub name: String,
    /// Status of the check
    pub status: DoctorStatus,
    /// Human-readable description of the result
    pub message: String,
    /// Suggested fix, if applicable
    pub fix: Option<String>,
}

/// Result of running all doctor checks
#[derive(Debug, Clone, Default)]
pub struct DoctorResult {
    /// All checks that were run
    pub checks: Vec<DoctorCheck>,
    /// Number of errors
    pub errors: usize,
    /// Number of warnings
    pub warnings: usize,
}

impl DoctorResult {
    /// Check if all checks passed without errors or warnings
    pub fn is_healthy(&self) -> bool {
        self.errors == 0 && self.warnings == 0
    }
}

impl Installer {
    /// Run diagnostic checks on the zerobrew installation
    pub async fn doctor(&self) -> DoctorResult {
        let mut result = DoctorResult::default();

        // Check 1: Prefix exists and is writable
        result.checks.push(self.check_prefix_writable());

        // Check 2: Cellar structure is valid
        result.checks.push(self.check_cellar_structure());

        // Check 3: Database integrity
        result.checks.push(self.check_database_integrity());

        // Check 4: Broken symlinks in bin/
        result.checks.push(self.check_broken_symlinks());

        // Check 5: Missing dependencies
        result
            .checks
            .extend(self.check_missing_dependencies().await);

        // Check 6: (Linux) patchelf installed
        #[cfg(target_os = "linux")]
        result.checks.push(self.check_patchelf());

        // Check 7: Permissions on key directories
        result.checks.extend(self.check_directory_permissions());

        // Count errors and warnings
        for check in &result.checks {
            match check.status {
                DoctorStatus::Error => result.errors += 1,
                DoctorStatus::Warning => result.warnings += 1,
                DoctorStatus::Ok => {}
            }
        }

        result
    }

    pub(crate) fn check_prefix_writable(&self) -> DoctorCheck {
        let prefix = &self.prefix;
        if !prefix.exists() {
            return DoctorCheck {
                name: "prefix_exists".to_string(),
                status: DoctorStatus::Error,
                message: format!("Prefix directory '{}' does not exist", prefix.display()),
                fix: Some(format!(
                    "Run: sudo mkdir -p {} && sudo chown $USER {}",
                    prefix.display(),
                    prefix.display()
                )),
            };
        }

        // Check if writable
        let test_file = prefix.join(".zb_doctor_test");
        if std::fs::write(&test_file, b"test").is_err() {
            return DoctorCheck {
                name: "prefix_writable".to_string(),
                status: DoctorStatus::Error,
                message: format!("Prefix directory '{}' is not writable", prefix.display()),
                fix: Some(format!("Run: sudo chown -R $USER {}", prefix.display())),
            };
        }
        let _ = std::fs::remove_file(&test_file);

        DoctorCheck {
            name: "prefix_writable".to_string(),
            status: DoctorStatus::Ok,
            message: "Prefix directory exists and is writable".to_string(),
            fix: None,
        }
    }

    pub(crate) fn check_cellar_structure(&self) -> DoctorCheck {
        let cellar = &self.cellar_path;
        if !cellar.exists() {
            return DoctorCheck {
                name: "cellar_exists".to_string(),
                status: DoctorStatus::Warning,
                message: "Cellar directory does not exist (will be created on first install)"
                    .to_string(),
                fix: None,
            };
        }

        // Check if any installed packages have corrupted structure
        if let Ok(installed) = self.db.list_installed() {
            for keg in &installed {
                let keg_path = self.cellar.keg_path(&keg.name, &keg.version);
                if !keg_path.exists() {
                    return DoctorCheck {
                        name: "cellar_structure".to_string(),
                        status: DoctorStatus::Error,
                        message: format!(
                            "Installed package '{}' missing from Cellar at {}",
                            keg.name,
                            keg_path.display()
                        ),
                        fix: Some(format!(
                            "Run: zb uninstall {} && zb install {}",
                            keg.name, keg.name
                        )),
                    };
                }
            }
        }

        DoctorCheck {
            name: "cellar_structure".to_string(),
            status: DoctorStatus::Ok,
            message: "Cellar structure is valid".to_string(),
            fix: None,
        }
    }

    pub(crate) fn check_database_integrity(&self) -> DoctorCheck {
        // Check if we can list installed packages
        if let Err(e) = self.db.list_installed() {
            return DoctorCheck {
                name: "database_integrity".to_string(),
                status: DoctorStatus::Error,
                message: format!("Database error: {}", e),
                fix: Some("Try: zb reset && zb init".to_string()),
            };
        }

        // Check for orphaned database entries (installed but not in Cellar)
        if let Ok(installed) = self.db.list_installed() {
            for keg in &installed {
                let keg_path = self.cellar.keg_path(&keg.name, &keg.version);
                if !keg_path.exists() {
                    return DoctorCheck {
                        name: "database_integrity".to_string(),
                        status: DoctorStatus::Warning,
                        message: format!(
                            "Database references '{}' but it's not in Cellar",
                            keg.name
                        ),
                        fix: Some(format!("Run: zb uninstall {}", keg.name)),
                    };
                }
            }
        }

        DoctorCheck {
            name: "database_integrity".to_string(),
            status: DoctorStatus::Ok,
            message: "Database is consistent".to_string(),
            fix: None,
        }
    }

    pub(crate) fn check_broken_symlinks(&self) -> DoctorCheck {
        let bin_dir = self.prefix.join("bin");
        if !bin_dir.exists() {
            return DoctorCheck {
                name: "broken_symlinks".to_string(),
                status: DoctorStatus::Ok,
                message: "No bin directory yet".to_string(),
                fix: None,
            };
        }

        let mut broken = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink()
                    && let Ok(target) = std::fs::read_link(&path)
                {
                    let full_target = if target.is_relative() {
                        path.parent().unwrap().join(&target)
                    } else {
                        target
                    };
                    if !full_target.exists() {
                        broken.push(path);
                    }
                }
            }
        }

        if broken.is_empty() {
            DoctorCheck {
                name: "broken_symlinks".to_string(),
                status: DoctorStatus::Ok,
                message: "No broken symlinks in bin/".to_string(),
                fix: None,
            }
        } else {
            DoctorCheck {
                name: "broken_symlinks".to_string(),
                status: DoctorStatus::Warning,
                message: format!(
                    "{} broken symlinks in bin/: {}",
                    broken.len(),
                    broken
                        .iter()
                        .take(3)
                        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                fix: Some("Run: zb cleanup".to_string()),
            }
        }
    }

    pub(crate) async fn check_missing_dependencies(&self) -> Vec<DoctorCheck> {
        let mut checks = Vec::new();
        let installed = match self.db.list_installed() {
            Ok(i) => i,
            Err(_) => return checks,
        };

        for keg in &installed {
            // Get formula to check dependencies
            if let Ok(formula) = self.api_client.get_formula(&keg.name).await {
                let deps = formula.effective_dependencies();
                let missing: Vec<_> = deps
                    .iter()
                    .filter(|d| !self.is_installed(d))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    checks.push(DoctorCheck {
                        name: "missing_dependencies".to_string(),
                        status: DoctorStatus::Warning,
                        message: format!(
                            "'{}' is missing dependencies: {}",
                            keg.name,
                            missing.join(", ")
                        ),
                        fix: Some(format!("Run: zb install {}", missing.join(" "))),
                    });
                }
            }
        }

        if checks.is_empty() {
            checks.push(DoctorCheck {
                name: "missing_dependencies".to_string(),
                status: DoctorStatus::Ok,
                message: "All dependencies are installed".to_string(),
                fix: None,
            });
        }

        checks
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn check_patchelf(&self) -> DoctorCheck {
        // Check if patchelf is available
        match std::process::Command::new("patchelf")
            .arg("--version")
            .output()
        {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                DoctorCheck {
                    name: "patchelf".to_string(),
                    status: DoctorStatus::Ok,
                    message: format!("patchelf is installed ({})", version.trim()),
                    fix: None,
                }
            }
            _ => DoctorCheck {
                name: "patchelf".to_string(),
                status: DoctorStatus::Warning,
                message: "patchelf is not installed - binary patching will be skipped".to_string(),
                fix: Some("Install with: apt install patchelf, dnf install patchelf, or zb install patchelf".to_string()),
            },
        }
    }

    pub(crate) fn check_directory_permissions(&self) -> Vec<DoctorCheck> {
        let mut checks = Vec::new();
        let prefix = &self.prefix;

        let dirs_to_check = [
            prefix.to_path_buf(),
            prefix.join("bin"),
            prefix.join("Cellar"),
            prefix.join("opt"),
        ];

        for dir in &dirs_to_check {
            if !dir.exists() {
                continue; // Will be created on first use
            }

            // Check if writable
            let test_file = dir.join(".zb_doctor_test");
            if std::fs::write(&test_file, b"test").is_err() {
                checks.push(DoctorCheck {
                    name: "directory_permissions".to_string(),
                    status: DoctorStatus::Error,
                    message: format!("Directory '{}' is not writable", dir.display()),
                    fix: Some(format!("Run: sudo chown -R $USER {}", dir.display())),
                });
            } else {
                let _ = std::fs::remove_file(&test_file);
            }
        }

        if checks.is_empty() {
            checks.push(DoctorCheck {
                name: "directory_permissions".to_string(),
                status: DoctorStatus::Ok,
                message: "All directories have correct permissions".to_string(),
                fix: None,
            });
        }

        checks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    use crate::api::ApiClient;
    use crate::blob::BlobCache;
    use crate::db::Database;
    use crate::link::Linker;
    use crate::materialize::Cellar;
    use crate::store::Store;
    use crate::tap::TapManager;

    /// Create an Installer for testing with minimal setup
    fn create_test_installer_for_doctor(tmp: &TempDir) -> Installer {
        let root = tmp.path().join("zerobrew");
        let prefix = tmp.path().join("homebrew");
        fs::create_dir_all(root.join("db")).unwrap();
        fs::create_dir_all(&prefix).unwrap();

        let api_client = ApiClient::new();
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

    #[test]
    fn check_prefix_writable_fails_for_readonly() {
        let tmp = TempDir::new().unwrap();
        let installer = create_test_installer_for_doctor(&tmp);

        // Make the prefix read-only
        let prefix = &installer.prefix;
        let permissions = fs::Permissions::from_mode(0o555);
        fs::set_permissions(prefix, permissions).unwrap();

        let check = installer.check_prefix_writable();

        // Restore permissions before assertions (for cleanup)
        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(prefix, permissions).unwrap();

        assert_eq!(check.status, DoctorStatus::Error);
        assert!(check.message.contains("not writable"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn check_prefix_writable_succeeds_for_writable() {
        let tmp = TempDir::new().unwrap();
        let installer = create_test_installer_for_doctor(&tmp);

        let check = installer.check_prefix_writable();

        assert_eq!(check.status, DoctorStatus::Ok);
        assert!(check.message.contains("writable"));
    }

    #[test]
    fn check_broken_symlinks_finds_dangling() {
        let tmp = TempDir::new().unwrap();
        let installer = create_test_installer_for_doctor(&tmp);

        // Create bin directory with a broken symlink
        let bin_dir = installer.prefix.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        // Create a symlink to a non-existent target
        let broken_link = bin_dir.join("broken-cmd");
        let nonexistent_target = tmp.path().join("nonexistent-target");
        symlink(&nonexistent_target, &broken_link).unwrap();

        let check = installer.check_broken_symlinks();

        assert_eq!(check.status, DoctorStatus::Warning);
        assert!(check.message.contains("broken symlinks"));
        assert!(check.message.contains("broken-cmd"));
        assert!(check.fix.is_some());
    }

    #[test]
    fn check_broken_symlinks_ok_when_no_broken_links() {
        let tmp = TempDir::new().unwrap();
        let installer = create_test_installer_for_doctor(&tmp);

        // Create bin directory with a valid symlink
        let bin_dir = installer.prefix.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();

        // Create a valid target and symlink to it
        let target = bin_dir.join("real-binary");
        fs::write(&target, b"#!/bin/sh\necho hello").unwrap();
        let valid_link = bin_dir.join("valid-cmd");
        symlink(&target, &valid_link).unwrap();

        let check = installer.check_broken_symlinks();

        assert_eq!(check.status, DoctorStatus::Ok);
        assert!(check.message.contains("No broken symlinks"));
    }

    #[tokio::test]
    async fn check_missing_deps_detects_missing() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("zerobrew");
        let prefix = tmp.path().join("homebrew");
        fs::create_dir_all(root.join("db")).unwrap();
        fs::create_dir_all(&prefix).unwrap();

        // Create formula JSON that has a dependency
        let formula_json = r#"{
            "name": "testpkg",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["missing-dep"],
            "bottle": {
                "stable": {
                    "files": {}
                }
            }
        }"#;

        Mock::given(method("GET"))
            .and(path("/testpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
            .mount(&mock_server)
            .await;

        let api_client = ApiClient::with_base_url(mock_server.uri());
        let blob_cache = BlobCache::new(&root.join("cache")).unwrap();
        let store = Store::new(&root).unwrap();
        let cellar = Cellar::new(&root).unwrap();
        let linker = Linker::new(&prefix).unwrap();
        let db = Database::open(&root.join("db/zb.sqlite3")).unwrap();
        let taps_dir = root.join("taps");
        fs::create_dir_all(&taps_dir).unwrap();
        let tap_manager = TapManager::new(&taps_dir);

        let mut installer = Installer::new(
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
        );

        // Record testpkg as installed in the database
        {
            let tx = installer.db.transaction().unwrap();
            tx.record_install("testpkg", "1.0.0", "abc123", true)
                .unwrap();
            tx.commit().unwrap();
        }

        let checks = installer.check_missing_dependencies().await;

        // Should find the missing dependency
        assert!(!checks.is_empty());
        let missing_check = checks
            .iter()
            .find(|c| c.message.contains("missing-dep"))
            .expect("Should find missing-dep in checks");
        assert_eq!(missing_check.status, DoctorStatus::Warning);
        assert!(missing_check.fix.is_some());
    }

    #[tokio::test]
    async fn check_missing_deps_ok_when_all_present() {
        let tmp = TempDir::new().unwrap();
        let installer = create_test_installer_for_doctor(&tmp);

        // No packages installed, so no missing deps
        let checks = installer.check_missing_dependencies().await;

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, DoctorStatus::Ok);
        assert!(checks[0].message.contains("All dependencies are installed"));
    }

    use std::os::unix::fs::PermissionsExt;
}
