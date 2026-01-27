//! Upgrade and outdated command implementations.

use console::style;
use indicatif::MultiProgress;
use std::time::Instant;

use zb_io::install::Installer;

use crate::display::{ProgressStyles, create_progress_callback, finish_progress_bars};

/// Run the outdated command.
pub async fn run_outdated(installer: &mut Installer, json: bool) -> Result<(), zb_core::Error> {
    if !json {
        println!(
            "{} Checking for outdated packages...",
            style("==>").cyan().bold()
        );
    }

    let outdated = installer.get_outdated().await?;
    let pinned = installer.list_pinned()?;
    let pinned_count = pinned.len();

    let output_kind = determine_outdated_output_kind(json, outdated.len(), pinned_count);

    match output_kind {
        OutdatedOutputKind::Json => {
            let json_output = build_outdated_json(&outdated);
            match serde_json::to_string_pretty(&json_output) {
                Ok(json_str) => println!("{}", json_str),
                Err(e) => {
                    eprintln!(
                        "{} Failed to serialize JSON: {}",
                        style("error:").red().bold(),
                        e
                    );
                    std::process::exit(1);
                }
            }
        }
        OutdatedOutputKind::AllUpToDate { pinned_count } => {
            let (main_msg, pinned_msg) = format_all_up_to_date_message(pinned_count);
            println!("{}", main_msg);
            if let Some(msg) = pinned_msg {
                println!("    {} {}", style("→").dim(), msg);
            }
        }
        OutdatedOutputKind::HasOutdated {
            outdated_count,
            pinned_count,
        } => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                style(format_outdated_header(outdated_count))
                    .yellow()
                    .bold()
            );
            println!();

            let sorted = sort_outdated_packages(outdated);
            for pkg in &sorted {
                println!(
                    "  {} {} → {}",
                    style(&pkg.name).bold(),
                    style(&pkg.installed_version).red(),
                    style(&pkg.available_version).green()
                );
            }

            println!();
            println!(
                "    {} {}",
                style("→").cyan(),
                style(format_upgrade_suggestion()).cyan()
            );
            if pinned_count > 0 {
                println!(
                    "    {} {}",
                    style("→").dim(),
                    format_pinned_footer(pinned_count)
                );
            }
        }
    }

    Ok(())
}

/// Run the upgrade command.
pub async fn run_upgrade(
    installer: &mut Installer,
    formula: Option<String>,
    dry_run: bool,
) -> Result<(), zb_core::Error> {
    let start = Instant::now();

    // Get list of packages to upgrade
    let outdated = installer.get_outdated().await?;
    let to_upgrade = filter_outdated_by_name(outdated, formula.as_deref());

    // Check if formula is installed (for status messages)
    let is_installed = formula
        .as_ref()
        .map(|name| installer.is_installed(name))
        .unwrap_or(true);

    let output_kind =
        determine_upgrade_output_kind(formula.as_deref(), is_installed, to_upgrade.len(), dry_run);

    match output_kind {
        UpgradeOutputKind::NotInstalled { formula } => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                get_upgrade_status_message(Some(&formula), false, false)
            );
            return Ok(());
        }
        UpgradeOutputKind::AlreadyUpToDate { formula } => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                get_upgrade_status_message(Some(&formula), true, false)
            );
            return Ok(());
        }
        UpgradeOutputKind::AllUpToDate => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                get_upgrade_status_message(None, true, false)
            );
            return Ok(());
        }
        UpgradeOutputKind::DryRun { count } => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                style(format_dry_run_header(count)).yellow().bold()
            );
            println!();
            for pkg in &to_upgrade {
                println!(
                    "  {} {} → {}",
                    style(&pkg.name).bold(),
                    style(&pkg.installed_version).red(),
                    style(&pkg.available_version).green()
                );
            }
            return Ok(());
        }
        UpgradeOutputKind::Upgrade { count } => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                style(format_upgrade_header(count)).yellow().bold()
            );
        }
    }

    let multi = MultiProgress::new();
    let styles = ProgressStyles::default();
    let (progress_callback, bars) = create_progress_callback(multi, styles, "upgraded");

    // Perform the upgrades using UpgradeSummary to track results
    let mut summary = UpgradeSummary::new();
    for pkg in &to_upgrade {
        println!();
        println!(
            "{} {}",
            style("==>").cyan().bold(),
            format_upgrade_announcement(&pkg.name, &pkg.installed_version, &pkg.available_version)
        );

        match installer
            .upgrade_one(&pkg.name, true, Some(progress_callback.clone()))
            .await
        {
            Ok(Some((old_ver, new_ver))) => {
                summary.record_success(pkg.name.clone(), old_ver, new_ver);
            }
            Ok(None) => {
                println!(
                    "    {} {}",
                    style("✓").green(),
                    format_upgrade_success(&pkg.name)
                );
                summary.record_up_to_date(pkg.name.clone());
            }
            Err(e) => {
                eprintln!(
                    "    {} {}",
                    style("✗").red(),
                    format_upgrade_failure(&pkg.name, &e.to_string())
                );
                summary.record_failure(pkg.name.clone(), e.to_string());
            }
        }
    }

    finish_progress_bars(&bars);

    let elapsed = start.elapsed();
    println!();
    if !summary.has_upgrades() {
        println!(
            "{} {}",
            style("==>").cyan().bold(),
            format_no_upgrades_message()
        );
    } else {
        println!(
            "{} {}",
            style("==>").cyan().bold(),
            style(format_upgrade_summary(
                summary.upgraded_count(),
                elapsed.as_secs_f64()
            ))
            .green()
            .bold()
        );
        for (name, old_ver, new_ver) in &summary.upgraded {
            println!(
                "    {} {}",
                style("✓").green(),
                format_upgraded_package(name, old_ver, new_ver)
            );
        }
    }

    Ok(())
}

