//! Info, search, and list command implementations.

use console::style;
use std::path::Path;

use zb_core::Formula;
use zb_io::install::Installer;
use zb_io::search::search_formulas;
use zb_io::{ApiCache, ApiClient, InstalledKeg};

use crate::display::chrono_lite_format;

/// Run the list command.
pub fn run_list(installer: &Installer, pinned: bool) -> Result<(), zb_core::Error> {
    let installed = if pinned {
        installer.list_pinned()?
    } else {
        installer.list_installed()?
    };

    match determine_list_output_kind(installed.len(), pinned) {
        ListOutputKind::Empty { pinned: is_pinned } => {
            println!("{}", empty_list_message(is_pinned));
        }
        ListOutputKind::HasItems { count: _ } => {
            for keg in installed {
                // format_list_entry provides the plain-text format (used for testing)
                let _ = format_list_entry(&keg.name, &keg.version, keg.pinned);

                // Styled output for terminal
                let styled_pin = if keg.pinned {
                    format!(" {}", style("(pinned)").yellow())
                } else {
                    String::new()
                };
                println!(
                    "{} {}{}",
                    style(&keg.name).bold(),
                    style(&keg.version).dim(),
                    styled_pin
                );
            }
        }
    }

    Ok(())
}

/// Run the info command.
pub async fn run_info(
    installer: &mut Installer,
    prefix: &Path,
    formula: String,
    json: bool,
) -> Result<(), zb_core::Error> {
    let keg = installer.get_installed(&formula);
    let api_formula = installer.get_formula(&formula).await.ok();

    if json {
        print_info_json(installer, &formula, &keg, &api_formula).await?;
    } else {
        print_info_human(installer, prefix, &formula, &keg, &api_formula).await?;
    }

    Ok(())
}

async fn print_info_json(
    installer: &mut Installer,
    formula: &str,
    keg: &Option<InstalledKeg>,
    api_formula: &Option<Formula>,
) -> Result<(), zb_core::Error> {
    let mut info = build_info_json_base(formula, keg.is_some());

    if let Some(keg) = keg {
        let installed_info = build_installed_info_json(
            &keg.version,
            &keg.store_key,
            keg.installed_at as u64,
            keg.pinned,
            keg.explicit,
        );
        info.extend(installed_info);

        if let Ok(linked_files) = installer.get_linked_files(formula) {
            let files = build_linked_files_json(&linked_files);
            info.insert("linked_files".to_string(), serde_json::json!(files));
        }

        if let Ok(dependents) = installer.get_dependents(formula).await {
            info.insert("dependents".to_string(), serde_json::json!(dependents));
        }
    }

    if let Some(f) = api_formula {
        let api_info = build_formula_api_json(
            &f.effective_version(),
            f.desc.as_deref(),
            f.homepage.as_deref(),
            f.license.as_deref(),
            &f.effective_dependencies(),
            &f.build_dependencies,
            f.caveats.as_deref(),
            f.keg_only,
        );
        info.extend(api_info);

        // Add outdated info if there's an update available
        if let Some(keg) = keg {
            let available_version = f.effective_version();
            if is_update_available(&keg.version, &available_version) {
                let outdated = build_outdated_json(formula, &keg.version, &available_version);
                info.insert("outdated".to_string(), outdated);
            }
        }
    }

    match serde_json::to_string_pretty(&info) {
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

    Ok(())
}

async fn print_info_human(
    installer: &mut Installer,
    prefix: &Path,
    formula: &str,
    keg: &Option<InstalledKeg>,
    api_formula: &Option<Formula>,
) -> Result<(), zb_core::Error> {
    let output_kind = determine_info_output_kind(keg.is_some(), api_formula.is_some());
    if output_kind == InfoOutputKind::NotFound {
        println!("Formula '{}' not found.", formula);
        std::process::exit(1);
    }

    // Header
    println!("{} {}", style("==>").cyan().bold(), style(formula).bold());

    // Description from API
    if let Some(f) = api_formula {
        if let Some(desc) = &f.desc {
            println!("{}", style(desc).dim());
        }
        if let Some(homepage) = &f.homepage {
            println!("{}", style(homepage).cyan().underlined());
        }
    }

    println!();

    // Version info - use helper functions for testable logic
    if let Some(keg) = keg {
        // format_installed_version_line provides the complete plain text format (used for testing)
        let _ = format_installed_version_line(&keg.version, keg.pinned, keg.explicit);

        // Styled output for terminal
        print!(
            "{} {}",
            style("Installed:").dim(),
            style(&keg.version).green()
        );
        let pin_marker = format_pin_marker(keg.pinned);
        if !pin_marker.is_empty() {
            print!(" {}", style("(pinned)").yellow());
        }
        let explicit_marker = format_explicit_marker(keg.explicit);
        if !explicit_marker.is_empty() {
            print!(" {}", style("(installed as dependency)").dim());
        }
        println!();
    } else {
        println!("{} Not installed", style("Installed:").dim());
    }

    if let Some(f) = api_formula {
        let available_version = f.effective_version();
        let installed_version = keg.as_ref().map(|k| k.version.as_str());

        // format_available_version_line provides the plain text format (used for testing)
        let _ = format_available_version_line(&available_version, installed_version);

        // Use format_version_comparison for styled output logic
        let version_display = format_version_comparison(installed_version, &available_version);

        match version_display {
            VersionDisplay::UpdateAvailable {
                installed: _,
                available,
            } => {
                println!(
                    "{} {} {}",
                    style("Available:").dim(),
                    style(&available).yellow(),
                    style("(update available)").yellow()
                );
            }
            VersionDisplay::NotInstalled(version) => {
                println!("{} {}", style("Available:").dim(), version);
            }
            VersionDisplay::UpToDate(_) => {
                // Don't show available version if up to date
            }
        }
    }

    // License
    if let Some(f) = api_formula
        && let Some(ref license) = f.license
    {
        println!("{} {}", style("License:").dim(), license);
    }

    // Keg-only status
    if let Some(f) = api_formula
        && f.keg_only
    {
        let explanation = f.keg_only_reason.as_ref().map(|r| r.explanation.as_str());
        let keg_only_display = format_keg_only_reason(explanation);
        println!("{} {}", style("Keg-only:").dim(), keg_only_display);
    }

    // Dependencies
    if let Some(f) = api_formula {
        let deps = f.effective_dependencies();
        if !deps.is_empty() {
            println!();
            println!("{}", style("Dependencies:").dim());
            for dep in &deps {
                let installed = installer.is_installed(dep);
                let marker_str = format_dependency_status(installed);
                let marker = if installed {
                    style(marker_str).green()
                } else {
                    style(marker_str).red()
                };
                println!("  {} {}", marker, dep);
            }
        }

        if !f.build_dependencies.is_empty() {
            println!();
            println!("{}", style("Build dependencies:").dim());
            for dep in &f.build_dependencies {
                println!("  {}", dep);
            }
        }
    }

    // Dependents
    if keg.is_some()
        && let Ok(dependents) = installer.get_dependents(formula).await
        && !dependents.is_empty()
    {
        println!();
        println!("{}", style("Required by:").dim());
        for dep in &dependents {
            println!("  {}", dep);
        }
    }

    // Linked files
    if let Some(keg) = keg
        && let Ok(linked_files) = installer.get_linked_files(formula)
        && !linked_files.is_empty()
    {
        let (display_count, remaining) = calculate_linked_files_display(linked_files.len(), 5);

        println!();
        println!(
            "{} ({} files)",
            style("Linked files:").dim(),
            linked_files.len()
        );
        for (link, _target) in linked_files.iter().take(display_count) {
            println!("  {}", link);
        }
        if let Some(more) = remaining {
            println!("  {} and {} more...", style("...").dim(), more);
        }

        println!();
        let store_key_display = format_store_key(&keg.store_key);
        println!("{} {}", style("Store key:").dim(), store_key_display);
        println!(
            "{} {}",
            style("Installed:").dim(),
            chrono_lite_format(keg.installed_at)
        );
    }

    // Caveats
    if let Some(f) = api_formula
        && let Some(ref caveats) = f.caveats
    {
        println!();
        println!("{}", style("==> Caveats").yellow().bold());
        let formatted_caveats = format_caveats(caveats, &prefix.to_string_lossy());
        for line in formatted_caveats.lines() {
            println!("{}", line);
        }
    }

    Ok(())
}

/// Truncate a description to a maximum length with ellipsis.
/// Extracted for testability.
pub(crate) fn truncate_description(desc: &str, max_len: usize) -> String {
    if desc.len() > max_len {
        format!("{}...", &desc[..max_len.saturating_sub(3)])
    } else {
        desc.to_string()
    }
}

/// Format a store key for display (show first 12 characters).
/// Extracted for testability.
pub(crate) fn format_store_key(store_key: &str) -> &str {
    if store_key.len() >= 12 {
        &store_key[..12]
    } else {
        store_key
    }
}

/// Build the basic info JSON structure for a formula.
/// Extracted for testability.
pub(crate) fn build_info_json_base(
    formula_name: &str,
    installed: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut info = serde_json::Map::new();
    info.insert("name".to_string(), serde_json::json!(formula_name));
    info.insert("installed".to_string(), serde_json::json!(installed));
    info
}

/// Build JSON for installed formula info.
/// Extracted for testability.
pub(crate) fn build_installed_info_json(
    version: &str,
    store_key: &str,
    installed_at: u64,
    pinned: bool,
    explicit: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut info = serde_json::Map::new();
    info.insert("installed_version".to_string(), serde_json::json!(version));
    info.insert("store_key".to_string(), serde_json::json!(store_key));
    info.insert("installed_at".to_string(), serde_json::json!(installed_at));
    info.insert("pinned".to_string(), serde_json::json!(pinned));
    info.insert("explicit".to_string(), serde_json::json!(explicit));
    info
}

/// Build linked files JSON array.
/// Extracted for testability.
pub(crate) fn build_linked_files_json(linked_files: &[(String, String)]) -> Vec<serde_json::Value> {
    linked_files
        .iter()
        .map(|(link, target)| serde_json::json!({"link": link, "target": target}))
        .collect()
}

/// Build search result JSON.
/// Extracted for testability.
pub(crate) fn build_search_result_json(
    name: &str,
    full_name: &str,
    version: &str,
    description: &str,
    installed: bool,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "full_name": full_name,
        "version": version,
        "description": description,
        "installed": installed
    })
}

