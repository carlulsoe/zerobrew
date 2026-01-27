//! Execution of install plans
//!
//! This module handles:
//! - Downloading bottles
//! - Extracting packages
//! - Linking executables
//! - Garbage collection and cleanup

use std::sync::Arc;

use crate::download::{DownloadProgressCallback, DownloadRequest, DownloadResult};
use crate::progress::{InstallProgress, ProgressCallback};

use zb_core::{Error, Formula, SelectedBottle};

use super::{CleanupResult, Installer, InstallPlan, ProcessedPackage, MAX_CORRUPTION_RETRIES};

/// Result of executing an install plan
pub struct ExecuteResult {
    pub installed: usize,
}

impl Installer {
    /// Execute the install plan
    pub async fn execute(&mut self, plan: InstallPlan, link: bool) -> Result<ExecuteResult, Error> {
        self.execute_with_progress(plan, link, None).await
    }

    /// Execute the install plan with progress callback
    /// Uses streaming extraction - starts extracting each package as soon as its download completes
    pub async fn execute_with_progress(
        &mut self,
        plan: InstallPlan,
        link: bool,
        progress: Option<Arc<ProgressCallback>>,
    ) -> Result<ExecuteResult, Error> {
        let report = |event: InstallProgress| {
            if let Some(ref cb) = progress {
                cb(event);
            }
        };

        // Track which package was explicitly requested
        let root_name = plan.root_name.clone();

        // Pair formulas with bottles
        let to_install: Vec<(Formula, SelectedBottle)> = plan
            .formulas
            .into_iter()
            .zip(plan.bottles.into_iter())
            .collect();

        if to_install.is_empty() {
            return Ok(ExecuteResult { installed: 0 });
        }

        // Download all bottles
        let requests: Vec<DownloadRequest> = to_install
            .iter()
            .map(|(f, b)| DownloadRequest {
                url: b.url.clone(),
                sha256: b.sha256.clone(),
                name: f.name.clone(),
            })
            .collect();

        // Convert progress callback for download
        let download_progress: Option<DownloadProgressCallback> = progress.clone().map(|cb| {
            Arc::new(move |event: InstallProgress| {
                cb(event);
            }) as DownloadProgressCallback
        });

        // Use streaming downloads - process each as it completes
        let mut rx = self
            .downloader
            .download_streaming(requests, download_progress.clone());

        // Track results by index to maintain install order for database records
        let total = to_install.len();
        let mut completed: Vec<Option<ProcessedPackage>> = vec![None; total];
        let mut error: Option<Error> = None;

        // Process downloads as they complete
        while let Some(result) = rx.recv().await {
            match result {
                Ok(download) => {
                    let idx = download.index;
                    let (formula, bottle) = &to_install[idx];

                    report(InstallProgress::UnpackStarted {
                        name: formula.name.clone(),
                    });

                    // Try extraction with retry logic for corrupted downloads
                    let store_entry = match self
                        .extract_with_retry(&download, formula, bottle, download_progress.clone())
                        .await
                    {
                        Ok(entry) => entry,
                        Err(e) => {
                            error = Some(e);
                            continue;
                        }
                    };

                    // Materialize to cellar
                    // Use effective_version() which includes rebuild suffix if applicable
                    let keg_path = match self.cellar.materialize(
                        &formula.name,
                        &formula.effective_version(),
                        &store_entry,
                    ) {
                        Ok(path) => path,
                        Err(e) => {
                            error = Some(e);
                            continue;
                        }
                    };

                    report(InstallProgress::UnpackCompleted {
                        name: formula.name.clone(),
                    });

                    // Link executables if requested
                    let linked_files = if link {
                        report(InstallProgress::LinkStarted {
                            name: formula.name.clone(),
                        });
                        match self.linker.link_keg(&keg_path) {
                            Ok(files) => {
                                report(InstallProgress::LinkCompleted {
                                    name: formula.name.clone(),
                                });
                                files
                            }
                            Err(e) => {
                                error = Some(e);
                                continue;
                            }
                        }
                    } else {
                        Vec::new()
                    };

                    completed[idx] = Some(ProcessedPackage {
                        name: formula.name.clone(),
                        version: formula.effective_version(),
                        store_key: bottle.sha256.clone(),
                        linked_files,
                        explicit: formula.name == root_name,
                    });
                }
                Err(e) => {
                    error = Some(e);
                }
            }
        }

        // Return error if any download failed
        if let Some(e) = error {
            return Err(e);
        }

        // Record all successful installs in database (in order)
        for processed in completed.into_iter().flatten() {
            let tx = self.db.transaction()?;
            tx.record_install(
                &processed.name,
                &processed.version,
                &processed.store_key,
                processed.explicit,
            )?;

            for linked in &processed.linked_files {
                tx.record_linked_file(
                    &processed.name,
                    &processed.version,
                    &linked.link_path.to_string_lossy(),
                    &linked.target_path.to_string_lossy(),
                )?;
            }

            tx.commit()?;
        }

        Ok(ExecuteResult {
            installed: to_install.len(),
        })
    }

