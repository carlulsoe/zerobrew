//! Bundle/Brewfile support for Zerobrew.
//!
//! This module provides the ability to manage packages declaratively using
//! Brewfile-compatible syntax. It supports:
//! - Installing packages from a Brewfile
//! - Generating a Brewfile from installed packages
//! - Checking if all Brewfile entries are satisfied
//!
//! # Brewfile Syntax
//!
//! ```text
//! # Comments start with #
//! tap "user/repo"                    # Add a tap
//! brew "formula"                     # Install a formula
//! brew "formula", args: ["--HEAD"]   # Install with args
//! ```
//!
//! # Example
//!
//! ```text
//! tap "homebrew/cask"
//! brew "git"
//! brew "ripgrep"
//! brew "neovim", args: ["--HEAD"]
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use zb_core::Error;

/// A parsed entry from a Brewfile
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrewfileEntry {
    /// A tap to add: `tap "user/repo"`
    Tap { name: String },
    /// A formula to install: `brew "formula"` or `brew "formula", args: ["--HEAD"]`
    Brew { name: String, args: Vec<String> },
    /// A comment or empty line (ignored during install but preserved in dump)
    Comment(String),
}

impl BrewfileEntry {
    /// Format the entry as a Brewfile line
    pub fn to_brewfile_line(&self) -> String {
        match self {
            BrewfileEntry::Tap { name } => format!("tap \"{}\"", name),
            BrewfileEntry::Brew { name, args } => {
                if args.is_empty() {
                    format!("brew \"{}\"", name)
                } else {
                    let args_str = args
                        .iter()
                        .map(|a| format!("\"{}\"", a))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("brew \"{}\", args: [{}]", name, args_str)
                }
            }
            BrewfileEntry::Comment(text) => text.clone(),
        }
    }
}

/// Result of checking a Brewfile against installed packages
#[derive(Debug, Default)]
pub struct BundleCheckResult {
    /// Taps that need to be added
    pub missing_taps: Vec<String>,
    /// Formulas that need to be installed
    pub missing_formulas: Vec<String>,
    /// Formulas that are installed but need different args (e.g., HEAD vs stable)
    pub mismatched_formulas: Vec<(String, Vec<String>)>,
    /// Whether all entries are satisfied
    pub satisfied: bool,
}

impl BundleCheckResult {
    /// Create a new satisfied result
    pub fn satisfied() -> Self {
        Self {
            satisfied: true,
            ..Default::default()
        }
    }
}

/// Result of installing a Brewfile
#[derive(Debug, Default)]
pub struct BundleInstallResult {
    /// Taps that were added
    pub taps_added: Vec<String>,
    /// Formulas that were installed
    pub formulas_installed: Vec<String>,
    /// Formulas that were already installed (skipped)
    pub formulas_skipped: Vec<String>,
    /// Entries that failed to install
    pub failed: Vec<(String, String)>,
}

/// Parse a Brewfile into entries
pub fn parse_brewfile(content: &str) -> Result<Vec<BrewfileEntry>, Error> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            entries.push(BrewfileEntry::Comment(line.to_string()));
            continue;
        }

        // Parse tap directive: tap "user/repo"
        if let Some(rest) = trimmed.strip_prefix("tap ") {
            let name = parse_quoted_string(rest)?;
            entries.push(BrewfileEntry::Tap { name });
            continue;
        }

        // Parse brew directive: brew "formula" or brew "formula", args: [...]
        if let Some(rest) = trimmed.strip_prefix("brew ") {
            let (name, args) = parse_brew_directive(rest)?;
            entries.push(BrewfileEntry::Brew { name, args });
            continue;
        }

        // Unknown directive - treat as comment for forward compatibility
        entries.push(BrewfileEntry::Comment(line.to_string()));
    }

    Ok(entries)
}