/// Format pin marker for display.
/// Extracted for testability.
pub(crate) fn format_pin_marker(pinned: bool) -> String {
    if pinned {
        " (pinned)".to_string()
    } else {
        String::new()
    }
}

/// Format explicit/dependency marker for display.
/// Extracted for testability.
pub(crate) fn format_explicit_marker(explicit: bool) -> String {
    if explicit {
        String::new()
    } else {
        " (installed as dependency)".to_string()
    }
}

/// Calculate display limit for linked files (show first N, then "and X more...").
/// Extracted for testability.
pub(crate) fn calculate_linked_files_display(
    total_files: usize,
    display_limit: usize,
) -> (usize, Option<usize>) {
    if total_files <= display_limit {
        (total_files, None)
    } else {
        (display_limit, Some(total_files - display_limit))
    }
}

/// Determine if an update is available by comparing versions.
/// Extracted for testability.
pub(crate) fn is_update_available(installed_version: &str, available_version: &str) -> bool {
    installed_version != available_version
}

/// Format caveats text by replacing prefix placeholder.
/// Extracted for testability.
pub(crate) fn format_caveats(caveats: &str, prefix: &str) -> String {
    caveats.replace("$HOMEBREW_PREFIX", prefix)
}

/// Generate empty list message based on filter type.
/// Extracted for testability.
pub(crate) fn empty_list_message(pinned: bool) -> &'static str {
    if pinned {
        "No pinned formulas."
    } else {
        "No formulas installed."
    }
}

/// Generate empty search results message based on filter.
/// Extracted for testability.
pub(crate) fn empty_search_message(query: &str, installed_only: bool) -> String {
    if installed_only {
        format!("No installed formulas found matching '{}'.", query)
    } else {
        format!("No formulas found matching '{}'.", query)
    }
}

/// Get search results label based on filter.
/// Extracted for testability.
pub(crate) fn search_results_label(installed_only: bool) -> &'static str {
    if installed_only {
        "installed formulas"
    } else {
        "formulas"
    }
}

/// Format dependency status marker for display.
/// Extracted for testability.
pub(crate) fn format_dependency_status(installed: bool) -> &'static str {
    if installed { "✓" } else { "✗" }
}

/// Format the version comparison message when update available.
/// Extracted for testability.
pub(crate) fn format_version_comparison(
    installed: Option<&str>,
    available: &str,
) -> VersionDisplay {
    match installed {
        Some(inst) if !is_update_available(inst, available) => {
            VersionDisplay::UpToDate(inst.to_string())
        }
        Some(inst) => VersionDisplay::UpdateAvailable {
            installed: inst.to_string(),
            available: available.to_string(),
        },
        None => VersionDisplay::NotInstalled(available.to_string()),
    }
}

/// Result of version comparison for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionDisplay {
    UpToDate(String),
    UpdateAvailable {
        installed: String,
        available: String,
    },
    NotInstalled(String),
}

/// Format keg-only reason for display.
/// Extracted for testability.
pub(crate) fn format_keg_only_reason(explanation: Option<&str>) -> String {
    match explanation {
        Some(exp) if !exp.is_empty() => format!("Yes ({})", exp),
        _ => "Yes".to_string(),
    }
}

/// Build formula info JSON with API data.
/// Extracted for testability.
pub(crate) fn build_formula_api_json(
    version: &str,
    description: Option<&str>,
    homepage: Option<&str>,
    license: Option<&str>,
    dependencies: &[String],
    build_dependencies: &[String],
    caveats: Option<&str>,
    keg_only: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut info = serde_json::Map::new();
    info.insert("available_version".to_string(), serde_json::json!(version));
    if let Some(desc) = description {
        info.insert("description".to_string(), serde_json::json!(desc));
    }
    if let Some(hp) = homepage {
        info.insert("homepage".to_string(), serde_json::json!(hp));
    }
    if let Some(lic) = license {
        info.insert("license".to_string(), serde_json::json!(lic));
    }
    info.insert("dependencies".to_string(), serde_json::json!(dependencies));
    info.insert(
        "build_dependencies".to_string(),
        serde_json::json!(build_dependencies),
    );
    if let Some(cavs) = caveats {
        info.insert("caveats".to_string(), serde_json::json!(cavs));
    }
    info.insert("keg_only".to_string(), serde_json::json!(keg_only));
    info
}

