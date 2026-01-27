//! Bundle command implementations.

use console::style;
use std::path::PathBuf;

use zb_io::install::Installer;
use zb_io::{BrewfileEntry, BundleCheckResult, BundleInstallResult};

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
    let brewfile_path = match file {
        Some(path) => {
            // Validate explicit path exists
            validate_brewfile_path(Some(path), cwd)
                .map_err(|e| zb_core::Error::StoreCorruption { message: e })?
        }
        None => installer
            .find_brewfile(cwd)
            .ok_or_else(|| zb_core::Error::StoreCorruption {
                message: format_no_brewfile_error(),
            })?,
    };

    println!(
        "{} Installing from {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_install(&brewfile_path).await?;

    print!("{}", format_install_result(&result));

    if !result.failed.is_empty() {
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
            eprintln!("{}", format_dump_exists_error(&path));
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
        print!("{}", format_dump_output(&content));
    }

    Ok(())
}

fn run_check(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
    strict: bool,
) -> Result<(), zb_core::Error> {
    let brewfile_path = match file {
        Some(path) => {
            // Validate explicit path exists
            validate_brewfile_path(Some(path), cwd)
                .map_err(|e| zb_core::Error::StoreCorruption { message: e })?
        }
        None => installer
            .find_brewfile(cwd)
            .ok_or_else(|| zb_core::Error::StoreCorruption {
                message: format_no_brewfile_error(),
            })?,
    };

    println!(
        "{} Checking {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_check(&brewfile_path)?;

    print!("{}", format_check_result(&result));

    if !result.satisfied && strict {
        std::process::exit(1);
    }

    Ok(())
}

fn run_list(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
) -> Result<(), zb_core::Error> {
    let brewfile_path = match file {
        Some(path) => {
            // Validate explicit path exists
            validate_brewfile_path(Some(path), cwd)
                .map_err(|e| zb_core::Error::StoreCorruption { message: e })?
        }
        None => installer
            .find_brewfile(cwd)
            .ok_or_else(|| zb_core::Error::StoreCorruption {
                message: format_no_brewfile_error(),
            })?,
    };

    let entries = installer.parse_brewfile(&brewfile_path)?;

    println!(
        "{} Entries in {}:",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );
    print!("{}", format_list_output(&entries));

    Ok(())
}

// ============================================================================
// Pure functions extracted for testability
// ============================================================================

/// Count taps and formulas in Brewfile entries.
pub(crate) fn count_brewfile_entries(entries: &[BrewfileEntry]) -> (usize, usize) {
    let mut tap_count = 0;
    let mut brew_count = 0;

    for entry in entries {
        match entry {
            BrewfileEntry::Tap { .. } => tap_count += 1,
            BrewfileEntry::Brew { .. } => brew_count += 1,
            BrewfileEntry::Comment(_) => {}
        }
    }

    (tap_count, brew_count)
}

/// Format brew entry with args for display.
#[cfg(test)]
pub(crate) fn format_brew_entry(name: &str, args: &[String]) -> String {
    if args.is_empty() {
        format!("brew {}", name)
    } else {
        format!("brew {} ({})", name, args.join(", "))
    }
}

/// Format tap entry for display.
#[cfg(test)]
pub(crate) fn format_tap_entry(name: &str) -> String {
    format!("tap  {}", name)
}

/// Validate Brewfile path exists and is readable.
/// Returns the resolved path or error message.
pub(crate) fn validate_brewfile_path(
    path: Option<PathBuf>,
    cwd: &std::path::Path,
) -> Result<PathBuf, String> {
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
                    None => {
                        return Err(
                            "No Brewfile found in current directory or parent directories"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
}

/// Format the install result for display.
/// Returns formatted output without ANSI color codes for testability.
#[cfg(test)]
pub(crate) fn format_install_result_plain(result: &BundleInstallResult) -> String {
    let mut output = String::new();

    if !result.taps_added.is_empty() {
        output.push_str("\n==> Taps added:\n");
        for tap in &result.taps_added {
            output.push_str(&format!("    ✓ {}\n", tap));
        }
    }

    if !result.formulas_installed.is_empty() {
        output.push_str("\n==> Formulas installed:\n");
        for formula in &result.formulas_installed {
            output.push_str(&format!("    ✓ {}\n", formula));
        }
    }

    if !result.formulas_skipped.is_empty() {
        output.push_str("\n==> Already installed:\n");
        for formula in &result.formulas_skipped {
            output.push_str(&format!("    - {}\n", formula));
        }
    }

    if !result.failed.is_empty() {
        output.push_str("\n==> Failed:\n");
        for (name, error) in &result.failed {
            output.push_str(&format!("    ✗ {}: {}\n", name, error));
        }
    }

    // Summary
    output.push('\n');
    let total_installed = result.taps_added.len() + result.formulas_installed.len();
    if result.failed.is_empty() {
        output.push_str(&format!(
            "==> Bundle complete. {} installed, {} already satisfied.\n",
            total_installed,
            result.formulas_skipped.len()
        ));
    } else {
        output.push_str(&format!(
            "==> Bundle complete with errors. {} installed, {} already satisfied, {} failed.\n",
            total_installed,
            result.formulas_skipped.len(),
            result.failed.len()
        ));
    }

    output
}

/// Format the install result with ANSI colors for terminal display.
fn format_install_result(result: &BundleInstallResult) -> String {
    let mut output = String::new();

    if !result.taps_added.is_empty() {
        output.push_str(&format!("\n{} Taps added:\n", style("==>").cyan().bold()));
        for tap in &result.taps_added {
            output.push_str(&format!("    {} {}\n", style("✓").green(), tap));
        }
    }

    if !result.formulas_installed.is_empty() {
        output.push_str(&format!(
            "\n{} Formulas installed:\n",
            style("==>").cyan().bold()
        ));
        for formula in &result.formulas_installed {
            output.push_str(&format!("    {} {}\n", style("✓").green(), formula));
        }
    }

    if !result.formulas_skipped.is_empty() {
        output.push_str(&format!(
            "\n{} Already installed:\n",
            style("==>").cyan().bold()
        ));
        for formula in &result.formulas_skipped {
            output.push_str(&format!("    {} {}\n", style("-").dim(), formula));
        }
    }

    if !result.failed.is_empty() {
        output.push_str(&format!("\n{} Failed:\n", style("==>").red().bold()));
        for (name, error) in &result.failed {
            output.push_str(&format!("    {} {}: {}\n", style("✗").red(), name, error));
        }
    }

    // Summary
    output.push('\n');
    let (total_installed, skipped, failed, has_errors) = compute_install_summary(result);
    if !has_errors {
        output.push_str(&format!(
            "{} Bundle complete. {} installed, {} already satisfied.\n",
            style("==>").cyan().bold(),
            total_installed,
            skipped
        ));
    } else {
        output.push_str(&format!(
            "{} Bundle complete with errors. {} installed, {} already satisfied, {} failed.\n",
            style("==>").yellow().bold(),
            total_installed,
            skipped,
            failed
        ));
    }

    output
}

/// Format the check result for display (plain text).
#[cfg(test)]
pub(crate) fn format_check_result_plain(result: &BundleCheckResult) -> String {
    let mut output = String::new();

    if result.satisfied {
        output.push_str("\n==> All entries are satisfied!\n");
    } else {
        if !result.missing_taps.is_empty() {
            output.push_str("\n==> Missing taps:\n");
            for tap in &result.missing_taps {
                output.push_str(&format!("    ✗ {}\n", tap));
            }
        }

        if !result.missing_formulas.is_empty() {
            output.push_str("\n==> Missing formulas:\n");
            for formula in &result.missing_formulas {
                output.push_str(&format!("    ✗ {}\n", formula));
            }
        }

        output.push_str("\n    → Run zb bundle to install missing entries\n");
    }

    output
}

/// Format the check result with ANSI colors for terminal display.
fn format_check_result(result: &BundleCheckResult) -> String {
    let mut output = String::new();

    let (_missing_taps_count, _missing_formulas_count, all_satisfied) =
        compute_check_summary(result);

    if all_satisfied {
        output.push_str(&format!(
            "\n{} All entries are satisfied!\n",
            style("==>").green().bold()
        ));
    } else {
        if !result.missing_taps.is_empty() {
            output.push_str(&format!(
                "\n{} Missing taps:\n",
                style("==>").yellow().bold()
            ));
            for tap in &result.missing_taps {
                output.push_str(&format!("    {} {}\n", style("✗").red(), tap));
            }
        }

        if !result.missing_formulas.is_empty() {
            output.push_str(&format!(
                "\n{} Missing formulas:\n",
                style("==>").yellow().bold()
            ));
            for formula in &result.missing_formulas {
                output.push_str(&format!("    {} {}\n", style("✗").red(), formula));
            }
        }

        output.push_str(&format!(
            "\n    → Run {} bundle to install missing entries\n",
            style("zb").cyan()
        ));
    }

    output
}

/// Format the list output for display (plain text).
#[cfg(test)]
pub(crate) fn format_list_output_plain(entries: &[BrewfileEntry]) -> String {
    let mut output = String::new();
    output.push('\n');

    for entry in entries {
        match entry {
            BrewfileEntry::Tap { name } => {
                output.push_str(&format!("{}\n", format_tap_entry(name)));
            }
            BrewfileEntry::Brew { name, args } => {
                output.push_str(&format!("{}\n", format_brew_entry(name, args)));
            }
            BrewfileEntry::Comment(_) => {
                // Skip comments in list output
            }
        }
    }

    let (tap_count, brew_count) = count_brewfile_entries(entries);
    output.push_str(&format!(
        "\n==> {} taps, {} formulas\n",
        tap_count, brew_count
    ));

    output
}

/// Format the list output with ANSI colors for terminal display.
fn format_list_output(entries: &[BrewfileEntry]) -> String {
    let mut output = String::new();
    output.push('\n');

    for entry in entries {
        match entry {
            BrewfileEntry::Tap { name } => {
                // Use format_tap_entry pattern but with colors
                output.push_str(&format!("tap  {}\n", style(name).cyan()));
            }
            BrewfileEntry::Brew { name, args } => {
                // Use format_brew_entry pattern but with colors
                if args.is_empty() {
                    output.push_str(&format!("brew {}\n", style(name).green()));
                } else {
                    output.push_str(&format!(
                        "brew {} ({})\n",
                        style(name).green(),
                        args.join(", ")
                    ));
                }
            }
            BrewfileEntry::Comment(_) => {
                // Skip comments in list output
            }
        }
    }

    let (tap_count, brew_count) = count_brewfile_entries(entries);
    output.push_str(&format!(
        "\n{} {} taps, {} formulas\n",
        style("==>").cyan().bold(),
        tap_count,
        brew_count
    ));

    output
}

/// Format the dump output, ensuring it ends with a newline.
pub(crate) fn format_dump_output(content: &str) -> String {
    if content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{}\n", content)
    }
}

/// Format error message when dump file already exists.
pub(crate) fn format_dump_exists_error(path: &std::path::Path) -> String {
    format!(
        "{} File '{}' already exists. Use --force to overwrite.",
        style("error:").red().bold(),
        path.display()
    )
}

/// Format error message when dump file already exists (plain text).
#[cfg(test)]
pub(crate) fn format_dump_exists_error_plain(path: &std::path::Path) -> String {
    format!(
        "error: File '{}' already exists. Use --force to overwrite.",
        path.display()
    )
}

/// Format the "no brewfile found" error message.
pub(crate) fn format_no_brewfile_error() -> String {
    "No Brewfile found in current directory or parent directories".to_string()
}

/// Compute install summary statistics from result.
pub(crate) fn compute_install_summary(result: &BundleInstallResult) -> (usize, usize, usize, bool) {
    let total_installed = result.taps_added.len() + result.formulas_installed.len();
    let skipped = result.formulas_skipped.len();
    let failed = result.failed.len();
    let has_errors = !result.failed.is_empty();
    (total_installed, skipped, failed, has_errors)
}

/// Compute check summary from result.
pub(crate) fn compute_check_summary(result: &BundleCheckResult) -> (usize, usize, bool) {
    let missing_taps = result.missing_taps.len();
    let missing_formulas = result.missing_formulas.len();
    let all_satisfied = result.satisfied;
    (missing_taps, missing_formulas, all_satisfied)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // count_brewfile_entries tests
    // ========================================================================

    #[test]
    fn test_count_brewfile_entries_mixed() {
        let entries = vec![
            BrewfileEntry::Tap {
                name: "homebrew/core".to_string(),
            },
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "ripgrep".to_string(),
                args: vec![],
            },
            BrewfileEntry::Comment("# a comment".to_string()),
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 1);
        assert_eq!(brews, 2);
    }

    #[test]
    fn test_count_brewfile_entries_only_taps() {
        let entries = vec![
            BrewfileEntry::Tap {
                name: "homebrew/core".to_string(),
            },
            BrewfileEntry::Tap {
                name: "homebrew/cask".to_string(),
            },
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 2);
        assert_eq!(brews, 0);
    }

    #[test]
    fn test_count_brewfile_entries_only_brews() {
        let entries = vec![
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "ripgrep".to_string(),
                args: vec![],
            },
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
    fn test_count_brewfile_entries_large() {
        let mut entries = Vec::new();
        for i in 0..100 {
            entries.push(BrewfileEntry::Tap {
                name: format!("tap{}", i),
            });
            entries.push(BrewfileEntry::Brew {
                name: format!("brew{}", i),
                args: vec![],
            });
        }
        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 100);
        assert_eq!(brews, 100);
    }

    // ========================================================================
    // format_brew_entry tests
    // ========================================================================

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
    fn test_format_brew_entry_with_tap_prefix() {
        let result = format_brew_entry("user/repo/formula", &[]);
        assert_eq!(result, "brew user/repo/formula");
    }

    #[test]
    fn test_format_brew_entry_many_args() {
        let args = vec![
            "--HEAD".to_string(),
            "--with-foo".to_string(),
            "--with-bar".to_string(),
            "--with-baz".to_string(),
        ];
        let result = format_brew_entry("complex", &args);
        assert_eq!(
            result,
            "brew complex (--HEAD, --with-foo, --with-bar, --with-baz)"
        );
    }

    // ========================================================================
    // format_tap_entry tests
    // ========================================================================

    #[test]
    fn test_format_tap_entry_simple() {
        let result = format_tap_entry("homebrew/core");
        assert_eq!(result, "tap  homebrew/core");
    }

    #[test]
    fn test_format_tap_entry_user_repo() {
        let result = format_tap_entry("user/repo");
        assert_eq!(result, "tap  user/repo");
    }

    // ========================================================================
    // validate_brewfile_path tests
    // ========================================================================

    #[test]
    fn test_validate_brewfile_path_explicit_exists() {
        use std::env;
        use std::fs;

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

        let result = validate_brewfile_path(Some(missing_path), &temp_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_validate_brewfile_path_search_finds_in_cwd() {
        use std::env;
        use std::fs;

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
        use std::env;
        use std::fs;

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
    fn test_validate_brewfile_path_search_finds_in_grandparent() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-brewfile-grandparent");
        let _ = fs::remove_dir_all(&temp_dir);

        let grandchild_dir = temp_dir.join("subdir1").join("subdir2");
        fs::create_dir_all(&grandchild_dir).unwrap();

        let brewfile = temp_dir.join("Brewfile");
        fs::write(&brewfile, "brew \"git\"").unwrap();

        let result = validate_brewfile_path(None, &grandchild_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), brewfile);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_brewfile_path_search_not_found() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-brewfile-notfound");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // No Brewfile created
        let result = validate_brewfile_path(None, &temp_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No Brewfile found"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    // ========================================================================
    // format_install_result_plain tests
    // ========================================================================

    #[test]
    fn test_format_install_result_all_success() {
        let result = BundleInstallResult {
            taps_added: vec!["user/repo".to_string()],
            formulas_installed: vec!["git".to_string(), "ripgrep".to_string()],
            formulas_skipped: vec![],
            failed: vec![],
        };

        let output = format_install_result_plain(&result);
        assert!(output.contains("Taps added:"));
        assert!(output.contains("✓ user/repo"));
        assert!(output.contains("Formulas installed:"));
        assert!(output.contains("✓ git"));
        assert!(output.contains("✓ ripgrep"));
        assert!(output.contains("Bundle complete. 3 installed, 0 already satisfied."));
        assert!(!output.contains("Failed:"));
    }

    #[test]
    fn test_format_install_result_with_skipped() {
        let result = BundleInstallResult {
            taps_added: vec![],
            formulas_installed: vec!["git".to_string()],
            formulas_skipped: vec!["ripgrep".to_string(), "fd".to_string()],
            failed: vec![],
        };

        let output = format_install_result_plain(&result);
        assert!(output.contains("Already installed:"));
        assert!(output.contains("- ripgrep"));
        assert!(output.contains("- fd"));
        assert!(output.contains("1 installed, 2 already satisfied"));
    }

    #[test]
    fn test_format_install_result_with_failures() {
        let result = BundleInstallResult {
            taps_added: vec![],
            formulas_installed: vec!["git".to_string()],
            formulas_skipped: vec![],
            failed: vec![("badpkg".to_string(), "not found".to_string())],
        };

        let output = format_install_result_plain(&result);
        assert!(output.contains("Failed:"));
        assert!(output.contains("✗ badpkg: not found"));
        assert!(output.contains("Bundle complete with errors"));
        assert!(output.contains("1 failed"));
    }

    #[test]
    fn test_format_install_result_empty() {
        let result = BundleInstallResult::default();

        let output = format_install_result_plain(&result);
        assert!(output.contains("Bundle complete. 0 installed, 0 already satisfied."));
        assert!(!output.contains("Taps added:"));
        assert!(!output.contains("Formulas installed:"));
    }

    #[test]
    fn test_format_install_result_only_skipped() {
        let result = BundleInstallResult {
            taps_added: vec![],
            formulas_installed: vec![],
            formulas_skipped: vec!["git".to_string(), "ripgrep".to_string()],
            failed: vec![],
        };

        let output = format_install_result_plain(&result);
        assert!(output.contains("Already installed:"));
        assert!(output.contains("0 installed, 2 already satisfied"));
    }

    #[test]
    fn test_format_install_result_multiple_failures() {
        let result = BundleInstallResult {
            taps_added: vec![],
            formulas_installed: vec![],
            formulas_skipped: vec![],
            failed: vec![
                ("pkg1".to_string(), "network error".to_string()),
                ("pkg2".to_string(), "checksum mismatch".to_string()),
                ("pkg3".to_string(), "build failed".to_string()),
            ],
        };

        let output = format_install_result_plain(&result);
        assert!(output.contains("✗ pkg1: network error"));
        assert!(output.contains("✗ pkg2: checksum mismatch"));
        assert!(output.contains("✗ pkg3: build failed"));
        assert!(output.contains("3 failed"));
    }

    // ========================================================================
    // format_check_result_plain tests
    // ========================================================================

    #[test]
    fn test_format_check_result_satisfied() {
        let result = BundleCheckResult {
            missing_taps: vec![],
            missing_formulas: vec![],
            mismatched_formulas: vec![],
            satisfied: true,
        };

        let output = format_check_result_plain(&result);
        assert!(output.contains("All entries are satisfied!"));
        assert!(!output.contains("Missing"));
    }

    #[test]
    fn test_format_check_result_missing_taps() {
        let result = BundleCheckResult {
            missing_taps: vec!["user/repo".to_string(), "another/tap".to_string()],
            missing_formulas: vec![],
            mismatched_formulas: vec![],
            satisfied: false,
        };

        let output = format_check_result_plain(&result);
        assert!(output.contains("Missing taps:"));
        assert!(output.contains("✗ user/repo"));
        assert!(output.contains("✗ another/tap"));
        assert!(output.contains("Run zb bundle"));
    }

    #[test]
    fn test_format_check_result_missing_formulas() {
        let result = BundleCheckResult {
            missing_taps: vec![],
            missing_formulas: vec!["git".to_string(), "ripgrep".to_string()],
            mismatched_formulas: vec![],
            satisfied: false,
        };

        let output = format_check_result_plain(&result);
        assert!(output.contains("Missing formulas:"));
        assert!(output.contains("✗ git"));
        assert!(output.contains("✗ ripgrep"));
    }

    #[test]
    fn test_format_check_result_both_missing() {
        let result = BundleCheckResult {
            missing_taps: vec!["user/repo".to_string()],
            missing_formulas: vec!["git".to_string()],
            mismatched_formulas: vec![],
            satisfied: false,
        };

        let output = format_check_result_plain(&result);
        assert!(output.contains("Missing taps:"));
        assert!(output.contains("Missing formulas:"));
        assert!(output.contains("Run zb bundle"));
    }

    // ========================================================================
    // format_list_output_plain tests
    // ========================================================================

    #[test]
    fn test_format_list_output_mixed() {
        let entries = vec![
            BrewfileEntry::Tap {
                name: "homebrew/core".to_string(),
            },
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "neovim".to_string(),
                args: vec!["--HEAD".to_string()],
            },
            BrewfileEntry::Comment("# comment".to_string()),
        ];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("tap  homebrew/core"));
        assert!(output.contains("brew git"));
        assert!(output.contains("brew neovim (--HEAD)"));
        assert!(output.contains("1 taps, 2 formulas"));
        // Comments should be excluded
        assert!(!output.contains("# comment"));
    }

    #[test]
    fn test_format_list_output_only_taps() {
        let entries = vec![
            BrewfileEntry::Tap {
                name: "tap1".to_string(),
            },
            BrewfileEntry::Tap {
                name: "tap2".to_string(),
            },
        ];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("tap  tap1"));
        assert!(output.contains("tap  tap2"));
        assert!(output.contains("2 taps, 0 formulas"));
    }

    #[test]
    fn test_format_list_output_only_brews() {
        let entries = vec![
            BrewfileEntry::Brew {
                name: "git".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "ripgrep".to_string(),
                args: vec![],
            },
        ];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("brew git"));
        assert!(output.contains("brew ripgrep"));
        assert!(output.contains("0 taps, 2 formulas"));
    }

    #[test]
    fn test_format_list_output_empty() {
        let entries: Vec<BrewfileEntry> = vec![];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("0 taps, 0 formulas"));
    }

    #[test]
    fn test_format_list_output_only_comments() {
        let entries = vec![
            BrewfileEntry::Comment("# header".to_string()),
            BrewfileEntry::Comment("".to_string()),
            BrewfileEntry::Comment("# footer".to_string()),
        ];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("0 taps, 0 formulas"));
    }

    #[test]
    fn test_format_list_output_brew_with_multiple_args() {
        let entries = vec![BrewfileEntry::Brew {
            name: "complex".to_string(),
            args: vec![
                "--HEAD".to_string(),
                "--with-opt1".to_string(),
                "--with-opt2".to_string(),
            ],
        }];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("brew complex (--HEAD, --with-opt1, --with-opt2)"));
    }

    // ========================================================================
    // format_dump_output tests
    // ========================================================================

    #[test]
    fn test_format_dump_output_with_trailing_newline() {
        let content = "tap \"user/repo\"\nbrew \"git\"\n";
        let result = format_dump_output(content);
        assert_eq!(result, content);
    }

    #[test]
    fn test_format_dump_output_without_trailing_newline() {
        let content = "tap \"user/repo\"\nbrew \"git\"";
        let result = format_dump_output(content);
        assert_eq!(result, "tap \"user/repo\"\nbrew \"git\"\n");
    }

    #[test]
    fn test_format_dump_output_empty() {
        let content = "";
        let result = format_dump_output(content);
        assert_eq!(result, "\n");
    }

    #[test]
    fn test_format_dump_output_only_newline() {
        let content = "\n";
        let result = format_dump_output(content);
        assert_eq!(result, "\n");
    }

    // ========================================================================
    // format_dump_exists_error_plain tests
    // ========================================================================

    #[test]
    fn test_format_dump_exists_error_plain() {
        use std::path::Path;
        let path = Path::new("/some/path/Brewfile");
        let result = format_dump_exists_error_plain(path);
        assert!(result.contains("error:"));
        assert!(result.contains("/some/path/Brewfile"));
        assert!(result.contains("already exists"));
        assert!(result.contains("--force"));
    }

    #[test]
    fn test_format_dump_exists_error_plain_relative_path() {
        use std::path::Path;
        let path = Path::new("Brewfile");
        let result = format_dump_exists_error_plain(path);
        assert!(result.contains("Brewfile"));
        assert!(result.contains("--force"));
    }

    // ========================================================================
    // format_no_brewfile_error tests
    // ========================================================================

    #[test]
    fn test_format_no_brewfile_error() {
        let result = format_no_brewfile_error();
        assert!(result.contains("No Brewfile found"));
        assert!(result.contains("current directory"));
        assert!(result.contains("parent directories"));
    }

    // ========================================================================
    // compute_install_summary tests
    // ========================================================================

    #[test]
    fn test_compute_install_summary_success() {
        let result = BundleInstallResult {
            taps_added: vec!["tap1".to_string()],
            formulas_installed: vec!["git".to_string(), "ripgrep".to_string()],
            formulas_skipped: vec!["fd".to_string()],
            failed: vec![],
        };

        let (installed, skipped, failed, has_errors) = compute_install_summary(&result);
        assert_eq!(installed, 3); // 1 tap + 2 formulas
        assert_eq!(skipped, 1);
        assert_eq!(failed, 0);
        assert!(!has_errors);
    }

    #[test]
    fn test_compute_install_summary_with_errors() {
        let result = BundleInstallResult {
            taps_added: vec![],
            formulas_installed: vec!["git".to_string()],
            formulas_skipped: vec![],
            failed: vec![
                ("bad1".to_string(), "error".to_string()),
                ("bad2".to_string(), "error".to_string()),
            ],
        };

        let (installed, skipped, failed, has_errors) = compute_install_summary(&result);
        assert_eq!(installed, 1);
        assert_eq!(skipped, 0);
        assert_eq!(failed, 2);
        assert!(has_errors);
    }

    #[test]
    fn test_compute_install_summary_empty() {
        let result = BundleInstallResult::default();

        let (installed, skipped, failed, has_errors) = compute_install_summary(&result);
        assert_eq!(installed, 0);
        assert_eq!(skipped, 0);
        assert_eq!(failed, 0);
        assert!(!has_errors);
    }

    // ========================================================================
    // compute_check_summary tests
    // ========================================================================

    #[test]
    fn test_compute_check_summary_satisfied() {
        let result = BundleCheckResult {
            missing_taps: vec![],
            missing_formulas: vec![],
            mismatched_formulas: vec![],
            satisfied: true,
        };

        let (missing_taps, missing_formulas, satisfied) = compute_check_summary(&result);
        assert_eq!(missing_taps, 0);
        assert_eq!(missing_formulas, 0);
        assert!(satisfied);
    }

    #[test]
    fn test_compute_check_summary_unsatisfied() {
        let result = BundleCheckResult {
            missing_taps: vec!["tap1".to_string(), "tap2".to_string()],
            missing_formulas: vec!["git".to_string()],
            mismatched_formulas: vec![],
            satisfied: false,
        };

        let (missing_taps, missing_formulas, satisfied) = compute_check_summary(&result);
        assert_eq!(missing_taps, 2);
        assert_eq!(missing_formulas, 1);
        assert!(!satisfied);
    }

    // ========================================================================
    // Edge cases and integration-like tests
    // ========================================================================

    #[test]
    fn test_format_brew_entry_special_characters() {
        // Formula names can have special chars
        let result = format_brew_entry("pkg-config", &[]);
        assert_eq!(result, "brew pkg-config");

        let result = format_brew_entry("node@18", &[]);
        assert_eq!(result, "brew node@18");
    }

    #[test]
    fn test_format_install_result_preserves_order() {
        let result = BundleInstallResult {
            taps_added: vec!["tap1".to_string(), "tap2".to_string()],
            formulas_installed: vec!["aaa".to_string(), "zzz".to_string(), "mmm".to_string()],
            formulas_skipped: vec![],
            failed: vec![],
        };

        let output = format_install_result_plain(&result);
        // Check order is preserved
        let tap1_pos = output.find("tap1").unwrap();
        let tap2_pos = output.find("tap2").unwrap();
        assert!(tap1_pos < tap2_pos);

        let aaa_pos = output.find("aaa").unwrap();
        let zzz_pos = output.find("zzz").unwrap();
        let mmm_pos = output.find("mmm").unwrap();
        assert!(aaa_pos < zzz_pos);
        assert!(zzz_pos < mmm_pos);
    }

    #[test]
    fn test_count_brewfile_entries_with_args() {
        // Brews with args should still count as brews
        let entries = vec![
            BrewfileEntry::Brew {
                name: "neovim".to_string(),
                args: vec!["--HEAD".to_string()],
            },
            BrewfileEntry::Brew {
                name: "vim".to_string(),
                args: vec![],
            },
        ];

        let (taps, brews) = count_brewfile_entries(&entries);
        assert_eq!(taps, 0);
        assert_eq!(brews, 2);
    }

    #[test]
    fn test_format_list_output_versioned_formulas() {
        let entries = vec![
            BrewfileEntry::Brew {
                name: "python@3.9".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "python@3.11".to_string(),
                args: vec![],
            },
            BrewfileEntry::Brew {
                name: "node@18".to_string(),
                args: vec![],
            },
        ];

        let output = format_list_output_plain(&entries);
        assert!(output.contains("brew python@3.9"));
        assert!(output.contains("brew python@3.11"));
        assert!(output.contains("brew node@18"));
        assert!(output.contains("0 taps, 3 formulas"));
    }

    #[test]
    fn test_format_check_result_long_names() {
        let result = BundleCheckResult {
            missing_taps: vec!["very-long-username/very-long-repo-name".to_string()],
            missing_formulas: vec!["some-package-with-a-very-long-name@1.2.3".to_string()],
            mismatched_formulas: vec![],
            satisfied: false,
        };

        let output = format_check_result_plain(&result);
        assert!(output.contains("very-long-username/very-long-repo-name"));
        assert!(output.contains("some-package-with-a-very-long-name@1.2.3"));
    }
}
