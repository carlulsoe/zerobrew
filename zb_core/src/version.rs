//! Version comparison utilities for Homebrew formulas
//!
//! Homebrew versions follow a modified semver format:
//! - Basic: `1.2.3`
//! - With rebuild suffix: `1.2.3_1` (rebuild number)
//! - With prerelease: `1.2.3-beta1`
//! - HEAD versions: `HEAD`, `HEAD-abc123`
//!
//! Comparison rules:
//! - Numeric components compared numerically: `1.10.0 > 1.9.0`
//! - Rebuild suffix is separate: `1.0.0_2 > 1.0.0_1 > 1.0.0`
//! - Prerelease comes before release: `1.0.0-beta < 1.0.0`

use std::cmp::Ordering;

/// A parsed Homebrew version for comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Main version components (e.g., [1, 2, 3] for "1.2.3")
    components: Vec<VersionComponent>,
    /// Prerelease components (e.g., ["beta", 1] for "1.0.0-beta1")
    prerelease: Vec<VersionComponent>,
    /// Rebuild suffix (e.g., 1 for "1.0.0_1")
    rebuild: u32,
    /// Original string for display
    original: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionComponent {
    Numeric(u64),
    Alpha(String),
}

impl VersionComponent {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (VersionComponent::Numeric(a), VersionComponent::Numeric(b)) => a.cmp(b),
            (VersionComponent::Alpha(a), VersionComponent::Alpha(b)) => a.cmp(b),
            // Numeric sorts before alpha (e.g., "1" < "beta")
            (VersionComponent::Numeric(_), VersionComponent::Alpha(_)) => Ordering::Less,
            (VersionComponent::Alpha(_), VersionComponent::Numeric(_)) => Ordering::Greater,
        }
    }
}

impl Version {
    /// Parse a version string
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        let original = s.to_string();

        // Handle HEAD versions specially
        if s.starts_with("HEAD") {
            return Version {
                components: vec![VersionComponent::Alpha("HEAD".to_string())],
                prerelease: vec![],
                rebuild: 0,
                original,
            };
        }

        // Split off rebuild suffix (e.g., "1.0.0_1" -> "1.0.0", 1)
        let (version_part, rebuild) = if let Some(idx) = s.rfind('_') {
            let rebuild_str = &s[idx + 1..];
            if let Ok(r) = rebuild_str.parse::<u32>() {
                (&s[..idx], r)
            } else {
                (s, 0)
            }
        } else {
            (s, 0)
        };

        // Split off prerelease (e.g., "1.0.0-beta1" -> "1.0.0", "beta1")
        let (main_part, prerelease) = if let Some(idx) = version_part.find('-') {
            let prerelease_str = &version_part[idx + 1..];
            (&version_part[..idx], parse_components(prerelease_str))
        } else {
            (version_part, vec![])
        };

        // Parse main components
        let components = parse_components(main_part);

        Version {
            components,
            prerelease,
            rebuild,
            original,
        }
    }

    /// Get the original version string
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Compare two versions, returns true if other is newer than self
    pub fn is_older_than(&self, other: &Version) -> bool {
        self.cmp(other) == Ordering::Less
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare main version components
        let max_len = self.components.len().max(other.components.len());

        for i in 0..max_len {
            let a = self.components.get(i);
            let b = other.components.get(i);

            match (a, b) {
                (Some(a), Some(b)) => {
                    let cmp = a.cmp(b);
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                }
                (Some(_), None) => return Ordering::Greater,
                (None, Some(_)) => return Ordering::Less,
                (None, None) => break,
            }
        }

        // Main components equal, compare prerelease
        // A version with prerelease is LESS than the same version without
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, false) => return Ordering::Greater, // "1.0.0" > "1.0.0-beta"
            (false, true) => return Ordering::Less,    // "1.0.0-beta" < "1.0.0"
            (false, false) => {
                // Both have prerelease, compare them
                let pre_max_len = self.prerelease.len().max(other.prerelease.len());
                for i in 0..pre_max_len {
                    let a = self.prerelease.get(i);
                    let b = other.prerelease.get(i);

                    match (a, b) {
                        (Some(a), Some(b)) => {
                            let cmp = a.cmp(b);
                            if cmp != Ordering::Equal {
                                return cmp;
                            }
                        }
                        (Some(_), None) => return Ordering::Greater,
                        (None, Some(_)) => return Ordering::Less,
                        (None, None) => break,
                    }
                }
            }
            (true, true) => {} // Both have no prerelease, continue to rebuild
        }

        // Prerelease equal (or both empty), compare rebuild suffix
        self.rebuild.cmp(&other.rebuild)
    }
}

