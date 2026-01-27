//! Installer module for zerobrew
//!
//! This module provides the core installation functionality for managing
//! Homebrew-compatible packages. It is organized into focused submodules:
//!
//! - `planner` - Install planning and dependency resolution
//! - `executor` - Download, extraction, and linking orchestration
//! - `doctor` - Health check diagnostics
//! - `orphan` - Orphan detection and autoremove logic
//! - `upgrade` - Upgrade-specific functionality

mod doctor;
mod executor;
mod orphan;
mod planner;
mod upgrade;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::blob::BlobCache;
use crate::bundle::{self, BrewfileEntry, BundleCheckResult, BundleInstallResult};
use crate::db::{Database, InstalledTap};
use crate::download::ParallelDownloader;
use crate::link::{LinkedFile, Linker};
use crate::materialize::Cellar;
use crate::store::Store;
use crate::tap::TapManager;

use std::collections::HashSet;

use zb_core::{Error, Formula};

// Re-export public types
pub use doctor::{DoctorCheck, DoctorResult, DoctorStatus};
pub use executor::ExecuteResult;
pub use orphan::SourceBuildResult;
pub use planner::InstallPlan;
pub use upgrade::UpgradeResult;

/// Maximum number of retries for corrupted downloads
const MAX_CORRUPTION_RETRIES: usize = 3;

/// Result of a cleanup operation
#[derive(Debug, Default)]
pub struct CleanupResult {
    /// Number of unreferenced store entries removed
    pub store_entries_removed: usize,
    /// Number of old blob cache files removed
    pub blobs_removed: usize,
    /// Number of stale temp files/directories removed
    pub temp_files_removed: usize,
    /// Number of stale lock files removed
    pub locks_removed: usize,
    /// Number of HTTP cache entries removed
    pub http_cache_removed: usize,
    /// Total bytes freed
    pub bytes_freed: u64,
}

/// Dependency tree node for displaying hierarchical dependencies
#[derive(Debug, Clone)]
pub struct DepsTree {
    /// The formula name
    pub name: String,
    /// Whether this formula is installed
    pub installed: bool,
    /// Child dependencies
    pub children: Vec<DepsTree>,
}

/// Result of a link operation
#[derive(Debug, Clone)]
pub struct LinkResult {
    /// Number of files that were linked
    pub files_linked: usize,
    /// True if the keg was already linked (no changes made)
    pub already_linked: bool,
    /// True if --force was used to link a keg-only formula
    pub keg_only_forced: bool,
}

/// Internal struct for tracking processed packages during streaming install
#[derive(Clone)]
pub(crate) struct ProcessedPackage {
    pub name: String,
    pub version: String,
    pub store_key: String,
    pub linked_files: Vec<LinkedFile>,
    /// Whether this package was explicitly requested (true) or a dependency (false)
    pub explicit: bool,
}

pub struct Installer {
    pub(crate) api_client: ApiClient,
    pub(crate) downloader: ParallelDownloader,
    pub(crate) blob_cache: BlobCache,
    pub(crate) store: Store,
    pub(crate) cellar: Cellar,
    pub(crate) linker: Linker,
    pub(crate) db: Database,
    pub(crate) tap_manager: TapManager,
    pub(crate) prefix: PathBuf,
    pub(crate) cellar_path: PathBuf,
}