/// Parse a quoted string like `"foo"` and return `foo`
/// Handles escaped quotes within the string (e.g., `"foo\"bar"` -> `foo"bar`)
fn parse_quoted_string(s: &str) -> Result<String, Error> {
    let s = s.trim();

    if !s.starts_with('"') {
        return Err(Error::StoreCorruption {
            message: format!("expected quoted string, got: {}", s),
        });
    }

    // Parse the string character by character to handle escape sequences
    let mut result = String::new();
    let mut chars = s[1..].chars().peekable();
    let mut found_closing = false;

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Handle escape sequence
                if let Some(&next) = chars.peek() {
                    match next {
                        '"' | '\\' => {
                            result.push(next);
                            chars.next();
                        }
                        'n' => {
                            result.push('\n');
                            chars.next();
                        }
                        't' => {
                            result.push('\t');
                            chars.next();
                        }
                        _ => {
                            // Unknown escape, preserve the backslash
                            result.push('\\');
                        }
                    }
                } else {
                    // Trailing backslash
                    result.push('\\');
                }
            }
            '"' => {
                found_closing = true;
                break;
            }
            _ => result.push(c),
        }
    }

    if found_closing {
        Ok(result)
    } else {
        Err(Error::StoreCorruption {
            message: format!("unterminated string: {}", s),
        })
    }
}

/// Parse a brew directive: `"formula"` or `"formula", args: ["--HEAD"]`
fn parse_brew_directive(s: &str) -> Result<(String, Vec<String>), Error> {
    let s = s.trim();

    // Parse the formula name
    let name = parse_quoted_string(s)?;

    // Check for args
    let args_start = s.find(", args:");
    let args = if let Some(start) = args_start {
        let args_part = &s[start + 7..].trim();
        parse_args_array(args_part)?
    } else {
        Vec::new()
    };

    Ok((name, args))
}

