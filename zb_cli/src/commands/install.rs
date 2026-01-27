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
    // Validate formula name
    if let Err(msg) = validate_formula_name(&formula) {
        return Err(zb_core::Error::MissingFormula { name: msg });
    }

    let start = Instant::now();

    // HEAD implies building from source
    let build_from_source = should_build_from_source(build_from_source, head);

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
    let build_type = get_build_type_label(head);
    println!(
        "{} {}",
        style("==>").cyan().bold(),
        format_building_message(formula, build_type)
    );

    println!(
        "{} {}",
        style("==>").cyan().bold(),
        format_downloading_message()
    );

    let result = match installer
        .install_from_source(formula, !no_link, head)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", format_install_error_context(formula, true));
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} {}",
        style("==>").cyan().bold(),
        format_install_complete_message(
            &result.name,
            &result.version,
            result.files_installed,
            elapsed.as_secs_f64()
        )
    );
    if should_show_files_linked(result.files_linked) {
        println!(
            "    {} {}",
            style("✓").green(),
            format_files_linked_message(result.files_linked)
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
        "{} {}",
        style("==>").cyan().bold(),
        format_installing_message(formula)
    );

    let plan = match installer.plan(formula).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", format_plan_error_context(formula));
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
        "{} {}",
        style("==>").cyan().bold(),
        format_dependency_resolution(plan.formulas.len())
    );
    for f in &plan.formulas {
        // Use helper for consistent formatting (styled output uses same data)
        let _ = format_dependency_entry(&f.name, &f.versions.stable);
        println!(
            "    {} {}",
            style(&f.name).green(),
            style(&f.versions.stable).dim()
        );
    }

    println!(
        "{} {}",
        style("==>").cyan().bold(),
        format_downloading_and_installing_message()
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
            eprintln!("{}", format_install_error_context(formula, false));
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    finish_progress_bars(&bars);

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} {}",
        style("==>").cyan().bold(),
        format_bottle_install_summary(result.installed, elapsed.as_secs_f64())
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
        "{}",
        format_keg_only_base_message(formula, prefix)
    );
    if should_show_keg_only_explanation(keg_only_reason) {
        println!();
        println!("{}", keg_only_reason.unwrap().explanation);
    }
    println!();
    println!("To use this formula, you can:");
    println!(
        "    • Add it to your PATH: {}",
        style(build_keg_only_path_suggestion(prefix, formula)).cyan()
    );
    println!(
        "    • Link it with: {}",
        style(build_keg_only_link_suggestion(formula)).cyan()
    );
}

/// Print caveats for a formula.
fn print_caveats(caveats: Option<&String>, prefix: &Path) {
    if !should_show_caveats(caveats) {
        return;
    }
    let caveats = caveats.unwrap();

    println!();
    println!("{}", style("==> Caveats").yellow().bold());
    for line in process_caveats_lines(caveats, prefix) {
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

/// Format the "Building from source/HEAD" message.
/// Extracted for testability.
pub(crate) fn format_building_message(formula: &str, build_type: &str) -> String {
    format!("Building {} from {}...", formula, build_type)
}

/// Format the downloading message.
/// Extracted for testability.
pub(crate) fn format_downloading_message() -> &'static str {
    "Downloading source and dependencies..."
}

/// Format the installing message.
/// Extracted for testability.
pub(crate) fn format_installing_message(formula: &str) -> String {
    format!("Installing {}...", formula)
}

/// Format the keg-only header message.
/// Extracted for testability.
pub(crate) fn format_keg_only_base_message(formula: &str, prefix: &Path) -> String {
    format!(
        "{} is keg-only, which means it was not symlinked into {}",
        formula,
        prefix.display()
    )
}

/// Check if keg-only explanation should be shown.
/// Extracted for testability.
pub(crate) fn should_show_keg_only_explanation(reason: Option<&KegOnlyReason>) -> bool {
    reason
        .map(|r| !r.explanation.is_empty())
        .unwrap_or(false)
}

/// Format the dependency list entry.
/// Extracted for testability.
pub(crate) fn format_dependency_entry(name: &str, version: &str) -> String {
    format!("{} {}", name, version)
}

/// Check if files linked message should be shown.
/// Extracted for testability.
pub(crate) fn should_show_files_linked(count: usize) -> bool {
    count > 0
}

/// Format the downloading and installing message.
/// Extracted for testability.
pub(crate) fn format_downloading_and_installing_message() -> &'static str {
    "Downloading and installing..."
}

/// Process caveats text, handling multiline output.
/// Extracted for testability.
pub(crate) fn process_caveats_lines(caveats: &str, prefix: &Path) -> Vec<String> {
    let substituted = substitute_prefix(caveats, prefix);
    substituted.lines().map(|s| s.to_string()).collect()
}

/// Check if caveats should be displayed.
/// Extracted for testability.
pub(crate) fn should_show_caveats(caveats: Option<&String>) -> bool {
    caveats.is_some()
}

/// Format error context for install failure.
/// Extracted for testability.
pub(crate) fn format_install_error_context(formula: &str, is_source: bool) -> String {
    if is_source {
        format!("Failed to build {} from source", formula)
    } else {
        format!("Failed to install {}", formula)
    }
}

