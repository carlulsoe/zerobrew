//! Bundle command implementations.

use console::style;
use std::path::PathBuf;

use zb_io::install::Installer;

use crate::BundleAction;

/// Run the bundle command.
pub async fn run(
    installer: &mut Installer,
    action: Option<BundleAction>,
) -> Result<(), zb_core::Error> {
    let cwd = std::env::current_dir().map_err(|e| zb_core::Error::StoreCorruption {
        message: format!("failed to get current directory: {}", e),
    })?;

    match action {
        None | Some(BundleAction::Install { file: None }) => {
            run_install(installer, &cwd, None).await
        }
        Some(BundleAction::Install { file: Some(path) }) => {
            run_install(installer, &cwd, Some(path)).await
        }
        Some(BundleAction::Dump {
            file,
            describe,
            force,
        }) => run_dump(installer, file, describe, force),
        Some(BundleAction::Check { file, strict }) => run_check(installer, &cwd, file, strict),
        Some(BundleAction::List { file }) => run_list(installer, &cwd, file),
    }
}

async fn run_install(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
) -> Result<(), zb_core::Error> {
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    println!(
        "{} Installing from {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_install(&brewfile_path).await?;

    // Report results
    if !result.taps_added.is_empty() {
        println!();
        println!("{} Taps added:", style("==>").cyan().bold());
        for tap in &result.taps_added {
            println!("    {} {}", style("✓").green(), tap);
        }
    }

    if !result.formulas_installed.is_empty() {
        println!();
        println!("{} Formulas installed:", style("==>").cyan().bold());
        for formula in &result.formulas_installed {
            println!("    {} {}", style("✓").green(), formula);
        }
    }

    if !result.formulas_skipped.is_empty() {
        println!();
        println!("{} Already installed:", style("==>").cyan().bold());
        for formula in &result.formulas_skipped {
            println!("    {} {}", style("-").dim(), formula);
        }
    }

    if !result.failed.is_empty() {
        println!();
        println!("{} Failed:", style("==>").red().bold());
        for (name, error) in &result.failed {
            println!("    {} {}: {}", style("✗").red(), name, error);
        }
    }

    // Summary
    println!();
    let total_installed = result.taps_added.len() + result.formulas_installed.len();
    if result.failed.is_empty() {
        println!(
            "{} Bundle complete. {} installed, {} already satisfied.",
            style("==>").cyan().bold(),
            total_installed,
            result.formulas_skipped.len()
        );
    } else {
        println!(
            "{} Bundle complete with errors. {} installed, {} already satisfied, {} failed.",
            style("==>").yellow().bold(),
            total_installed,
            result.formulas_skipped.len(),
            result.failed.len()
        );
        std::process::exit(1);
    }

    Ok(())
}

fn run_dump(
    installer: &mut Installer,
    file: Option<PathBuf>,
    describe: bool,
    force: bool,
) -> Result<(), zb_core::Error> {
    let content = installer.bundle_dump(describe)?;

    if let Some(path) = file {
        if path.exists() && !force {
            eprintln!(
                "{} File '{}' already exists. Use --force to overwrite.",
                style("error:").red().bold(),
                path.display()
            );
            std::process::exit(1);
        }

        std::fs::write(&path, &content).map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("failed to write Brewfile: {}", e),
        })?;

        println!(
            "{} Brewfile written to {}",
            style("==>").cyan().bold(),
            path.display()
        );
    } else {
        print!("{}", content);
        if !content.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}

