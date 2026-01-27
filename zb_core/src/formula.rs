use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Formula {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub versions: Versions,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub build_dependencies: Vec<String>,
    /// Dependencies that macOS provides as system libraries.
    /// On Linux, these must be installed explicitly.
    /// Can be either strings or objects like {"pkg": "build"}.
    #[serde(default, deserialize_with = "deserialize_uses_from_macos")]
    pub uses_from_macos: Vec<String>,
    #[serde(default)]
    pub caveats: Option<String>,
    #[serde(default)]
    pub keg_only: bool,
    #[serde(default)]
    pub keg_only_reason: Option<KegOnlyReason>,
    #[serde(default)]
    pub bottle: Bottle,
    /// Source URLs for building from source
    #[serde(default)]
    pub urls: SourceUrls,
}

/// Source URLs for building from source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceUrls {
    /// Stable source tarball
    #[serde(default)]
    pub stable: Option<StableSource>,
    /// HEAD (git) source
    #[serde(default)]
    pub head: Option<HeadSource>,
}

/// Stable source tarball information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableSource {
    /// URL to download the source tarball
    pub url: String,
    /// SHA256 checksum of the tarball
    #[serde(default)]
    pub checksum: Option<String>,
    /// Git tag if applicable
    #[serde(default)]
    pub tag: Option<String>,
    /// Git revision if applicable
    #[serde(default)]
    pub revision: Option<String>,
    /// Special download method (e.g., "homebrew_curl")
    #[serde(default)]
    pub using: Option<String>,
}

/// HEAD source (git repository)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadSource {
    /// Git repository URL
    pub url: String,
    /// Branch to checkout
    #[serde(default)]
    pub branch: Option<String>,
    /// Special checkout method
    #[serde(default)]
    pub using: Option<String>,
}

/// Reason why a formula is keg-only
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KegOnlyReason {
    pub reason: String,
    pub explanation: String,
}

/// Deserialize uses_from_macos which can contain either strings or objects.
/// - Strings like "zlib" are runtime dependencies
/// - Objects like {"flex": "build"} or {"python": "test"} are build/test-time only
///   and are skipped since we use prebuilt bottles
fn deserialize_uses_from_macos<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct UsesFromMacosVisitor;

    impl<'de> Visitor<'de> for UsesFromMacosVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a sequence of strings or objects with package names as keys")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut result = Vec::new();

            while let Some(item) = seq.next_element::<UsesFromMacosItem>()? {
                // Only include runtime dependencies (plain strings)
                // Skip build-time or test-time only dependencies (objects)
                if let Some(name) = item.runtime_dependency() {
                    result.push(name);
                }
            }

            Ok(result)
        }
    }

    deserializer.deserialize_seq(UsesFromMacosVisitor)
}

/// An item in uses_from_macos: either a string or an object like {"pkg": "build"}.
/// Objects indicate the dependency is only needed at a specific phase (build/test).
#[derive(Debug, Clone)]
enum UsesFromMacosItem {
    /// Runtime dependency - always needed
    Runtime(String),
    /// Build or test time only dependency - not needed for prebuilt bottles
    BuildOrTestOnly,
}

impl UsesFromMacosItem {
    /// Returns the package name if this is a runtime dependency, None otherwise
    fn runtime_dependency(self) -> Option<String> {
        match self {
            UsesFromMacosItem::Runtime(s) => Some(s),
            UsesFromMacosItem::BuildOrTestOnly => None,
        }
    }
}

impl<'de> Deserialize<'de> for UsesFromMacosItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ItemVisitor;

        impl<'de> Visitor<'de> for ItemVisitor {
            type Value = UsesFromMacosItem;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or an object with a package name as key")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(UsesFromMacosItem::Runtime(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(UsesFromMacosItem::Runtime(value))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                // Objects like {"flex": "build"} or {"python": "test"} indicate
                // the dependency is only needed at build/test time, not runtime.
                // Since we use prebuilt bottles, we skip these.
                // Drain all entries
                while map.next_entry::<String, String>()?.is_some() {}
                Ok(UsesFromMacosItem::BuildOrTestOnly)
            }
        }

        deserializer.deserialize_any(ItemVisitor)
    }
}

