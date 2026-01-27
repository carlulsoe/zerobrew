//! Orphan detection and autoremove logic
//!
//! This module handles:
//! - Finding orphaned packages (dependencies no longer needed)
//! - Autoremove functionality
//! - Marking packages as explicit/dependency
//! - Source builds

use std::collections::HashSet;

use zb_core::{Error, resolve_closure};

use super::{Installer, copy_dir_recursive};

/// Result of a source build operation
#[derive(Debug, Clone)]
pub struct SourceBuildResult {
    /// Formula name
    pub name: String,
    /// Version that was built
    pub version: String,
    /// Number of files installed
    pub files_installed: usize,
    /// Number of files linked
    pub files_linked: usize,
    /// Whether this was a HEAD build
    pub head: bool,
}

impl Installer {
    /// Find orphaned packages - dependencies that are no longer needed by any explicit package.
    ///
    /// A package is considered an orphan if:
    /// 1. It was installed as a dependency (explicit = false)
    /// 2. No explicitly installed package depends on it (directly or transitively)
    pub async fn find_orphans(&self) -> Result<Vec<String>, Error> {
        let installed = self.db.list_installed()?;

        if installed.is_empty() {
            return Ok(Vec::new());
        }

        // Get packages that were installed as dependencies
        let dependency_pkgs: Vec<_> = installed.iter().filter(|k| !k.explicit).collect();

        if dependency_pkgs.is_empty() {
            return Ok(Vec::new());
        }

        // Get all explicit packages
        let explicit_pkgs: Vec<_> = installed.iter().filter(|k| k.explicit).collect();

        if explicit_pkgs.is_empty() {
            // If no explicit packages, all dependencies are orphans
            return Ok(dependency_pkgs.iter().map(|k| k.name.clone()).collect());
        }

        // Find all packages that are required by explicit packages
        let mut required: HashSet<String> = HashSet::new();

        for keg in &explicit_pkgs {
            // The explicit package itself is required
            required.insert(keg.name.clone());

            // Fetch its formula to get dependencies
            match self.api_client.get_formula(&keg.name).await {
                Ok(formula) => {
                    // Get all transitive dependencies
                    match self.fetch_all_formulas(&keg.name).await {
                        Ok(formulas) => {
                            if let Ok(deps) = resolve_closure(&keg.name, &formulas) {
                                required.extend(deps);
                            }
                        }
                        Err(_) => {
                            // If we can't fetch formulas, just use direct dependencies
                            required.extend(formula.effective_dependencies());
                        }
                    }
                }
                Err(_) => {
                    // Formula no longer in API - can't determine deps, keep it safe
                    continue;
                }
            }
        }

        // Find orphans: packages that are dependencies but not required
        let orphans: Vec<String> = dependency_pkgs
            .iter()
            .filter(|k| !required.contains(&k.name))
            .map(|k| k.name.clone())
            .collect();

        Ok(orphans)
    }

    /// Remove orphaned packages (dependencies no longer needed by any explicit package).
    ///
    /// Returns the list of packages that were removed.
    pub async fn autoremove(&mut self) -> Result<Vec<String>, Error> {
        let orphans = self.find_orphans().await?;

        if orphans.is_empty() {
            return Ok(Vec::new());
        }

        let mut removed = Vec::new();

        for name in orphans {
            match self.uninstall(&name) {
                Ok(()) => {
                    removed.push(name);
                }
                Err(e) => {
                    // Log warning but continue with other packages
                    eprintln!("    Warning: failed to remove {}: {}", name, e);
                }
            }
        }

        Ok(removed)
    }

    /// Mark a package as explicitly installed.
    ///
    /// Use this when a user explicitly installs a package that was previously
    /// installed as a dependency.
    pub fn mark_explicit(&self, name: &str) -> Result<bool, Error> {
        if self.db.get_installed(name).is_none() {
            return Err(Error::NotInstalled {
                name: name.to_string(),
            });
        }
        self.db.mark_explicit(name)
    }

    /// Mark a package as a dependency (not explicitly installed).
    pub fn mark_dependency(&self, name: &str) -> Result<bool, Error> {
        if self.db.get_installed(name).is_none() {
            return Err(Error::NotInstalled {
                name: name.to_string(),
            });
        }
        self.db.mark_dependency(name)
    }

    /// Check if a package was explicitly installed.
    pub fn is_explicit(&self, name: &str) -> bool {
        self.db.is_explicit(name)
    }

    /// List packages installed as dependencies (not explicitly).
    pub fn list_dependencies(&self) -> Result<Vec<crate::db::InstalledKeg>, Error> {
        self.db.list_dependencies()
    }

