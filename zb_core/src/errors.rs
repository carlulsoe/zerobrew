use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnsupportedBottle {
        name: String,
        available_platforms: Vec<String>,
    },
    ChecksumMismatch {
        expected: String,
        actual: String,
        file_name: Option<String>,
    },
    LinkConflict {
        path: PathBuf,
        existing_type: LinkConflictType,
    },
    StoreCorruption {
        message: String,
    },
    NetworkFailure {
        message: String,
    },
    MissingFormula {
        name: String,
    },
    DependencyCycle {
        cycle: Vec<String>,
    },
    NotInstalled {
        name: String,
    },
}

/// Type of existing file at a link conflict path
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkConflictType {
    RegularFile,
    Directory,
    SymlinkToOther { target: PathBuf },
    Unknown,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnsupportedBottle {
                name,
                available_platforms,
            } => {
                write!(
                    f,
                    "no compatible bottle for formula '{}' on this platform",
                    name
                )?;
                if !available_platforms.is_empty() {
                    write!(f, " (available for: {})", available_platforms.join(", "))?;
                }
                write!(
                    f,
                    "\n  hint: try 'zb install --build-from-source {}' to build from source",
                    name
                )
            }
            Error::ChecksumMismatch {
                expected,
                actual,
                file_name,
            } => {
                write!(f, "checksum verification failed")?;
                if let Some(name) = file_name {
                    write!(f, " for '{}'", name)?;
                }
                write!(f, "\n  expected: {}\n  got:      {}", expected, actual)?;
                write!(
                    f,
                    "\n  hint: this may indicate a corrupted download or CDN issue; try again"
                )
            }
            Error::LinkConflict {
                path,
                existing_type,
            } => {
                let path_str = path.to_string_lossy();
                match existing_type {
                    LinkConflictType::RegularFile => {
                        write!(
                            f,
                            "cannot link '{}' (file already exists)\n  hint: remove the existing file or use --overwrite",
                            path_str
                        )
                    }
                    LinkConflictType::Directory => {
                        write!(
                            f,
                            "cannot link '{}' (directory already exists)\n  hint: remove the existing directory first",
                            path_str
                        )
                    }
                    LinkConflictType::SymlinkToOther { target } => {
                        write!(
                            f,
                            "cannot link '{}' (symlink to '{}' already exists)\n  hint: use --overwrite to replace the existing symlink",
                            path_str,
                            target.to_string_lossy()
                        )
                    }
                    LinkConflictType::Unknown => {
                        write!(f, "cannot link '{}' (path already exists)", path_str)
                    }
                }
            }
            Error::StoreCorruption { message } => {
                write!(
                    f,
                    "store corruption detected: {}\n  hint: run 'zb doctor' to diagnose and 'zb gc' to clean up",
                    message
                )
            }
            Error::NetworkFailure { message } => {
                write!(
                    f,
                    "network error: {}\n  hint: check your internet connection and try again",
                    message
                )
            }
            Error::MissingFormula { name } => {
                write!(
                    f,
                    "formula '{}' not found\n  hint: run 'zb search {}' to find available formulas",
                    name, name
                )
            }
            Error::DependencyCycle { cycle } => {
                let rendered = cycle.join(" -> ");
                write!(
                    f,
                    "dependency cycle detected: {}\n  hint: this is likely a formula bug; please report it upstream",
                    rendered
                )
            }
            Error::NotInstalled { name } => {
                write!(
                    f,
                    "formula '{}' is not installed\n  hint: run 'zb install {}' to install it",
                    name, name
                )
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_bottle_display_includes_name() {
        let err = Error::UnsupportedBottle {
            name: "libheif".to_string(),
            available_platforms: vec!["x86_64_linux".to_string()],
        };

        assert!(err.to_string().contains("libheif"));
        assert!(err.to_string().contains("x86_64_linux"));
        assert!(err.to_string().contains("hint:"));
    }

    #[test]
    fn checksum_mismatch_display_includes_hint() {
        let err = Error::ChecksumMismatch {
            expected: "abc123".to_string(),
            actual: "def456".to_string(),
            file_name: Some("wget".to_string()),
        };

        let msg = err.to_string();
        assert!(msg.contains("wget"));
        assert!(msg.contains("abc123"));
        assert!(msg.contains("def456"));
        assert!(msg.contains("hint:"));
    }

    #[test]
    fn link_conflict_display_shows_type() {
        let err = Error::LinkConflict {
            path: PathBuf::from("/opt/zerobrew/bin/foo"),
            existing_type: LinkConflictType::RegularFile,
        };

        let msg = err.to_string();
        assert!(msg.contains("foo"));
        assert!(msg.contains("file already exists"));
        assert!(msg.contains("hint:"));
    }

    #[test]
    fn missing_formula_display_includes_search_hint() {
        let err = Error::MissingFormula {
            name: "nonexistent".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("nonexistent"));
        assert!(msg.contains("zb search"));
    }
}