/// Parse an args array like `["--HEAD", "--with-foo"]`
fn parse_args_array(s: &str) -> Result<Vec<String>, Error> {
    let s = s.trim();

    if !s.starts_with('[') {
        return Err(Error::StoreCorruption {
            message: format!("expected args array starting with [, got: {}", s),
        });
    }

    let end = s.find(']').ok_or_else(|| Error::StoreCorruption {
        message: format!("unterminated args array: {}", s),
    })?;

    let inner = &s[1..end];
    let mut args = Vec::new();

    // Parse comma-separated quoted strings
    let mut current = inner.trim();
    while !current.is_empty() {
        // Skip leading comma and whitespace
        current = current.trim_start_matches(',').trim();
        if current.is_empty() {
            break;
        }

        // Parse quoted string
        if current.starts_with('"') {
            let arg = parse_quoted_string(current)?;
            args.push(arg.clone());

            // Move past this string
            let skip = current.find('"').unwrap() + 1;
            let rest = &current[skip..];
            if let Some(end_quote) = rest.find('"') {
                current = &rest[end_quote + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Ok(args)
}

/// Read and parse a Brewfile from a path
pub fn read_brewfile(path: &Path) -> Result<Vec<BrewfileEntry>, Error> {
    let content = fs::read_to_string(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to read Brewfile at {}: {}", path.display(), e),
    })?;

    parse_brewfile(&content)
}

/// Find a Brewfile in the current directory or parent directories
pub fn find_brewfile(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir;

    loop {
        // Check for Brewfile in current directory
        let brewfile = current.join("Brewfile");
        if brewfile.exists() {
            return Some(brewfile);
        }

        // Move to parent directory
        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
}

/// Generate Brewfile content from installed packages and taps
pub fn generate_brewfile(taps: &[String], formulas: &[String], include_comments: bool) -> String {
    let mut lines = Vec::new();

    // Add header comment
    if include_comments {
        lines.push("# Generated by zerobrew bundle dump".to_string());
        lines.push("# https://github.com/zerobrew/zerobrew".to_string());
        lines.push(String::new());
    }

    // Add taps
    if !taps.is_empty() {
        if include_comments {
            lines.push("# Taps".to_string());
        }
        for tap in taps {
            lines.push(format!("tap \"{}\"", tap));
        }
        if include_comments {
            lines.push(String::new());
        }
    }

    // Add formulas
    if !formulas.is_empty() {
        if include_comments {
            lines.push("# Formulas".to_string());
        }
        for formula in formulas {
            lines.push(format!("brew \"{}\"", formula));
        }
    }

    lines.join("\n")
}

/// Check which entries from a Brewfile are not satisfied
pub fn check_brewfile(
    entries: &[BrewfileEntry],
    installed_formulas: &HashSet<String>,
    installed_taps: &HashSet<String>,
) -> BundleCheckResult {
    let mut result = BundleCheckResult::default();

    for entry in entries {
        match entry {
            BrewfileEntry::Tap { name } => {
                // Normalize tap name (remove homebrew- prefix if present)
                let normalized = normalize_tap_name(name);
                if !installed_taps.contains(&normalized) {
                    result.missing_taps.push(name.clone());
                }
            }
            BrewfileEntry::Brew { name, args } => {
                // Extract formula name (may be user/repo/formula)
                let formula_name = extract_formula_name(name);
                if !installed_formulas.contains(&formula_name) {
                    result.missing_formulas.push(name.clone());
                } else if !args.is_empty() {
                    // Formula is installed, but we can't easily verify args
                    // For now, we consider this satisfied
                    // TODO: Track install args in database for proper checking
                }
            }
            BrewfileEntry::Comment(_) => {}
        }
    }

    result.satisfied = result.missing_taps.is_empty() && result.missing_formulas.is_empty();

    result
}

/// Normalize a tap name by removing the homebrew- prefix from the repo
fn normalize_tap_name(name: &str) -> String {
    if let Some((user, repo)) = name.split_once('/') {
        let repo = repo.strip_prefix("homebrew-").unwrap_or(repo);
        format!("{}/{}", user, repo)
    } else {
        name.to_string()
    }
}

/// Extract the formula name from a potentially qualified name
fn extract_formula_name(name: &str) -> String {
    // user/repo/formula -> formula
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() == 3 {
        // user/repo/formula format
        return parts[2].to_string();
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_tap() {
        let content = r#"tap "homebrew/core""#;
        let entries = parse_brewfile(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            BrewfileEntry::Tap {
                name: "homebrew/core".to_string()
            }
        );
    }

    #[test]
    fn parse_simple_brew() {
        let content = r#"brew "git""#;
        let entries = parse_brewfile(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![]
            }
        );
    }

    #[test]
    fn parse_brew_with_args() {
        let content = r#"brew "neovim", args: ["--HEAD"]"#;
        let entries = parse_brewfile(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            BrewfileEntry::Brew {
                name: "neovim".to_string(),
                args: vec!["--HEAD".to_string()]
            }
        );
    }

    #[test]
    fn parse_brew_with_multiple_args() {
        let content = r#"brew "pkg", args: ["--HEAD", "--with-foo"]"#;
        let entries = parse_brewfile(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            BrewfileEntry::Brew {
                name: "pkg".to_string(),
                args: vec!["--HEAD".to_string(), "--with-foo".to_string()]
            }
        );
    }

    #[test]
    fn parse_comments_and_empty_lines() {
        let content = "# This is a comment\n\nbrew \"git\"\n# Another comment";
        let entries = parse_brewfile(content).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(matches!(&entries[0], BrewfileEntry::Comment(c) if c.starts_with("#")));
        assert!(matches!(&entries[1], BrewfileEntry::Comment(c) if c.is_empty()));
        assert!(matches!(&entries[2], BrewfileEntry::Brew { name, .. } if name == "git"));
        assert!(matches!(&entries[3], BrewfileEntry::Comment(c) if c.starts_with("#")));
    }

    #[test]
    fn parse_full_brewfile() {
        let content = r#"# My Brewfile
tap "homebrew/cask"
tap "user/repo"

# CLI tools
brew "git"
brew "ripgrep"
brew "neovim", args: ["--HEAD"]
"#;
        let entries = parse_brewfile(content).unwrap();

        // Count non-comment entries
        let taps: Vec<_> = entries
            .iter()
            .filter_map(|e| match e {
                BrewfileEntry::Tap { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        let brews: Vec<_> = entries
            .iter()
            .filter_map(|e| match e {
                BrewfileEntry::Brew { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(taps, vec!["homebrew/cask", "user/repo"]);
        assert_eq!(brews, vec!["git", "ripgrep", "neovim"]);
    }

    #[test]
    fn entry_to_brewfile_line() {
        let tap = BrewfileEntry::Tap {
            name: "user/repo".to_string(),
        };
        assert_eq!(tap.to_brewfile_line(), r#"tap "user/repo""#);

        let brew = BrewfileEntry::Brew {
            name: "git".to_string(),
            args: vec![],
        };
        assert_eq!(brew.to_brewfile_line(), r#"brew "git""#);

        let brew_with_args = BrewfileEntry::Brew {
            name: "neovim".to_string(),
            args: vec!["--HEAD".to_string()],
        };
        assert_eq!(
            brew_with_args.to_brewfile_line(),
            r#"brew "neovim", args: ["--HEAD"]"#
        );
    }

    #[test]
    fn generate_brewfile_basic() {
        let taps = vec!["homebrew/cask".to_string()];
        let formulas = vec!["git".to_string(), "ripgrep".to_string()];

        let content = generate_brewfile(&taps, &formulas, false);

        assert!(content.contains(r#"tap "homebrew/cask""#));
        assert!(content.contains(r#"brew "git""#));
        assert!(content.contains(r#"brew "ripgrep""#));
    }

    #[test]
    fn check_brewfile_all_satisfied() {
        let entries = vec![
            BrewfileEntry::Tap {
                name: "user/repo".to_string(),
            },
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![],
            },
        ];

        let mut installed_formulas = HashSet::new();
        installed_formulas.insert("git".to_string());

        let mut installed_taps = HashSet::new();
        installed_taps.insert("user/repo".to_string());

        let result = check_brewfile(&entries, &installed_formulas, &installed_taps);

        assert!(result.satisfied);
        assert!(result.missing_taps.is_empty());
        assert!(result.missing_formulas.is_empty());
    }

    #[test]
    fn check_brewfile_missing_formula() {
        let entries = vec![BrewfileEntry::Brew {
            name: "git".to_string(),
            args: vec![],
        }];

        let installed_formulas = HashSet::new();
        let installed_taps = HashSet::new();

        let result = check_brewfile(&entries, &installed_formulas, &installed_taps);

        assert!(!result.satisfied);
        assert_eq!(result.missing_formulas, vec!["git"]);
    }

    #[test]
    fn check_brewfile_missing_tap() {
        let entries = vec![BrewfileEntry::Tap {
            name: "user/repo".to_string(),
        }];

        let installed_formulas = HashSet::new();
        let installed_taps = HashSet::new();

        let result = check_brewfile(&entries, &installed_formulas, &installed_taps);

        assert!(!result.satisfied);
        assert_eq!(result.missing_taps, vec!["user/repo"]);
    }

    #[test]
    fn normalize_tap_name_strips_homebrew_prefix() {
        assert_eq!(normalize_tap_name("user/homebrew-repo"), "user/repo");
        assert_eq!(normalize_tap_name("user/repo"), "user/repo");
        assert_eq!(normalize_tap_name("homebrew/core"), "homebrew/core");
    }

    #[test]
    fn extract_formula_name_handles_qualified_names() {
        assert_eq!(extract_formula_name("git"), "git");
        assert_eq!(extract_formula_name("user/repo/formula"), "formula");
        assert_eq!(extract_formula_name("python@3.11"), "python@3.11");
    }

    #[test]
    fn parse_quoted_string_works() {
        assert_eq!(parse_quoted_string(r#""hello""#).unwrap(), "hello");
        assert_eq!(parse_quoted_string(r#""foo/bar""#).unwrap(), "foo/bar");
        assert!(parse_quoted_string("hello").is_err());
        assert!(parse_quoted_string(r#""unterminated"#).is_err());

        // Test escaped quotes
        assert_eq!(parse_quoted_string(r#""foo\"bar""#).unwrap(), "foo\"bar");
        assert_eq!(
            parse_quoted_string(r#""escaped\\backslash""#).unwrap(),
            "escaped\\backslash"
        );
        assert_eq!(parse_quoted_string(r#""tab\there""#).unwrap(), "tab\there");
        assert_eq!(
            parse_quoted_string(r#""newline\nhere""#).unwrap(),
            "newline\nhere"
        );
    }

    #[test]
    fn parse_args_array_works() {
        let args = parse_args_array(r#"["--HEAD"]"#).unwrap();
        assert_eq!(args, vec!["--HEAD"]);

        let args = parse_args_array(r#"["--HEAD", "--with-foo"]"#).unwrap();
        assert_eq!(args, vec!["--HEAD", "--with-foo"]);

        let args = parse_args_array(r#"[]"#).unwrap();
        assert!(args.is_empty());
    }
}
