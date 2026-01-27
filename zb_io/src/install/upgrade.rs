//! Upgrade-specific logic
//!
//! This module handles:
//! - Upgrading packages
//! - Detecting outdated packages
//! - Pin/unpin functionality

use std::sync::Arc;

use crate::progress::ProgressCallback;

use zb_core::{Error, OutdatedPackage, Version};

use super::Installer;

/// Result of an upgrade operation
pub struct UpgradeResult {
    /// Number of packages upgraded
    pub upgraded: usize,
    /// Packages that were upgraded (name, old_version, new_version)
    pub packages: Vec<(String, String, String)>,
}

impl Installer {
    /// Check for outdated packages by comparing installed versions against API.
    /// By default, excludes pinned packages.
    pub async fn get_outdated(&self) -> Result<Vec<OutdatedPackage>, Error> {
        self.get_outdated_impl(false).await
    }

    /// Check for outdated packages, optionally including pinned packages
    pub async fn get_outdated_with_pinned(
        &self,
        include_pinned: bool,
    ) -> Result<Vec<OutdatedPackage>, Error> {
        self.get_outdated_impl(include_pinned).await
    }

    async fn get_outdated_impl(&self, include_pinned: bool) -> Result<Vec<OutdatedPackage>, Error> {
        let installed = self.db.list_installed()?;

        if installed.is_empty() {
            return Ok(Vec::new());
        }

        // Filter out pinned packages unless explicitly requested
        let to_check: Vec<_> = if include_pinned {
            installed
        } else {
            installed.into_iter().filter(|keg| !keg.pinned).collect()
        };

        if to_check.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch all formulas from API in parallel
        let futures: Vec<_> = to_check
            .iter()
            .map(|keg| self.api_client.get_formula(&keg.name))
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut outdated = Vec::new();

        for (keg, result) in to_check.iter().zip(results.into_iter()) {
            match result {
                Ok(formula) => {
                    let installed_ver = Version::parse(&keg.version);
                    let available_ver = Version::parse(&formula.effective_version());

                    if installed_ver.is_older_than(&available_ver) {
                        outdated.push(OutdatedPackage {
                            name: keg.name.clone(),
                            installed_version: keg.version.clone(),
                            available_version: formula.effective_version(),
                        });
                    }
                }
                Err(Error::MissingFormula { .. }) => {
                    // Formula no longer exists in API, skip it
                    continue;
                }
                Err(e) => {
                    // Log warning but continue checking other packages
                    eprintln!("    Warning: failed to check {}: {}", keg.name, e);
                }
            }
        }

        // Sort by name for consistent output
        outdated.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(outdated)
    }

    /// Upgrade a single package to its latest version
    /// Returns the old and new version if upgraded, None if already up to date
    pub async fn upgrade_one(
        &mut self,
        name: &str,
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<Option<(String, String)>, Error> {
        // Check if installed
        let installed = self.db.get_installed(name).ok_or(Error::NotInstalled {
            name: name.to_string(),
        })?;

        // Fetch new formula to check version
        let new_formula = self.api_client.get_formula(name).await?;
        let new_version = new_formula.effective_version();

        // Check if already up to date using version comparison
        let installed_ver = Version::parse(&installed.version);
        let available_ver = Version::parse(&new_version);

        if !installed_ver.is_older_than(&available_ver) {
            return Ok(None); // Already up to date
        }

        let old_version = installed.version.clone();

        // Plan the new installation (handles dependencies)
        let plan = self.plan(name).await?;

        // Unlink the old version
        let old_keg_path = self.cellar.keg_path(name, &old_version);
        self.linker.unlink_keg(&old_keg_path)?;

        // Install new version
        // Note: execute_with_progress uses INSERT OR REPLACE for database,
        // so it will automatically update the record for this package
        self.execute_with_progress(plan, link, progress).await?;

        // Remove old keg (only for the upgraded package, not dependencies)
        self.cellar.remove_keg(name, &old_version)?;

        Ok(Some((old_version, new_version)))
    }

    /// Upgrade all outdated packages
    pub async fn upgrade_all(
        &mut self,
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<UpgradeResult, Error> {
        let outdated = self.get_outdated().await?;

        if outdated.is_empty() {
            return Ok(UpgradeResult {
                upgraded: 0,
                packages: Vec::new(),
            });
        }

        let mut packages = Vec::new();

        for pkg in outdated {
            if let Some((old_ver, new_ver)) =
                self.upgrade_one(&pkg.name, link, progress.clone()).await?
            {
                packages.push((pkg.name, old_ver, new_ver));
            }
        }

        Ok(UpgradeResult {
            upgraded: packages.len(),
            packages,
        })
    }

    // ========== Pin Operations ==========

    /// List only pinned formulas
    pub fn list_pinned(&self) -> Result<Vec<crate::db::InstalledKeg>, Error> {
        self.db.list_pinned()
    }

    /// Pin a formula to prevent upgrades
    pub fn pin(&self, name: &str) -> Result<bool, Error> {
        // Check if installed first
        if self.db.get_installed(name).is_none() {
            return Err(Error::NotInstalled {
                name: name.to_string(),
            });
        }
        self.db.pin(name)
    }

    /// Unpin a formula to allow upgrades
    pub fn unpin(&self, name: &str) -> Result<bool, Error> {
        // Check if installed first
        if self.db.get_installed(name).is_none() {
            return Err(Error::NotInstalled {
                name: name.to_string(),
            });
        }
        self.db.unpin(name)
    }

    /// Check if a formula is pinned
    pub fn is_pinned(&self, name: &str) -> bool {
        self.db.is_pinned(name)
    }
}
