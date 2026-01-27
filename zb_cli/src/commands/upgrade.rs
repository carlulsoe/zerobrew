//! Upgrade and outdated command implementations.

use console::style;
use indicatif::MultiProgress;
use std::time::Instant;

use zb_io::install::Installer;

use crate::display::{create_progress_callback, finish_progress_bars, ProgressStyles};

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

    if json {
        let json_output: Vec<serde_json::Value> = outdated
            .iter()
            .map(|pkg| {
                serde_json::json!({
                    "name": pkg.name,
                    "installed_version": pkg.installed_version,
                    "available_version": pkg.available_version
                })
            })
            .collect();
        match serde_json::to_string_pretty(&json_output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!(
                    "{} Failed to serialize JSON: {}",
                    style("error:").red().bold(),
                    e
                );
                std::process::exit(1);
            }
        }
    } else if outdated.is_empty() {
        println!("All packages are up to date.");
        if pinned_count > 0 {
            println!(
                "    {} {} pinned packages not checked",
                style("→").dim(),
                pinned_count
            );
        }
    } else {
        println!(
            "{} {} outdated packages:",
            style("==>").cyan().bold(),
            style(outdated.len()).yellow().bold()
        );
        println!();

        for pkg in &outdated {
            println!(
                "  {} {} → {}",
                style(&pkg.name).bold(),
                style(&pkg.installed_version).red(),
                style(&pkg.available_version).green()
            );
        }

        println!();
        println!(
            "    {} Run {} to upgrade all",
            style("→").cyan(),
            style("zb upgrade").cyan()
        );
        if pinned_count > 0 {
            println!(
                "    {} {} pinned packages not shown (use {} to see them)",
                style("→").dim(),
                pinned_count,
                style("zb list --pinned").dim()
            );
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
    let to_upgrade = if let Some(ref name) = formula {
        let outdated = installer.get_outdated().await?;
        outdated
            .into_iter()
            .filter(|p| p.name == *name)
            .collect::<Vec<_>>()
    } else {
        installer.get_outdated().await?
    };

    if to_upgrade.is_empty() {
        if let Some(ref name) = formula {
            if installer.is_installed(name) {
                println!(
                    "{} {} is already up to date.",
                    style("==>").cyan().bold(),
                    style(name).bold()
                );
            } else {
                println!(
                    "{} {} is not installed.",
                    style("==>").cyan().bold(),
                    style(name).bold()
                );
            }
        } else {
            println!(
                "{} All packages are up to date.",
                style("==>").cyan().bold()
            );
        }
        return Ok(());
    }

    if dry_run {
        println!(
            "{} Would upgrade {} packages:",
            style("==>").cyan().bold(),
            style(to_upgrade.len()).yellow().bold()
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

    println!(
        "{} Upgrading {} packages...",
        style("==>").cyan().bold(),
        style(to_upgrade.len()).yellow().bold()
    );

    let multi = MultiProgress::new();
    let styles = ProgressStyles::default();
    let (progress_callback, bars) = create_progress_callback(multi, styles, "upgraded");

    // Perform the upgrades
    let mut upgraded_packages = Vec::new();
    for pkg in &to_upgrade {
        println!();
        println!(
            "{} Upgrading {} {} → {}...",
            style("==>").cyan().bold(),
            style(&pkg.name).bold(),
            style(&pkg.installed_version).red(),
            style(&pkg.available_version).green()
        );

        match installer
            .upgrade_one(&pkg.name, true, Some(progress_callback.clone()))
            .await
        {
            Ok(Some((old_ver, new_ver))) => {
                upgraded_packages.push((pkg.name.clone(), old_ver, new_ver));
            }
            Ok(None) => {
                println!(
                    "    {} {} is already up to date",
                    style("✓").green(),
                    pkg.name
                );
            }
            Err(e) => {
                eprintln!(
                    "    {} Failed to upgrade {}: {}",
                    style("✗").red(),
                    pkg.name,
                    e
                );
            }
        }
    }

    finish_progress_bars(&bars);

    let elapsed = start.elapsed();
    println!();
    if upgraded_packages.is_empty() {
        println!("{} No packages were upgraded.", style("==>").cyan().bold());
    } else {
        println!(
            "{} Upgraded {} packages in {:.2}s:",
            style("==>").cyan().bold(),
            style(upgraded_packages.len()).green().bold(),
            elapsed.as_secs_f64()
        );
        for (name, old_ver, new_ver) in &upgraded_packages {
            println!(
                "    {} {} {} → {}",
                style("✓").green(),
                style(name).bold(),
                style(old_ver).dim(),
                style(new_ver).green()
            );
        }
    }

    Ok(())
}

/// Run the pin command.
pub fn run_pin(installer: &mut Installer, formula: &str) -> Result<(), zb_core::Error> {
    match installer.pin(formula) {
        Ok(true) => {
            println!(
                "{} Pinned {} - it will not be upgraded",
                style("==>").cyan().bold(),
                style(formula).green().bold()
            );
        }
        Ok(false) => {
            println!("Formula '{}' is not installed.", formula);
        }
        Err(zb_core::Error::NotInstalled { .. }) => {
            println!("Formula '{}' is not installed.", formula);
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Run the unpin command.
pub fn run_unpin(installer: &mut Installer, formula: &str) -> Result<(), zb_core::Error> {
    match installer.unpin(formula) {
        Ok(true) => {
            println!(
                "{} Unpinned {} - it will be upgraded when outdated",
                style("==>").cyan().bold(),
                style(formula).green().bold()
            );
        }
        Ok(false) => {
            println!("Formula '{}' is not installed.", formula);
        }
        Err(zb_core::Error::NotInstalled { .. }) => {
            println!("Formula '{}' is not installed.", formula);
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
/// Extracted for testability.
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
/// Extracted for testability.
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
/// Extracted for testability.
pub(crate) fn format_outdated_package_line(
    name: &str,
    installed: &str,
    available: &str,
) -> String {
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
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// Check if any packages were upgraded.
    pub fn has_upgrades(&self) -> bool {
        !self.upgraded.is_empty()
    }

    /// Check if any upgrades failed.
    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }

    /// Get total attempted upgrades.
    pub fn total_attempted(&self) -> usize {
        self.upgraded.len() + self.already_up_to_date.len() + self.failed.len()
    }
}

/// Format the complete upgrade summary output.
/// Extracted for testability.
pub(crate) fn format_upgrade_summary_output(
    summary: &UpgradeSummary,
    elapsed_secs: f64,
) -> Vec<String> {
    let mut lines = Vec::new();

    if summary.upgraded.is_empty() {
        lines.push(format_no_upgrades_message());
    } else {
        lines.push(format_upgrade_summary(summary.upgraded_count(), elapsed_secs));
        for (name, old_ver, new_ver) in &summary.upgraded {
            lines.push(format!("    ✓ {}", format_upgraded_package(name, old_ver, new_ver)));
        }
    }

    if !summary.failed.is_empty() {
        lines.push(String::new());
        lines.push(format!("Failed to upgrade {} packages:", summary.failed_count()));
        for (name, error) in &summary.failed {
            lines.push(format!("    ✗ {}: {}", name, error));
        }
    }

    lines
}

/// Format the dry-run output lines.
/// Extracted for testability.
pub(crate) fn format_dry_run_output(
    packages: &[zb_core::version::OutdatedPackage],
) -> Vec<String> {
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
/// Extracted for testability.
pub(crate) fn should_exclude_pinned(
    package_name: &str,
    pinned_packages: &[String],
) -> bool {
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
/// Extracted for testability.
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
/// Extracted for testability.
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
/// Extracted for testability.
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
    if !name.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Can contain letters, numbers, hyphens, underscores, and @ for versioned formulas
    name.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '@' || c == '.'
    })
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
}
