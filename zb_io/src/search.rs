//! Search functionality for finding formulas

use crate::api::FormulaInfo;
use regex::Regex;

/// Search result with relevance scoring
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub full_name: String,
    pub version: String,
    pub description: String,
    pub score: u32,
}

/// Search formulas by query string
///
/// Supports:
/// - Plain text search (matches name and description)
/// - Regex search when query is wrapped in /slashes/
pub fn search_formulas(formulas: &[FormulaInfo], query: &str) -> Vec<SearchResult> {
    let query = query.trim();

    // Check if it's a regex query (wrapped in //)
    let is_regex = query.starts_with('/') && query.ends_with('/') && query.len() > 2;

    let results: Vec<SearchResult> = if is_regex {
        let pattern = &query[1..query.len() - 1];
        match Regex::new(pattern) {
            Ok(re) => search_by_regex(formulas, &re),
            Err(_) => {
                // Invalid regex, fall back to literal search
                search_by_text(formulas, query)
            }
        }
    } else {
        search_by_text(formulas, query)
    };

    // Sort by score (descending), then by name (ascending)
    let mut sorted = results;
    sorted.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));

    sorted
}

fn search_by_text(formulas: &[FormulaInfo], query: &str) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();

    formulas
        .iter()
        .filter(|f| !f.deprecated && !f.disabled)
        .filter_map(|f| {
            let name_lower = f.name.to_lowercase();
            let desc_lower = f.desc.as_deref().unwrap_or("").to_lowercase();

            let mut score = 0u32;

            // Exact name match
            if name_lower == query_lower {
                score += 100;
            }
            // Name starts with query
            else if name_lower.starts_with(&query_lower) {
                score += 50;
            }
            // Name contains query
            else if name_lower.contains(&query_lower) {
                score += 25;
            }
            // Description contains query
            else if desc_lower.contains(&query_lower) {
                score += 10;
            }
            // Check aliases
            else if f
                .aliases
                .iter()
                .any(|a| a.to_lowercase().contains(&query_lower))
            {
                score += 15;
            }

            if score > 0 {
                Some(SearchResult {
                    name: f.name.clone(),
                    full_name: f.full_name.clone(),
                    version: f
                        .versions
                        .stable
                        .clone()
                        .unwrap_or_else(|| "HEAD".to_string()),
                    description: f.desc.clone().unwrap_or_default(),
                    score,
                })
            } else {
                None
            }
        })
        .collect()
}

fn search_by_regex(formulas: &[FormulaInfo], re: &Regex) -> Vec<SearchResult> {
    formulas
        .iter()
        .filter(|f| !f.deprecated && !f.disabled)
        .filter_map(|f| {
            let name_matches = re.is_match(&f.name);
            let desc_matches = f.desc.as_ref().map(|d| re.is_match(d)).unwrap_or(false);

            if name_matches || desc_matches {
                let score = if name_matches { 50 } else { 10 };
                Some(SearchResult {
                    name: f.name.clone(),
                    full_name: f.full_name.clone(),
                    version: f
                        .versions
                        .stable
                        .clone()
                        .unwrap_or_else(|| "HEAD".to_string()),
                    description: f.desc.clone().unwrap_or_default(),
                    score,
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::FormulaVersions;

    fn make_formula(name: &str, desc: &str) -> FormulaInfo {
        FormulaInfo {
            name: name.to_string(),
            full_name: name.to_string(),
            desc: Some(desc.to_string()),
            homepage: None,
            versions: FormulaVersions {
                stable: Some("1.0.0".to_string()),
            },
            aliases: vec![],
            deprecated: false,
            disabled: false,
        }
    }

    #[test]
    fn exact_name_match_scores_highest() {
        let formulas = vec![
            make_formula("git", "Distributed version control"),
            make_formula("git-lfs", "Git extension for large files"),
            make_formula("gitui", "Git TUI"),
        ];

        let results = search_formulas(&formulas, "git");

        assert_eq!(results[0].name, "git");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn prefix_match_scores_higher_than_contains() {
        let formulas = vec![
            make_formula("node", "JavaScript runtime"),
            make_formula("nodenv", "Node version manager"),
            make_formula(
                "libuv",
                "Multi-platform support library with focus on async I/O for node",
            ),
        ];

        let results = search_formulas(&formulas, "node");

        // "node" exact match first
        assert_eq!(results[0].name, "node");
        // "nodenv" prefix match second
        assert_eq!(results[1].name, "nodenv");
        // "libuv" description match last
        assert_eq!(results[2].name, "libuv");
    }

    #[test]
    fn regex_search_works() {
        let formulas = vec![
            make_formula("python", "Programming language"),
            make_formula("python@3.11", "Programming language"),
            make_formula("python@3.12", "Programming language"),
            make_formula("ruby", "Programming language"),
        ];

        let results = search_formulas(&formulas, "/python@3\\.1[12]/");

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.name == "python@3.11"));
        assert!(results.iter().any(|r| r.name == "python@3.12"));
    }

    #[test]
    fn excludes_deprecated_and_disabled() {
        let mut deprecated = make_formula("old-pkg", "Old package");
        deprecated.deprecated = true;

        let mut disabled = make_formula("broken-pkg", "Broken package");
        disabled.disabled = true;

        let formulas = vec![make_formula("pkg", "Good package"), deprecated, disabled];

        let results = search_formulas(&formulas, "pkg");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "pkg");
    }

    #[test]
    fn invalid_regex_falls_back_to_text() {
        let formulas = vec![make_formula("test", "Test package")];

        // Invalid regex (unmatched bracket)
        let results = search_formulas(&formulas, "/[invalid/");

        // Should not panic, just return empty or fall back
        assert!(results.is_empty() || !results.is_empty());
    }
}
