//! Install command implementation.

use console::style;
use indicatif::MultiProgress;
use std::path::Path;
use std::time::Instant;

use zb_core::formula::KegOnlyReason;
use zb_io::install::Installer;

use crate::display::{
    create_progress_callback, finish_progress_bars, suggest_homebrew, ProgressStyles,
};

/// Run the install command.
pub async fn run(
    installer: &mut Installer,
    prefix: &Path,
    formula: String,
    no_link: bool,
    build_from_source: bool,
    head: bool,
) -> Result<(), zb_core::Error> {
    let start = Instant::now();

    // HEAD implies building from source
    let build_from_source = build_from_source || head;

    if build_from_source {
        run_source_install(installer, prefix, &formula, no_link, head, start).await
    } else {
        run_bottle_install(installer, prefix, &formula, no_link, start).await
    }
}

async fn run_source_install(
    installer: &mut Installer,
    prefix: &Path,
    formula: &str,
    no_link: bool,
    head: bool,
    start: Instant,
) -> Result<(), zb_core::Error> {
    let build_type = if head { "HEAD" } else { "source" };
    println!(
        "{} Building {} from {}...",
        style("==>").cyan().bold(),
        style(formula).bold(),
        build_type
    );

    println!(
        "{} Downloading source and dependencies...",
        style("==>").cyan().bold()
    );

    let result = match installer
        .install_from_source(formula, !no_link, head)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} Built and installed {} {} ({} files) in {:.2}s",
        style("==>").cyan().bold(),
        style(&result.name).green().bold(),
        style(&result.version).dim(),
        result.files_installed,
        elapsed.as_secs_f64()
    );
    if result.files_linked > 0 {
        println!(
            "    {} Linked {} files",
            style("✓").green(),
            result.files_linked
        );
    }

    // Display keg-only and caveats info if present
    if let Ok(formula_info) = installer.get_formula(formula).await {
        print_keg_only_info(
            formula_info.keg_only,
            formula_info.keg_only_reason.as_ref(),
            prefix,
            formula,
        );
        print_caveats(formula_info.caveats.as_ref(), prefix);
    }

    Ok(())
}

async fn run_bottle_install(
    installer: &mut Installer,
    prefix: &Path,
    formula: &str,
    no_link: bool,
    start: Instant,
) -> Result<(), zb_core::Error> {
    println!(
        "{} Installing {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    let plan = match installer.plan(formula).await {
        Ok(p) => p,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    // Extract info from the root formula before executing the plan
    let root_formula = plan.formulas.iter().find(|f| f.name == plan.root_name);
    let root_caveats = root_formula.and_then(|f| f.caveats.clone());
    let root_keg_only = root_formula.map(|f| f.keg_only).unwrap_or(false);
    let root_keg_only_reason = root_formula.and_then(|f| f.keg_only_reason.clone());

    println!(
        "{} Resolving dependencies ({} packages)...",
        style("==>").cyan().bold(),
        plan.formulas.len()
    );
    for f in &plan.formulas {
        println!(
            "    {} {}",
            style(&f.name).green(),
            style(&f.versions.stable).dim()
        );
    }

    println!(
        "{} Downloading and installing...",
        style("==>").cyan().bold()
    );

    let multi = MultiProgress::new();
    let styles = ProgressStyles::default();
    let (progress_callback, bars) = create_progress_callback(multi, styles, "installed");

    let result = match installer
        .execute_with_progress(plan, !no_link, Some(progress_callback))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    finish_progress_bars(&bars);

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} Installed {} packages in {:.2}s",
        style("==>").cyan().bold(),
        style(result.installed).green().bold(),
        elapsed.as_secs_f64()
    );

    // Display keg-only and caveats info if present
    print_keg_only_info(root_keg_only, root_keg_only_reason.as_ref(), prefix, formula);
    print_caveats(root_caveats.as_ref(), prefix);

    Ok(())
}

/// Print keg-only information for a formula.
fn print_keg_only_info(
    keg_only: bool,
    keg_only_reason: Option<&KegOnlyReason>,
    prefix: &Path,
    formula: &str,
) {
    if !keg_only {
        return;
    }

    println!();
    println!("{}", style("==> Keg-only").yellow().bold());
    println!(
        "{} is keg-only, which means it was not symlinked into {}",
        style(formula).bold(),
        prefix.display()
    );
    if let Some(reason) = keg_only_reason
        && !reason.explanation.is_empty()
    {
        println!();
        println!("{}", reason.explanation);
    }
    println!();
    println!("To use this formula, you can:");
    println!(
        "    • Add it to your PATH: {}",
        style(format!(
            "export PATH=\"{}/opt/{}/bin:$PATH\"",
            prefix.display(),
            formula
        ))
        .cyan()
    );
    println!(
        "    • Link it with: {}",
        style(format!("zb link {} --force", formula)).cyan()
    );
}