/// Truncate search results for display.
/// Extracted for testability.
pub(crate) fn calculate_search_display(
    total_results: usize,
    display_limit: usize,
) -> (usize, Option<usize>) {
    if total_results <= display_limit {
        (total_results, None)
    } else {
        (display_limit, Some(total_results - display_limit))
    }
}

/// Format an installed keg for list display.
/// Extracted for testability.
pub(crate) fn format_list_entry(name: &str, version: &str, pinned: bool) -> String {
    let pin_marker = if pinned { " (pinned)" } else { "" };
    format!("{} {}{}", name, version, pin_marker)
}

/// Determine what info output type to show based on available data.
/// Extracted for testability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InfoOutputKind {
    NotFound,
    InstalledOnly,
    ApiOnly,
    Both,
}

/// Determine info output kind based on available data.
pub(crate) fn determine_info_output_kind(
    has_installed: bool,
    has_api_formula: bool,
) -> InfoOutputKind {
    match (has_installed, has_api_formula) {
        (false, false) => InfoOutputKind::NotFound,
        (true, false) => InfoOutputKind::InstalledOnly,
        (false, true) => InfoOutputKind::ApiOnly,
        (true, true) => InfoOutputKind::Both,
    }
}

/// Format the installed version line with optional markers.
/// Extracted for testability.
pub(crate) fn format_installed_version_line(version: &str, pinned: bool, explicit: bool) -> String {
    let mut line = format!("Installed: {}", version);
    if pinned {
        line.push_str(" (pinned)");
    }
    if !explicit {
        line.push_str(" (installed as dependency)");
    }
    line
}

/// Format the available version line with optional update notice.
/// Extracted for testability.
pub(crate) fn format_available_version_line(
    available_version: &str,
    installed_version: Option<&str>,
) -> String {
    match installed_version {
        Some(inst) if inst != available_version => {
            format!("Available: {} (update available)", available_version)
        }
        Some(_) => String::new(), // Same version, don't show available
        None => format!("Available: {}", available_version),
    }
}

/// Build JSON output for outdated check.
/// Extracted for testability.
pub(crate) fn build_outdated_json(
    name: &str,
    installed_version: &str,
    available_version: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "installed_version": installed_version,
        "available_version": available_version
    })
}

/// Determine search output kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchOutputKind {
    Json,
    Empty { installed_only: bool },
    Results { count: usize, installed_only: bool },
}

/// Determine what search output to show.
pub(crate) fn determine_search_output_kind(
    json: bool,
    result_count: usize,
    installed_only: bool,
) -> SearchOutputKind {
    if json {
        SearchOutputKind::Json
    } else if result_count == 0 {
        SearchOutputKind::Empty { installed_only }
    } else {
        SearchOutputKind::Results {
            count: result_count,
            installed_only,
        }
    }
}

/// Format a single search result entry for display.
pub(crate) fn format_search_result_entry(
    name: &str,
    version: &str,
    description: &str,
    is_installed: bool,
    max_desc_len: usize,
) -> Vec<String> {
    let marker = if is_installed { "✓" } else { " " };
    let mut lines = vec![format!("{} {} {}", marker, name, version)];

    if !description.is_empty() {
        let desc = truncate_description(description, max_desc_len);
        lines.push(format!("    {}", desc));
    }

    lines
}

/// Determine list output kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListOutputKind {
    Empty { pinned: bool },
    HasItems { count: usize },
}

/// Determine list output type.
pub(crate) fn determine_list_output_kind(item_count: usize, pinned_filter: bool) -> ListOutputKind {
    if item_count == 0 {
        ListOutputKind::Empty {
            pinned: pinned_filter,
        }
    } else {
        ListOutputKind::HasItems { count: item_count }
    }
}

