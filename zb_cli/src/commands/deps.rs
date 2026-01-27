//! Deps, uses, and leaves command implementations.

use console::style;

use zb_io::install::Installer;

use crate::display::print_deps_tree;

// ============================================================================
// Formatting helpers (pure functions for testability)
// ============================================================================

/// Format the header for deps command.
pub fn format_deps_header(formula: &str, tree: bool, all: bool) -> String {
    let suffix = if tree {
        " (tree view)"
    } else if all {
        " (all)"
    } else {
        ""
    };
    format!(
        "{} Dependencies for {}{}:",
        style("==>").cyan().bold(),
        style(formula).bold(),
        suffix
    )
}

/// Format the "no dependencies" message.
pub fn format_no_deps_message(formula: &str, installed_only: bool) -> String {
    let qualifier = if installed_only { " installed " } else { " " };
    format!(
        "{} {} has no{}dependencies.",
        style("==>").cyan().bold(),
        style(formula).bold(),
        qualifier
    )
}

/// Format a single dependency line with installation status marker.
pub fn format_dep_line(name: &str, is_installed: bool) -> String {
    let marker = if is_installed {
        style("✓").green().to_string()
    } else {
        style("✗").red().to_string()
    };
    format!("  {} {}", marker, name)
}

/// Format all dependency lines without installation status (plain list).
pub fn format_deps_plain(deps: &[String]) -> Vec<String> {
    deps.iter().map(|d| format!("  {}", d)).collect()
}

// ============================================================================
// Command implementations
// ============================================================================

/// Run the deps command.
pub async fn run_deps(
    installer: &mut Installer,
    formula: String,
    tree: bool,
    installed: bool,
    all: bool,
) -> Result<(), zb_core::Error> {
    if tree {
        println!("{}", format_deps_header(&formula, true, false));
        println!();

        let tree_data = installer.get_deps_tree(&formula, installed).await?;
        print_deps_tree(&tree_data, "", true);
    } else {
        let deps = installer.get_deps(&formula, installed, all).await?;

        if deps.is_empty() {
            println!("{}", format_no_deps_message(&formula, installed));
        } else {
            println!("{}", format_deps_header(&formula, false, all));
            println!();

            for dep in &deps {
                println!("{}", format_dep_line(dep, installer.is_installed(dep)));
            }
        }
    }

    Ok(())
}