/// Print caveats for a formula.
fn print_caveats(caveats: Option<&String>, prefix: &Path) {
    let Some(caveats) = caveats else { return };

    println!();
    println!("{}", style("==> Caveats").yellow().bold());
    let caveats = caveats.replace("$HOMEBREW_PREFIX", &prefix.to_string_lossy());
    for line in caveats.lines() {
        println!("{}", line);
    }
}

/// Substitute $HOMEBREW_PREFIX in caveats text.
/// Extracted for testability.
pub(crate) fn substitute_prefix(text: &str, prefix: &Path) -> String {
    text.replace("$HOMEBREW_PREFIX", &prefix.to_string_lossy())
}

/// Build keg-only PATH suggestion.
/// Extracted for testability.
pub(crate) fn build_keg_only_path_suggestion(prefix: &Path, formula: &str) -> String {
    format!(
        "export PATH=\"{}/opt/{}/bin:$PATH\"",
        prefix.display(),
        formula
    )
}

/// Build keg-only link suggestion.
/// Extracted for testability.
pub(crate) fn build_keg_only_link_suggestion(formula: &str) -> String {
    format!("zb link {} --force", formula)
}

/// Determine if we should build from source based on flags.
/// Extracted for testability.
pub(crate) fn should_build_from_source(build_from_source: bool, head: bool) -> bool {
    build_from_source || head
}

/// Get the build type label for display.
/// Extracted for testability.
pub(crate) fn get_build_type_label(head: bool) -> &'static str {
    if head { "HEAD" } else { "source" }
}

/// Format the install completion message.
/// Extracted for testability.
pub(crate) fn format_install_complete_message(
    name: &str,
    version: &str,
    files_installed: usize,
    elapsed_secs: f64,
) -> String {
    format!(
        "Built and installed {} {} ({} files) in {:.2}s",
        name, version, files_installed, elapsed_secs
    )
}

/// Format files linked message.
/// Extracted for testability.
pub(crate) fn format_files_linked_message(count: usize) -> String {
    format!("Linked {} files", count)
}

/// Format bottle install summary.
/// Extracted for testability.
pub(crate) fn format_bottle_install_summary(package_count: usize, elapsed_secs: f64) -> String {
    format!("Installed {} packages in {:.2}s", package_count, elapsed_secs)
}

/// Format dependency resolution message.
/// Extracted for testability.
pub(crate) fn format_dependency_resolution(count: usize) -> String {
    format!("Resolving dependencies ({} packages)...", count)
}

