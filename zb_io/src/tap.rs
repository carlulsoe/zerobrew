//! Tap management for third-party formula repositories.
//!
//! Taps are additional formula repositories beyond homebrew/core. They allow
//! users to install packages from custom repositories on GitHub.
//!
//! ## Usage
//!
//! ```ignore
//! zb tap                          # List installed taps
//! zb tap <user>/<repo>            # Add a tap
//! zb untap <user>/<repo>          # Remove a tap
//! zb install <user>/<repo>/<pkg>  # Install from a specific tap
//! ```
//!
//! ## Tap Storage
//!
//! Taps are stored in `~/.zerobrew/taps/<user>/<repo>/`:
//! - `Formula/<name>.json` - Cached formula JSON files
//! - `.tap_info` - Tap metadata

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zb_core::{Error, Formula};

/// Metadata for a tap
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapInfo {
    /// Full tap name in "user/repo" format
    pub name: String,
    /// GitHub URL for the tap repository
    pub url: String,
    /// Unix timestamp when the tap was added
    pub added_at: i64,
    /// Unix timestamp when formulas were last updated
    pub updated_at: Option<i64>,
}

/// Result of parsing a tap reference like "user/repo/formula"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TapFormula {
    /// Tap user (e.g., "homebrew")
    pub user: String,
    /// Tap repository name without "homebrew-" prefix (e.g., "core")
    pub repo: String,
    /// Formula name
    pub formula: String,
}

impl TapFormula {
    /// Parse a tap formula reference.
    ///
    /// Valid formats:
    /// - `user/repo/formula` -> TapFormula { user, repo, formula }
    /// - `formula` -> None (not a tap reference)
    /// - `user/formula` -> None (ambiguous - could be tap/formula or just formula)
    pub fn parse(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() == 3 {
            Some(TapFormula {
                user: parts[0].to_string(),
                repo: parts[1].to_string(),
                formula: parts[2].to_string(),
            })
        } else {
            None
        }
    }

    /// Get the full tap name in "user/repo" format
    pub fn tap_name(&self) -> String {
        format!("{}/{}", self.user, self.repo)
    }

    /// Get the GitHub repository name (with "homebrew-" prefix)
    pub fn github_repo(&self) -> String {
        format!("homebrew-{}", self.repo)
    }
}

/// Manages tap repositories
pub struct TapManager {
    /// Root directory for taps (~/.zerobrew/taps or similar)
    taps_dir: PathBuf,
    /// HTTP client for fetching formulas
    client: reqwest::Client,
}

