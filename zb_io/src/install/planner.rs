//! Install planning and dependency resolution
//!
//! This module handles:
//! - Creating installation plans
//! - Fetching formulas from API or taps
//! - Resolving dependency trees

use std::collections::{BTreeMap, HashSet};

use crate::tap::TapFormula;

use zb_core::{Error, Formula, SelectedBottle, resolve_closure, select_bottle};

use super::Installer;

/// An installation plan containing formulas and their selected bottles
pub struct InstallPlan {
    pub formulas: Vec<Formula>,
    pub bottles: Vec<SelectedBottle>,
    /// The name of the root package (the one explicitly requested by the user)
    pub root_name: String,
}

impl Installer {
    /// Resolve dependencies and plan the install
    pub async fn plan(&self, name: &str) -> Result<InstallPlan, Error> {
        // Recursively fetch all formulas we need
        let formulas = self.fetch_all_formulas(name).await?;

        // Resolve in topological order
        let ordered = resolve_closure(name, &formulas)?;

        // Build list of formulas in order, selecting bottles
        // Skip dependencies that don't have compatible bottles (e.g., macOS-only packages)
        let mut result_formulas = Vec::new();
        let mut bottles = Vec::new();

        for formula_name in &ordered {
            let formula = formulas.get(formula_name).cloned().unwrap();
            match select_bottle(&formula) {
                Ok(bottle) => {
                    result_formulas.push(formula);
                    bottles.push(bottle);
                }
                Err(Error::UnsupportedBottle { .. }) if formula_name != name => {
                    // Skip dependencies without compatible bottles (e.g., libiconv on Linux)
                    // But fail if the root package doesn't have a compatible bottle
                    eprintln!(
                        "    Note: skipping dependency '{}' (no compatible bottle for this platform)",
                        formula_name
                    );
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(InstallPlan {
            formulas: result_formulas,
            bottles,
            root_name: name.to_string(),
        })
    }

    /// Fetch a single formula, checking taps if it's a tap reference
    pub(crate) async fn fetch_formula(&self, name: &str) -> Result<Formula, Error> {
        // Check if this is a tap formula reference (user/repo/formula)
        if let Some(tap_ref) = TapFormula::parse(name) {
            return self
                .tap_manager
                .get_formula(&tap_ref.user, &tap_ref.repo, &tap_ref.formula)
                .await;
        }

        // Try the main API first
        match self.api_client.get_formula(name).await {
            Ok(formula) => Ok(formula),
            Err(Error::MissingFormula { .. }) => {
                // Try installed taps in order
                let taps = self.db.list_taps().unwrap_or_default();
                for tap in &taps {
                    let parts: Vec<&str> = tap.name.split('/').collect();
                    if parts.len() == 2
                        && let Ok(formula) =
                            self.tap_manager.get_formula(parts[0], parts[1], name).await
                    {
                        return Ok(formula);
                    }
                }
                // No tap had the formula, return the original error
                Err(Error::MissingFormula {
                    name: name.to_string(),
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Recursively fetch a formula and all its dependencies in parallel batches
    pub(crate) async fn fetch_all_formulas(
        &self,
        name: &str,
    ) -> Result<BTreeMap<String, Formula>, Error> {
        let mut formulas = BTreeMap::new();
        let mut fetched: HashSet<String> = HashSet::new();
        let mut skipped: HashSet<String> = HashSet::new();
        let mut to_fetch: Vec<String> = vec![name.to_string()];
        let root_name = name.to_string();

        while !to_fetch.is_empty() {
            // Fetch current batch in parallel
            let batch: Vec<String> = to_fetch
                .drain(..)
                .filter(|n| !fetched.contains(n) && !skipped.contains(n))
                .collect();

            if batch.is_empty() {
                break;
            }

            // Mark as fetched before starting (to avoid re-queueing)
            for n in &batch {
                fetched.insert(n.clone());
            }

            // Fetch all in parallel using our tap-aware fetch method
            let futures: Vec<_> = batch.iter().map(|n| self.fetch_formula(n)).collect();

            let results = futures::future::join_all(futures).await;

            // Process results and queue new dependencies
            for (i, result) in results.into_iter().enumerate() {
                let pkg_name = &batch[i];

                match result {
                    Ok(formula) => {
                        // Queue dependencies for next batch
                        // Use effective_dependencies() to include uses_from_macos on Linux
                        for dep in formula.effective_dependencies() {
                            if !fetched.contains(&dep)
                                && !to_fetch.contains(&dep)
                                && !skipped.contains(&dep)
                            {
                                to_fetch.push(dep);
                            }
                        }

                        formulas.insert(pkg_name.clone(), formula);
                    }
                    Err(Error::MissingFormula { .. }) if *pkg_name != root_name => {
                        // For dependencies (not the root package), skip missing formulas.
                        // This can happen with uses_from_macos deps like "python" that
                        // don't have an exact Homebrew formula match.
                        eprintln!(
                            "    Note: skipping dependency '{}' (formula not found)",
                            pkg_name
                        );
                        skipped.insert(pkg_name.clone());
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(formulas)
    }
}