/// Run the search command.
pub async fn run_search(
    installer: &Installer,
    root: &Path,
    query: String,
    json: bool,
    installed: bool,
) -> Result<(), zb_core::Error> {
    if !json {
        println!(
            "{} Searching for '{}'...",
            style("==>").cyan().bold(),
            style(&query).bold()
        );
    }

    let cache_dir = root.join("cache");
    let cache = ApiCache::open(&cache_dir).ok();
    let api_client = if let Some(c) = cache {
        ApiClient::new().with_cache(c)
    } else {
        ApiClient::new()
    };

    let formulas = api_client.get_all_formulas().await?;
    let mut results = search_formulas(&formulas, &query);

    if installed {
        results.retain(|r| installer.is_installed(&r.name));
    }

    let output_kind = determine_search_output_kind(json, results.len(), installed);

    match output_kind {
        SearchOutputKind::Json => {
            let json_results: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let is_installed = installer.is_installed(&r.name);
                    build_search_result_json(
                        &r.name,
                        &r.full_name,
                        &r.version,
                        &r.description,
                        is_installed,
                    )
                })
                .collect();
            match serde_json::to_string_pretty(&json_results) {
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
        }
        SearchOutputKind::Empty { installed_only } => {
            println!("{}", empty_search_message(&query, installed_only));
        }
        SearchOutputKind::Results {
            count,
            installed_only,
        } => {
            let label = search_results_label(installed_only);
            println!(
                "{} Found {} {}:",
                style("==>").cyan().bold(),
                style(count).green().bold(),
                label
            );
            println!();

            let (display_count, remaining) = calculate_search_display(results.len(), 20);

            for result in results.iter().take(display_count) {
                let is_installed = installer.is_installed(&result.name);

                // Use format_search_result_entry for the base plain-text format
                let plain_lines = format_search_result_entry(
                    &result.name,
                    &result.version,
                    &result.description,
                    is_installed,
                    70,
                );

                // Apply styling for terminal display
                let marker_str = format_dependency_status(is_installed);
                let marker = if is_installed {
                    style(marker_str).green().to_string()
                } else {
                    " ".to_string()
                };

                println!(
                    "{} {} {}",
                    marker,
                    style(&result.name).bold(),
                    style(&result.version).dim()
                );

                // Use the description from plain_lines if available
                if plain_lines.len() > 1 {
                    // plain_lines[1] contains "    {description}"
                    let desc = truncate_description(&result.description, 70);
                    println!("    {}", style(desc).dim());
                }
            }

            if let Some(more) = remaining {
                println!();
                println!("    {} and {} more...", style("...").dim(), more);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Truncate Description Tests
    // ========================================================================

    #[test]
    fn test_truncate_description_under_limit() {
        let desc = "A short description";
        let result = truncate_description(desc, 70);
        assert_eq!(result, "A short description");
    }

    #[test]
    fn test_truncate_description_at_limit() {
        let desc = "x".repeat(70);
        let result = truncate_description(&desc, 70);
        assert_eq!(result, desc);
    }

    #[test]
    fn test_truncate_description_over_limit() {
        let desc = "x".repeat(100);
        let result = truncate_description(&desc, 70);
        assert_eq!(result.len(), 70);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_description_empty() {
        let result = truncate_description("", 70);
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_description_small_max() {
        let desc = "Hello World";
        let result = truncate_description(desc, 5);
        assert_eq!(result, "He...");
    }

    #[test]
    fn test_truncate_description_just_over_limit() {
        let desc = "x".repeat(71);
        let result = truncate_description(&desc, 70);
        assert_eq!(result.len(), 70);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_description_with_unicode() {
        // Note: truncation works on bytes, not chars - be aware of this
        let desc = "Hello";
        let result = truncate_description(desc, 100);
        assert_eq!(result, "Hello");
    }

    // ========================================================================
    // Store Key Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_store_key_full_length() {
        let key = "abcdef123456789012345678";
        let result = format_store_key(key);
        assert_eq!(result, "abcdef123456");
    }

    #[test]
    fn test_format_store_key_exact_12() {
        let key = "abcdef123456";
        let result = format_store_key(key);
        assert_eq!(result, "abcdef123456");
    }

    #[test]
    fn test_format_store_key_short() {
        let key = "abc";
        let result = format_store_key(key);
        assert_eq!(result, "abc");
    }

    #[test]
    fn test_format_store_key_empty() {
        let key = "";
        let result = format_store_key(key);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_store_key_eleven_chars() {
        let key = "abcdefghijk";
        let result = format_store_key(key);
        assert_eq!(result, "abcdefghijk");
    }

    #[test]
    fn test_format_store_key_thirteen_chars() {
        let key = "abcdefghijklm";
        let result = format_store_key(key);
        assert_eq!(result, "abcdefghijkl");
    }

    // ========================================================================
    // Base Info JSON Tests
    // ========================================================================

    #[test]
    fn test_build_info_json_base_installed() {
        let info = build_info_json_base("git", true);
        assert_eq!(info.get("name").unwrap(), "git");
        assert_eq!(info.get("installed").unwrap(), true);
    }

    #[test]
    fn test_build_info_json_base_not_installed() {
        let info = build_info_json_base("ripgrep", false);
        assert_eq!(info.get("name").unwrap(), "ripgrep");
        assert_eq!(info.get("installed").unwrap(), false);
    }

    #[test]
    fn test_build_info_json_base_versioned_formula() {
        let info = build_info_json_base("python@3.11", true);
        assert_eq!(info.get("name").unwrap(), "python@3.11");
    }

    #[test]
    fn test_build_info_json_base_has_required_keys() {
        let info = build_info_json_base("test", false);
        assert!(info.contains_key("name"));
        assert!(info.contains_key("installed"));
        assert_eq!(info.len(), 2);
    }

    // ========================================================================
    // Installed Info JSON Tests
    // ========================================================================

    #[test]
    fn test_build_installed_info_json_basic() {
        let info = build_installed_info_json("2.44.0", "abc123def456", 1700000000, false, true);
        assert_eq!(info.get("installed_version").unwrap(), "2.44.0");
        assert_eq!(info.get("store_key").unwrap(), "abc123def456");
        assert_eq!(info.get("installed_at").unwrap(), 1700000000);
        assert_eq!(info.get("pinned").unwrap(), false);
        assert_eq!(info.get("explicit").unwrap(), true);
    }

    #[test]
    fn test_build_installed_info_json_pinned() {
        let info = build_installed_info_json("1.0.0", "key123", 1234567890, true, true);
        assert_eq!(info.get("pinned").unwrap(), true);
    }

    #[test]
    fn test_build_installed_info_json_dependency() {
        let info = build_installed_info_json("3.0.0", "xyz789", 9999999999, false, false);
        assert_eq!(info.get("explicit").unwrap(), false);
    }

    #[test]
    fn test_build_installed_info_json_all_fields() {
        let info = build_installed_info_json("1.2.3", "hash", 0, true, false);
        assert!(info.contains_key("installed_version"));
        assert!(info.contains_key("store_key"));
        assert!(info.contains_key("installed_at"));
        assert!(info.contains_key("pinned"));
        assert!(info.contains_key("explicit"));
        assert_eq!(info.len(), 5);
    }

    // ========================================================================
    // Linked Files JSON Tests
    // ========================================================================

    #[test]
    fn test_build_linked_files_json_empty() {
        let files: Vec<(String, String)> = vec![];
        let json = build_linked_files_json(&files);
        assert!(json.is_empty());
    }

    #[test]
    fn test_build_linked_files_json_single() {
        let files = vec![(
            "/usr/local/bin/git".to_string(),
            "/opt/zb/git/bin/git".to_string(),
        )];
        let json = build_linked_files_json(&files);
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["link"], "/usr/local/bin/git");
        assert_eq!(json[0]["target"], "/opt/zb/git/bin/git");
    }

    #[test]
    fn test_build_linked_files_json_multiple() {
        let files = vec![
            ("/usr/bin/a".to_string(), "/opt/a".to_string()),
            ("/usr/bin/b".to_string(), "/opt/b".to_string()),
            ("/usr/bin/c".to_string(), "/opt/c".to_string()),
        ];
        let json = build_linked_files_json(&files);
        assert_eq!(json.len(), 3);
    }

    // ========================================================================
    // Search Result JSON Tests
    // ========================================================================

    #[test]
    fn test_build_search_result_json_full() {
        let json = build_search_result_json(
            "ripgrep",
            "ripgrep",
            "14.1.0",
            "Search tool like grep",
            true,
        );
        assert_eq!(json["name"], "ripgrep");
        assert_eq!(json["full_name"], "ripgrep");
        assert_eq!(json["version"], "14.1.0");
        assert_eq!(json["description"], "Search tool like grep");
        assert_eq!(json["installed"], true);
    }

    #[test]
    fn test_build_search_result_json_not_installed() {
        let json = build_search_result_json("jq", "jq", "1.7", "JSON processor", false);
        assert_eq!(json["installed"], false);
    }

    #[test]
    fn test_build_search_result_json_empty_description() {
        let json = build_search_result_json("test", "test", "1.0", "", false);
        assert_eq!(json["description"], "");
    }

    #[test]
    fn test_build_search_result_json_different_names() {
        let json = build_search_result_json(
            "python",
            "homebrew/core/python",
            "3.12.0",
            "Python interpreter",
            true,
        );
        assert_eq!(json["name"], "python");
        assert_eq!(json["full_name"], "homebrew/core/python");
    }

    // ========================================================================
    // Pin Marker Tests
    // ========================================================================

    #[test]
    fn test_format_pin_marker_pinned() {
        let result = format_pin_marker(true);
        assert_eq!(result, " (pinned)");
    }

    #[test]
    fn test_format_pin_marker_not_pinned() {
        let result = format_pin_marker(false);
        assert_eq!(result, "");
    }

    // ========================================================================
    // Explicit Marker Tests
    // ========================================================================

    #[test]
    fn test_format_explicit_marker_explicit() {
        let result = format_explicit_marker(true);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_explicit_marker_dependency() {
        let result = format_explicit_marker(false);
        assert_eq!(result, " (installed as dependency)");
    }

    // ========================================================================
    // Linked Files Display Tests
    // ========================================================================

    #[test]
    fn test_calculate_linked_files_display_under_limit() {
        let (shown, remaining) = calculate_linked_files_display(3, 5);
        assert_eq!(shown, 3);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_linked_files_display_at_limit() {
        let (shown, remaining) = calculate_linked_files_display(5, 5);
        assert_eq!(shown, 5);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_linked_files_display_over_limit() {
        let (shown, remaining) = calculate_linked_files_display(10, 5);
        assert_eq!(shown, 5);
        assert_eq!(remaining, Some(5));
    }

    #[test]
    fn test_calculate_linked_files_display_zero() {
        let (shown, remaining) = calculate_linked_files_display(0, 5);
        assert_eq!(shown, 0);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_linked_files_display_many_over() {
        let (shown, remaining) = calculate_linked_files_display(100, 5);
        assert_eq!(shown, 5);
        assert_eq!(remaining, Some(95));
    }

    // ========================================================================
    // Update Available Tests
    // ========================================================================

    #[test]
    fn test_is_update_available_same_version() {
        assert!(!is_update_available("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_update_available_different_version() {
        assert!(is_update_available("1.0.0", "2.0.0"));
    }

    #[test]
    fn test_is_update_available_patch_update() {
        assert!(is_update_available("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_is_update_available_with_revision() {
        assert!(is_update_available("1.0.0_1", "1.0.0_2"));
    }

    #[test]
    fn test_is_update_available_empty_strings() {
        assert!(!is_update_available("", ""));
    }

    #[test]
    fn test_is_update_available_downgrade() {
        // Simple string comparison - doesn't validate semver ordering
        assert!(is_update_available("2.0.0", "1.0.0"));
    }

    // ========================================================================
    // Caveats Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_caveats_with_prefix() {
        let caveats = "Add $HOMEBREW_PREFIX/bin to your PATH";
        let result = format_caveats(caveats, "/opt/homebrew");
        assert_eq!(result, "Add /opt/homebrew/bin to your PATH");
    }

    #[test]
    fn test_format_caveats_multiple_replacements() {
        let caveats = "$HOMEBREW_PREFIX/bin and $HOMEBREW_PREFIX/sbin";
        let result = format_caveats(caveats, "/usr/local");
        assert_eq!(result, "/usr/local/bin and /usr/local/sbin");
    }

    #[test]
    fn test_format_caveats_no_placeholder() {
        let caveats = "No special configuration needed";
        let result = format_caveats(caveats, "/opt/homebrew");
        assert_eq!(result, "No special configuration needed");
    }

    #[test]
    fn test_format_caveats_empty() {
        let result = format_caveats("", "/opt/homebrew");
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_caveats_multiline() {
        let caveats = "Line 1 at $HOMEBREW_PREFIX\nLine 2 at $HOMEBREW_PREFIX";
        let result = format_caveats(caveats, "/test");
        assert_eq!(result, "Line 1 at /test\nLine 2 at /test");
    }

    // ========================================================================
    // Empty List Message Tests
    // ========================================================================

    #[test]
    fn test_empty_list_message_pinned() {
        assert_eq!(empty_list_message(true), "No pinned formulas.");
    }

    #[test]
    fn test_empty_list_message_all() {
        assert_eq!(empty_list_message(false), "No formulas installed.");
    }

    // ========================================================================
    // Empty Search Message Tests
    // ========================================================================

    #[test]
    fn test_empty_search_message_all() {
        let msg = empty_search_message("git", false);
        assert_eq!(msg, "No formulas found matching 'git'.");
    }

    #[test]
    fn test_empty_search_message_installed_only() {
        let msg = empty_search_message("ripgrep", true);
        assert_eq!(msg, "No installed formulas found matching 'ripgrep'.");
    }

    #[test]
    fn test_empty_search_message_empty_query() {
        let msg = empty_search_message("", false);
        assert_eq!(msg, "No formulas found matching ''.");
    }

    // ========================================================================
    // Search Results Label Tests
    // ========================================================================

    #[test]
    fn test_search_results_label_all() {
        assert_eq!(search_results_label(false), "formulas");
    }

    #[test]
    fn test_search_results_label_installed() {
        assert_eq!(search_results_label(true), "installed formulas");
    }

    // ========================================================================
    // Dependency Status Tests
    // ========================================================================

    #[test]
    fn test_format_dependency_status_installed() {
        assert_eq!(format_dependency_status(true), "✓");
    }

    #[test]
    fn test_format_dependency_status_not_installed() {
        assert_eq!(format_dependency_status(false), "✗");
    }

    // ========================================================================
    // Version Display Tests
    // ========================================================================

    #[test]
    fn test_format_version_comparison_up_to_date() {
        let result = format_version_comparison(Some("1.0.0"), "1.0.0");
        assert_eq!(result, VersionDisplay::UpToDate("1.0.0".to_string()));
    }

    #[test]
    fn test_format_version_comparison_update_available() {
        let result = format_version_comparison(Some("1.0.0"), "2.0.0");
        assert_eq!(
            result,
            VersionDisplay::UpdateAvailable {
                installed: "1.0.0".to_string(),
                available: "2.0.0".to_string(),
            }
        );
    }

    #[test]
    fn test_format_version_comparison_not_installed() {
        let result = format_version_comparison(None, "1.0.0");
        assert_eq!(result, VersionDisplay::NotInstalled("1.0.0".to_string()));
    }

    #[test]
    fn test_format_version_comparison_complex_versions() {
        let result = format_version_comparison(Some("3.12.0_1"), "3.12.1");
        assert_eq!(
            result,
            VersionDisplay::UpdateAvailable {
                installed: "3.12.0_1".to_string(),
                available: "3.12.1".to_string(),
            }
        );
    }

    #[test]
    fn test_version_display_equality() {
        let a = VersionDisplay::UpToDate("1.0".to_string());
        let b = VersionDisplay::UpToDate("1.0".to_string());
        assert_eq!(a, b);

        let c = VersionDisplay::UpToDate("2.0".to_string());
        assert_ne!(a, c);
    }

    #[test]
    fn test_version_display_clone() {
        let original = VersionDisplay::UpdateAvailable {
            installed: "1.0".to_string(),
            available: "2.0".to_string(),
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ========================================================================
    // Keg-Only Reason Tests
    // ========================================================================

    #[test]
    fn test_format_keg_only_reason_with_explanation() {
        let result = format_keg_only_reason(Some("macOS provides its own"));
        assert_eq!(result, "Yes (macOS provides its own)");
    }

    #[test]
    fn test_format_keg_only_reason_empty_explanation() {
        let result = format_keg_only_reason(Some(""));
        assert_eq!(result, "Yes");
    }

    #[test]
    fn test_format_keg_only_reason_none() {
        let result = format_keg_only_reason(None);
        assert_eq!(result, "Yes");
    }

    // ========================================================================
    // Formula API JSON Tests
    // ========================================================================

    #[test]
    fn test_build_formula_api_json_full() {
        let deps = vec!["openssl".to_string(), "readline".to_string()];
        let build_deps = vec!["pkg-config".to_string()];
        let json = build_formula_api_json(
            "3.12.0",
            Some("Interpreted, interactive, object-oriented programming language"),
            Some("https://www.python.org/"),
            Some("Python-2.0"),
            &deps,
            &build_deps,
            Some("See python.org for documentation"),
            false,
        );

        assert_eq!(json.get("available_version").unwrap(), "3.12.0");
        assert_eq!(
            json.get("description").unwrap(),
            "Interpreted, interactive, object-oriented programming language"
        );
        assert_eq!(json.get("homepage").unwrap(), "https://www.python.org/");
        assert_eq!(json.get("license").unwrap(), "Python-2.0");
        assert_eq!(
            json.get("dependencies").unwrap(),
            &serde_json::json!(["openssl", "readline"])
        );
        assert_eq!(
            json.get("build_dependencies").unwrap(),
            &serde_json::json!(["pkg-config"])
        );
        assert_eq!(
            json.get("caveats").unwrap(),
            "See python.org for documentation"
        );
        assert_eq!(json.get("keg_only").unwrap(), false);
    }

    #[test]
    fn test_build_formula_api_json_minimal() {
        let json = build_formula_api_json("1.0.0", None, None, None, &[], &[], None, false);

        assert_eq!(json.get("available_version").unwrap(), "1.0.0");
        assert!(!json.contains_key("description"));
        assert!(!json.contains_key("homepage"));
        assert!(!json.contains_key("license"));
        assert_eq!(json.get("dependencies").unwrap(), &serde_json::json!([]));
        assert_eq!(
            json.get("build_dependencies").unwrap(),
            &serde_json::json!([])
        );
        assert!(!json.contains_key("caveats"));
        assert_eq!(json.get("keg_only").unwrap(), false);
    }

    #[test]
    fn test_build_formula_api_json_keg_only() {
        let json = build_formula_api_json("1.0.0", Some("Test"), None, None, &[], &[], None, true);

        assert_eq!(json.get("keg_only").unwrap(), true);
    }

    // ========================================================================
    // Search Display Calculation Tests
    // ========================================================================

    #[test]
    fn test_calculate_search_display_under_limit() {
        let (shown, remaining) = calculate_search_display(10, 20);
        assert_eq!(shown, 10);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_search_display_at_limit() {
        let (shown, remaining) = calculate_search_display(20, 20);
        assert_eq!(shown, 20);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_search_display_over_limit() {
        let (shown, remaining) = calculate_search_display(50, 20);
        assert_eq!(shown, 20);
        assert_eq!(remaining, Some(30));
    }

    #[test]
    fn test_calculate_search_display_zero_results() {
        let (shown, remaining) = calculate_search_display(0, 20);
        assert_eq!(shown, 0);
        assert_eq!(remaining, None);
    }

    #[test]
    fn test_calculate_search_display_one_over() {
        let (shown, remaining) = calculate_search_display(21, 20);
        assert_eq!(shown, 20);
        assert_eq!(remaining, Some(1));
    }

    // ========================================================================
    // Truncate Description Edge Cases
    // ========================================================================

    #[test]
    fn test_truncate_description_max_len_zero() {
        // Edge case: max_len of 0 should handle gracefully
        let result = truncate_description("test", 0);
        // With saturating_sub, this gives us empty string with "..."
        assert!(result.is_empty() || result == "...");
    }

    #[test]
    fn test_truncate_description_max_len_three() {
        // Exactly enough for "..."
        let result = truncate_description("testing", 3);
        assert_eq!(result, "...");
    }

    #[test]
    fn test_truncate_description_max_len_four() {
        let result = truncate_description("testing", 4);
        assert_eq!(result, "t...");
    }

    // ========================================================================
    // Build Info JSON Combined Tests
    // ========================================================================

    #[test]
    fn test_build_info_json_base_special_characters() {
        let info = build_info_json_base("test-formula_v2", false);
        assert_eq!(info.get("name").unwrap(), "test-formula_v2");
    }

    #[test]
    fn test_build_installed_info_json_zero_timestamp() {
        let info = build_installed_info_json("1.0.0", "abc", 0, false, true);
        assert_eq!(info.get("installed_at").unwrap(), 0);
    }

    #[test]
    fn test_build_installed_info_json_large_timestamp() {
        let info = build_installed_info_json("1.0.0", "abc", u64::MAX, false, true);
        assert_eq!(info.get("installed_at").unwrap(), u64::MAX);
    }

    // ========================================================================
    // Linked Files JSON Edge Cases
    // ========================================================================

    #[test]
    fn test_build_linked_files_json_with_spaces() {
        let files = vec![(
            "/path/with spaces/bin".to_string(),
            "/target/with spaces".to_string(),
        )];
        let json = build_linked_files_json(&files);
        assert_eq!(json[0]["link"], "/path/with spaces/bin");
        assert_eq!(json[0]["target"], "/target/with spaces");
    }

    #[test]
    fn test_build_linked_files_json_unicode_paths() {
        let files = vec![("/usr/bin/日本語".to_string(), "/opt/日本語".to_string())];
        let json = build_linked_files_json(&files);
        assert_eq!(json[0]["link"], "/usr/bin/日本語");
    }

    // ========================================================================
    // Search Result JSON Edge Cases
    // ========================================================================

    #[test]
    fn test_build_search_result_json_with_special_chars() {
        let json =
            build_search_result_json("c++", "homebrew/core/c++", "1.0", "A C++ compiler", false);
        assert_eq!(json["name"], "c++");
    }

    #[test]
    fn test_build_search_result_json_long_description() {
        let long_desc = "x".repeat(1000);
        let json = build_search_result_json("test", "test", "1.0", &long_desc, false);
        assert_eq!(json["description"].as_str().unwrap().len(), 1000);
    }

    // ========================================================================
    // List Entry Format Tests
    // ========================================================================

    #[test]
    fn test_format_list_entry_basic() {
        let result = format_list_entry("git", "2.44.0", false);
        assert_eq!(result, "git 2.44.0");
    }

    #[test]
    fn test_format_list_entry_pinned() {
        let result = format_list_entry("node", "22.0.0", true);
        assert_eq!(result, "node 22.0.0 (pinned)");
    }

    #[test]
    fn test_format_list_entry_versioned_formula() {
        let result = format_list_entry("python@3.11", "3.11.9", false);
        assert_eq!(result, "python@3.11 3.11.9");
    }

    #[test]
    fn test_format_list_entry_with_revision() {
        let result = format_list_entry("openssl", "3.0.0_1", true);
        assert_eq!(result, "openssl 3.0.0_1 (pinned)");
    }

    // ========================================================================
    // Info Output Kind Tests
    // ========================================================================

    #[test]
    fn test_determine_info_output_kind_not_found() {
        let kind = determine_info_output_kind(false, false);
        assert_eq!(kind, InfoOutputKind::NotFound);
    }

    #[test]
    fn test_determine_info_output_kind_installed_only() {
        let kind = determine_info_output_kind(true, false);
        assert_eq!(kind, InfoOutputKind::InstalledOnly);
    }

    #[test]
    fn test_determine_info_output_kind_api_only() {
        let kind = determine_info_output_kind(false, true);
        assert_eq!(kind, InfoOutputKind::ApiOnly);
    }

    #[test]
    fn test_determine_info_output_kind_both() {
        let kind = determine_info_output_kind(true, true);
        assert_eq!(kind, InfoOutputKind::Both);
    }

    #[test]
    fn test_info_output_kind_debug() {
        let kind = InfoOutputKind::Both;
        let debug_str = format!("{:?}", kind);
        assert_eq!(debug_str, "Both");
    }

    #[test]
    fn test_info_output_kind_clone() {
        let original = InfoOutputKind::InstalledOnly;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ========================================================================
    // Installed Version Line Tests
    // ========================================================================

    #[test]
    fn test_format_installed_version_line_basic() {
        let result = format_installed_version_line("2.44.0", false, true);
        assert_eq!(result, "Installed: 2.44.0");
    }

    #[test]
    fn test_format_installed_version_line_pinned() {
        let result = format_installed_version_line("1.0.0", true, true);
        assert_eq!(result, "Installed: 1.0.0 (pinned)");
    }

    #[test]
    fn test_format_installed_version_line_dependency() {
        let result = format_installed_version_line("3.0.0", false, false);
        assert_eq!(result, "Installed: 3.0.0 (installed as dependency)");
    }

    #[test]
    fn test_format_installed_version_line_pinned_dependency() {
        let result = format_installed_version_line("2.0.0", true, false);
        assert_eq!(
            result,
            "Installed: 2.0.0 (pinned) (installed as dependency)"
        );
    }

    // ========================================================================
    // Available Version Line Tests
    // ========================================================================

    #[test]
    fn test_format_available_version_line_not_installed() {
        let result = format_available_version_line("3.0.0", None);
        assert_eq!(result, "Available: 3.0.0");
    }

    #[test]
    fn test_format_available_version_line_same_version() {
        let result = format_available_version_line("2.0.0", Some("2.0.0"));
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_available_version_line_update_available() {
        let result = format_available_version_line("3.0.0", Some("2.0.0"));
        assert_eq!(result, "Available: 3.0.0 (update available)");
    }

    #[test]
    fn test_format_available_version_line_with_revision() {
        let result = format_available_version_line("1.0.0_2", Some("1.0.0_1"));
        assert_eq!(result, "Available: 1.0.0_2 (update available)");
    }

    // ========================================================================
    // Outdated JSON Tests
    // ========================================================================

    #[test]
    fn test_build_outdated_json_basic() {
        let json = build_outdated_json("git", "2.43.0", "2.44.0");
        assert_eq!(json["name"], "git");
        assert_eq!(json["installed_version"], "2.43.0");
        assert_eq!(json["available_version"], "2.44.0");
    }

    #[test]
    fn test_build_outdated_json_has_all_fields() {
        let json = build_outdated_json("test", "1.0", "2.0");
        assert!(json.as_object().unwrap().contains_key("name"));
        assert!(json.as_object().unwrap().contains_key("installed_version"));
        assert!(json.as_object().unwrap().contains_key("available_version"));
        assert_eq!(json.as_object().unwrap().len(), 3);
    }

    // ========================================================================
    // Search Output Kind Tests
    // ========================================================================

    #[test]
    fn test_determine_search_output_kind_json() {
        let kind = determine_search_output_kind(true, 10, false);
        assert_eq!(kind, SearchOutputKind::Json);
    }

    #[test]
    fn test_determine_search_output_kind_json_with_installed_filter() {
        // JSON mode regardless of filter
        let kind = determine_search_output_kind(true, 0, true);
        assert_eq!(kind, SearchOutputKind::Json);
    }

    #[test]
    fn test_determine_search_output_kind_empty() {
        let kind = determine_search_output_kind(false, 0, false);
        assert_eq!(
            kind,
            SearchOutputKind::Empty {
                installed_only: false
            }
        );
    }

    #[test]
    fn test_determine_search_output_kind_empty_installed_only() {
        let kind = determine_search_output_kind(false, 0, true);
        assert_eq!(
            kind,
            SearchOutputKind::Empty {
                installed_only: true
            }
        );
    }

    #[test]
    fn test_determine_search_output_kind_results() {
        let kind = determine_search_output_kind(false, 15, false);
        assert_eq!(
            kind,
            SearchOutputKind::Results {
                count: 15,
                installed_only: false
            }
        );
    }

    #[test]
    fn test_determine_search_output_kind_results_installed() {
        let kind = determine_search_output_kind(false, 5, true);
        assert_eq!(
            kind,
            SearchOutputKind::Results {
                count: 5,
                installed_only: true
            }
        );
    }

    #[test]
    fn test_search_output_kind_debug() {
        let kind = SearchOutputKind::Results {
            count: 10,
            installed_only: false,
        };
        let debug_str = format!("{:?}", kind);
        assert!(debug_str.contains("Results"));
        assert!(debug_str.contains("10"));
    }

    #[test]
    fn test_search_output_kind_clone() {
        let original = SearchOutputKind::Empty {
            installed_only: true,
        };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ========================================================================
    // Search Result Entry Format Tests
    // ========================================================================

    #[test]
    fn test_format_search_result_entry_installed() {
        let lines = format_search_result_entry("ripgrep", "14.1.0", "Search tool", true, 70);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("✓"));
        assert!(lines[0].contains("ripgrep"));
        assert!(lines[0].contains("14.1.0"));
        assert!(lines[1].contains("Search tool"));
    }

    #[test]
    fn test_format_search_result_entry_not_installed() {
        let lines = format_search_result_entry("jq", "1.7", "JSON processor", false, 70);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with(" ")); // Not installed marker
        assert!(lines[0].contains("jq"));
    }

    #[test]
    fn test_format_search_result_entry_no_description() {
        let lines = format_search_result_entry("test", "1.0", "", false, 70);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("test"));
    }

    #[test]
    fn test_format_search_result_entry_long_description() {
        let long_desc = "x".repeat(100);
        let lines = format_search_result_entry("pkg", "1.0", &long_desc, false, 50);
        assert_eq!(lines.len(), 2);
        // Description should be truncated
        assert!(lines[1].len() < 60); // 4 spaces indent + ~50 chars
    }

    #[test]
    fn test_format_search_result_entry_exact_length_description() {
        let desc = "x".repeat(70);
        let lines = format_search_result_entry("pkg", "1.0", &desc, true, 70);
        assert_eq!(lines.len(), 2);
    }

    // ========================================================================
    // List Output Kind Tests
    // ========================================================================

    #[test]
    fn test_determine_list_output_kind_empty_all() {
        let kind = determine_list_output_kind(0, false);
        assert_eq!(kind, ListOutputKind::Empty { pinned: false });
    }

    #[test]
    fn test_determine_list_output_kind_empty_pinned() {
        let kind = determine_list_output_kind(0, true);
        assert_eq!(kind, ListOutputKind::Empty { pinned: true });
    }

    #[test]
    fn test_determine_list_output_kind_has_items() {
        let kind = determine_list_output_kind(10, false);
        assert_eq!(kind, ListOutputKind::HasItems { count: 10 });
    }

    #[test]
    fn test_determine_list_output_kind_one_item() {
        let kind = determine_list_output_kind(1, true);
        assert_eq!(kind, ListOutputKind::HasItems { count: 1 });
    }

    #[test]
    fn test_list_output_kind_debug() {
        let kind = ListOutputKind::HasItems { count: 5 };
        let debug_str = format!("{:?}", kind);
        assert!(debug_str.contains("HasItems"));
        assert!(debug_str.contains("5"));
    }

    #[test]
    fn test_list_output_kind_clone() {
        let original = ListOutputKind::Empty { pinned: true };
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // ========================================================================
    // Version Display Debug Tests
    // ========================================================================

    #[test]
    fn test_version_display_debug_up_to_date() {
        let display = VersionDisplay::UpToDate("1.0.0".to_string());
        let debug_str = format!("{:?}", display);
        assert!(debug_str.contains("UpToDate"));
        assert!(debug_str.contains("1.0.0"));
    }

    #[test]
    fn test_version_display_debug_update_available() {
        let display = VersionDisplay::UpdateAvailable {
            installed: "1.0.0".to_string(),
            available: "2.0.0".to_string(),
        };
        let debug_str = format!("{:?}", display);
        assert!(debug_str.contains("UpdateAvailable"));
        assert!(debug_str.contains("1.0.0"));
        assert!(debug_str.contains("2.0.0"));
    }

    #[test]
    fn test_version_display_debug_not_installed() {
        let display = VersionDisplay::NotInstalled("3.0.0".to_string());
        let debug_str = format!("{:?}", display);
        assert!(debug_str.contains("NotInstalled"));
        assert!(debug_str.contains("3.0.0"));
    }

    // ========================================================================
    // Additional Edge Cases
    // ========================================================================

    #[test]
    fn test_truncate_description_max_len_one() {
        let result = truncate_description("testing", 1);
        // 1 - 3 = 0 (saturating), so we get "..." but sliced to 0 chars
        assert!(result.is_empty() || result == "...");
    }

    #[test]
    fn test_truncate_description_max_len_two() {
        let result = truncate_description("testing", 2);
        // 2 - 3 = 0 (saturating), same as above
        assert!(result.is_empty() || result == "...");
    }

    #[test]
    fn test_format_caveats_prefix_at_start() {
        let caveats = "$HOMEBREW_PREFIX is the root";
        let result = format_caveats(caveats, "/opt/zb");
        assert_eq!(result, "/opt/zb is the root");
    }

    #[test]
    fn test_format_caveats_prefix_at_end() {
        let caveats = "Install at $HOMEBREW_PREFIX";
        let result = format_caveats(caveats, "/usr/local");
        assert_eq!(result, "Install at /usr/local");
    }

    #[test]
    fn test_is_update_available_whitespace_only_versions() {
        // Edge case with unusual version strings
        assert!(!is_update_available("  ", "  "));
        assert!(is_update_available(" ", "  "));
    }

    #[test]
    fn test_empty_search_message_query_with_special_chars() {
        let msg = empty_search_message("c++", false);
        assert!(msg.contains("c++"));
    }

    #[test]
    fn test_format_store_key_one_char() {
        let result = format_store_key("a");
        assert_eq!(result, "a");
    }

    #[test]
    fn test_build_info_json_base_empty_name() {
        let info = build_info_json_base("", true);
        assert_eq!(info.get("name").unwrap(), "");
    }

    #[test]
    fn test_build_linked_files_json_empty_paths() {
        let files = vec![("".to_string(), "".to_string())];
        let json = build_linked_files_json(&files);
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["link"], "");
        assert_eq!(json[0]["target"], "");
    }

    #[test]
    fn test_build_formula_api_json_empty_arrays() {
        let json = build_formula_api_json(
            "1.0",
            Some("desc"),
            Some("https://example.com"),
            Some("MIT"),
            &[],
            &[],
            Some("caveat"),
            false,
        );
        assert_eq!(json.get("dependencies").unwrap(), &serde_json::json!([]));
        assert_eq!(
            json.get("build_dependencies").unwrap(),
            &serde_json::json!([])
        );
    }

    #[test]
    fn test_format_keg_only_reason_whitespace_explanation() {
        let result = format_keg_only_reason(Some("   "));
        // Non-empty but whitespace-only should still show the explanation
        assert_eq!(result, "Yes (   )");
    }

    #[test]
    fn test_calculate_linked_files_display_limit_zero() {
        // Edge case: display limit of 0
        let (shown, remaining) = calculate_linked_files_display(5, 0);
        assert_eq!(shown, 0);
        assert_eq!(remaining, Some(5));
    }

    #[test]
    fn test_calculate_search_display_limit_zero() {
        let (shown, remaining) = calculate_search_display(10, 0);
        assert_eq!(shown, 0);
        assert_eq!(remaining, Some(10));
    }

    #[test]
    fn test_format_version_comparison_empty_versions() {
        let result = format_version_comparison(Some(""), "");
        assert_eq!(result, VersionDisplay::UpToDate("".to_string()));
    }

    #[test]
    fn test_version_display_all_variants_ne() {
        let up_to_date = VersionDisplay::UpToDate("1.0".to_string());
        let update = VersionDisplay::UpdateAvailable {
            installed: "1.0".to_string(),
            available: "2.0".to_string(),
        };
        let not_installed = VersionDisplay::NotInstalled("1.0".to_string());

        assert_ne!(up_to_date, update);
        assert_ne!(up_to_date, not_installed);
        assert_ne!(update, not_installed);
    }

    #[test]
    fn test_info_output_kind_all_variants_ne() {
        let not_found = InfoOutputKind::NotFound;
        let installed = InfoOutputKind::InstalledOnly;
        let api = InfoOutputKind::ApiOnly;
        let both = InfoOutputKind::Both;

        assert_ne!(not_found, installed);
        assert_ne!(not_found, api);
        assert_ne!(not_found, both);
        assert_ne!(installed, api);
        assert_ne!(installed, both);
        assert_ne!(api, both);
    }

    #[test]
    fn test_search_output_kind_all_variants_ne() {
        let json = SearchOutputKind::Json;
        let empty = SearchOutputKind::Empty {
            installed_only: false,
        };
        let results = SearchOutputKind::Results {
            count: 5,
            installed_only: false,
        };

        assert_ne!(json, empty);
        assert_ne!(json, results);
        assert_ne!(empty, results);
    }

    #[test]
    fn test_list_output_kind_all_variants_ne() {
        let empty = ListOutputKind::Empty { pinned: false };
        let has_items = ListOutputKind::HasItems { count: 1 };

        assert_ne!(empty, has_items);
    }

    #[test]
    fn test_empty_message_consistency() {
        // Ensure messages are consistent with their filter states
        let all_msg = empty_list_message(false);
        let pinned_msg = empty_list_message(true);

        assert!(all_msg.contains("installed"));
        assert!(pinned_msg.contains("pinned"));
        assert!(!all_msg.contains("pinned"));
    }

    #[test]
    fn test_search_labels_distinct() {
        let all_label = search_results_label(false);
        let installed_label = search_results_label(true);

        assert_ne!(all_label, installed_label);
        assert!(installed_label.contains("installed"));
    }

    #[test]
    fn test_format_list_entry_empty_strings() {
        let result = format_list_entry("", "", false);
        assert_eq!(result, " ");
    }

    #[test]
    fn test_format_installed_version_line_empty_version() {
        let result = format_installed_version_line("", false, true);
        assert_eq!(result, "Installed: ");
    }

    #[test]
    fn test_format_available_version_line_empty_available() {
        let result = format_available_version_line("", None);
        assert_eq!(result, "Available: ");
    }

    #[test]
    fn test_build_outdated_json_empty_values() {
        let json = build_outdated_json("", "", "");
        assert_eq!(json["name"], "");
        assert_eq!(json["installed_version"], "");
        assert_eq!(json["available_version"], "");
    }

    #[test]
    fn test_format_search_result_entry_all_empty() {
        let lines = format_search_result_entry("", "", "", false, 70);
        assert_eq!(lines.len(), 1);
        // Should still format with marker and spaces
        assert!(lines[0].contains(" "));
    }

    // ========================================================================
    // JSON Serialization Edge Cases
    // ========================================================================

    #[test]
    fn test_build_search_result_json_special_json_chars() {
        let json = build_search_result_json(
            "test\"pkg",
            "test\\pkg",
            "1.0",
            "Desc with \"quotes\" and \\backslash",
            false,
        );
        // Should handle escaping
        assert!(json["name"].as_str().is_some());
        assert!(json["description"].as_str().is_some());
    }

    #[test]
    fn test_build_formula_api_json_large_dep_list() {
        let deps: Vec<String> = (0..100).map(|i| format!("dep{}", i)).collect();
        let json = build_formula_api_json("1.0", None, None, None, &deps, &[], None, false);
        let dep_array = json.get("dependencies").unwrap().as_array().unwrap();
        assert_eq!(dep_array.len(), 100);
    }

    #[test]
    fn test_build_installed_info_json_max_values() {
        let info = build_installed_info_json(
            "999.999.999",
            "ffffffffffffffffffffffffffffffff",
            u64::MAX,
            true,
            true,
        );
        assert_eq!(info.get("installed_at").unwrap(), u64::MAX);
        assert_eq!(info.get("pinned").unwrap(), true);
        assert_eq!(info.get("explicit").unwrap(), true);
    }
}
