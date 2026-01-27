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