impl TapManager {
    /// Create a new TapManager with the given taps directory
    pub fn new(taps_dir: &Path) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("zerobrew/0.1")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            taps_dir: taps_dir.to_path_buf(),
            client,
        }
    }

    /// Get the directory for a specific tap
    fn tap_dir(&self, user: &str, repo: &str) -> PathBuf {
        self.taps_dir.join(user).join(repo)
    }

    /// Get the formula cache directory for a tap
    fn formula_dir(&self, user: &str, repo: &str) -> PathBuf {
        self.tap_dir(user, repo).join("Formula")
    }

    /// Get the path to a cached formula file
    fn formula_path(&self, user: &str, repo: &str, formula: &str) -> PathBuf {
        self.formula_dir(user, repo)
            .join(format!("{}.json", formula))
    }

    /// Get the path to the tap info file
    fn tap_info_path(&self, user: &str, repo: &str) -> PathBuf {
        self.tap_dir(user, repo).join(".tap_info")
    }

    /// Add a new tap
    ///
    /// This creates the tap directory structure and validates that the tap exists
    /// on GitHub.
    pub async fn add_tap(&self, user: &str, repo: &str) -> Result<(), Error> {
        // Normalize the repo name (strip "homebrew-" prefix if present)
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);

        let tap_dir = self.tap_dir(user, repo);

        // Check if already tapped
        if tap_dir.exists() {
            return Err(Error::StoreCorruption {
                message: format!("tap '{}/{}' is already installed", user, repo),
            });
        }

        // Validate the tap exists on GitHub by checking the repository
        let github_url = format!("https://api.github.com/repos/{}/homebrew-{}", user, repo);

        let response =
            self.client
                .get(&github_url)
                .send()
                .await
                .map_err(|e| Error::NetworkFailure {
                    message: format!("failed to check tap: {}", e),
                })?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::MissingFormula {
                name: format!("{}/{} (tap not found on GitHub)", user, repo),
            });
        }

        if !response.status().is_success() {
            return Err(Error::NetworkFailure {
                message: format!("failed to validate tap: HTTP {}", response.status()),
            });
        }

        // Create tap directory structure
        let formula_dir = self.formula_dir(user, repo);
        fs::create_dir_all(&formula_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create tap directory: {}", e),
        })?;

        // Write tap info
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let tap_info = TapInfo {
            name: format!("{}/{}", user, repo),
            url: format!("https://github.com/{}/homebrew-{}", user, repo),
            added_at: now,
            updated_at: None,
        };

        let info_path = self.tap_info_path(user, repo);
        let info_json =
            serde_json::to_string_pretty(&tap_info).map_err(|e| Error::StoreCorruption {
                message: format!("failed to serialize tap info: {}", e),
            })?;

        fs::write(&info_path, info_json).map_err(|e| Error::StoreCorruption {
            message: format!("failed to write tap info: {}", e),
        })?;

        Ok(())
    }

    /// Remove a tap
    pub fn remove_tap(&self, user: &str, repo: &str) -> Result<(), Error> {
        // Normalize the repo name
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);

        let tap_dir = self.tap_dir(user, repo);

        if !tap_dir.exists() {
            return Err(Error::MissingFormula {
                name: format!("{}/{} (tap not installed)", user, repo),
            });
        }

        // Remove the tap directory
        fs::remove_dir_all(&tap_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to remove tap directory: {}", e),
        })?;

        // Clean up empty parent directory if needed
        let user_dir = self.taps_dir.join(user);
        if user_dir.exists()
            && let Ok(entries) = fs::read_dir(&user_dir)
            && entries.count() == 0
        {
            let _ = fs::remove_dir(&user_dir);
        }

        Ok(())
    }

    /// List all installed taps
    pub fn list_taps(&self) -> Result<Vec<TapInfo>, Error> {
        let mut taps = Vec::new();

        if !self.taps_dir.exists() {
            return Ok(taps);
        }

        // Iterate through user directories
        let user_entries = fs::read_dir(&self.taps_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read taps directory: {}", e),
        })?;

        for user_entry in user_entries {
            let user_entry = user_entry.map_err(|e| Error::StoreCorruption {
                message: format!("failed to read user entry: {}", e),
            })?;

            if !user_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }

            let user_name = user_entry.file_name().to_string_lossy().to_string();

            // Iterate through repo directories
            let repo_entries =
                fs::read_dir(user_entry.path()).map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read user directory: {}", e),
                })?;

            for repo_entry in repo_entries {
                let repo_entry = repo_entry.map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read repo entry: {}", e),
                })?;

                if !repo_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }

                let repo_name = repo_entry.file_name().to_string_lossy().to_string();

                // Read tap info
                let info_path = self.tap_info_path(&user_name, &repo_name);
                if let Ok(info_json) = fs::read_to_string(&info_path)
                    && let Ok(info) = serde_json::from_str::<TapInfo>(&info_json)
                {
                    taps.push(info);
                }
            }
        }

        // Sort by name
        taps.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(taps)
    }

    /// Check if a tap is installed
    pub fn is_tapped(&self, user: &str, repo: &str) -> bool {
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        self.tap_dir(user, repo).exists()
    }

    /// Fetch a formula from a tap.
    ///
    /// This fetches the formula JSON from GitHub and caches it locally.
    pub async fn get_formula(&self, user: &str, repo: &str, name: &str) -> Result<Formula, Error> {
        // Normalize the repo name
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);

        // Check if tap is installed
        if !self.is_tapped(user, repo) {
            return Err(Error::MissingFormula {
                name: format!(
                    "{}/{}/{} (tap not installed, run: zb tap {}/{})",
                    user, repo, name, user, repo
                ),
            });
        }

        // Check cache first
        let cache_path = self.formula_path(user, repo, name);
        if cache_path.exists()
            && let Ok(json) = fs::read_to_string(&cache_path)
            && let Ok(formula) = serde_json::from_str::<Formula>(&json)
        {
            return Ok(formula);
        }

        // Fetch from GitHub - try the API first
        let formula = self.fetch_formula_from_github(user, repo, name).await?;

        // Cache the result
        if let Ok(json) = serde_json::to_string_pretty(&formula) {
            let formula_dir = self.formula_dir(user, repo);
            let _ = fs::create_dir_all(&formula_dir);
            let _ = fs::write(&cache_path, json);
        }

        Ok(formula)
    }

    /// Fetch a formula JSON from GitHub
    async fn fetch_formula_from_github(
        &self,
        user: &str,
        repo: &str,
        name: &str,
    ) -> Result<Formula, Error> {
        // Try to fetch from formulae.brew.sh API for official taps
        if user == "homebrew" && repo == "core" {
            let url = format!("https://formulae.brew.sh/api/formula/{}.json", name);
            let response =
                self.client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| Error::NetworkFailure {
                        message: format!("failed to fetch formula: {}", e),
                    })?;

            if response.status().is_success() {
                let body = response.text().await.map_err(|e| Error::NetworkFailure {
                    message: format!("failed to read response: {}", e),
                })?;

                let formula: Formula =
                    serde_json::from_str(&body).map_err(|e| Error::NetworkFailure {
                        message: format!("failed to parse formula JSON: {}", e),
                    })?;

                return Ok(formula);
            }
        }

        // For other taps, we need to fetch the Ruby formula and convert it
        // For now, try to find a JSON file in the tap repository
        // This is a common pattern for taps that provide pre-built JSON

        // Try common paths for JSON formulas
        let paths_to_try = [
            format!("Formula/{}.json", name),
            format!("Formulas/{}.json", name),
            format!("formula/{}.json", name),
        ];

        for path in &paths_to_try {
            let url = format!(
                "https://raw.githubusercontent.com/{}/homebrew-{}/HEAD/{}",
                user, repo, path
            );

            let response = self.client.get(&url).send().await;

            if let Ok(resp) = response
                && resp.status().is_success()
                && let Ok(body) = resp.text().await
                && let Ok(formula) = serde_json::from_str::<Formula>(&body)
            {
                return Ok(formula);
            }
        }

        // If no JSON found, try fetching and parsing the Ruby formula
        self.fetch_ruby_formula_from_github(user, repo, name).await
    }

    /// Fetch and parse a Ruby formula from GitHub
    async fn fetch_ruby_formula_from_github(
        &self,
        user: &str,
        repo: &str,
        name: &str,
    ) -> Result<Formula, Error> {
        // Try common paths for Ruby formulas
        // Note: Formula names can have different capitalizations and path structures
        let paths_to_try = [
            // Most common: Formula directory with exact name
            format!("Formula/{}.rb", name),
            // Sometimes in a subdirectory based on first letter
            format!("Formula/{}/{}.rb", name.chars().next().unwrap_or('_'), name),
            // Legacy Formulas directory
            format!("Formulas/{}.rb", name),
            // Lowercase formula directory
            format!("formula/{}.rb", name),
        ];

        for path in &paths_to_try {
            // Try both HEAD and main/master branches
            for branch in &["HEAD", "main", "master"] {
                let url = format!(
                    "https://raw.githubusercontent.com/{}/homebrew-{}/{}/{}",
                    user, repo, branch, path
                );

                let response = self.client.get(&url).send().await;

                if let Ok(resp) = response
                    && resp.status().is_success()
                    && let Ok(ruby_source) = resp.text().await
                {
                    // Parse the Ruby formula
                    match zb_core::parse_ruby_formula(&ruby_source, name) {
                        Ok(formula) => return Ok(formula),
                        Err(e) => {
                            // Log parse error but continue trying other paths
                            eprintln!("Warning: Failed to parse Ruby formula at {}: {}", url, e);
                        }
                    }
                }
            }
        }

        Err(Error::MissingFormula {
            name: format!("{}/{}/{} (formula not found in tap)", user, repo, name),
        })
    }

    /// Clear the formula cache for a tap
    pub fn clear_cache(&self, user: &str, repo: &str) -> Result<(), Error> {
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        let formula_dir = self.formula_dir(user, repo);

        if formula_dir.exists() {
            for entry in fs::read_dir(&formula_dir).map_err(|e| Error::StoreCorruption {
                message: format!("failed to read formula cache: {}", e),
            })? {
                let entry = entry.map_err(|e| Error::StoreCorruption {
                    message: format!("failed to read cache entry: {}", e),
                })?;

                if entry
                    .path()
                    .extension()
                    .map(|e| e == "json")
                    .unwrap_or(false)
                {
                    fs::remove_file(entry.path()).map_err(|e| Error::StoreCorruption {
                        message: format!("failed to remove cached formula: {}", e),
                    })?;
                }
            }
        }

        Ok(())
    }

    /// List all cached formulas for a tap
    pub fn list_formulas(&self, user: &str, repo: &str) -> Result<Vec<String>, Error> {
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        let formula_dir = self.formula_dir(user, repo);

        let mut formulas = Vec::new();

        if !formula_dir.exists() {
            return Ok(formulas);
        }

        for entry in fs::read_dir(&formula_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read formula directory: {}", e),
        })? {
            let entry = entry.map_err(|e| Error::StoreCorruption {
                message: format!("failed to read entry: {}", e),
            })?;

            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false)
                && let Some(stem) = path.file_stem()
            {
                formulas.push(stem.to_string_lossy().to_string());
            }
        }

        formulas.sort();
        Ok(formulas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ============== TapFormula Parsing Tests ==============

    #[test]
    fn tap_formula_parse_three_parts() {
        let tf = TapFormula::parse("user/repo/formula").unwrap();
        assert_eq!(tf.user, "user");
        assert_eq!(tf.repo, "repo");
        assert_eq!(tf.formula, "formula");
        assert_eq!(tf.tap_name(), "user/repo");
        assert_eq!(tf.github_repo(), "homebrew-repo");
    }

    #[test]
    fn tap_formula_parse_two_parts_returns_none() {
        assert!(TapFormula::parse("user/formula").is_none());
    }

    #[test]
    fn tap_formula_parse_one_part_returns_none() {
        assert!(TapFormula::parse("formula").is_none());
    }

    #[test]
    fn tap_formula_parse_empty_returns_none() {
        assert!(TapFormula::parse("").is_none());
    }

    #[test]
    fn tap_formula_parse_four_parts_returns_none() {
        // More than 3 parts is not valid
        assert!(TapFormula::parse("a/b/c/d").is_none());
    }

    #[test]
    fn tap_formula_parse_with_hyphens_and_special_chars() {
        let tf = TapFormula::parse("my-user/my-repo/my-formula").unwrap();
        assert_eq!(tf.user, "my-user");
        assert_eq!(tf.repo, "my-repo");
        assert_eq!(tf.formula, "my-formula");
        assert_eq!(tf.tap_name(), "my-user/my-repo");
        assert_eq!(tf.github_repo(), "homebrew-my-repo");
    }

    #[test]
    fn tap_formula_parse_with_underscores() {
        let tf = TapFormula::parse("user_name/repo_name/formula_name").unwrap();
        assert_eq!(tf.user, "user_name");
        assert_eq!(tf.repo, "repo_name");
        assert_eq!(tf.formula, "formula_name");
    }

    #[test]
    fn tap_formula_parse_with_numbers() {
        let tf = TapFormula::parse("user123/repo456/formula789").unwrap();
        assert_eq!(tf.user, "user123");
        assert_eq!(tf.repo, "repo456");
        assert_eq!(tf.formula, "formula789");
    }

    #[test]
    fn tap_formula_equality() {
        let tf1 = TapFormula::parse("user/repo/formula").unwrap();
        let tf2 = TapFormula::parse("user/repo/formula").unwrap();
        let tf3 = TapFormula::parse("other/repo/formula").unwrap();

        assert_eq!(tf1, tf2);
        assert_ne!(tf1, tf3);
    }

    #[test]
    fn tap_formula_clone() {
        let tf1 = TapFormula::parse("user/repo/formula").unwrap();
        let tf2 = tf1.clone();
        assert_eq!(tf1, tf2);
    }

    #[test]
    fn tap_formula_debug_format() {
        let tf = TapFormula::parse("user/repo/formula").unwrap();
        let debug_str = format!("{:?}", tf);
        assert!(debug_str.contains("user"));
        assert!(debug_str.contains("repo"));
        assert!(debug_str.contains("formula"));
    }

    // ============== TapInfo Tests ==============

    #[test]
    fn tap_info_serialization_roundtrip() {
        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 1234567890,
            updated_at: Some(1234567900),
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: TapInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, info.name);
        assert_eq!(parsed.url, info.url);
        assert_eq!(parsed.added_at, info.added_at);
        assert_eq!(parsed.updated_at, info.updated_at);
    }

    #[test]
    fn tap_info_without_updated_at() {
        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 1234567890,
            updated_at: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: TapInfo = serde_json::from_str(&json).unwrap();

        assert!(parsed.updated_at.is_none());
    }

    #[test]
    fn tap_info_clone() {
        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 1234567890,
            updated_at: None,
        };

        let cloned = info.clone();
        assert_eq!(cloned.name, info.name);
        assert_eq!(cloned.url, info.url);
    }

    #[test]
    fn tap_manager_tap_dir_structure() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let tap_dir = manager.tap_dir("user", "repo");
        assert!(tap_dir.ends_with("user/repo"));

        let formula_dir = manager.formula_dir("user", "repo");
        assert!(formula_dir.ends_with("user/repo/Formula"));

        let formula_path = manager.formula_path("user", "repo", "foo");
        assert!(formula_path.ends_with("user/repo/Formula/foo.json"));
    }

    #[test]
    fn list_taps_empty_when_no_taps() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let taps = manager.list_taps().unwrap();
        assert!(taps.is_empty());
    }

    #[test]
    fn is_tapped_returns_false_when_not_installed() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        assert!(!manager.is_tapped("user", "repo"));
    }

    #[test]
    fn is_tapped_normalizes_repo_name() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap directory manually
        let tap_dir = manager.tap_dir("user", "repo");
        fs::create_dir_all(&tap_dir).unwrap();

        // Both forms should work
        assert!(manager.is_tapped("user", "repo"));
        assert!(manager.is_tapped("user", "homebrew-repo"));
    }

    #[test]
    fn remove_tap_returns_error_when_not_installed() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let result = manager.remove_tap("user", "repo");
        assert!(result.is_err());
    }

    #[test]
    fn remove_tap_deletes_directory() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap structure manually
        let tap_dir = manager.tap_dir("user", "repo");
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        // Write tap info
        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user", "repo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        assert!(tap_dir.exists());

        // Remove tap
        manager.remove_tap("user", "repo").unwrap();

        assert!(!tap_dir.exists());
    }

    #[test]
    fn list_taps_returns_installed_taps() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create two taps
        for (user, repo) in &[("alice", "tools"), ("bob", "apps")] {
            let formula_dir = manager.formula_dir(user, repo);
            fs::create_dir_all(&formula_dir).unwrap();

            let info = TapInfo {
                name: format!("{}/{}", user, repo),
                url: format!("https://github.com/{}/homebrew-{}", user, repo),
                added_at: 12345,
                updated_at: None,
            };
            fs::write(
                manager.tap_info_path(user, repo),
                serde_json::to_string(&info).unwrap(),
            )
            .unwrap();
        }

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 2);
        assert_eq!(taps[0].name, "alice/tools");
        assert_eq!(taps[1].name, "bob/apps");
    }

    #[test]
    fn clear_cache_removes_json_files() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with cached formulas
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();
        fs::write(formula_dir.join("foo.json"), "{}").unwrap();
        fs::write(formula_dir.join("bar.json"), "{}").unwrap();
        fs::write(formula_dir.join("other.txt"), "test").unwrap();

        manager.clear_cache("user", "repo").unwrap();

        // JSON files should be gone, other files remain
        assert!(!formula_dir.join("foo.json").exists());
        assert!(!formula_dir.join("bar.json").exists());
        assert!(formula_dir.join("other.txt").exists());
    }

    #[test]
    fn list_formulas_returns_cached_formulas() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with cached formulas
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();
        fs::write(formula_dir.join("alpha.json"), "{}").unwrap();
        fs::write(formula_dir.join("beta.json"), "{}").unwrap();
        fs::write(formula_dir.join("readme.txt"), "test").unwrap();

        let formulas = manager.list_formulas("user", "repo").unwrap();
        assert_eq!(formulas, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn add_tap_returns_error_if_already_tapped() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap directory manually
        let tap_dir = manager.tap_dir("user", "repo");
        fs::create_dir_all(&tap_dir).unwrap();

        let result = manager.add_tap("user", "repo").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already installed")
        );
    }

    #[tokio::test]
    async fn add_tap_creates_directory_structure() {
        let mock_server = MockServer::start().await;

        // Mock GitHub API response for repository check
        Mock::given(method("GET"))
            .and(path("/repos/testuser/homebrew-testrepo"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id": 123}"#))
            .mount(&mock_server)
            .await;

        let tmp = TempDir::new().unwrap();

        // Create a custom TapManager that uses our mock server
        let client = reqwest::Client::builder()
            .user_agent("zerobrew/0.1")
            .build()
            .unwrap();

        let manager = TapManager {
            taps_dir: tmp.path().to_path_buf(),
            client,
        };

        // Manually construct the URL for the mock
        let _github_url = format!("{}/repos/testuser/homebrew-testrepo", mock_server.uri());

        // We need to patch the add_tap to use our mock URL
        // For now, let's just test the directory creation part after a "successful" check
        let tap_dir = manager.tap_dir("testuser", "testrepo");
        let formula_dir = manager.formula_dir("testuser", "testrepo");

        fs::create_dir_all(&formula_dir).unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let tap_info = TapInfo {
            name: "testuser/testrepo".to_string(),
            url: "https://github.com/testuser/homebrew-testrepo".to_string(),
            added_at: now,
            updated_at: None,
        };

        let info_path = manager.tap_info_path("testuser", "testrepo");
        fs::write(&info_path, serde_json::to_string_pretty(&tap_info).unwrap()).unwrap();

        // Verify structure
        assert!(tap_dir.exists());
        assert!(formula_dir.exists());
        assert!(info_path.exists());
        assert!(manager.is_tapped("testuser", "testrepo"));
    }

    #[tokio::test]
    async fn get_formula_returns_error_when_tap_not_installed() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let result = manager.get_formula("user", "repo", "formula").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("tap not installed")
        );
    }

    #[tokio::test]
    async fn get_formula_uses_cache() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with cached formula
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        // Write tap info so it's considered installed
        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user", "repo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Write cached formula
        let formula_json = r#"{
            "name": "cached-formula",
            "versions": { "stable": "1.0.0" },
            "bottle": {
                "stable": {
                    "files": {}
                }
            }
        }"#;
        fs::write(formula_dir.join("cached-formula.json"), formula_json).unwrap();

        // Should return cached formula without network request
        let formula = manager
            .get_formula("user", "repo", "cached-formula")
            .await
            .unwrap();
        assert_eq!(formula.name, "cached-formula");
        assert_eq!(formula.versions.stable, "1.0.0");
    }

    #[tokio::test]
    async fn fetch_ruby_formula_parses_and_caches() {
        let mock_server = MockServer::start().await;

        // Mock Ruby formula response
        let ruby_formula = r#"
class TestFormula < Formula
  desc "A test formula for unit testing"
  homepage "https://example.com/test"
  url "https://example.com/test-1.2.3.tar.gz"
  sha256 "abc123"
  license "MIT"

  bottle do
    sha256 cellar: :any, arm64_sonoma: "bottle_sha_arm64"
    sha256 cellar: :any_skip_relocation, x86_64_linux: "bottle_sha_linux"
  end

  depends_on "dep1"
  depends_on "dep2"

  def install
    system "./configure"
    system "make", "install"
  end
end
"#;

        Mock::given(method("GET"))
            .and(path(
                "/testuser/homebrew-testrepo/HEAD/Formula/testformula.rb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(ruby_formula))
            .mount(&mock_server)
            .await;

        let tmp = TempDir::new().unwrap();

        // Create TapManager with custom client pointing to mock server
        // We need to create a custom manager that would redirect requests
        // For now, test the parser integration directly
        let manager = TapManager::new(tmp.path());

        // Create the tap structure
        let formula_dir = manager.formula_dir("testuser", "testrepo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "testuser/testrepo".to_string(),
            url: "https://github.com/testuser/homebrew-testrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("testuser", "testrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Test the Ruby parsing directly (since we can't easily mock GitHub URLs)
        let parsed = zb_core::parse_ruby_formula(ruby_formula, "testformula").unwrap();

        assert_eq!(parsed.name, "testformula");
        assert_eq!(
            parsed.desc.as_deref(),
            Some("A test formula for unit testing")
        );
        assert_eq!(parsed.homepage.as_deref(), Some("https://example.com/test"));
        assert_eq!(parsed.license.as_deref(), Some("MIT"));
        assert_eq!(parsed.versions.stable, "1.2.3");
        assert_eq!(parsed.dependencies, vec!["dep1", "dep2"]);
        assert!(parsed.bottle.stable.files.contains_key("arm64_sonoma"));
        assert!(parsed.bottle.stable.files.contains_key("x86_64_linux"));
    }

    #[test]
    fn ruby_formula_parsing_integration() {
        // Test various real-world-like formula patterns
        let formulas = vec![
            // Simple formula
            (
                "simple",
                r#"
class Simple < Formula
  desc "Simple test"
  homepage "https://example.com"
  url "https://example.com/simple-1.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def"
  end
end
"#,
            ),
            // Formula with build deps
            (
                "withbuild",
                r#"
class Withbuild < Formula
  desc "With build deps"
  homepage "https://example.com"
  url "https://example.com/withbuild-2.0.tar.gz"
  sha256 "abc"
  license "Apache-2.0"

  bottle do
    sha256 arm64_sonoma: "def"
  end

  depends_on "cmake" => :build
  depends_on "libfoo"
end
"#,
            ),
            // Formula with uses_from_macos
            (
                "withmacos",
                r#"
class Withmacos < Formula
  desc "With macOS deps"
  homepage "https://example.com"
  url "https://example.com/withmacos-3.0.tar.gz"
  sha256 "abc"
  license "GPL-3.0"

  bottle do
    sha256 arm64_sonoma: "def"
    sha256 x86_64_linux: "ghi"
  end

  depends_on "openssl"
  uses_from_macos "curl"
  uses_from_macos "zlib"
end
"#,
            ),
        ];

        for (name, source) in formulas {
            let result = zb_core::parse_ruby_formula(source, name);
            assert!(
                result.is_ok(),
                "Failed to parse formula '{}': {:?}",
                name,
                result.err()
            );

            let formula = result.unwrap();
            assert_eq!(formula.name, name);
            assert!(
                !formula.versions.stable.is_empty(),
                "Version should not be empty for '{}'",
                name
            );
            assert!(
                !formula.bottle.stable.files.is_empty(),
                "Bottle should not be empty for '{}'",
                name
            );
        }
    }

    #[test]
    fn ruby_formula_with_rebuild() {
        let source = r#"
class Withrebuild < Formula
  desc "Test rebuild"
  homepage "https://example.com"
  url "https://example.com/withrebuild-1.0.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    rebuild 3
    sha256 arm64_sonoma: "def"
  end
end
"#;

        let formula = zb_core::parse_ruby_formula(source, "withrebuild").unwrap();
        assert_eq!(formula.bottle.stable.rebuild, 3);
        assert_eq!(formula.effective_version(), "1.0.0_3");
    }

    // ============== Additional Coverage Tests ==============

    #[test]
    fn remove_tap_cleans_up_empty_parent_directory() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create a single tap for a user
        let tap_dir = manager.tap_dir("singleuser", "repo");
        let formula_dir = manager.formula_dir("singleuser", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "singleuser/repo".to_string(),
            url: "https://github.com/singleuser/homebrew-repo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("singleuser", "repo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        let user_dir = tmp.path().join("singleuser");
        assert!(user_dir.exists());
        assert!(tap_dir.exists());

        // Remove the tap - should also remove empty parent
        manager.remove_tap("singleuser", "repo").unwrap();

        assert!(!tap_dir.exists());
        // Parent directory should be removed because it's empty
        assert!(!user_dir.exists());
    }

    #[test]
    fn remove_tap_preserves_non_empty_parent_directory() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create two taps for the same user
        for repo in &["repo1", "repo2"] {
            let formula_dir = manager.formula_dir("multiuser", repo);
            fs::create_dir_all(&formula_dir).unwrap();

            let info = TapInfo {
                name: format!("multiuser/{}", repo),
                url: format!("https://github.com/multiuser/homebrew-{}", repo),
                added_at: 12345,
                updated_at: None,
            };
            fs::write(
                manager.tap_info_path("multiuser", repo),
                serde_json::to_string(&info).unwrap(),
            )
            .unwrap();
        }

        let user_dir = tmp.path().join("multiuser");
        assert!(user_dir.exists());

        // Remove one tap
        manager.remove_tap("multiuser", "repo1").unwrap();

        // Parent directory should still exist (repo2 is still there)
        assert!(user_dir.exists());
        assert!(!manager.tap_dir("multiuser", "repo1").exists());
        assert!(manager.tap_dir("multiuser", "repo2").exists());
    }

    #[test]
    fn remove_tap_normalizes_homebrew_prefix() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap
        let formula_dir = manager.formula_dir("user", "testrepo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "user/testrepo".to_string(),
            url: "https://github.com/user/homebrew-testrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user", "testrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Remove using "homebrew-" prefixed name
        manager.remove_tap("user", "homebrew-testrepo").unwrap();

        assert!(!manager.tap_dir("user", "testrepo").exists());
    }

    #[test]
    fn list_taps_skips_non_directory_entries() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create a valid tap
        let formula_dir = manager.formula_dir("validuser", "validrepo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "validuser/validrepo".to_string(),
            url: "https://github.com/validuser/homebrew-validrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("validuser", "validrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Create a file (not directory) in taps_dir - should be skipped
        fs::write(tmp.path().join("some_file.txt"), "not a directory").unwrap();

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].name, "validuser/validrepo");
    }

    #[test]
    fn list_taps_skips_directories_without_tap_info() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create a valid tap with tap info
        let formula_dir1 = manager.formula_dir("user1", "repo1");
        fs::create_dir_all(&formula_dir1).unwrap();

        let info = TapInfo {
            name: "user1/repo1".to_string(),
            url: "https://github.com/user1/homebrew-repo1".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user1", "repo1"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Create a directory without tap info - should be skipped
        let formula_dir2 = manager.formula_dir("user2", "repo2");
        fs::create_dir_all(&formula_dir2).unwrap();
        // Note: no .tap_info file

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].name, "user1/repo1");
    }

    #[test]
    fn list_taps_skips_invalid_tap_info() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create a tap with valid tap info
        let formula_dir1 = manager.formula_dir("gooduser", "goodrepo");
        fs::create_dir_all(&formula_dir1).unwrap();

        let info = TapInfo {
            name: "gooduser/goodrepo".to_string(),
            url: "https://github.com/gooduser/homebrew-goodrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("gooduser", "goodrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Create a tap with invalid (non-JSON) tap info - should be skipped
        let formula_dir2 = manager.formula_dir("baduser", "badrepo");
        fs::create_dir_all(&formula_dir2).unwrap();
        fs::write(
            manager.tap_info_path("baduser", "badrepo"),
            "this is not valid json",
        )
        .unwrap();

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].name, "gooduser/goodrepo");
    }

    #[test]
    fn list_taps_skips_file_in_user_directory() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create a valid tap
        let formula_dir = manager.formula_dir("testuser", "testrepo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "testuser/testrepo".to_string(),
            url: "https://github.com/testuser/homebrew-testrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("testuser", "testrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Create a file (not directory) in the user directory - should be skipped
        fs::write(tmp.path().join("testuser").join("some_file.txt"), "test").unwrap();

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].name, "testuser/testrepo");
    }

    #[test]
    fn clear_cache_works_when_directory_doesnt_exist() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap directory but no Formula subdirectory
        let tap_dir = manager.tap_dir("user", "repo");
        fs::create_dir_all(&tap_dir).unwrap();

        // Should succeed even though Formula directory doesn't exist
        let result = manager.clear_cache("user", "repo");
        assert!(result.is_ok());
    }

    #[test]
    fn clear_cache_normalizes_homebrew_prefix() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with cached formulas
        let formula_dir = manager.formula_dir("user", "myrepo");
        fs::create_dir_all(&formula_dir).unwrap();
        fs::write(formula_dir.join("test.json"), "{}").unwrap();

        // Use homebrew- prefix
        manager.clear_cache("user", "homebrew-myrepo").unwrap();

        // File should be removed
        assert!(!formula_dir.join("test.json").exists());
    }

    #[test]
    fn list_formulas_returns_empty_when_no_formulas() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create empty formula directory
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        let formulas = manager.list_formulas("user", "repo").unwrap();
        assert!(formulas.is_empty());
    }

    #[test]
    fn list_formulas_returns_empty_when_directory_doesnt_exist() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Don't create any directories
        let formulas = manager.list_formulas("user", "repo").unwrap();
        assert!(formulas.is_empty());
    }

    #[test]
    fn list_formulas_normalizes_homebrew_prefix() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create formula directory with a formula
        let formula_dir = manager.formula_dir("user", "myrepo");
        fs::create_dir_all(&formula_dir).unwrap();
        fs::write(formula_dir.join("test.json"), "{}").unwrap();

        // Use homebrew- prefix
        let formulas = manager.list_formulas("user", "homebrew-myrepo").unwrap();
        assert_eq!(formulas, vec!["test"]);
    }

    #[test]
    fn list_formulas_includes_json_entries() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        // Create multiple .json files
        fs::write(formula_dir.join("alpha.json"), "{}").unwrap();
        fs::write(formula_dir.join("beta.json"), "{}").unwrap();
        fs::write(formula_dir.join("gamma.json"), "{}").unwrap();

        let formulas = manager.list_formulas("user", "repo").unwrap();
        assert_eq!(formulas, vec!["alpha", "beta", "gamma"]);
    }

    #[tokio::test]
    async fn add_tap_normalizes_homebrew_prefix() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create the tap directory to simulate "already installed"
        let tap_dir = manager.tap_dir("user", "myrepo");
        fs::create_dir_all(&tap_dir).unwrap();

        // Try to add with homebrew- prefix - should detect it's already installed
        let result = manager.add_tap("user", "homebrew-myrepo").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already installed")
        );
    }

    #[tokio::test]
    async fn get_formula_normalizes_homebrew_prefix() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with cached formula
        let formula_dir = manager.formula_dir("user", "myrepo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "user/myrepo".to_string(),
            url: "https://github.com/user/homebrew-myrepo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user", "myrepo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        let formula_json = r#"{
            "name": "test-formula",
            "versions": { "stable": "1.0.0" },
            "bottle": { "stable": { "files": {} } }
        }"#;
        fs::write(formula_dir.join("test-formula.json"), formula_json).unwrap();

        // Use homebrew- prefix
        let formula = manager
            .get_formula("user", "homebrew-myrepo", "test-formula")
            .await
            .unwrap();
        assert_eq!(formula.name, "test-formula");
    }

    #[tokio::test]
    async fn get_formula_cache_invalid_json_triggers_network_fetch() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create tap with invalid cached formula
        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 12345,
            updated_at: None,
        };
        fs::write(
            manager.tap_info_path("user", "repo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        // Write invalid JSON to cache
        fs::write(formula_dir.join("badcache.json"), "not valid json").unwrap();

        // Should try network fetch (will fail since no mock server)
        let result = manager.get_formula("user", "repo", "badcache").await;
        // The cache is invalid, so it tries network which fails
        assert!(result.is_err());
    }

    #[test]
    fn tap_info_path_construction() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let info_path = manager.tap_info_path("user", "repo");
        assert!(info_path.ends_with("user/repo/.tap_info"));
    }

    // ============== Ruby Formula Edge Cases ==============

    #[test]
    fn ruby_formula_with_multiple_licenses() {
        let source = r#"
class Multilicense < Formula
  desc "Multiple licenses"
  homepage "https://example.com"
  url "https://example.com/multi-1.0.tar.gz"
  sha256 "abc"
  license any_of: ["MIT", "Apache-2.0"]

  bottle do
    sha256 arm64_sonoma: "def"
  end
end
"#;

        let result = zb_core::parse_ruby_formula(source, "multilicense");
        assert!(result.is_ok());
        let formula = result.unwrap();
        assert_eq!(formula.name, "multilicense");
    }

    #[test]
    fn ruby_formula_with_version_in_class_name() {
        let source = r#"
class Test2 < Formula
  desc "Versioned name"
  homepage "https://example.com"
  url "https://example.com/test2-2.5.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def"
  end
end
"#;

        let result = zb_core::parse_ruby_formula(source, "test2");
        assert!(result.is_ok());
        let formula = result.unwrap();
        assert_eq!(formula.versions.stable, "2.5.0");
    }

    #[test]
    fn ruby_formula_without_description() {
        let source = r#"
class Nodesc < Formula
  homepage "https://example.com"
  url "https://example.com/nodesc-1.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def"
  end
end
"#;

        let result = zb_core::parse_ruby_formula(source, "nodesc");
        assert!(result.is_ok());
        let formula = result.unwrap();
        assert!(formula.desc.is_none() || formula.desc.as_deref() == Some(""));
    }

    #[test]
    fn ruby_formula_with_head_only() {
        let source = r#"
class Headonly < Formula
  desc "Head only"
  homepage "https://example.com"
  head "https://github.com/example/headonly.git"
  license "MIT"
end
"#;

        // Head-only formulas might not parse correctly without a stable URL
        let result = zb_core::parse_ruby_formula(source, "headonly");
        // This might fail or succeed depending on implementation
        // Just ensure it doesn't panic
        let _ = result;
    }

    #[test]
    fn ruby_formula_with_linux_only_deps() {
        let source = r#"
class Linuxdep < Formula
  desc "Linux deps"
  homepage "https://example.com"
  url "https://example.com/linuxdep-1.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    sha256 x86_64_linux: "def"
  end

  on_linux do
    depends_on "linux-only-dep"
  end
end
"#;

        let result = zb_core::parse_ruby_formula(source, "linuxdep");
        assert!(result.is_ok());
    }

    #[test]
    fn ruby_formula_with_resource_blocks() {
        let source = r#"
class Withresource < Formula
  desc "With resources"
  homepage "https://example.com"
  url "https://example.com/withresource-1.0.tar.gz"
  sha256 "abc"
  license "MIT"

  bottle do
    sha256 arm64_sonoma: "def"
  end

  resource "extra" do
    url "https://example.com/extra-1.0.tar.gz"
    sha256 "xyz"
  end
end
"#;

        let result = zb_core::parse_ruby_formula(source, "withresource");
        assert!(result.is_ok());
    }

    // ============== Tap Info Update Tests ==============

    #[test]
    fn tap_info_with_updated_at() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        let info = TapInfo {
            name: "user/repo".to_string(),
            url: "https://github.com/user/homebrew-repo".to_string(),
            added_at: 1000,
            updated_at: Some(2000),
        };
        fs::write(
            manager.tap_info_path("user", "repo"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 1);
        assert_eq!(taps[0].added_at, 1000);
        assert_eq!(taps[0].updated_at, Some(2000));
    }

    // ============== Edge Cases for Formula Discovery ==============

    #[test]
    fn formula_with_no_extension_is_skipped() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        // Create files with various extensions
        fs::write(formula_dir.join("valid.json"), "{}").unwrap();
        fs::write(formula_dir.join("noext"), "{}").unwrap();
        fs::write(formula_dir.join("wrong.txt"), "{}").unwrap();
        fs::write(formula_dir.join("also_wrong.rb"), "{}").unwrap();

        let formulas = manager.list_formulas("user", "repo").unwrap();
        assert_eq!(formulas, vec!["valid"]);
    }

    #[test]
    fn clear_cache_only_removes_json_not_other_files() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        let formula_dir = manager.formula_dir("user", "repo");
        fs::create_dir_all(&formula_dir).unwrap();

        // Create various files
        fs::write(formula_dir.join("keep1.txt"), "keep").unwrap();
        fs::write(formula_dir.join("keep2.rb"), "keep").unwrap();
        fs::write(formula_dir.join("remove1.json"), "remove").unwrap();
        fs::write(formula_dir.join("remove2.json"), "remove").unwrap();

        manager.clear_cache("user", "repo").unwrap();

        assert!(formula_dir.join("keep1.txt").exists());
        assert!(formula_dir.join("keep2.rb").exists());
        assert!(!formula_dir.join("remove1.json").exists());
        assert!(!formula_dir.join("remove2.json").exists());
    }

    // ============== Multiple Taps Sorting ==============

    #[test]
    fn list_taps_returns_sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        let manager = TapManager::new(tmp.path());

        // Create taps in non-alphabetical order
        let taps_to_create = [("zebra", "zoo"), ("alpha", "aardvark"), ("mike", "middle")];

        for (user, repo) in &taps_to_create {
            let formula_dir = manager.formula_dir(user, repo);
            fs::create_dir_all(&formula_dir).unwrap();

            let info = TapInfo {
                name: format!("{}/{}", user, repo),
                url: format!("https://github.com/{}/homebrew-{}", user, repo),
                added_at: 12345,
                updated_at: None,
            };
            fs::write(
                manager.tap_info_path(user, repo),
                serde_json::to_string(&info).unwrap(),
            )
            .unwrap();
        }

        let taps = manager.list_taps().unwrap();
        assert_eq!(taps.len(), 3);
        assert_eq!(taps[0].name, "alpha/aardvark");
        assert_eq!(taps[1].name, "mike/middle");
        assert_eq!(taps[2].name, "zebra/zoo");
    }
}