/// Parse version string into components
fn parse_components(s: &str) -> Vec<VersionComponent> {
    let mut components = Vec::new();
    let mut current = String::new();
    let mut in_numeric = false;

    for c in s.chars() {
        if c == '.' || c == '-' || c == '+' {
            // Delimiter - push current component
            if !current.is_empty() {
                components.push(parse_component(&current));
                current.clear();
            }
            in_numeric = false;
        } else if c.is_ascii_digit() {
            if !in_numeric && !current.is_empty() {
                // Switching from alpha to numeric
                components.push(parse_component(&current));
                current.clear();
            }
            in_numeric = true;
            current.push(c);
        } else if c.is_alphanumeric() {
            if in_numeric && !current.is_empty() {
                // Switching from numeric to alpha
                components.push(parse_component(&current));
                current.clear();
            }
            in_numeric = false;
            current.push(c);
        }
        // Ignore other characters
    }

    if !current.is_empty() {
        components.push(parse_component(&current));
    }

    components
}

fn parse_component(s: &str) -> VersionComponent {
    if let Ok(n) = s.parse::<u64>() {
        VersionComponent::Numeric(n)
    } else {
        VersionComponent::Alpha(s.to_lowercase())
    }
}

/// Represents an outdated package
#[derive(Debug, Clone)]
pub struct OutdatedPackage {
    pub name: String,
    pub installed_version: String,
    pub available_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_version() {
        let v = Version::parse("1.2.3");
        assert_eq!(v.components.len(), 3);
        assert_eq!(v.rebuild, 0);
    }

    #[test]
    fn parses_version_with_rebuild() {
        let v = Version::parse("1.0.0_1");
        assert_eq!(v.rebuild, 1);

        let v2 = Version::parse("1.0.0_23");
        assert_eq!(v2.rebuild, 23);
    }

    #[test]
    fn compares_simple_versions() {
        assert!(Version::parse("1.0.0") < Version::parse("1.0.1"));
        assert!(Version::parse("1.0.0") < Version::parse("1.1.0"));
        assert!(Version::parse("1.0.0") < Version::parse("2.0.0"));
        assert!(Version::parse("1.9.0") < Version::parse("1.10.0"));
    }

    #[test]
    fn compares_versions_with_rebuild() {
        assert!(Version::parse("1.0.0") < Version::parse("1.0.0_1"));
        assert!(Version::parse("1.0.0_1") < Version::parse("1.0.0_2"));
        assert!(Version::parse("1.0.0_1") < Version::parse("1.0.1"));
    }

    #[test]
    fn compares_versions_with_different_lengths() {
        assert!(Version::parse("1.0") < Version::parse("1.0.1"));
        assert!(Version::parse("1.0.0.0") > Version::parse("1.0.0"));
    }

    #[test]
    fn handles_alpha_components() {
        assert!(Version::parse("1.0.0-beta") < Version::parse("1.0.0"));
        assert!(Version::parse("1.0.0-alpha") < Version::parse("1.0.0-beta"));
    }

    #[test]
    fn handles_head_versions() {
        let v = Version::parse("HEAD");
        assert_eq!(v.original, "HEAD");

        let v2 = Version::parse("HEAD-abc123");
        assert_eq!(v2.original, "HEAD-abc123");
    }

    #[test]
    fn equality_works() {
        assert_eq!(Version::parse("1.0.0"), Version::parse("1.0.0"));
        assert_eq!(Version::parse("1.0.0_1"), Version::parse("1.0.0_1"));
        assert_ne!(Version::parse("1.0.0"), Version::parse("1.0.1"));
    }

    #[test]
    fn is_older_than_works() {
        assert!(Version::parse("1.0.0").is_older_than(&Version::parse("1.0.1")));
        assert!(!Version::parse("1.0.1").is_older_than(&Version::parse("1.0.0")));
        assert!(!Version::parse("1.0.0").is_older_than(&Version::parse("1.0.0")));
    }

    #[test]
    fn real_world_versions() {
        // git versions
        assert!(Version::parse("2.43.0") < Version::parse("2.44.0"));
        assert!(Version::parse("2.43.0") < Version::parse("2.43.1"));

        // python versions with @ syntax
        assert!(Version::parse("3.11.8") < Version::parse("3.12.2"));

        // node versions
        assert!(Version::parse("20.11.1") < Version::parse("21.6.2"));

        // Versions with long rebuild suffixes
        assert!(Version::parse("1.0.0_12") < Version::parse("1.0.0_123"));
    }
}