/// Run the pin command.
pub fn run_pin(installer: &mut Installer, formula: &str) -> Result<(), zb_core::Error> {
    if !is_valid_formula_name(formula) {
        eprintln!(
            "{} Invalid formula name: {}",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    match installer.pin(formula) {
        Ok(true) => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                format_pin_status_message(formula, true)
            );
        }
        Ok(false) => {
            println!("{}", format_not_installed_error(formula));
        }
        Err(zb_core::Error::NotInstalled { .. }) => {
            println!("{}", format_not_installed_error(formula));
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Run the unpin command.
pub fn run_unpin(installer: &mut Installer, formula: &str) -> Result<(), zb_core::Error> {
    if !is_valid_formula_name(formula) {
        eprintln!(
            "{} Invalid formula name: {}",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    match installer.unpin(formula) {
        Ok(true) => {
            println!(
                "{} {}",
                style("==>").cyan().bold(),
                format_pin_status_message(formula, false)
            );
        }
        Ok(false) => {
            println!("{}", format_not_installed_error(formula));
        }
        Err(zb_core::Error::NotInstalled { .. }) => {
            println!("{}", format_not_installed_error(formula));
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Filter outdated packages by name.
/// Returns packages matching the given name, or all packages if name is None.
/// Extracted for testability.
pub(crate) fn filter_outdated_by_name(
    outdated: Vec<zb_core::version::OutdatedPackage>,
    name: Option<&str>,
) -> Vec<zb_core::version::OutdatedPackage> {
    match name {
        Some(n) => outdated.into_iter().filter(|p| p.name == n).collect(),
        None => outdated,
    }
}

/// Format an outdated package as a version transition string.
/// Extracted for testability. Used in tests and available for logging/API output.
#[allow(dead_code)]
pub(crate) fn format_version_transition(
    name: &str,
    old_version: &str,
    new_version: &str,
) -> String {
    format!("{}: {} → {}", name, old_version, new_version)
}

/// Build JSON output for outdated packages.
/// Extracted for testability.
pub(crate) fn build_outdated_json(
    outdated: &[zb_core::version::OutdatedPackage],
) -> Vec<serde_json::Value> {
    outdated
        .iter()
        .map(|pkg| {
            serde_json::json!({
                "name": pkg.name,
                "installed_version": pkg.installed_version,
                "available_version": pkg.available_version
            })
        })
        .collect()
}

/// Format the dry-run header message.
/// Extracted for testability.
pub(crate) fn format_dry_run_header(count: usize) -> String {
    format!("Would upgrade {} packages:", count)
}

/// Format the upgrade header message.
/// Extracted for testability.
pub(crate) fn format_upgrade_header(count: usize) -> String {
    format!("Upgrading {} packages...", count)
}

/// Format the upgrade summary message.
/// Extracted for testability.
pub(crate) fn format_upgrade_summary(count: usize, elapsed_secs: f64) -> String {
    format!("Upgraded {} packages in {:.2}s:", count, elapsed_secs)
}

/// Format the pinned packages notice.
/// Extracted for testability.
pub(crate) fn format_pinned_notice(pinned_count: usize) -> String {
    format!("{} pinned packages not checked", pinned_count)
}

/// Format the upgrade all suggestion.
/// Extracted for testability.
pub(crate) fn format_upgrade_suggestion() -> String {
    "Run zb upgrade to upgrade all".to_string()
}

/// Format a single upgraded package line.
/// Extracted for testability.
pub(crate) fn format_upgraded_package(name: &str, old_version: &str, new_version: &str) -> String {
    format!("{} {} → {}", name, old_version, new_version)
}

/// Determine the status message for upgrade command based on formula state.
/// Extracted for testability.
pub(crate) fn get_upgrade_status_message(
    formula: Option<&str>,
    is_installed: bool,
    is_outdated: bool,
) -> String {
    match formula {
        Some(name) if !is_installed => {
            format!("{} is not installed.", name)
        }
        Some(name) if !is_outdated => {
            format!("{} is already up to date.", name)
        }
        Some(_) => String::new(), // Has updates, no status message needed
        None => "All packages are up to date.".to_string(),
    }
}

/// Format pinned message for list command.
/// Extracted for testability.
pub(crate) fn format_pin_status_message(formula: &str, pinned: bool) -> String {
    if pinned {
        format!("Pinned {} - it will not be upgraded", formula)
    } else {
        format!("Unpinned {} - it will be upgraded when outdated", formula)
    }
}

/// Check if any packages need upgrading.
/// Extracted for testability. Used in tests and available for programmatic checks.
#[allow(dead_code)]
pub(crate) fn has_upgrades(outdated: &[zb_core::version::OutdatedPackage]) -> bool {
    !outdated.is_empty()
}

// ============================================================================
// Additional pure functions for improved testability
// ============================================================================

/// Format the outdated count header message.
/// Extracted for testability.
pub(crate) fn format_outdated_header(count: usize) -> String {
    format!("{} outdated packages:", count)
}

/// Format the "all up to date" message with optional pinned notice.
/// Extracted for testability.
pub(crate) fn format_all_up_to_date_message(pinned_count: usize) -> (String, Option<String>) {
    let main_msg = "All packages are up to date.".to_string();
    let pinned_msg = if pinned_count > 0 {
        Some(format_pinned_notice(pinned_count))
    } else {
        None
    };
    (main_msg, pinned_msg)
}

/// Format a package upgrade announcement line.
/// Extracted for testability.
pub(crate) fn format_upgrade_announcement(
    name: &str,
    old_version: &str,
    new_version: &str,
) -> String {
    format!("Upgrading {} {} → {}...", name, old_version, new_version)
}

/// Format upgrade success message for a single package.
/// Extracted for testability.
pub(crate) fn format_upgrade_success(name: &str) -> String {
    format!("{} is already up to date", name)
}

/// Format upgrade failure message for a single package.
/// Extracted for testability.
pub(crate) fn format_upgrade_failure(name: &str, error: &str) -> String {
    format!("Failed to upgrade {}: {}", name, error)
}

/// Format the "no packages upgraded" message.
/// Extracted for testability.
pub(crate) fn format_no_upgrades_message() -> String {
    "No packages were upgraded.".to_string()
}

/// Format a single outdated package display line.
/// Extracted for testability. Used in tests and available for plain-text output.
#[allow(dead_code)]
pub(crate) fn format_outdated_package_line(name: &str, installed: &str, available: &str) -> String {
    format!("{} {} → {}", name, installed, available)
}

/// Format the pinned packages footer for outdated command.
/// Extracted for testability.
pub(crate) fn format_pinned_footer(pinned_count: usize) -> String {
    format!(
        "{} pinned packages not shown (use zb list --pinned to see them)",
        pinned_count
    )
}

/// Determine what kind of output to show for outdated command.
/// Returns: (is_json, has_outdated, pinned_count)
/// Extracted for testability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutdatedOutputKind {
    /// JSON output requested
    Json,
    /// No outdated packages, all up to date
    AllUpToDate { pinned_count: usize },
    /// Has outdated packages to display
    HasOutdated {
        outdated_count: usize,
        pinned_count: usize,
    },
}

/// Determine the output kind for the outdated command.
/// Extracted for testability.
pub(crate) fn determine_outdated_output_kind(
    json: bool,
    outdated_count: usize,
    pinned_count: usize,
) -> OutdatedOutputKind {
    if json {
        OutdatedOutputKind::Json
    } else if outdated_count == 0 {
        OutdatedOutputKind::AllUpToDate { pinned_count }
    } else {
        OutdatedOutputKind::HasOutdated {
            outdated_count,
            pinned_count,
        }
    }
}

/// Determine what kind of output to show for upgrade command.
/// Extracted for testability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpgradeOutputKind {
    /// Nothing to upgrade, formula specified but not installed
    NotInstalled { formula: String },
    /// Nothing to upgrade, formula specified but already up to date
    AlreadyUpToDate { formula: String },
    /// Nothing to upgrade, all packages up to date
    AllUpToDate,
    /// Dry run mode - show what would be upgraded
    DryRun { count: usize },
    /// Actual upgrade mode
    Upgrade { count: usize },
}

/// Determine the output kind for the upgrade command.
/// Extracted for testability.
pub(crate) fn determine_upgrade_output_kind(
    formula: Option<&str>,
    is_installed: bool,
    to_upgrade_count: usize,
    dry_run: bool,
) -> UpgradeOutputKind {
    if to_upgrade_count == 0 {
        match formula {
            Some(name) if !is_installed => UpgradeOutputKind::NotInstalled {
                formula: name.to_string(),
            },
            Some(name) => UpgradeOutputKind::AlreadyUpToDate {
                formula: name.to_string(),
            },
            None => UpgradeOutputKind::AllUpToDate,
        }
    } else if dry_run {
        UpgradeOutputKind::DryRun {
            count: to_upgrade_count,
        }
    } else {
        UpgradeOutputKind::Upgrade {
            count: to_upgrade_count,
        }
    }
}

/// Collect upgrade results into a summary structure.
/// Extracted for testability.
#[derive(Debug, Clone, Default)]
pub(crate) struct UpgradeSummary {
    /// Successfully upgraded packages: (name, old_version, new_version)
    pub upgraded: Vec<(String, String, String)>,
    /// Packages that were already up to date
    pub already_up_to_date: Vec<String>,
    /// Failed upgrades: (name, error_message)
    pub failed: Vec<(String, String)>,
}

impl UpgradeSummary {
    /// Create a new empty summary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful upgrade.
    pub fn record_success(&mut self, name: String, old_version: String, new_version: String) {
        self.upgraded.push((name, old_version, new_version));
    }

    /// Record a package that was already up to date.
    pub fn record_up_to_date(&mut self, name: String) {
        self.already_up_to_date.push(name);
    }

    /// Record a failed upgrade.
    pub fn record_failure(&mut self, name: String, error: String) {
        self.failed.push((name, error));
    }

    /// Get the count of successfully upgraded packages.
    pub fn upgraded_count(&self) -> usize {
        self.upgraded.len()
    }

    /// Get the count of failed upgrades.
    #[allow(dead_code)]
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// Check if any packages were upgraded.
    pub fn has_upgrades(&self) -> bool {
        !self.upgraded.is_empty()
    }

    /// Check if any upgrades failed.
    #[allow(dead_code)]
    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }

    /// Get total attempted upgrades.
    #[allow(dead_code)]
    pub fn total_attempted(&self) -> usize {
        self.upgraded.len() + self.already_up_to_date.len() + self.failed.len()
    }
}

/// Format the complete upgrade summary output.
/// Extracted for testability. Used in tests and available for batch output formatting.
#[allow(dead_code)]
pub(crate) fn format_upgrade_summary_output(
    summary: &UpgradeSummary,
    elapsed_secs: f64,
) -> Vec<String> {
    let mut lines = Vec::new();

    if summary.upgraded.is_empty() {
        lines.push(format_no_upgrades_message());
    } else {
        lines.push(format_upgrade_summary(
            summary.upgraded_count(),
            elapsed_secs,
        ));
        for (name, old_ver, new_ver) in &summary.upgraded {
            lines.push(format!(
                "    ✓ {}",
                format_upgraded_package(name, old_ver, new_ver)
            ));
        }
    }

    if !summary.failed.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Failed to upgrade {} packages:",
            summary.failed_count()
        ));
        for (name, error) in &summary.failed {
            lines.push(format!("    ✗ {}: {}", name, error));
        }
    }

    lines
}

/// Format the dry-run output lines.
/// Extracted for testability. Used in tests and available for batch output formatting.
#[allow(dead_code)]
pub(crate) fn format_dry_run_output(packages: &[zb_core::version::OutdatedPackage]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format_dry_run_header(packages.len()));
    lines.push(String::new());
    for pkg in packages {
        lines.push(format!(
            "  {}",
            format_outdated_package_line(&pkg.name, &pkg.installed_version, &pkg.available_version)
        ));
    }
    lines
}

/// Check if a package should be excluded from upgrade due to pinning.
/// In the current implementation, pinned packages are excluded at the Installer level,
/// but this function documents the logic.
/// Extracted for testability. Used in tests and available for custom filtering.
#[allow(dead_code)]
pub(crate) fn should_exclude_pinned(package_name: &str, pinned_packages: &[String]) -> bool {
    pinned_packages.contains(&package_name.to_string())
}

/// Sort outdated packages by name for consistent display.
/// Extracted for testability.
pub(crate) fn sort_outdated_packages(
    mut packages: Vec<zb_core::version::OutdatedPackage>,
) -> Vec<zb_core::version::OutdatedPackage> {
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

/// Group packages by their version change type.
/// Extracted for testability. Used in tests and available for version analysis features.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionChangeType {
    /// Major version bump (e.g., 1.x.x -> 2.x.x)
    Major,
    /// Minor version bump (e.g., 1.1.x -> 1.2.x)
    Minor,
    /// Patch version bump (e.g., 1.1.1 -> 1.1.2)
    Patch,
    /// Unknown or non-semver change
    Unknown,
}

/// Determine the type of version change between two versions.
/// Extracted for testability. Used in tests and available for version analysis features.
#[allow(dead_code)]
pub(crate) fn classify_version_change(old_version: &str, new_version: &str) -> VersionChangeType {
    let old_parts: Vec<&str> = old_version.split('.').collect();
    let new_parts: Vec<&str> = new_version.split('.').collect();

    // Need at least major.minor for comparison
    if old_parts.len() < 2 || new_parts.len() < 2 {
        return VersionChangeType::Unknown;
    }

    // Parse major versions (strip any suffix like _1 or -beta)
    let old_major = old_parts[0]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    let new_major = new_parts[0]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u32>().ok());

    match (old_major, new_major) {
        (Some(old), Some(new)) if new > old => return VersionChangeType::Major,
        (Some(_), Some(_)) => {}
        _ => return VersionChangeType::Unknown,
    }

    // Parse minor versions
    let old_minor = old_parts[1]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    let new_minor = new_parts[1]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse::<u32>().ok());

    match (old_minor, new_minor) {
        (Some(old), Some(new)) if new > old => return VersionChangeType::Minor,
        (Some(_), Some(_)) => {}
        _ => return VersionChangeType::Unknown,
    }

    // Check patch if available
    if old_parts.len() >= 3 && new_parts.len() >= 3 {
        let old_patch = old_parts[2]
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|s| s.parse::<u32>().ok());
        let new_patch = new_parts[2]
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|s| s.parse::<u32>().ok());

        match (old_patch, new_patch) {
            (Some(old), Some(new)) if new > old => return VersionChangeType::Patch,
            (Some(_), Some(_)) => return VersionChangeType::Unknown,
            _ => return VersionChangeType::Unknown,
        }
    }

    VersionChangeType::Unknown
}