/// Validate formula name is not empty.
/// Extracted for testability.
pub(crate) fn validate_formula_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("Formula name cannot be empty".to_string())
    } else if name.starts_with('-') {
        Err("Formula name cannot start with a dash".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ========================================================================
    // Prefix Substitution Tests
    // ========================================================================

    #[test]
    fn test_substitute_prefix_basic() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let text = "Add $HOMEBREW_PREFIX/bin to your PATH";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "Add /opt/zerobrew/prefix/bin to your PATH");
    }

    #[test]
    fn test_substitute_prefix_multiple_occurrences() {
        let prefix = PathBuf::from("/usr/local");
        let text = "$HOMEBREW_PREFIX/bin and $HOMEBREW_PREFIX/sbin";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "/usr/local/bin and /usr/local/sbin");
    }

    #[test]
    fn test_substitute_prefix_no_placeholder() {
        let prefix = PathBuf::from("/opt/zerobrew");
        let text = "No placeholder here";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "No placeholder here");
    }

    #[test]
    fn test_substitute_prefix_empty_string() {
        let prefix = PathBuf::from("/opt/zerobrew");
        let text = "";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "");
    }

    #[test]
    fn test_substitute_prefix_at_start() {
        let prefix = PathBuf::from("/home/brew");
        let text = "$HOMEBREW_PREFIX is the prefix";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "/home/brew is the prefix");
    }

    #[test]
    fn test_substitute_prefix_at_end() {
        let prefix = PathBuf::from("/opt/zb");
        let text = "Prefix is $HOMEBREW_PREFIX";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "Prefix is /opt/zb");
    }

    #[test]
    fn test_substitute_prefix_multiline() {
        let prefix = PathBuf::from("/opt/brew");
        let text = "Line 1: $HOMEBREW_PREFIX/bin\nLine 2: $HOMEBREW_PREFIX/lib";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "Line 1: /opt/brew/bin\nLine 2: /opt/brew/lib");
    }

    // ========================================================================
    // Keg-Only Path Suggestion Tests
    // ========================================================================

    #[test]
    fn test_build_keg_only_path_suggestion() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let result = build_keg_only_path_suggestion(&prefix, "openssl@3");
        assert_eq!(
            result,
            "export PATH=\"/opt/zerobrew/prefix/opt/openssl@3/bin:$PATH\""
        );
    }

    #[test]
    fn test_build_keg_only_path_suggestion_versioned_formula() {
        let prefix = PathBuf::from("/usr/local");
        let result = build_keg_only_path_suggestion(&prefix, "python@3.11");
        assert_eq!(
            result,
            "export PATH=\"/usr/local/opt/python@3.11/bin:$PATH\""
        );
    }

    #[test]
    fn test_build_keg_only_path_suggestion_simple_name() {
        let prefix = PathBuf::from("/home/linuxbrew/.linuxbrew");
        let result = build_keg_only_path_suggestion(&prefix, "readline");
        assert_eq!(
            result,
            "export PATH=\"/home/linuxbrew/.linuxbrew/opt/readline/bin:$PATH\""
        );
    }

    #[test]
    fn test_build_keg_only_path_suggestion_contains_export() {
        let prefix = PathBuf::from("/opt/zb");
        let result = build_keg_only_path_suggestion(&prefix, "curl");
        assert!(result.starts_with("export PATH="));
        assert!(result.contains(":$PATH"));
    }

    // ========================================================================
    // Keg-Only Link Suggestion Tests
    // ========================================================================

    #[test]
    fn test_build_keg_only_link_suggestion() {
        let result = build_keg_only_link_suggestion("openssl@3");
        assert_eq!(result, "zb link openssl@3 --force");
    }

    #[test]
    fn test_build_keg_only_link_suggestion_simple_formula() {
        let result = build_keg_only_link_suggestion("readline");
        assert_eq!(result, "zb link readline --force");
    }

    #[test]
    fn test_build_keg_only_link_suggestion_complex_name() {
        let result = build_keg_only_link_suggestion("llvm@17");
        assert_eq!(result, "zb link llvm@17 --force");
    }

    #[test]
    fn test_build_keg_only_link_suggestion_has_force_flag() {
        let result = build_keg_only_link_suggestion("ncurses");
        assert!(result.contains("--force"));
        assert!(result.starts_with("zb link"));
    }

    // ========================================================================
    // Build Source Logic Tests
    // ========================================================================

    #[test]
    fn test_should_build_from_source_both_false() {
        assert!(!should_build_from_source(false, false));
    }

    #[test]
    fn test_should_build_from_source_source_true() {
        assert!(should_build_from_source(true, false));
    }

    #[test]
    fn test_should_build_from_source_head_true() {
        assert!(should_build_from_source(false, true));
    }

    #[test]
    fn test_should_build_from_source_both_true() {
        assert!(should_build_from_source(true, true));
    }

    #[test]
    fn test_get_build_type_label_head() {
        assert_eq!(get_build_type_label(true), "HEAD");
    }

    #[test]
    fn test_get_build_type_label_source() {
        assert_eq!(get_build_type_label(false), "source");
    }

    // ========================================================================
    // Install Message Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_install_complete_message() {
        let result = format_install_complete_message("git", "2.44.0", 150, 5.5);
        assert_eq!(result, "Built and installed git 2.44.0 (150 files) in 5.50s");
    }

    #[test]
    fn test_format_install_complete_message_zero_files() {
        let result = format_install_complete_message("empty-pkg", "1.0", 0, 0.1);
        assert_eq!(result, "Built and installed empty-pkg 1.0 (0 files) in 0.10s");
    }

    #[test]
    fn test_format_install_complete_message_many_files() {
        let result = format_install_complete_message("neovim", "0.10.0", 2500, 120.5);
        assert!(result.contains("2500 files"));
        assert!(result.contains("120.50s"));
    }

    #[test]
    fn test_format_files_linked_message() {
        assert_eq!(format_files_linked_message(42), "Linked 42 files");
    }

    #[test]
    fn test_format_files_linked_message_one() {
        assert_eq!(format_files_linked_message(1), "Linked 1 files");
    }

    #[test]
    fn test_format_files_linked_message_zero() {
        assert_eq!(format_files_linked_message(0), "Linked 0 files");
    }

    #[test]
    fn test_format_bottle_install_summary() {
        let result = format_bottle_install_summary(5, 12.34);
        assert_eq!(result, "Installed 5 packages in 12.34s");
    }

    #[test]
    fn test_format_bottle_install_summary_single() {
        let result = format_bottle_install_summary(1, 2.0);
        assert_eq!(result, "Installed 1 packages in 2.00s");
    }

    #[test]
    fn test_format_dependency_resolution() {
        let result = format_dependency_resolution(7);
        assert_eq!(result, "Resolving dependencies (7 packages)...");
    }

    #[test]
    fn test_format_dependency_resolution_single() {
        let result = format_dependency_resolution(1);
        assert_eq!(result, "Resolving dependencies (1 packages)...");
    }

    // ========================================================================
    // Formula Name Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_formula_name_valid() {
        assert!(validate_formula_name("git").is_ok());
    }

    #[test]
    fn test_validate_formula_name_versioned() {
        assert!(validate_formula_name("python@3.11").is_ok());
    }

    #[test]
    fn test_validate_formula_name_with_dash() {
        assert!(validate_formula_name("lib-png").is_ok());
    }

    #[test]
    fn test_validate_formula_name_empty() {
        let result = validate_formula_name("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_validate_formula_name_starts_with_dash() {
        let result = validate_formula_name("-git");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("dash"));
    }

    #[test]
    fn test_validate_formula_name_double_dash() {
        // Valid: dash in middle is okay
        assert!(validate_formula_name("a--b").is_ok());
    }
}