    /// Install a formula from source
    ///
    /// This method:
    /// 1. Fetches the formula metadata
    /// 2. Installs build dependencies (as bottles)
    /// 3. Downloads and verifies the source tarball
    /// 4. Builds using the detected build system
    /// 5. Installs the built files to the cellar
    /// 6. Links executables
    pub async fn install_from_source(
        &mut self,
        name: &str,
        link: bool,
        head: bool,
    ) -> Result<SourceBuildResult, Error> {
        use crate::build::{
            BuildEnvironment, Builder, clone_git_repo, download_source, extract_tarball,
        };
        use tempfile::TempDir;

        // Fetch formula
        let formula = self.fetch_formula(name).await?;

        // Check source availability
        let source_url = if head {
            formula
                .urls
                .head
                .as_ref()
                .map(|h| h.url.clone())
                .ok_or_else(|| Error::StoreCorruption {
                    message: format!("formula '{}' does not have a HEAD source", name),
                })?
        } else {
            formula
                .urls
                .stable
                .as_ref()
                .map(|s| s.url.clone())
                .ok_or_else(|| Error::StoreCorruption {
                    message: format!("formula '{}' does not have a stable source URL", name),
                })?
        };

        // Get checksum for stable builds
        let checksum = if head {
            None
        } else {
            formula
                .urls
                .stable
                .as_ref()
                .and_then(|s| s.checksum.clone())
        };

        // Install build dependencies first (as bottles if available)
        let all_deps: Vec<String> = formula
            .dependencies
            .iter()
            .chain(formula.build_dependencies.iter())
            .cloned()
            .collect();

        for dep in &all_deps {
            if !self.is_installed(dep) {
                // Try to install the dependency as a bottle
                match self.install(dep, true).await {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!(
                            "    Warning: failed to install build dependency '{}': {}",
                            dep, e
                        );
                    }
                }
            }
        }

        // Create temporary directories for build
        let build_tmp = TempDir::new().map_err(|e| Error::StoreCorruption {
            message: format!("failed to create temp directory: {}", e),
        })?;
        let staging_tmp = TempDir::new().map_err(|e| Error::StoreCorruption {
            message: format!("failed to create staging directory: {}", e),
        })?;

        // Download or clone source
        let source_dir = if head {
            let clone_dir = build_tmp.path().join("source");
            let branch = formula.urls.head.as_ref().and_then(|h| h.branch.as_deref());
            clone_git_repo(&source_url, branch, &clone_dir)?;
            clone_dir
        } else {
            let tarball_path = build_tmp.path().join("source.tar.gz");
            download_source(&source_url, &tarball_path, checksum.as_deref())?;
            extract_tarball(&tarball_path, build_tmp.path())?
        };

        // Determine version
        let version = if head {
            // For HEAD builds, use current timestamp or commit
            let now = chrono::Utc::now();
            format!("HEAD-{}", now.format("%Y%m%d%H%M%S"))
        } else {
            formula.versions.stable.clone()
        };

        // Create build environment
        let opt_dir = self.prefix.join("opt");
        let build_env = BuildEnvironment::new(
            &formula,
            source_dir.clone(),
            &self.prefix,
            &opt_dir,
            staging_tmp.path().to_path_buf(),
        );

        // Build
        let builder = Builder::new(build_env);
        let build_result = builder.build_auto(&[])?;

        if build_result.installed_files.is_empty() {
            return Err(Error::StoreCorruption {
                message: format!("build succeeded but no files were installed for '{}'", name),
            });
        }

        // Create keg in cellar from staging directory
        let keg_path = self.cellar_path.join(&formula.name).join(&version);
        if keg_path.exists() {
            std::fs::remove_dir_all(&keg_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove existing keg: {}", e),
            })?;
        }
        std::fs::create_dir_all(&keg_path).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create keg directory: {}", e),
        })?;

        // Copy files from staging to keg
        copy_dir_recursive(staging_tmp.path(), &keg_path)?;

        // Generate a unique store key for source builds
        let store_key = format!("source-{}-{}", formula.name, version);

        // Link executables if requested
        let linked_files = if link {
            self.linker.link_keg(&keg_path)?
        } else {
            Vec::new()
        };

        // Record in database
        {
            let tx = self.db.transaction()?;
            tx.record_install(&formula.name, &version, &store_key, true)?;

            for linked in &linked_files {
                tx.record_linked_file(
                    &formula.name,
                    &version,
                    &linked.link_path.to_string_lossy(),
                    &linked.target_path.to_string_lossy(),
                )?;
            }

            tx.commit()?;
        }

        Ok(SourceBuildResult {
            name: formula.name.clone(),
            version,
            files_installed: build_result.installed_files.len(),
            files_linked: linked_files.len(),
            head,
        })
    }
}