    /// Convenience method to plan and execute in one call
    pub async fn install(&mut self, name: &str, link: bool) -> Result<ExecuteResult, Error> {
        let plan = self.plan(name).await?;
        self.execute(plan, link).await
    }

    /// Uninstall a formula
    pub fn uninstall(&mut self, name: &str) -> Result<(), Error> {
        // Check if installed
        let installed = self.db.get_installed(name).ok_or(Error::NotInstalled {
            name: name.to_string(),
        })?;

        // Unlink executables
        let keg_path = self.cellar.keg_path(name, &installed.version);
        self.linker.unlink_keg(&keg_path)?;

        // Remove from database (decrements store ref)
        {
            let tx = self.db.transaction()?;
            tx.record_uninstall(name)?;
            tx.commit()?;
        }

        // Remove cellar entry
        self.cellar.remove_keg(name, &installed.version)?;

        Ok(())
    }

    /// Try to extract a download, with automatic retry on corruption
    pub(crate) async fn extract_with_retry(
        &self,
        download: &DownloadResult,
        formula: &Formula,
        bottle: &SelectedBottle,
        progress: Option<DownloadProgressCallback>,
    ) -> Result<std::path::PathBuf, Error> {
        let mut blob_path = download.blob_path.clone();
        let mut last_error = None;

        for attempt in 0..MAX_CORRUPTION_RETRIES {
            match self.store.ensure_entry(&bottle.sha256, &blob_path) {
                Ok(entry) => return Ok(entry),
                Err(Error::StoreCorruption { message }) => {
                    // Remove the corrupted blob
                    self.downloader.remove_blob(&bottle.sha256);

                    if attempt + 1 < MAX_CORRUPTION_RETRIES {
                        // Log retry attempt
                        eprintln!(
                            "    Corrupted download detected for {}, retrying ({}/{})...",
                            formula.name,
                            attempt + 2,
                            MAX_CORRUPTION_RETRIES
                        );

                        // Re-download
                        let request = DownloadRequest {
                            url: bottle.url.clone(),
                            sha256: bottle.sha256.clone(),
                            name: formula.name.clone(),
                        };

                        match self
                            .downloader
                            .download_single(request, progress.clone())
                            .await
                        {
                            Ok(new_path) => {
                                blob_path = new_path;
                                // Continue to next iteration to retry extraction
                            }
                            Err(e) => {
                                last_error = Some(e);
                                break;
                            }
                        }
                    } else {
                        last_error = Some(Error::StoreCorruption {
                            message: format!(
                                "{message}\n\nFailed after {MAX_CORRUPTION_RETRIES} attempts. The download may be corrupted at the source."
                            ),
                        });
                    }
                }
                Err(e) => {
                    last_error = Some(e);
                    break;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| Error::StoreCorruption {
            message: "extraction failed with unknown error".to_string(),
        }))
    }

    /// Garbage collect unreferenced store entries
    pub fn gc(&mut self) -> Result<Vec<String>, Error> {
        let unreferenced = self.db.get_unreferenced_store_keys()?;
        let mut removed = Vec::new();

        for store_key in unreferenced {
            self.store.remove_entry(&store_key)?;
            removed.push(store_key);
        }

        Ok(removed)
    }

    /// Result of a cleanup operation
    pub fn cleanup(&mut self, prune_days: Option<u32>) -> Result<CleanupResult, Error> {
        let mut result = CleanupResult::default();

        // 1. Run GC to remove unreferenced store entries
        let gc_removed = self.gc()?;
        result.store_entries_removed = gc_removed.len();

        // 2. Get the set of store keys still in use (to keep their blobs)
        let installed = self.db.list_installed()?;
        let used_store_keys: std::collections::HashSet<String> =
            installed.iter().map(|k| k.store_key.clone()).collect();

        // 3. Clean up blobs based on prune_days
        if let Some(days) = prune_days {
            // Remove blobs older than N days that are not currently used
            let max_age = std::time::Duration::from_secs(days as u64 * 24 * 60 * 60);
            let blobs = self
                .blob_cache
                .list_blobs()
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to list blobs: {e}"),
                })?;

            for (sha256, mtime) in blobs {
                // Skip if this blob is still in use
                if used_store_keys.contains(&sha256) {
                    continue;
                }

                // Check age
                if let Ok(age) = std::time::SystemTime::now().duration_since(mtime)
                    && age > max_age
                    && self.blob_cache.remove_blob(&sha256).unwrap_or(false)
                {
                    result.blobs_removed += 1;
                    // Get size from path before removal (already removed, so estimate)
                    // Note: We can't get the size after removal, but this is fine for the result
                }
            }
        } else {
            // Remove all blobs not currently in use
            let (removed, bytes) = self
                .blob_cache
                .remove_blobs_except(&used_store_keys)
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to remove blobs: {e}"),
                })?;
            result.blobs_removed = removed.len();
            result.bytes_freed += bytes;
        }

        // 4. Clean up stale temp files in blob cache
        let (temp_count, temp_bytes) =
            self.blob_cache
                .cleanup_temp_files()
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to cleanup temp files: {e}"),
                })?;
        result.temp_files_removed += temp_count;
        result.bytes_freed += temp_bytes;

        // 5. Clean up stale temp directories in store
        let (temp_dirs, temp_dir_bytes) =
            self.store
                .cleanup_temp_dirs()
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to cleanup temp dirs: {e}"),
                })?;
        result.temp_files_removed += temp_dirs;
        result.bytes_freed += temp_dir_bytes;

        // 6. Clean up stale lock files
        let locks_removed =
            self.store
                .cleanup_stale_locks()
                .map_err(|e| Error::StoreCorruption {
                    message: format!("failed to cleanup stale locks: {e}"),
                })?;
        result.locks_removed = locks_removed;

        // 7. Clean up HTTP cache
        if let Some(days) = prune_days {
            if let Some((removed, size)) = self.api_client.cleanup_cache_older_than(days) {
                result.http_cache_removed = removed;
                result.bytes_freed += size;
            }
        } else if let Some((removed, size)) = self.api_client.clear_cache() {
            result.http_cache_removed = removed;
            result.bytes_freed += size;
        }

        Ok(result)
    }

    /// Preview what would be cleaned up (dry run)
    pub fn cleanup_dry_run(&self, prune_days: Option<u32>) -> Result<CleanupResult, Error> {
        let mut result = CleanupResult::default();

        // 1. Count unreferenced store entries
        let unreferenced = self.db.get_unreferenced_store_keys()?;
        result.store_entries_removed = unreferenced.len();

        // 2. Get the set of store keys still in use
        let installed = self.db.list_installed()?;
        let used_store_keys: std::collections::HashSet<String> =
            installed.iter().map(|k| k.store_key.clone()).collect();

        // 3. Count blobs to remove
        let blobs = self
            .blob_cache
            .list_blobs()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to list blobs: {e}"),
            })?;

        for (sha256, mtime) in blobs {
            // Skip if this blob is still in use
            if used_store_keys.contains(&sha256) {
                continue;
            }

            let blob_path = self.blob_cache.blob_path(&sha256);
            let blob_size = std::fs::metadata(&blob_path).map(|m| m.len()).unwrap_or(0);

            if let Some(days) = prune_days {
                let max_age = std::time::Duration::from_secs(days as u64 * 24 * 60 * 60);
                if let Ok(age) = std::time::SystemTime::now().duration_since(mtime)
                    && age > max_age
                {
                    result.blobs_removed += 1;
                    result.bytes_freed += blob_size;
                }
            } else {
                result.blobs_removed += 1;
                result.bytes_freed += blob_size;
            }
        }

        // 4. Count HTTP cache entries to remove
        if let Some(days) = prune_days {
            if let Some((count, size)) = self.api_client.cache_count_older_than(days) {
                result.http_cache_removed = count;
                result.bytes_freed += size;
            }
        } else if let Some((count, size)) = self.api_client.cache_stats() {
            result.http_cache_removed = count;
            result.bytes_freed += size;
        }

        Ok(result)
    }
}