impl Installer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_client: ApiClient,
        blob_cache: BlobCache,
        store: Store,
        cellar: Cellar,
        linker: Linker,
        db: Database,
        tap_manager: TapManager,
        prefix: PathBuf,
        cellar_path: PathBuf,
        download_concurrency: usize,
    ) -> Self {
        Self {
            api_client,
            downloader: ParallelDownloader::new(blob_cache.clone(), download_concurrency),
            blob_cache,
            store,
            cellar,
            linker,
            db,
            tap_manager,
            prefix,
            cellar_path,
        }
    }

    // ========== Query Methods ==========

    /// Check if a formula is installed
    pub fn is_installed(&self, name: &str) -> bool {
        self.db.get_installed(name).is_some()
    }

    /// Get info about an installed formula
    pub fn get_installed(&self, name: &str) -> Option<crate::db::InstalledKeg> {
        self.db.get_installed(name)
    }

    /// List all installed formulas
    pub fn list_installed(&self) -> Result<Vec<crate::db::InstalledKeg>, Error> {
        self.db.list_installed()
    }

    /// Get API client reference for external use (e.g., outdated checks)
    pub fn api_client(&self) -> &ApiClient {
        &self.api_client
    }

    /// Get linked files for a package
    pub fn get_linked_files(&self, name: &str) -> Result<Vec<(String, String)>, Error> {
        self.db.get_linked_files(name)
    }

    /// Get formula info from API
    pub async fn get_formula(&self, name: &str) -> Result<Formula, Error> {
        self.api_client.get_formula(name).await
    }

    /// Get installed packages that depend on a given package (reverse dependencies)
    pub async fn get_dependents(&self, name: &str) -> Result<Vec<String>, Error> {
        let installed = self.db.list_installed()?;

        let mut dependents = Vec::new();

        // For each installed package, check if it depends on the target
        for keg in &installed {
            if keg.name == name {
                continue;
            }

            // Fetch the formula to get its dependencies
            if let Ok(formula) = self.api_client.get_formula(&keg.name).await {
                let deps = formula.effective_dependencies();
                if deps.iter().any(|d| d == name) {
                    dependents.push(keg.name.clone());
                }
            }
        }

        Ok(dependents)
    }

    /// Get dependencies for a formula.
    /// Returns a flat list of dependencies in topological order.
    ///
    /// # Arguments
    /// * `name` - The formula name to get dependencies for
    /// * `installed_only` - If true, only return installed dependencies
    /// * `recursive` - If true, return all transitive dependencies; if false, only direct deps
    pub async fn get_deps(
        &self,
        name: &str,
        installed_only: bool,
        recursive: bool,
    ) -> Result<Vec<String>, Error> {
        let formula = self.fetch_formula(name).await?;

        if recursive {
            // Get all transitive dependencies
            let formulas = self.fetch_all_formulas(name).await?;
            let mut deps = zb_core::resolve_closure(name, &formulas)?;
            // Remove the package itself
            deps.retain(|n| n != name);

            if installed_only {
                deps.retain(|n| self.is_installed(n));
            }

            Ok(deps)
        } else {
            // Just direct dependencies
            let mut deps = formula.effective_dependencies();

            if installed_only {
                deps.retain(|n| self.is_installed(n));
            }

            Ok(deps)
        }
    }

    /// Get a dependency tree for a formula.
    /// Returns a tree structure showing hierarchical dependencies.
    pub async fn get_deps_tree(&self, name: &str, installed_only: bool) -> Result<DepsTree, Error> {
        // Fetch all formulas for the dependency closure
        let formulas = self.fetch_all_formulas(name).await?;

        // Build the tree iteratively to avoid async recursion issues
        fn build_tree_from_formula(
            name: &str,
            formulas: &BTreeMap<String, Formula>,
            installed_only: bool,
            is_installed: &dyn Fn(&str) -> bool,
            visited: &mut std::collections::HashSet<String>,
        ) -> DepsTree {
            let installed = is_installed(name);

            // Check for cycles
            if visited.contains(name) {
                return DepsTree {
                    name: name.to_string(),
                    installed,
                    children: Vec::new(),
                };
            }
            visited.insert(name.to_string());

            // Get dependencies
            let children = if let Some(formula) = formulas.get(name) {
                let deps = formula.effective_dependencies();
                deps.into_iter()
                    .filter(|dep| !installed_only || is_installed(dep))
                    .map(|dep| {
                        build_tree_from_formula(
                            &dep,
                            formulas,
                            installed_only,
                            is_installed,
                            visited,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // Remove from visited so we can visit again on different paths (but not cycles)
            visited.remove(name);

            DepsTree {
                name: name.to_string(),
                installed,
                children,
            }
        }

        let is_installed = |n: &str| self.is_installed(n);
        let mut visited = std::collections::HashSet::new();
        Ok(build_tree_from_formula(
            name,
            &formulas,
            installed_only,
            &is_installed,
            &mut visited,
        ))
    }

    /// Get packages that use (depend on) a given formula.
    /// For installed packages, this checks which installed packages depend on this formula.
    /// This is a wrapper around get_dependents with the same logic.
    ///
    /// # Arguments
    /// * `name` - The formula name to check
    /// * `installed_only` - If true, only check installed packages (default behavior)
    /// * `recursive` - If true, also include packages that transitively depend on this formula
    pub async fn get_uses(
        &self,
        name: &str,
        installed_only: bool,
        recursive: bool,
    ) -> Result<Vec<String>, Error> {
        // For uses, we only support checking installed packages
        // (checking all formulas would require fetching the entire formula index)
        if !installed_only {
            // For now, installed_only=false behaves the same as installed_only=true
            // A full implementation would need to scan all formulas in the API
        }

        let direct_dependents = self.get_dependents(name).await?;

        if !recursive {
            return Ok(direct_dependents);
        }

        // For recursive, also find packages that transitively depend on this formula
        let mut all_dependents = std::collections::HashSet::new();
        let mut to_check = direct_dependents.clone();

        while let Some(pkg) = to_check.pop() {
            if all_dependents.insert(pkg.clone()) {
                // Find packages that depend on this dependent
                if let Ok(indirect) = self.get_dependents(&pkg).await {
                    for name in indirect {
                        if !all_dependents.contains(&name) {
                            to_check.push(name);
                        }
                    }
                }
            }
        }

        let mut result: Vec<_> = all_dependents.into_iter().collect();
        result.sort();
        Ok(result)
    }

    /// Get "leaf" packages - installed packages that no other installed package depends on.
    /// These are typically top-level packages that the user explicitly installed.
    pub async fn get_leaves(&self) -> Result<Vec<String>, Error> {
        let installed = self.db.list_installed()?;

        if installed.is_empty() {
            return Ok(Vec::new());
        }

        // Collect all dependencies of all installed packages
        let mut all_deps = std::collections::HashSet::new();

        for keg in &installed {
            // Fetch formula to get its dependencies
            if let Ok(formula) = self.api_client.get_formula(&keg.name).await {
                let deps = formula.effective_dependencies();
                for dep in deps {
                    all_deps.insert(dep);
                }
            }
        }

        // Leaves are packages that are not in any dependency list
        let mut leaves: Vec<_> = installed
            .iter()
            .filter(|keg| !all_deps.contains(&keg.name))
            .map(|keg| keg.name.clone())
            .collect();

        leaves.sort();
        Ok(leaves)
    }

    /// Get the keg path for an installed package
    pub fn keg_path(&self, name: &str) -> Option<PathBuf> {
        self.db
            .get_installed(name)
            .map(|keg| self.cellar.keg_path(name, &keg.version))
    }

    // ========== Tap Operations ==========

    /// Add a tap repository
    pub async fn add_tap(&self, user: &str, repo: &str) -> Result<(), Error> {
        // Normalize repo name
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        let tap_name = format!("{}/{}", user, repo);

        // Check if already tapped in database
        if self.db.is_tapped(&tap_name) {
            return Err(Error::StoreCorruption {
                message: format!("tap '{}' is already installed", tap_name),
            });
        }

        // Add tap via TapManager (validates and creates directory structure)
        self.tap_manager.add_tap(user, repo).await?;

        // Record in database
        let url = format!("https://github.com/{}/homebrew-{}", user, repo);
        self.db.add_tap(&tap_name, &url)?;

        Ok(())
    }

    /// Remove a tap repository
    pub fn remove_tap(&self, user: &str, repo: &str) -> Result<(), Error> {
        // Normalize repo name
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        let tap_name = format!("{}/{}", user, repo);

        // Check if tapped
        if !self.db.is_tapped(&tap_name) {
            return Err(Error::MissingFormula {
                name: format!("{} (tap not installed)", tap_name),
            });
        }

        // Remove from TapManager (deletes directory)
        self.tap_manager.remove_tap(user, repo)?;

        // Remove from database
        self.db.remove_tap(&tap_name)?;

        Ok(())
    }

    /// List all installed taps
    pub fn list_taps(&self) -> Result<Vec<InstalledTap>, Error> {
        self.db.list_taps()
    }

    /// Check if a tap is installed
    pub fn is_tapped(&self, user: &str, repo: &str) -> bool {
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        let tap_name = format!("{}/{}", user, repo);
        self.db.is_tapped(&tap_name)
    }

    /// Get reference to the TapManager
    pub fn tap_manager(&self) -> &TapManager {
        &self.tap_manager
    }

    // ========== Link Operations ==========

    /// Link an installed keg's executables to the prefix.
    ///
    /// This creates symlinks in `prefix/bin` and `prefix/opt` for the installed package.
    /// By default, will error if conflicting symlinks exist (from other packages).
    ///
    /// # Arguments
    /// * `name` - The package name to link
    /// * `overwrite` - If true, overwrite any existing symlinks that conflict
    /// * `force` - If true, link even if the formula is keg-only
    ///
    /// Returns the number of files linked
    pub fn link(&mut self, name: &str, overwrite: bool, force: bool) -> Result<LinkResult, Error> {
        // Check if installed
        let installed = self.db.get_installed(name).ok_or(Error::NotInstalled {
            name: name.to_string(),
        })?;

        let keg_path = self.cellar.keg_path(name, &installed.version);

        // Check if already linked
        if self.linker.is_linked(&keg_path) {
            return Ok(LinkResult {
                files_linked: 0,
                already_linked: true,
                keg_only_forced: false,
            });
        }

        // If overwrite is requested, unlink first (removes any conflicting symlinks)
        if overwrite {
            // First unlink this package's old links if any exist in the database
            let _ = self.linker.unlink_keg(&keg_path);
            self.db.clear_linked_files(name)?;
        }

        // Perform the link
        let linked_files = self.linker.link_keg(&keg_path)?;

        // Record the links in the database
        for linked in &linked_files {
            self.db.record_linked_file(
                name,
                &installed.version,
                &linked.link_path.to_string_lossy(),
                &linked.target_path.to_string_lossy(),
            )?;
        }

        Ok(LinkResult {
            files_linked: linked_files.len(),
            already_linked: false,
            keg_only_forced: force,
        })
    }

    /// Unlink an installed keg's executables from the prefix.
    ///
    /// This removes symlinks in `prefix/bin` and `prefix/opt` for the installed package
    /// but keeps the package installed in the Cellar.
    ///
    /// Returns the number of files unlinked
    pub fn unlink(&mut self, name: &str) -> Result<usize, Error> {
        // Check if installed
        let installed = self.db.get_installed(name).ok_or(Error::NotInstalled {
            name: name.to_string(),
        })?;

        let keg_path = self.cellar.keg_path(name, &installed.version);

        // Unlink the keg
        let unlinked = self.linker.unlink_keg(&keg_path)?;

        // Clear linked files from database
        self.db.clear_linked_files(name)?;

        Ok(unlinked.len())
    }

    /// Check if a keg is currently linked
    pub fn is_linked(&self, name: &str) -> bool {
        if let Some(installed) = self.db.get_installed(name) {
            let keg_path = self.cellar.keg_path(name, &installed.version);
            self.linker.is_linked(&keg_path)
        } else {
            false
        }
    }

    // ==================== Bundle/Brewfile Methods ====================

    /// Check which entries from a Brewfile are not satisfied
    pub fn bundle_check(&self, brewfile_path: &Path) -> Result<BundleCheckResult, Error> {
        let entries = bundle::read_brewfile(brewfile_path)?;

        // Get installed formulas
        let installed_kegs = self.db.list_installed()?;
        let installed_formulas: HashSet<String> =
            installed_kegs.iter().map(|k| k.name.clone()).collect();

        // Get installed taps
        let installed_taps_list = self.db.list_taps()?;
        let installed_taps: HashSet<String> =
            installed_taps_list.iter().map(|t| t.name.clone()).collect();

        Ok(bundle::check_brewfile(
            &entries,
            &installed_formulas,
            &installed_taps,
        ))
    }

    /// Generate a Brewfile from installed packages and taps
    pub fn bundle_dump(&self, include_comments: bool) -> Result<String, Error> {
        // Get installed taps
        let taps: Vec<String> = self
            .db
            .list_taps()?
            .iter()
            .map(|t| t.name.clone())
            .collect();

        // Get explicitly installed formulas (not dependencies)
        let formulas: Vec<String> = self
            .db
            .list_installed()?
            .iter()
            .filter(|k| k.explicit)
            .map(|k| k.name.clone())
            .collect();

        Ok(bundle::generate_brewfile(
            &taps,
            &formulas,
            include_comments,
        ))
    }

    /// Install packages from a Brewfile
    pub async fn bundle_install(
        &mut self,
        brewfile_path: &Path,
    ) -> Result<BundleInstallResult, Error> {
        let entries = bundle::read_brewfile(brewfile_path)?;

        let mut result = BundleInstallResult::default();

        // Get currently installed formulas and taps
        let installed_kegs = self.db.list_installed()?;
        let installed_formulas: HashSet<String> =
            installed_kegs.iter().map(|k| k.name.clone()).collect();

        let installed_taps_list = self.db.list_taps()?;
        let installed_taps: HashSet<String> =
            installed_taps_list.iter().map(|t| t.name.clone()).collect();

        // Process taps first
        for entry in &entries {
            if let BrewfileEntry::Tap { name } = entry {
                let normalized = bundle::check_brewfile(
                    std::slice::from_ref(entry),
                    &HashSet::new(),
                    &installed_taps,
                );
                if !normalized.missing_taps.is_empty() {
                    // Parse tap name (user/repo)
                    if let Some((user, repo)) = name.split_once('/') {
                        match self.add_tap(user, repo).await {
                            Ok(_) => {
                                result.taps_added.push(name.clone());
                            }
                            Err(e) => {
                                result.failed.push((name.clone(), e.to_string()));
                            }
                        }
                    } else {
                        result
                            .failed
                            .push((name.clone(), "invalid tap name".to_string()));
                    }
                }
            }
        }

        // Process formulas
        for entry in &entries {
            if let BrewfileEntry::Brew { name, args } = entry {
                // Extract formula name (handle user/repo/formula format)
                let formula_name = {
                    let parts: Vec<_> = name.split('/').collect();
                    if parts.len() == 3 {
                        parts[2].to_string()
                    } else {
                        name.clone()
                    }
                };

                // Check if already installed
                if installed_formulas.contains(&formula_name) {
                    result.formulas_skipped.push(name.clone());
                    continue;
                }

                // Check for HEAD flag in args
                let is_head = args.iter().any(|a| a == "--HEAD" || a == "-H");
                let is_source = args.iter().any(|a| a == "--build-from-source" || a == "-s");

                // Install the formula
                let install_result = if is_head || is_source {
                    self.install_from_source(name, true, is_head)
                        .await
                        .map(|r| r.name)
                } else {
                    self.install(name, true).await.map(|_| name.clone())
                };

                match install_result {
                    Ok(_) => {
                        result.formulas_installed.push(name.clone());
                    }
                    Err(e) => {
                        result.failed.push((name.clone(), e.to_string()));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Parse a Brewfile and return its entries
    pub fn parse_brewfile(&self, path: &Path) -> Result<Vec<BrewfileEntry>, Error> {
        bundle::read_brewfile(path)
    }

    /// Find a Brewfile in the given directory or its parents
    pub fn find_brewfile(&self, start_dir: &Path) -> Option<PathBuf> {
        bundle::find_brewfile(start_dir)
    }
}

/// Recursively copy a directory
pub(crate) fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), Error> {
    if !dst.exists() {
        std::fs::create_dir_all(dst).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create directory '{}': {}", dst.display(), e),
        })?;
    }

    for entry in std::fs::read_dir(src).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read directory '{}': {}", src.display(), e),
    })? {
        let entry = entry.map_err(|e| Error::StoreCorruption {
            message: format!("failed to read entry: {}", e),
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| Error::StoreCorruption {
                message: format!(
                    "failed to copy '{}' to '{}': {}",
                    src_path.display(),
                    dst_path.display(),
                    e
                ),
            })?;
        }
    }

    Ok(())
}

/// Create an Installer with standard paths
pub fn create_installer(
    root: &Path,
    prefix: &Path,
    download_concurrency: usize,
) -> Result<Installer, Error> {
    use std::fs;

    // First ensure the root directory exists
    if !root.exists() {
        fs::create_dir_all(root).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                Error::StoreCorruption {
                    message: format!(
                        "cannot create root directory '{}': permission denied.\n\n\
                        Create it with:\n  sudo mkdir -p {} && sudo chown $USER {}",
                        root.display(),
                        root.display(),
                        root.display()
                    ),
                }
            } else {
                Error::StoreCorruption {
                    message: format!("failed to create root directory '{}': {e}", root.display()),
                }
            }
        })?;
    }

    // Ensure all subdirectories exist
    fs::create_dir_all(root.join("db")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create db directory: {e}"),
    })?;

    // Create taps directory
    let taps_dir = root.join("taps");
    fs::create_dir_all(&taps_dir).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create taps directory: {e}"),
    })?;

    let api_client = ApiClient::new();
    let blob_cache = BlobCache::new(&root.join("cache")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create blob cache: {e}"),
    })?;
    let store = Store::new(root).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create store: {e}"),
    })?;
    // Use prefix/Cellar so bottles' hardcoded rpaths work
    let cellar = Cellar::new_at(prefix.join("Cellar")).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create cellar: {e}"),
    })?;
    let linker = Linker::new(prefix).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create linker: {e}"),
    })?;
    let db = Database::open(&root.join("db/zb.sqlite3"))?;
    let tap_manager = TapManager::new(&taps_dir);

    let cellar_path = prefix.join("Cellar");

    Ok(Installer::new(
        api_client,
        blob_cache,
        store,
        cellar,
        linker,
        db,
        tap_manager,
        prefix.to_path_buf(),
        cellar_path,
        download_concurrency,
    ))
}

#[cfg(test)]
mod tests;