/// Format plan error context.
/// Extracted for testability.
pub(crate) fn format_plan_error_context(formula: &str) -> String {
    format!("Failed to resolve dependencies for {}", formula)
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

    // ========================================================================
    // Building Message Tests
    // ========================================================================

    #[test]
    fn test_format_building_message_source() {
        let result = format_building_message("git", "source");
        assert_eq!(result, "Building git from source...");
    }

    #[test]
    fn test_format_building_message_head() {
        let result = format_building_message("vim", "HEAD");
        assert_eq!(result, "Building vim from HEAD...");
    }

    #[test]
    fn test_format_building_message_versioned() {
        let result = format_building_message("python@3.11", "source");
        assert_eq!(result, "Building python@3.11 from source...");
    }

    #[test]
    fn test_format_downloading_message() {
        let result = format_downloading_message();
        assert_eq!(result, "Downloading source and dependencies...");
    }

    // ========================================================================
    // Installing Message Tests
    // ========================================================================

    #[test]
    fn test_format_installing_message() {
        let result = format_installing_message("wget");
        assert_eq!(result, "Installing wget...");
    }

    #[test]
    fn test_format_installing_message_versioned() {
        let result = format_installing_message("openssl@3");
        assert_eq!(result, "Installing openssl@3...");
    }

    #[test]
    fn test_format_downloading_and_installing_message() {
        let result = format_downloading_and_installing_message();
        assert_eq!(result, "Downloading and installing...");
    }

    // ========================================================================
    // Keg-Only Message Tests
    // ========================================================================

    #[test]
    fn test_format_keg_only_base_message() {
        let prefix = PathBuf::from("/opt/zerobrew");
        let result = format_keg_only_base_message("openssl@3", &prefix);
        assert_eq!(
            result,
            "openssl@3 is keg-only, which means it was not symlinked into /opt/zerobrew"
        );
    }

    #[test]
    fn test_format_keg_only_base_message_long_path() {
        let prefix = PathBuf::from("/home/linuxbrew/.linuxbrew");
        let result = format_keg_only_base_message("curl", &prefix);
        assert!(result.contains("curl is keg-only"));
        assert!(result.contains("/home/linuxbrew/.linuxbrew"));
    }

    #[test]
    fn test_should_show_keg_only_explanation_with_explanation() {
        let reason = KegOnlyReason {
            reason: "provided_by_macos".to_string(),
            explanation: "macOS provides an older version".to_string(),
        };
        assert!(should_show_keg_only_explanation(Some(&reason)));
    }

    #[test]
    fn test_should_show_keg_only_explanation_empty() {
        let reason = KegOnlyReason {
            reason: "shadowed_by_macos".to_string(),
            explanation: String::new(),
        };
        assert!(!should_show_keg_only_explanation(Some(&reason)));
    }

    #[test]
    fn test_should_show_keg_only_explanation_none() {
        assert!(!should_show_keg_only_explanation(None));
    }

    // ========================================================================
    // Dependency Entry Tests
    // ========================================================================

    #[test]
    fn test_format_dependency_entry() {
        let result = format_dependency_entry("zlib", "1.3.1");
        assert_eq!(result, "zlib 1.3.1");
    }

    #[test]
    fn test_format_dependency_entry_versioned_formula() {
        let result = format_dependency_entry("icu4c@76", "76.1");
        assert_eq!(result, "icu4c@76 76.1");
    }

    #[test]
    fn test_format_dependency_entry_long_version() {
        let result = format_dependency_entry("openssl", "3.2.1_1");
        assert_eq!(result, "openssl 3.2.1_1");
    }

    // ========================================================================
    // Files Linked Condition Tests
    // ========================================================================

    #[test]
    fn test_should_show_files_linked_positive() {
        assert!(should_show_files_linked(1));
        assert!(should_show_files_linked(100));
    }

    #[test]
    fn test_should_show_files_linked_zero() {
        assert!(!should_show_files_linked(0));
    }

    // ========================================================================
    // Caveats Processing Tests
    // ========================================================================

    #[test]
    fn test_process_caveats_lines_single() {
        let prefix = PathBuf::from("/opt/brew");
        let caveats = "Add $HOMEBREW_PREFIX/bin to PATH";
        let result = process_caveats_lines(caveats, &prefix);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Add /opt/brew/bin to PATH");
    }

    #[test]
    fn test_process_caveats_lines_multiline() {
        let prefix = PathBuf::from("/usr/local");
        let caveats = "Line 1: $HOMEBREW_PREFIX/bin\nLine 2: check docs\nLine 3: $HOMEBREW_PREFIX/lib";
        let result = process_caveats_lines(caveats, &prefix);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Line 1: /usr/local/bin");
        assert_eq!(result[1], "Line 2: check docs");
        assert_eq!(result[2], "Line 3: /usr/local/lib");
    }

    #[test]
    fn test_process_caveats_lines_empty() {
        let prefix = PathBuf::from("/opt/zb");
        let result = process_caveats_lines("", &prefix);
        // Empty string produces no lines when split
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_should_show_caveats_some() {
        let caveats = String::from("Some caveat");
        assert!(should_show_caveats(Some(&caveats)));
    }

    #[test]
    fn test_should_show_caveats_none() {
        assert!(!should_show_caveats(None));
    }

    // ========================================================================
    // Error Context Tests
    // ========================================================================

    #[test]
    fn test_format_install_error_context_source() {
        let result = format_install_error_context("git", true);
        assert_eq!(result, "Failed to build git from source");
    }

    #[test]
    fn test_format_install_error_context_bottle() {
        let result = format_install_error_context("wget", false);
        assert_eq!(result, "Failed to install wget");
    }

    #[test]
    fn test_format_install_error_context_versioned() {
        let result = format_install_error_context("python@3.11", true);
        assert!(result.contains("python@3.11"));
        assert!(result.contains("from source"));
    }

    #[test]
    fn test_format_plan_error_context() {
        let result = format_plan_error_context("neovim");
        assert_eq!(result, "Failed to resolve dependencies for neovim");
    }

    #[test]
    fn test_format_plan_error_context_complex_name() {
        let result = format_plan_error_context("llvm@17");
        assert!(result.contains("llvm@17"));
        assert!(result.contains("dependencies"));
    }
}