/// Count packages by version change type.
/// Extracted for testability. Used in tests and available for version analysis features.
#[allow(dead_code)]
pub(crate) fn count_by_change_type(
    packages: &[zb_core::version::OutdatedPackage],
) -> (usize, usize, usize, usize) {
    let mut major = 0;
    let mut minor = 0;
    let mut patch = 0;
    let mut unknown = 0;

    for pkg in packages {
        match classify_version_change(&pkg.installed_version, &pkg.available_version) {
            VersionChangeType::Major => major += 1,
            VersionChangeType::Minor => minor += 1,
            VersionChangeType::Patch => patch += 1,
            VersionChangeType::Unknown => unknown += 1,
        }
    }

    (major, minor, patch, unknown)
}

/// Validate that a formula name is valid for pinning operations.
/// Extracted for testability.
pub(crate) fn is_valid_formula_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Must start with a letter
    if !name
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_alphabetic())
    {
        return false;
    }
    // Can contain letters, numbers, hyphens, underscores, and @ for versioned formulas
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '@' || c == '.')
}

/// Format the "not installed" error message.
/// Extracted for testability.
pub(crate) fn format_not_installed_error(formula: &str) -> String {
    format!("Formula '{}' is not installed.", formula)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zb_core::version::OutdatedPackage;

    fn make_outdated_pkg(name: &str, installed: &str, available: &str) -> OutdatedPackage {
        OutdatedPackage {
            name: name.to_string(),
            installed_version: installed.to_string(),
            available_version: available.to_string(),
        }
    }

    // ========================================================================
    // Filter Outdated Tests
    // ========================================================================

    #[test]
    fn test_filter_outdated_by_name_with_match() {
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
            make_outdated_pkg("jq", "1.6", "1.7"),
        ];

        let filtered = filter_outdated_by_name(outdated, Some("git"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "git");
    }

    #[test]
    fn test_filter_outdated_by_name_no_match() {
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];

        let filtered = filter_outdated_by_name(outdated, Some("nonexistent"));
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_outdated_by_name_none_returns_all() {
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];

        let filtered = filter_outdated_by_name(outdated, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_outdated_by_name_empty_list() {
        let outdated: Vec<OutdatedPackage> = vec![];
        let filtered = filter_outdated_by_name(outdated, Some("git"));
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_outdated_by_name_multiple_matches() {
        // Only one should match since names are unique
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("git", "2.42.0", "2.43.0"),
        ];
        let filtered = filter_outdated_by_name(outdated, Some("git"));
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_outdated_by_name_case_sensitive() {
        let outdated = vec![make_outdated_pkg("Git", "2.43.0", "2.44.0")];
        let filtered = filter_outdated_by_name(outdated, Some("git"));
        assert!(filtered.is_empty());
    }

    // ========================================================================
    // Version Transition Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_version_transition() {
        let result = format_version_transition("git", "2.43.0", "2.44.0");
        assert_eq!(result, "git: 2.43.0 → 2.44.0");
    }

    #[test]
    fn test_format_version_transition_with_rebuild() {
        let result = format_version_transition("openssl@3", "3.2.0_1", "3.3.0");
        assert_eq!(result, "openssl@3: 3.2.0_1 → 3.3.0");
    }

    #[test]
    fn test_format_version_transition_same_version() {
        let result = format_version_transition("pkg", "1.0.0", "1.0.0");
        assert_eq!(result, "pkg: 1.0.0 → 1.0.0");
    }

    #[test]
    fn test_format_version_transition_long_versions() {
        let result = format_version_transition(
            "complex-package",
            "1.2.3-beta.4+build.567",
            "2.0.0-rc.1+build.890",
        );
        assert!(result.contains("1.2.3-beta.4+build.567"));
        assert!(result.contains("2.0.0-rc.1+build.890"));
    }

    // ========================================================================
    // Outdated JSON Building Tests
    // ========================================================================

    #[test]
    fn test_build_outdated_json_single() {
        let outdated = vec![make_outdated_pkg("git", "2.43.0", "2.44.0")];
        let json = build_outdated_json(&outdated);

        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["name"], "git");
        assert_eq!(json[0]["installed_version"], "2.43.0");
        assert_eq!(json[0]["available_version"], "2.44.0");
    }

    #[test]
    fn test_build_outdated_json_multiple() {
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];
        let json = build_outdated_json(&outdated);

        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["name"], "git");
        assert_eq!(json[1]["name"], "ripgrep");
    }

    #[test]
    fn test_build_outdated_json_empty() {
        let outdated: Vec<OutdatedPackage> = vec![];
        let json = build_outdated_json(&outdated);
        assert!(json.is_empty());
    }

    #[test]
    fn test_build_outdated_json_versioned_formula() {
        let outdated = vec![make_outdated_pkg("python@3.11", "3.11.8", "3.11.9")];
        let json = build_outdated_json(&outdated);

        assert_eq!(json[0]["name"], "python@3.11");
        assert_eq!(json[0]["installed_version"], "3.11.8");
        assert_eq!(json[0]["available_version"], "3.11.9");
    }

    #[test]
    fn test_build_outdated_json_has_required_fields() {
        let outdated = vec![make_outdated_pkg("test", "1.0", "2.0")];
        let json = build_outdated_json(&outdated);

        let obj = json[0].as_object().unwrap();
        assert!(obj.contains_key("name"));
        assert!(obj.contains_key("installed_version"));
        assert!(obj.contains_key("available_version"));
        assert_eq!(obj.len(), 3);
    }

    // ========================================================================
    // Dry Run Header Tests
    // ========================================================================

    #[test]
    fn test_format_dry_run_header_multiple() {
        let result = format_dry_run_header(5);
        assert_eq!(result, "Would upgrade 5 packages:");
    }

    #[test]
    fn test_format_dry_run_header_single() {
        let result = format_dry_run_header(1);
        assert_eq!(result, "Would upgrade 1 packages:");
    }

    #[test]
    fn test_format_dry_run_header_zero() {
        let result = format_dry_run_header(0);
        assert_eq!(result, "Would upgrade 0 packages:");
    }

    // ========================================================================
    // Upgrade Header Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_header_multiple() {
        let result = format_upgrade_header(10);
        assert_eq!(result, "Upgrading 10 packages...");
    }

    #[test]
    fn test_format_upgrade_header_single() {
        let result = format_upgrade_header(1);
        assert_eq!(result, "Upgrading 1 packages...");
    }

    // ========================================================================
    // Upgrade Summary Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_summary() {
        let result = format_upgrade_summary(5, 12.34);
        assert_eq!(result, "Upgraded 5 packages in 12.34s:");
    }

    #[test]
    fn test_format_upgrade_summary_fast() {
        let result = format_upgrade_summary(1, 0.5);
        assert_eq!(result, "Upgraded 1 packages in 0.50s:");
    }

    #[test]
    fn test_format_upgrade_summary_slow() {
        let result = format_upgrade_summary(20, 300.0);
        assert_eq!(result, "Upgraded 20 packages in 300.00s:");
    }

    // ========================================================================
    // Pinned Notice Tests
    // ========================================================================

    #[test]
    fn test_format_pinned_notice_multiple() {
        let result = format_pinned_notice(3);
        assert_eq!(result, "3 pinned packages not checked");
    }

    #[test]
    fn test_format_pinned_notice_single() {
        let result = format_pinned_notice(1);
        assert_eq!(result, "1 pinned packages not checked");
    }

    #[test]
    fn test_format_pinned_notice_zero() {
        let result = format_pinned_notice(0);
        assert_eq!(result, "0 pinned packages not checked");
    }

    // ========================================================================
    // Upgrade Suggestion Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_suggestion() {
        let result = format_upgrade_suggestion();
        assert!(result.contains("zb upgrade"));
    }

    // ========================================================================
    // Upgraded Package Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_upgraded_package() {
        let result = format_upgraded_package("git", "2.43.0", "2.44.0");
        assert_eq!(result, "git 2.43.0 → 2.44.0");
    }

    #[test]
    fn test_format_upgraded_package_versioned() {
        let result = format_upgraded_package("python@3.11", "3.11.8", "3.11.9");
        assert_eq!(result, "python@3.11 3.11.8 → 3.11.9");
    }

    // ========================================================================
    // Upgrade Status Message Tests
    // ========================================================================

    #[test]
    fn test_get_upgrade_status_message_not_installed() {
        let result = get_upgrade_status_message(Some("git"), false, false);
        assert_eq!(result, "git is not installed.");
    }

    #[test]
    fn test_get_upgrade_status_message_up_to_date() {
        let result = get_upgrade_status_message(Some("git"), true, false);
        assert_eq!(result, "git is already up to date.");
    }

    #[test]
    fn test_get_upgrade_status_message_has_updates() {
        let result = get_upgrade_status_message(Some("git"), true, true);
        assert_eq!(result, "");
    }

    #[test]
    fn test_get_upgrade_status_message_all_up_to_date() {
        let result = get_upgrade_status_message(None, true, false);
        assert_eq!(result, "All packages are up to date.");
    }

    // ========================================================================
    // Pin Status Message Tests
    // ========================================================================

    #[test]
    fn test_format_pin_status_message_pinned() {
        let result = format_pin_status_message("git", true);
        assert_eq!(result, "Pinned git - it will not be upgraded");
    }

    #[test]
    fn test_format_pin_status_message_unpinned() {
        let result = format_pin_status_message("git", false);
        assert_eq!(result, "Unpinned git - it will be upgraded when outdated");
    }

    // ========================================================================
    // Has Upgrades Tests
    // ========================================================================

    #[test]
    fn test_has_upgrades_empty() {
        let outdated: Vec<OutdatedPackage> = vec![];
        assert!(!has_upgrades(&outdated));
    }

    #[test]
    fn test_has_upgrades_non_empty() {
        let outdated = vec![make_outdated_pkg("git", "2.43.0", "2.44.0")];
        assert!(has_upgrades(&outdated));
    }

    #[test]
    fn test_has_upgrades_multiple() {
        let outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];
        assert!(has_upgrades(&outdated));
    }

    // ========================================================================
    // Outdated Header Tests
    // ========================================================================

    #[test]
    fn test_format_outdated_header_multiple() {
        let result = format_outdated_header(5);
        assert_eq!(result, "5 outdated packages:");
    }

    #[test]
    fn test_format_outdated_header_single() {
        let result = format_outdated_header(1);
        assert_eq!(result, "1 outdated packages:");
    }

    #[test]
    fn test_format_outdated_header_zero() {
        let result = format_outdated_header(0);
        assert_eq!(result, "0 outdated packages:");
    }

    // ========================================================================
    // All Up To Date Message Tests
    // ========================================================================

    #[test]
    fn test_format_all_up_to_date_message_no_pinned() {
        let (main, pinned) = format_all_up_to_date_message(0);
        assert_eq!(main, "All packages are up to date.");
        assert!(pinned.is_none());
    }

    #[test]
    fn test_format_all_up_to_date_message_with_pinned() {
        let (main, pinned) = format_all_up_to_date_message(3);
        assert_eq!(main, "All packages are up to date.");
        assert_eq!(pinned, Some("3 pinned packages not checked".to_string()));
    }

    #[test]
    fn test_format_all_up_to_date_message_single_pinned() {
        let (main, pinned) = format_all_up_to_date_message(1);
        assert_eq!(main, "All packages are up to date.");
        assert!(pinned.is_some());
        assert!(pinned.unwrap().contains("1 pinned"));
    }

    // ========================================================================
    // Upgrade Announcement Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_announcement() {
        let result = format_upgrade_announcement("git", "2.43.0", "2.44.0");
        assert_eq!(result, "Upgrading git 2.43.0 → 2.44.0...");
    }

    #[test]
    fn test_format_upgrade_announcement_versioned() {
        let result = format_upgrade_announcement("python@3.11", "3.11.8", "3.11.9");
        assert!(result.contains("python@3.11"));
        assert!(result.contains("3.11.8"));
        assert!(result.contains("3.11.9"));
    }

    // ========================================================================
    // Upgrade Success/Failure Message Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_success() {
        let result = format_upgrade_success("git");
        assert_eq!(result, "git is already up to date");
    }

    #[test]
    fn test_format_upgrade_failure() {
        let result = format_upgrade_failure("git", "network timeout");
        assert_eq!(result, "Failed to upgrade git: network timeout");
    }

    #[test]
    fn test_format_upgrade_failure_complex_error() {
        let result = format_upgrade_failure("openssl@3", "checksum mismatch: expected abc123");
        assert!(result.contains("openssl@3"));
        assert!(result.contains("checksum mismatch"));
    }

    // ========================================================================
    // No Upgrades Message Tests
    // ========================================================================

    #[test]
    fn test_format_no_upgrades_message() {
        let result = format_no_upgrades_message();
        assert_eq!(result, "No packages were upgraded.");
    }

    // ========================================================================
    // Outdated Package Line Tests
    // ========================================================================

    #[test]
    fn test_format_outdated_package_line() {
        let result = format_outdated_package_line("git", "2.43.0", "2.44.0");
        assert_eq!(result, "git 2.43.0 → 2.44.0");
    }

    #[test]
    fn test_format_outdated_package_line_versioned() {
        let result = format_outdated_package_line("python@3.11", "3.11.8", "3.11.9");
        assert!(result.contains("python@3.11"));
    }

    // ========================================================================
    // Pinned Footer Tests
    // ========================================================================

    #[test]
    fn test_format_pinned_footer() {
        let result = format_pinned_footer(3);
        assert!(result.contains("3 pinned packages"));
        assert!(result.contains("zb list --pinned"));
    }

    #[test]
    fn test_format_pinned_footer_single() {
        let result = format_pinned_footer(1);
        assert!(result.contains("1 pinned"));
    }

    // ========================================================================
    // Outdated Output Kind Tests
    // ========================================================================

    #[test]
    fn test_determine_outdated_output_kind_json() {
        let kind = determine_outdated_output_kind(true, 5, 2);
        assert_eq!(kind, OutdatedOutputKind::Json);
    }

    #[test]
    fn test_determine_outdated_output_kind_all_up_to_date() {
        let kind = determine_outdated_output_kind(false, 0, 3);
        assert_eq!(kind, OutdatedOutputKind::AllUpToDate { pinned_count: 3 });
    }

    #[test]
    fn test_determine_outdated_output_kind_has_outdated() {
        let kind = determine_outdated_output_kind(false, 5, 2);
        assert_eq!(
            kind,
            OutdatedOutputKind::HasOutdated {
                outdated_count: 5,
                pinned_count: 2
            }
        );
    }

    #[test]
    fn test_determine_outdated_output_kind_no_pinned() {
        let kind = determine_outdated_output_kind(false, 0, 0);
        assert_eq!(kind, OutdatedOutputKind::AllUpToDate { pinned_count: 0 });
    }

    // ========================================================================
    // Upgrade Output Kind Tests
    // ========================================================================

    #[test]
    fn test_determine_upgrade_output_kind_not_installed() {
        let kind = determine_upgrade_output_kind(Some("git"), false, 0, false);
        assert_eq!(
            kind,
            UpgradeOutputKind::NotInstalled {
                formula: "git".to_string()
            }
        );
    }

    #[test]
    fn test_determine_upgrade_output_kind_already_up_to_date() {
        let kind = determine_upgrade_output_kind(Some("git"), true, 0, false);
        assert_eq!(
            kind,
            UpgradeOutputKind::AlreadyUpToDate {
                formula: "git".to_string()
            }
        );
    }

    #[test]
    fn test_determine_upgrade_output_kind_all_up_to_date() {
        let kind = determine_upgrade_output_kind(None, true, 0, false);
        assert_eq!(kind, UpgradeOutputKind::AllUpToDate);
    }

    #[test]
    fn test_determine_upgrade_output_kind_dry_run() {
        let kind = determine_upgrade_output_kind(None, true, 5, true);
        assert_eq!(kind, UpgradeOutputKind::DryRun { count: 5 });
    }

    #[test]
    fn test_determine_upgrade_output_kind_upgrade() {
        let kind = determine_upgrade_output_kind(None, true, 3, false);
        assert_eq!(kind, UpgradeOutputKind::Upgrade { count: 3 });
    }

    #[test]
    fn test_determine_upgrade_output_kind_dry_run_specific_formula() {
        let kind = determine_upgrade_output_kind(Some("git"), true, 1, true);
        assert_eq!(kind, UpgradeOutputKind::DryRun { count: 1 });
    }

    // ========================================================================
    // Upgrade Summary Tests
    // ========================================================================

    #[test]
    fn test_upgrade_summary_new() {
        let summary = UpgradeSummary::new();
        assert!(summary.upgraded.is_empty());
        assert!(summary.already_up_to_date.is_empty());
        assert!(summary.failed.is_empty());
    }

    #[test]
    fn test_upgrade_summary_record_success() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );

        assert_eq!(summary.upgraded.len(), 1);
        assert_eq!(summary.upgraded[0].0, "git");
        assert_eq!(summary.upgraded[0].1, "2.43.0");
        assert_eq!(summary.upgraded[0].2, "2.44.0");
    }

    #[test]
    fn test_upgrade_summary_record_up_to_date() {
        let mut summary = UpgradeSummary::new();
        summary.record_up_to_date("git".to_string());

        assert_eq!(summary.already_up_to_date.len(), 1);
        assert_eq!(summary.already_up_to_date[0], "git");
    }

    #[test]
    fn test_upgrade_summary_record_failure() {
        let mut summary = UpgradeSummary::new();
        summary.record_failure("git".to_string(), "network error".to_string());

        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].0, "git");
        assert_eq!(summary.failed[0].1, "network error");
    }

    #[test]
    fn test_upgrade_summary_upgraded_count() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        summary.record_success(
            "ripgrep".to_string(),
            "14.0.0".to_string(),
            "14.1.0".to_string(),
        );

        assert_eq!(summary.upgraded_count(), 2);
    }

    #[test]
    fn test_upgrade_summary_failed_count() {
        let mut summary = UpgradeSummary::new();
        summary.record_failure("git".to_string(), "error1".to_string());
        summary.record_failure("ripgrep".to_string(), "error2".to_string());

        assert_eq!(summary.failed_count(), 2);
    }

    #[test]
    fn test_upgrade_summary_has_upgrades() {
        let mut summary = UpgradeSummary::new();
        assert!(!summary.has_upgrades());

        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        assert!(summary.has_upgrades());
    }

    #[test]
    fn test_upgrade_summary_has_failures() {
        let mut summary = UpgradeSummary::new();
        assert!(!summary.has_failures());

        summary.record_failure("git".to_string(), "error".to_string());
        assert!(summary.has_failures());
    }

    #[test]
    fn test_upgrade_summary_total_attempted() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        summary.record_up_to_date("ripgrep".to_string());
        summary.record_failure("jq".to_string(), "error".to_string());

        assert_eq!(summary.total_attempted(), 3);
    }

    #[test]
    fn test_upgrade_summary_mixed_results() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        summary.record_success(
            "ripgrep".to_string(),
            "14.0.0".to_string(),
            "14.1.0".to_string(),
        );
        summary.record_up_to_date("jq".to_string());
        summary.record_failure("curl".to_string(), "checksum mismatch".to_string());

        assert_eq!(summary.upgraded_count(), 2);
        assert_eq!(summary.failed_count(), 1);
        assert_eq!(summary.already_up_to_date.len(), 1);
        assert_eq!(summary.total_attempted(), 4);
        assert!(summary.has_upgrades());
        assert!(summary.has_failures());
    }

    // ========================================================================
    // Upgrade Summary Output Tests
    // ========================================================================

    #[test]
    fn test_format_upgrade_summary_output_no_upgrades() {
        let summary = UpgradeSummary::new();
        let lines = format_upgrade_summary_output(&summary, 1.5);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("No packages were upgraded"));
    }

    #[test]
    fn test_format_upgrade_summary_output_with_upgrades() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        summary.record_success(
            "ripgrep".to_string(),
            "14.0.0".to_string(),
            "14.1.0".to_string(),
        );

        let lines = format_upgrade_summary_output(&summary, 5.25);

        assert!(lines[0].contains("Upgraded 2 packages"));
        assert!(lines[0].contains("5.25s"));
        assert!(lines.iter().any(|l| l.contains("git")));
        assert!(lines.iter().any(|l| l.contains("ripgrep")));
    }

    #[test]
    fn test_format_upgrade_summary_output_with_failures() {
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        summary.record_failure("ripgrep".to_string(), "network error".to_string());

        let lines = format_upgrade_summary_output(&summary, 3.0);

        assert!(lines.iter().any(|l| l.contains("Upgraded 1 packages")));
        assert!(lines.iter().any(|l| l.contains("Failed to upgrade 1")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("ripgrep") && l.contains("network error"))
        );
    }

    #[test]
    fn test_format_upgrade_summary_output_only_failures() {
        let mut summary = UpgradeSummary::new();
        summary.record_failure("git".to_string(), "error1".to_string());
        summary.record_failure("ripgrep".to_string(), "error2".to_string());

        let lines = format_upgrade_summary_output(&summary, 2.0);

        assert!(lines[0].contains("No packages were upgraded"));
        assert!(lines.iter().any(|l| l.contains("Failed to upgrade 2")));
    }

    // ========================================================================
    // Dry Run Output Tests
    // ========================================================================

    #[test]
    fn test_format_dry_run_output_single() {
        let packages = vec![make_outdated_pkg("git", "2.43.0", "2.44.0")];
        let lines = format_dry_run_output(&packages);

        assert!(lines[0].contains("Would upgrade 1 packages"));
        assert!(lines.iter().any(|l| l.contains("git")));
        assert!(lines.iter().any(|l| l.contains("2.43.0")));
        assert!(lines.iter().any(|l| l.contains("2.44.0")));
    }

    #[test]
    fn test_format_dry_run_output_multiple() {
        let packages = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
            make_outdated_pkg("jq", "1.6", "1.7"),
        ];
        let lines = format_dry_run_output(&packages);

        assert!(lines[0].contains("Would upgrade 3 packages"));
        assert!(lines.iter().any(|l| l.contains("git")));
        assert!(lines.iter().any(|l| l.contains("ripgrep")));
        assert!(lines.iter().any(|l| l.contains("jq")));
    }

    #[test]
    fn test_format_dry_run_output_empty() {
        let packages: Vec<OutdatedPackage> = vec![];
        let lines = format_dry_run_output(&packages);

        assert!(lines[0].contains("Would upgrade 0 packages"));
    }

    #[test]
    fn test_format_dry_run_output_structure() {
        let packages = vec![make_outdated_pkg("git", "2.43.0", "2.44.0")];
        let lines = format_dry_run_output(&packages);

        // First line is header
        assert!(lines[0].contains("Would upgrade"));
        // Second line is empty (spacing)
        assert_eq!(lines[1], "");
        // Third line+ are packages
        assert!(lines[2].contains("git"));
    }

    // ========================================================================
    // Pinned Package Exclusion Tests
    // ========================================================================

    #[test]
    fn test_should_exclude_pinned_true() {
        let pinned = vec!["git".to_string(), "ripgrep".to_string()];
        assert!(should_exclude_pinned("git", &pinned));
    }

    #[test]
    fn test_should_exclude_pinned_false() {
        let pinned = vec!["git".to_string(), "ripgrep".to_string()];
        assert!(!should_exclude_pinned("jq", &pinned));
    }

    #[test]
    fn test_should_exclude_pinned_empty_list() {
        let pinned: Vec<String> = vec![];
        assert!(!should_exclude_pinned("git", &pinned));
    }

    #[test]
    fn test_should_exclude_pinned_case_sensitive() {
        let pinned = vec!["Git".to_string()];
        assert!(!should_exclude_pinned("git", &pinned));
    }

    // ========================================================================
    // Sort Outdated Packages Tests
    // ========================================================================

    #[test]
    fn test_sort_outdated_packages() {
        let packages = vec![
            make_outdated_pkg("zsh", "5.8", "5.9"),
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];

        let sorted = sort_outdated_packages(packages);

        assert_eq!(sorted[0].name, "git");
        assert_eq!(sorted[1].name, "ripgrep");
        assert_eq!(sorted[2].name, "zsh");
    }

    #[test]
    fn test_sort_outdated_packages_already_sorted() {
        let packages = vec![
            make_outdated_pkg("aaa", "1.0", "2.0"),
            make_outdated_pkg("bbb", "1.0", "2.0"),
            make_outdated_pkg("ccc", "1.0", "2.0"),
        ];

        let sorted = sort_outdated_packages(packages);

        assert_eq!(sorted[0].name, "aaa");
        assert_eq!(sorted[1].name, "bbb");
        assert_eq!(sorted[2].name, "ccc");
    }

    #[test]
    fn test_sort_outdated_packages_empty() {
        let packages: Vec<OutdatedPackage> = vec![];
        let sorted = sort_outdated_packages(packages);
        assert!(sorted.is_empty());
    }

    #[test]
    fn test_sort_outdated_packages_single() {
        let packages = vec![make_outdated_pkg("git", "2.43.0", "2.44.0")];
        let sorted = sort_outdated_packages(packages);
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].name, "git");
    }

    // ========================================================================
    // Version Change Classification Tests
    // ========================================================================

    #[test]
    fn test_classify_version_change_major() {
        assert_eq!(
            classify_version_change("1.0.0", "2.0.0"),
            VersionChangeType::Major
        );
        assert_eq!(
            classify_version_change("1.5.3", "2.0.0"),
            VersionChangeType::Major
        );
    }

    #[test]
    fn test_classify_version_change_minor() {
        assert_eq!(
            classify_version_change("1.0.0", "1.1.0"),
            VersionChangeType::Minor
        );
        assert_eq!(
            classify_version_change("1.5.3", "1.6.0"),
            VersionChangeType::Minor
        );
    }

    #[test]
    fn test_classify_version_change_patch() {
        assert_eq!(
            classify_version_change("1.0.0", "1.0.1"),
            VersionChangeType::Patch
        );
        assert_eq!(
            classify_version_change("1.5.3", "1.5.4"),
            VersionChangeType::Patch
        );
    }

    #[test]
    fn test_classify_version_change_unknown() {
        // Single component versions
        assert_eq!(
            classify_version_change("1", "2"),
            VersionChangeType::Unknown
        );
        // Non-numeric
        assert_eq!(
            classify_version_change("abc", "def"),
            VersionChangeType::Unknown
        );
    }

    #[test]
    fn test_classify_version_change_with_suffix() {
        assert_eq!(
            classify_version_change("1.0.0_1", "2.0.0"),
            VersionChangeType::Major
        );
        assert_eq!(
            classify_version_change("1.0.0-beta", "1.1.0"),
            VersionChangeType::Minor
        );
    }

    #[test]
    fn test_classify_version_change_same_version() {
        assert_eq!(
            classify_version_change("1.0.0", "1.0.0"),
            VersionChangeType::Unknown
        );
    }

    // ========================================================================
    // Count By Change Type Tests
    // ========================================================================

    #[test]
    fn test_count_by_change_type_all_major() {
        let packages = vec![
            make_outdated_pkg("git", "1.0.0", "2.0.0"),
            make_outdated_pkg("ripgrep", "1.0.0", "3.0.0"),
        ];
        let (major, minor, patch, unknown) = count_by_change_type(&packages);
        assert_eq!(major, 2);
        assert_eq!(minor, 0);
        assert_eq!(patch, 0);
        assert_eq!(unknown, 0);
    }

    #[test]
    fn test_count_by_change_type_mixed() {
        let packages = vec![
            make_outdated_pkg("git", "1.0.0", "2.0.0"),     // major
            make_outdated_pkg("ripgrep", "1.0.0", "1.1.0"), // minor
            make_outdated_pkg("jq", "1.0.0", "1.0.1"),      // patch
            make_outdated_pkg("foo", "abc", "def"),         // unknown
        ];
        let (major, minor, patch, unknown) = count_by_change_type(&packages);
        assert_eq!(major, 1);
        assert_eq!(minor, 1);
        assert_eq!(patch, 1);
        assert_eq!(unknown, 1);
    }

    #[test]
    fn test_count_by_change_type_empty() {
        let packages: Vec<OutdatedPackage> = vec![];
        let (major, minor, patch, unknown) = count_by_change_type(&packages);
        assert_eq!(major, 0);
        assert_eq!(minor, 0);
        assert_eq!(patch, 0);
        assert_eq!(unknown, 0);
    }

    // ========================================================================
    // Formula Name Validation Tests
    // ========================================================================

    #[test]
    fn test_is_valid_formula_name_simple() {
        assert!(is_valid_formula_name("git"));
        assert!(is_valid_formula_name("ripgrep"));
    }

    #[test]
    fn test_is_valid_formula_name_with_numbers() {
        assert!(is_valid_formula_name("python3"));
        assert!(is_valid_formula_name("go123"));
    }

    #[test]
    fn test_is_valid_formula_name_versioned() {
        assert!(is_valid_formula_name("python@3.11"));
        assert!(is_valid_formula_name("openssl@3"));
    }

    #[test]
    fn test_is_valid_formula_name_with_hyphens() {
        assert!(is_valid_formula_name("my-package"));
        assert!(is_valid_formula_name("test-package-name"));
    }

    #[test]
    fn test_is_valid_formula_name_with_underscores() {
        assert!(is_valid_formula_name("my_package"));
        assert!(is_valid_formula_name("test_package_name"));
    }

    #[test]
    fn test_is_valid_formula_name_empty() {
        assert!(!is_valid_formula_name(""));
    }

    #[test]
    fn test_is_valid_formula_name_starts_with_number() {
        assert!(!is_valid_formula_name("123abc"));
    }

    #[test]
    fn test_is_valid_formula_name_starts_with_hyphen() {
        assert!(!is_valid_formula_name("-package"));
    }

    #[test]
    fn test_is_valid_formula_name_invalid_chars() {
        assert!(!is_valid_formula_name("pkg!name"));
        assert!(!is_valid_formula_name("pkg name"));
        assert!(!is_valid_formula_name("pkg/name"));
    }

    #[test]
    fn test_is_valid_formula_name_with_dots() {
        assert!(is_valid_formula_name("pkg.name"));
        assert!(is_valid_formula_name("python@3.11.8"));
    }

    // ========================================================================
    // Not Installed Error Tests
    // ========================================================================

    #[test]
    fn test_format_not_installed_error() {
        let result = format_not_installed_error("git");
        assert_eq!(result, "Formula 'git' is not installed.");
    }

    #[test]
    fn test_format_not_installed_error_versioned() {
        let result = format_not_installed_error("python@3.11");
        assert!(result.contains("python@3.11"));
        assert!(result.contains("not installed"));
    }

    // ========================================================================
    // Dry Run vs Actual Output Difference Tests
    // ========================================================================

    #[test]
    fn test_dry_run_vs_actual_header_difference() {
        let dry_run_header = format_dry_run_header(5);
        let actual_header = format_upgrade_header(5);

        assert!(dry_run_header.contains("Would upgrade"));
        assert!(actual_header.contains("Upgrading"));
        assert!(!dry_run_header.contains("Upgrading"));
        assert!(!actual_header.contains("Would"));
    }

    #[test]
    fn test_dry_run_shows_packages_without_performing() {
        let packages = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
        ];

        // Dry run just lists packages
        let dry_run_output = format_dry_run_output(&packages);
        assert!(dry_run_output[0].contains("Would upgrade"));

        // Actual upgrade would show summary after completion
        let mut summary = UpgradeSummary::new();
        summary.record_success(
            "git".to_string(),
            "2.43.0".to_string(),
            "2.44.0".to_string(),
        );
        let actual_output = format_upgrade_summary_output(&summary, 1.0);
        assert!(actual_output[0].contains("Upgraded"));
    }

    #[test]
    fn test_dry_run_output_kind_determination() {
        // Same state, different mode
        let dry_run_kind = determine_upgrade_output_kind(None, true, 5, true);
        let actual_kind = determine_upgrade_output_kind(None, true, 5, false);

        assert_eq!(dry_run_kind, UpgradeOutputKind::DryRun { count: 5 });
        assert_eq!(actual_kind, UpgradeOutputKind::Upgrade { count: 5 });
    }

    // ========================================================================
    // Pinned Package Handling Integration Tests
    // ========================================================================

    #[test]
    fn test_pinned_packages_shown_in_outdated_output() {
        // When all packages up to date but some pinned
        let kind = determine_outdated_output_kind(false, 0, 3);
        match kind {
            OutdatedOutputKind::AllUpToDate { pinned_count } => {
                assert_eq!(pinned_count, 3);
                let (_, pinned_msg) = format_all_up_to_date_message(pinned_count);
                assert!(pinned_msg.is_some());
                assert!(pinned_msg.unwrap().contains("3 pinned"));
            }
            _ => panic!("Expected AllUpToDate"),
        }
    }

    #[test]
    fn test_pinned_packages_shown_in_has_outdated_output() {
        let kind = determine_outdated_output_kind(false, 5, 2);
        match kind {
            OutdatedOutputKind::HasOutdated {
                outdated_count,
                pinned_count,
            } => {
                assert_eq!(outdated_count, 5);
                assert_eq!(pinned_count, 2);
                let footer = format_pinned_footer(pinned_count);
                assert!(footer.contains("2 pinned"));
            }
            _ => panic!("Expected HasOutdated"),
        }
    }

    #[test]
    fn test_pinned_exclusion_logic() {
        let pinned = vec!["git".to_string(), "node".to_string()];
        let all_outdated = vec![
            make_outdated_pkg("git", "2.43.0", "2.44.0"),
            make_outdated_pkg("ripgrep", "14.0.0", "14.1.0"),
            make_outdated_pkg("node", "20.0.0", "22.0.0"),
            make_outdated_pkg("jq", "1.6", "1.7"),
        ];

        // Simulate filtering out pinned packages
        let upgradable: Vec<_> = all_outdated
            .iter()
            .filter(|p| !should_exclude_pinned(&p.name, &pinned))
            .collect();

        assert_eq!(upgradable.len(), 2);
        assert!(
            upgradable
                .iter()
                .all(|p| p.name != "git" && p.name != "node")
        );
    }

    #[test]
    fn test_pin_status_messages_are_informative() {
        let pin_msg = format_pin_status_message("git", true);
        let unpin_msg = format_pin_status_message("git", false);

        // Pin message should indicate no upgrades
        assert!(pin_msg.contains("will not be upgraded"));
        // Unpin message should indicate upgrades will happen
        assert!(unpin_msg.contains("will be upgraded when outdated"));
    }
}