/// Format the "checking uses" header.
pub fn format_uses_header(formula: &str) -> String {
    format!(
        "{} Checking what uses {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    )
}

/// Format the "no uses" message.
pub fn format_no_uses_message(formula: &str) -> String {
    format!(
        "{} No installed formulas use {}.",
        style("==>").cyan().bold(),
        style(formula).bold()
    )
}

/// Format the uses result header.
pub fn format_uses_result_header(formula: &str, count: usize, recursive: bool) -> String {
    let suffix = if recursive {
        " (directly or indirectly)"
    } else {
        ""
    };
    format!(
        "{} {} installed formulas use {}{}:",
        style("==>").cyan().bold(),
        style(count).green().bold(),
        style(formula).bold(),
        suffix
    )
}

/// Format the leaves header.
pub fn format_leaves_header() -> String {
    format!("{} Finding leaf packages...", style("==>").cyan().bold())
}

/// Format the leaves result header.
pub fn format_leaves_result_header(count: usize) -> String {
    format!(
        "{} {} leaf packages (not dependencies of other installed packages):",
        style("==>").cyan().bold(),
        style(count).green().bold()
    )
}

/// Run the uses command.
pub async fn run_uses(
    installer: &mut Installer,
    formula: String,
    recursive: bool,
) -> Result<(), zb_core::Error> {
    println!("{}", format_uses_header(&formula));

    // Check if the formula exists (either installed or in API)
    let formula_exists =
        installer.is_installed(&formula) || installer.get_formula(&formula).await.is_ok();

    if !formula_exists {
        println!("Formula '{}' not found.", formula);
        std::process::exit(1);
    }

    // uses command defaults to installed-only (installed flag is ignored, always true)
    let uses = installer.get_uses(&formula, true, recursive).await?;

    if uses.is_empty() {
        println!("{}", format_no_uses_message(&formula));
    } else {
        println!(
            "{}",
            format_uses_result_header(&formula, uses.len(), recursive)
        );
        println!();

        for name in &uses {
            println!("  {}", name);
        }
    }

    Ok(())
}

/// Run the leaves command.
pub async fn run_leaves(installer: &mut Installer) -> Result<(), zb_core::Error> {
    println!("{}", format_leaves_header());

    let leaves = installer.get_leaves().await?;

    if leaves.is_empty() {
        println!("No installed packages, or all packages are dependencies.");
    } else {
        println!("{}", format_leaves_result_header(leaves.len()));
        println!();

        for name in &leaves {
            println!("  {}", name);
        }
    }

    Ok(())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Deps Header Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_deps_header_simple() {
        let result = format_deps_header("git", false, false);
        assert!(result.contains("Dependencies for"));
        assert!(result.contains("git"));
        assert!(!result.contains("(all)"));
        assert!(!result.contains("(tree view)"));
    }

    #[test]
    fn test_format_deps_header_tree_view() {
        let result = format_deps_header("neovim", true, false);
        assert!(result.contains("Dependencies for"));
        assert!(result.contains("neovim"));
        assert!(result.contains("(tree view)"));
    }

    #[test]
    fn test_format_deps_header_all_flag() {
        let result = format_deps_header("python@3.11", false, true);
        assert!(result.contains("Dependencies for"));
        assert!(result.contains("python@3.11"));
        assert!(result.contains("(all)"));
    }

    #[test]
    fn test_format_deps_header_tree_takes_precedence() {
        // Tree view should be shown even if all is also true
        let result = format_deps_header("curl", true, true);
        assert!(result.contains("(tree view)"));
        // Note: tree takes precedence over all in the actual formatting
    }

    // ========================================================================
    // No Dependencies Message Tests
    // ========================================================================

    #[test]
    fn test_format_no_deps_message_default() {
        let result = format_no_deps_message("ripgrep", false);
        assert!(result.contains("ripgrep"));
        assert!(result.contains("has no"));
        assert!(result.contains("dependencies"));
        assert!(!result.contains("installed"));
    }

    #[test]
    fn test_format_no_deps_message_installed_only() {
        let result = format_no_deps_message("fd", true);
        assert!(result.contains("fd"));
        assert!(result.contains("has no"));
        assert!(result.contains("installed"));
        assert!(result.contains("dependencies"));
    }

    #[test]
    fn test_format_no_deps_message_versioned_formula() {
        let result = format_no_deps_message("node@20", false);
        assert!(result.contains("node@20"));
    }

    // ========================================================================
    // Dependency Line Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_dep_line_installed() {
        let result = format_dep_line("openssl", true);
        assert!(result.contains("openssl"));
        assert!(result.contains("✓"));
    }

    #[test]
    fn test_format_dep_line_not_installed() {
        let result = format_dep_line("libpng", false);
        assert!(result.contains("libpng"));
        assert!(result.contains("✗"));
    }

    #[test]
    fn test_format_dep_line_indentation() {
        let result = format_dep_line("zlib", true);
        assert!(result.starts_with("  ")); // Two space indent
    }

    // ========================================================================
    // Plain Dependencies List Tests
    // ========================================================================

    #[test]
    fn test_format_deps_plain_empty() {
        let deps: Vec<String> = vec![];
        let result = format_deps_plain(&deps);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_deps_plain_single() {
        let deps = vec!["openssl".to_string()];
        let result = format_deps_plain(&deps);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "  openssl");
    }

    #[test]
    fn test_format_deps_plain_multiple() {
        let deps = vec![
            "openssl".to_string(),
            "readline".to_string(),
            "zlib".to_string(),
        ];
        let result = format_deps_plain(&deps);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "  openssl");
        assert_eq!(result[1], "  readline");
        assert_eq!(result[2], "  zlib");
    }

    #[test]
    fn test_format_deps_plain_preserves_order() {
        let deps = vec!["z".to_string(), "a".to_string(), "m".to_string()];
        let result = format_deps_plain(&deps);
        assert_eq!(result[0], "  z");
        assert_eq!(result[1], "  a");
        assert_eq!(result[2], "  m");
    }

    // ========================================================================
    // Uses Header Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_uses_header() {
        let result = format_uses_header("openssl");
        assert!(result.contains("Checking what uses"));
        assert!(result.contains("openssl"));
    }

    #[test]
    fn test_format_no_uses_message() {
        let result = format_no_uses_message("zlib");
        assert!(result.contains("No installed formulas use"));
        assert!(result.contains("zlib"));
    }

    #[test]
    fn test_format_uses_result_header_non_recursive() {
        let result = format_uses_result_header("readline", 5, false);
        assert!(result.contains("5"));
        assert!(result.contains("installed formulas use"));
        assert!(result.contains("readline"));
        assert!(!result.contains("directly or indirectly"));
    }

    #[test]
    fn test_format_uses_result_header_recursive() {
        let result = format_uses_result_header("ncurses", 12, true);
        assert!(result.contains("12"));
        assert!(result.contains("ncurses"));
        assert!(result.contains("(directly or indirectly)"));
    }

    #[test]
    fn test_format_uses_result_header_single() {
        let result = format_uses_result_header("libffi", 1, false);
        assert!(result.contains("1"));
        // Count is still shown as-is (no plural handling)
    }

    // ========================================================================
    // Leaves Header Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_leaves_header() {
        let result = format_leaves_header();
        assert!(result.contains("Finding leaf packages"));
    }

    #[test]
    fn test_format_leaves_result_header() {
        let result = format_leaves_result_header(7);
        assert!(result.contains("7"));
        assert!(result.contains("leaf packages"));
        assert!(result.contains("not dependencies of other installed packages"));
    }

    #[test]
    fn test_format_leaves_result_header_zero() {
        let result = format_leaves_result_header(0);
        assert!(result.contains("0"));
    }

    #[test]
    fn test_format_leaves_result_header_large_count() {
        let result = format_leaves_result_header(100);
        assert!(result.contains("100"));
    }
}