fn run_check(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
    strict: bool,
) -> Result<(), zb_core::Error> {
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    println!(
        "{} Checking {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_check(&brewfile_path)?;

    if result.satisfied {
        println!();
        println!("{} All entries are satisfied!", style("==>").green().bold());
    } else {
        if !result.missing_taps.is_empty() {
            println!();
            println!("{} Missing taps:", style("==>").yellow().bold());
            for tap in &result.missing_taps {
                println!("    {} {}", style("✗").red(), tap);
            }
        }

        if !result.missing_formulas.is_empty() {
            println!();
            println!("{} Missing formulas:", style("==>").yellow().bold());
            for formula in &result.missing_formulas {
                println!("    {} {}", style("✗").red(), formula);
            }
        }

        println!();
        println!(
            "    → Run {} bundle to install missing entries",
            style("zb").cyan()
        );

        if strict {
            std::process::exit(1);
        }
    }

    Ok(())
}

fn run_list(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
) -> Result<(), zb_core::Error> {
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    let entries = installer.parse_brewfile(&brewfile_path)?;

    println!(
        "{} Entries in {}:",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );
    println!();

    #[allow(unused_mut)]
    let mut tap_count = 0;
    let mut brew_count = 0;

    for entry in &entries {
        match entry {
            zb_io::BrewfileEntry::Tap { name } => {
                println!("tap  {}", style(name).cyan());
                tap_count += 1;
            }
            zb_io::BrewfileEntry::Brew { name, args } => {
                if args.is_empty() {
                    println!("brew {}", style(name).green());
                } else {
                    println!("brew {} ({})", style(name).green(), args.join(", "));
                }
                brew_count += 1;
            }
            zb_io::BrewfileEntry::Comment(_) => {
                // Skip comments in list output
            }
        }
    }

    println!();
    println!(
        "{} {} taps, {} formulas",
        style("==>").cyan().bold(),
        tap_count,
        brew_count
    );

    Ok(())
}

/// Count taps and formulas in Brewfile entries.
/// Extracted for testability.
pub(crate) fn count_brewfile_entries(entries: &[zb_io::BrewfileEntry]) -> (usize, usize) {
    let mut tap_count = 0;
    let mut brew_count = 0;

    for entry in entries {
        match entry {
            zb_io::BrewfileEntry::Tap { .. } => tap_count += 1,
            zb_io::BrewfileEntry::Brew { .. } => brew_count += 1,
            zb_io::BrewfileEntry::Comment(_) => {}
        }
    }

    (tap_count, brew_count)
}

/// Format brew entry with args for display.
/// Extracted for testability.
pub(crate) fn format_brew_entry(name: &str, args: &[String]) -> String {
    if args.is_empty() {
        format!("brew {}", name)
    } else {
        format!("brew {} ({})", name, args.join(", "))
    }
}

/// Validate Brewfile path exists and is readable.
/// Extracted for testability - returns the resolved path or error message.
pub(crate) fn validate_brewfile_path(path: Option<PathBuf>, cwd: &std::path::Path) -> Result<PathBuf, String> {
    match path {
        Some(p) => {
            if p.exists() {
                Ok(p)
            } else {
                Err(format!("Brewfile not found at: {}", p.display()))
            }
        }
        None => {
            // Walk up directories looking for Brewfile
            let mut current = cwd;
            loop {
                let brewfile = current.join("Brewfile");
                if brewfile.exists() {
                    return Ok(brewfile);
                }
                match current.parent() {
                    Some(parent) => current = parent,
                    None => return Err("No Brewfile found in current directory or parent directories".to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zb_io::BrewfileEntry;

    #[test]
    fn test_count_brewfile_entries_mixed() {
        let entries = vec![
            BrewfileEntry::Tap { name: "homebrew/core".to_string() },
            BrewfileEntry::Brew { name: "git".to_string(), args: vec![] },
            BrewfileEntry::Brew { name: "ripgrep".to_string(), args: vec![] },
            BrewfileEntry::Comment("# a comment".to_string()),
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 1);
        assert_eq!(brews, 2);
    }

    #[test]
    fn test_count_brewfile_entries_only_taps() {
        let entries = vec![
            BrewfileEntry::Tap { name: "homebrew/core".to_string() },
            BrewfileEntry::Tap { name: "homebrew/cask".to_string() },
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 2);
        assert_eq!(brews, 0);
    }

    #[test]
    fn test_count_brewfile_entries_only_brews() {
        let entries = vec![
            BrewfileEntry::Brew { name: "git".to_string(), args: vec![] },
            BrewfileEntry::Brew { name: "ripgrep".to_string(), args: vec![] },
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 0);
        assert_eq!(brews, 2);
    }

    #[test]
    fn test_count_brewfile_entries_only_comments() {
        let entries = vec![
            BrewfileEntry::Comment("# header".to_string()),
            BrewfileEntry::Comment("".to_string()),
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 0);
        assert_eq!(brews, 0);
    }

    #[test]
    fn test_count_brewfile_entries_empty() {
        let entries: Vec<BrewfileEntry> = vec![];
        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 0);
        assert_eq!(brews, 0);
    }

    #[test]
    fn test_format_brew_entry_no_args() {
        let result = format_brew_entry("git", &[]);
        assert_eq!(result, "brew git");
    }

    #[test]
    fn test_format_brew_entry_single_arg() {
        let result = format_brew_entry("neovim", &["--HEAD".to_string()]);
        assert_eq!(result, "brew neovim (--HEAD)");
    }

    #[test]
    fn test_format_brew_entry_multiple_args() {
        let result = format_brew_entry("pkg", &["--HEAD".to_string(), "--with-foo".to_string()]);
        assert_eq!(result, "brew pkg (--HEAD, --with-foo)");
    }

    #[test]
    fn test_format_brew_entry_versioned_formula() {
        let result = format_brew_entry("python@3.11", &[]);
        assert_eq!(result, "brew python@3.11");
    }

    #[test]
    fn test_validate_brewfile_path_explicit_exists() {
        use std::fs;
        use std::env;
        
        let temp_dir = env::temp_dir().join("zb-test-brewfile-exists");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        
        let brewfile = temp_dir.join("Brewfile");
        fs::write(&brewfile, "brew \"git\"").unwrap();
        
        let result = validate_brewfile_path(Some(brewfile.clone()), &temp_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), brewfile);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_brewfile_path_explicit_missing() {
        use std::env;
        
        let temp_dir = env::temp_dir().join("zb-test-brewfile-missing");
        let missing_path = temp_dir.join("nonexistent").join("Brewfile");
        
        let result = validate_brewfile_path(Some(missing_path.clone()), &temp_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_validate_brewfile_path_search_finds_in_cwd() {
        use std::fs;
        use std::env;
        
        let temp_dir = env::temp_dir().join("zb-test-brewfile-cwd");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        
        let brewfile = temp_dir.join("Brewfile");
        fs::write(&brewfile, "brew \"git\"").unwrap();
        
        let result = validate_brewfile_path(None, &temp_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), brewfile);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_brewfile_path_search_finds_in_parent() {
        use std::fs;
        use std::env;
        
        let temp_dir = env::temp_dir().join("zb-test-brewfile-parent");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let child_dir = temp_dir.join("subdir");
        fs::create_dir_all(&child_dir).unwrap();
        
        let brewfile = temp_dir.join("Brewfile");
        fs::write(&brewfile, "brew \"git\"").unwrap();
        
        let result = validate_brewfile_path(None, &child_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), brewfile);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_brewfile_path_search_not_found() {
        use std::fs;
        use std::env;
        
        let temp_dir = env::temp_dir().join("zb-test-brewfile-notfound");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        
        // No Brewfile created
        let result = validate_brewfile_path(None, &temp_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No Brewfile found"));
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
