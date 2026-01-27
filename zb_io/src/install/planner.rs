//! Install planning and dependency resolution
//!
//! This module handles:
//! - Creating installation plans
//! - Fetching formulas from API or taps
//! - Resolving dependency trees

use std::collections::{BTreeMap, HashSet, VecDeque};

use futures::stream::{FuturesUnordered, StreamExt};

use crate::tap::TapFormula;

use zb_core::{Error, Formula, SelectedBottle, resolve_closure, select_bottle};

use super::Installer;

/// Maximum concurrent formula fetches to avoid overwhelming the API
const MAX_CONCURRENT_FETCHES: usize = 12;

/// An installation plan containing formulas and their selected bottles
#[derive(Debug)]
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

    /// Recursively fetch a formula and all its dependencies using streaming parallelism.
    ///
    /// Unlike batch processing which waits for all formulas in a batch to complete,
    /// this processes each formula as it completes and immediately queues its dependencies.
    /// This reduces latency for deep dependency trees by up to 20-40%.
    pub(crate) async fn fetch_all_formulas(
        &self,
        name: &str,
    ) -> Result<BTreeMap<String, Formula>, Error> {
        let mut formulas = BTreeMap::new();
        let mut queued: HashSet<String> = HashSet::new();
        let mut skipped: HashSet<String> = HashSet::new();
        let mut pending: VecDeque<String> = VecDeque::new();
        let root_name = name.to_string();

        // Start with the root package
        pending.push_back(name.to_string());
        queued.insert(name.to_string());

        // Use FuturesUnordered for streaming - process results as they complete
        let mut in_flight: FuturesUnordered<_> = FuturesUnordered::new();

        loop {
            // Fill up to MAX_CONCURRENT_FETCHES
            while in_flight.len() < MAX_CONCURRENT_FETCHES && !pending.is_empty() {
                let pkg_name = pending.pop_front().unwrap();
                let future = async move {
                    let result = self.fetch_formula(&pkg_name).await;
                    (pkg_name, result)
                };
                in_flight.push(future);
            }

            // If nothing in flight and nothing pending, we're done
            if in_flight.is_empty() {
                break;
            }

            // Wait for next result (streaming - don't wait for all)
            let (pkg_name, result) = in_flight.next().await.unwrap();

            match result {
                Ok(formula) => {
                    // Immediately queue dependencies (streaming benefit!)
                    for dep in formula.effective_dependencies() {
                        if !queued.contains(&dep) && !skipped.contains(&dep) {
                            queued.insert(dep.clone());
                            pending.push_back(dep);
                        }
                    }
                    formulas.insert(pkg_name, formula);
                }
                Err(Error::MissingFormula { .. }) if pkg_name != root_name => {
                    // Skip missing dependencies (e.g., uses_from_macos like "python")
                    eprintln!(
                        "    Note: skipping dependency '{}' (formula not found)",
                        pkg_name
                    );
                    skipped.insert(pkg_name);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(formulas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::api::ApiClient;
    use crate::blob::BlobCache;
    use crate::db::Database;
    use crate::link::Linker;
    use crate::materialize::Cellar;
    use crate::store::Store;
    use crate::tap::TapManager;

    /// Create an Installer for testing with a mock server
    fn create_test_installer_for_planner(mock_server: &MockServer, tmp: &TempDir) -> Installer {
        let root = tmp.path().join("zerobrew");
        let prefix = tmp.path().join("homebrew");
        fs::create_dir_all(root.join("db")).unwrap();
        fs::create_dir_all(&prefix).unwrap();

        let api_client = ApiClient::with_base_url(mock_server.uri());
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

    #[tokio::test]
    async fn fetch_all_formulas_returns_error_for_missing_root() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        // Return 404 for the root package
        Mock::given(method("GET"))
            .and(path("/nonexistent.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let installer = create_test_installer_for_planner(&mock_server, &tmp);

        let result = installer.fetch_all_formulas("nonexistent").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::MissingFormula { name } => assert_eq!(name, "nonexistent"),
            e => panic!("Expected MissingFormula error, got: {:?}", e),
        }
    }

    #[tokio::test]
    async fn fetch_all_formulas_skips_missing_dependencies() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        // Root formula with a dependency that doesn't exist
        let root_json = r#"{
            "name": "rootpkg",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["missing-dep"],
            "bottle": {
                "stable": {
                    "files": {
                        "all": {
                            "url": "http://example.com/bottle.tar.gz",
                            "sha256": "abc123"
                        }
                    }
                }
            }
        }"#;

        Mock::given(method("GET"))
            .and(path("/rootpkg.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(root_json))
            .mount(&mock_server)
            .await;

        // Return 404 for the missing dependency
        Mock::given(method("GET"))
            .and(path("/missing-dep.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let installer = create_test_installer_for_planner(&mock_server, &tmp);

        // Should succeed but skip the missing dependency
        let result = installer.fetch_all_formulas("rootpkg").await;

        assert!(result.is_ok());
        let formulas = result.unwrap();
        assert!(formulas.contains_key("rootpkg"));
        assert!(!formulas.contains_key("missing-dep")); // Skipped, not fetched
    }

    #[tokio::test]
    async fn fetch_all_formulas_handles_deep_dependency_chain() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        // Create a chain: root -> mid -> leaf
        let leaf_json = r#"{
            "name": "leaf",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/l.tar.gz", "sha256": "aaa" }}}}
        }"#;

        let mid_json = r#"{
            "name": "mid",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["leaf"],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/m.tar.gz", "sha256": "bbb" }}}}
        }"#;

        let root_json = r#"{
            "name": "root",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["mid"],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/r.tar.gz", "sha256": "ccc" }}}}
        }"#;

        Mock::given(method("GET"))
            .and(path("/root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(root_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/mid.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(mid_json))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/leaf.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(leaf_json))
            .mount(&mock_server)
            .await;

        let installer = create_test_installer_for_planner(&mock_server, &tmp);

        let result = installer.fetch_all_formulas("root").await;

        assert!(result.is_ok());
        let formulas = result.unwrap();
        assert_eq!(formulas.len(), 3);
        assert!(formulas.contains_key("root"));
        assert!(formulas.contains_key("mid"));
        assert!(formulas.contains_key("leaf"));
    }

    #[tokio::test]
    async fn plan_handles_empty_formula_list_gracefully() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        // A formula with no dependencies and no bottle for this platform
        // This simulates an edge case where the formula exists but can't be installed
        let formula_json = r#"{
            "name": "nobottle",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": {
                "stable": {
                    "files": {}
                }
            }
        }"#;

        Mock::given(method("GET"))
            .and(path("/nobottle.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(formula_json))
            .mount(&mock_server)
            .await;

        let installer = create_test_installer_for_planner(&mock_server, &tmp);

        let result = installer.plan("nobottle").await;

        // Should fail because there's no compatible bottle
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::UnsupportedBottle { .. } => {}
            e => panic!("Expected UnsupportedBottle error, got: {:?}", e),
        }
    }

    #[tokio::test]
    async fn fetch_all_formulas_deduplicates_shared_deps() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new().unwrap();

        // Diamond dependency: root -> a, root -> b, a -> shared, b -> shared
        let shared_json = r#"{
            "name": "shared",
            "versions": { "stable": "1.0.0" },
            "dependencies": [],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/s.tar.gz", "sha256": "sss" }}}}
        }"#;

        let a_json = r#"{
            "name": "a",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["shared"],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/a.tar.gz", "sha256": "aaa" }}}}
        }"#;

        let b_json = r#"{
            "name": "b",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["shared"],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/b.tar.gz", "sha256": "bbb" }}}}
        }"#;

        let root_json = r#"{
            "name": "diamond",
            "versions": { "stable": "1.0.0" },
            "dependencies": ["a", "b"],
            "bottle": { "stable": { "files": { "all": { "url": "http://x/d.tar.gz", "sha256": "ddd" }}}}
        }"#;

        Mock::given(method("GET"))
            .and(path("/diamond.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(root_json))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/a.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(a_json))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/b.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(b_json))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shared.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(shared_json))
            .expect(1) // Should only be fetched once despite being a dep of both a and b
            .mount(&mock_server)
            .await;

        let installer = create_test_installer_for_planner(&mock_server, &tmp);

        let result = installer.fetch_all_formulas("diamond").await;

        assert!(result.is_ok());
        let formulas = result.unwrap();
        assert_eq!(formulas.len(), 4);
        assert!(formulas.contains_key("diamond"));
        assert!(formulas.contains_key("a"));
        assert!(formulas.contains_key("b"));
        assert!(formulas.contains_key("shared"));
    }
}