impl Formula {
    /// Returns the effective version including rebuild suffix if applicable.
    /// Homebrew bottles with rebuild > 0 have paths like `{version}_{rebuild}`.
    pub fn effective_version(&self) -> String {
        let rebuild = self.bottle.stable.rebuild;
        if rebuild > 0 {
            format!("{}_{}", self.versions.stable, rebuild)
        } else {
            self.versions.stable.clone()
        }
    }

    /// Returns the effective dependencies for the current platform.
    /// On Linux, this includes `uses_from_macos` dependencies since they
    /// aren't available as system libraries like on macOS.
    pub fn effective_dependencies(&self) -> Vec<String> {
        let mut deps = self.dependencies.clone();

        #[cfg(target_os = "linux")]
        {
            for dep in &self.uses_from_macos {
                if !deps.contains(dep) {
                    deps.push(dep.clone());
                }
            }
        }

        deps
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Versions {
    #[serde(default)]
    pub stable: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Bottle {
    #[serde(default)]
    pub stable: BottleStable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BottleStable {
    #[serde(default)]
    pub files: BTreeMap<String, BottleFile>,
    /// Rebuild number for the bottle. When > 0, the bottle's internal paths
    /// use `{version}_{rebuild}` instead of just `{version}`.
    #[serde(default)]
    pub rebuild: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BottleFile {
    pub url: String,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_formula_fixtures() {
        let fixtures = [
            include_str!("../fixtures/formula_foo.json"),
            include_str!("../fixtures/formula_bar.json"),
        ];

        for fixture in fixtures {
            let formula: Formula = serde_json::from_str(fixture).unwrap();
            assert!(!formula.name.is_empty());
            assert!(!formula.versions.stable.is_empty());
            assert!(!formula.bottle.stable.files.is_empty());
        }
    }

    #[test]
    fn effective_version_without_rebuild() {
        let fixture = include_str!("../fixtures/formula_foo.json");
        let formula: Formula = serde_json::from_str(fixture).unwrap();

        // Without rebuild, effective_version should equal stable version
        assert_eq!(formula.bottle.stable.rebuild, 0);
        assert_eq!(formula.effective_version(), "1.2.3");
    }

    #[test]
    fn effective_version_with_rebuild() {
        let fixture = include_str!("../fixtures/formula_with_rebuild.json");
        let formula: Formula = serde_json::from_str(fixture).unwrap();

        // With rebuild=1, effective_version should be "8.0.1_1"
        assert_eq!(formula.bottle.stable.rebuild, 1);
        assert_eq!(formula.effective_version(), "8.0.1_1");
    }

    #[test]
    fn rebuild_field_defaults_to_zero() {
        // Formulas without rebuild field should default to 0
        let fixture = include_str!("../fixtures/formula_foo.json");
        let formula: Formula = serde_json::from_str(fixture).unwrap();
        assert_eq!(formula.bottle.stable.rebuild, 0);
    }

    #[test]
    fn uses_from_macos_handles_mixed_formats() {
        // Test that uses_from_macos handles both strings and objects:
        // - Strings like "zlib" are runtime dependencies and included
        // - Objects like {"flex": "build"} are build/test-time only and excluded
        let json = r#"{
            "name": "test",
            "versions": {"stable": "1.0.0"},
            "dependencies": [],
            "uses_from_macos": [
                {"flex": "build"},
                "libffi",
                {"python": "test"},
                "zlib"
            ],
            "bottle": {
                "stable": {
                    "files": {
                        "arm64_sonoma": {
                            "url": "https://example.com/test.tar.gz",
                            "sha256": "abc123"
                        }
                    }
                }
            }
        }"#;

        let formula: Formula = serde_json::from_str(json).unwrap();
        // Only runtime deps (strings) should be included, not build/test-time (objects)
        assert_eq!(formula.uses_from_macos, vec!["libffi", "zlib"]);
    }
}
