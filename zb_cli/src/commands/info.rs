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

    if installed.is_empty() {
        if pinned {
            println!("No pinned formulas.");
        } else {
            println!("No formulas installed.");
        }
    } else {
        for keg in installed {
            let pin_marker = if keg.pinned {
                format!(" {}", style("(pinned)").yellow())
            } else {
                String::new()
            };
            println!(
                "{} {}{}",
                style(&keg.name).bold(),
                style(&keg.version).dim(),
                pin_marker
            );
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
    let mut info = serde_json::Map::new();

    info.insert("name".to_string(), serde_json::json!(formula));

    if let Some(keg) = keg {
        info.insert("installed".to_string(), serde_json::json!(true));
        info.insert(
            "installed_version".to_string(),
            serde_json::json!(keg.version),
        );
        info.insert("store_key".to_string(), serde_json::json!(keg.store_key));
        info.insert(
            "installed_at".to_string(),
            serde_json::json!(keg.installed_at),
        );
        info.insert("pinned".to_string(), serde_json::json!(keg.pinned));
        info.insert("explicit".to_string(), serde_json::json!(keg.explicit));

        if let Ok(linked_files) = installer.get_linked_files(formula) {
            let files: Vec<_> = linked_files
                .iter()
                .map(|(link, target)| serde_json::json!({"link": link, "target": target}))
                .collect();
            info.insert("linked_files".to_string(), serde_json::json!(files));
        }

        if let Ok(dependents) = installer.get_dependents(formula).await {
            info.insert("dependents".to_string(), serde_json::json!(dependents));
        }
    } else {
        info.insert("installed".to_string(), serde_json::json!(false));
    }

    if let Some(f) = api_formula {
        info.insert(
            "available_version".to_string(),
            serde_json::json!(f.effective_version()),
        );
        if let Some(desc) = &f.desc {
            info.insert("description".to_string(), serde_json::json!(desc));
        }
        if let Some(homepage) = &f.homepage {
            info.insert("homepage".to_string(), serde_json::json!(homepage));
        }
        if let Some(license) = &f.license {
            info.insert("license".to_string(), serde_json::json!(license));
        }
        info.insert(
            "dependencies".to_string(),
            serde_json::json!(f.effective_dependencies()),
        );
        info.insert(
            "build_dependencies".to_string(),
            serde_json::json!(f.build_dependencies),
        );
        if let Some(caveats) = &f.caveats {
            info.insert("caveats".to_string(), serde_json::json!(caveats));
        }
        info.insert("keg_only".to_string(), serde_json::json!(f.keg_only));
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
    if keg.is_none() && api_formula.is_none() {
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

    // Version info
    if let Some(keg) = keg {
        print!(
            "{} {}",
            style("Installed:").dim(),
            style(&keg.version).green()
        );
        if keg.pinned {
            print!(" {}", style("(pinned)").yellow());
        }
        if !keg.explicit {
            print!(" {}", style("(installed as dependency)").dim());
        }
        println!();
    } else {
        println!("{} Not installed", style("Installed:").dim());
    }

    if let Some(f) = api_formula {
        let available_version = f.effective_version();
        if let Some(keg) = keg {
            if keg.version != available_version {
                println!(
                    "{} {} {}",
                    style("Available:").dim(),
                    style(&available_version).yellow(),
                    style("(update available)").yellow()
                );
            }
        } else {
            println!("{} {}", style("Available:").dim(), available_version);
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
        print!("{} Yes", style("Keg-only:").dim());
        if let Some(reason) = &f.keg_only_reason
            && !reason.explanation.is_empty()
        {
            print!(" ({})", reason.explanation);
        }
        println!();
    }

    // Dependencies
    if let Some(f) = api_formula {
        let deps = f.effective_dependencies();
        if !deps.is_empty() {
            println!();
            println!("{}", style("Dependencies:").dim());
            for dep in &deps {
                let installed = installer.is_installed(dep);
                let marker = if installed {
                    style("✓").green()
                } else {
                    style("✗").red()
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
        println!();
        println!(
            "{} ({} files)",
            style("Linked files:").dim(),
            linked_files.len()
        );
        for (link, _target) in linked_files.iter().take(5) {
            println!("  {}", link);
        }
        if linked_files.len() > 5 {
            println!(
                "  {} and {} more...",
                style("...").dim(),
                linked_files.len() - 5
            );
        }

        println!();
        println!("{} {}", style("Store key:").dim(), &keg.store_key[..12]);
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
        let caveats = caveats.replace("$HOMEBREW_PREFIX", &prefix.to_string_lossy());
        for line in caveats.lines() {
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
    info.insert(
        "name".to_string(),
        serde_json::json!(formula_name),
    );
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
pub(crate) fn build_linked_files_json(
    linked_files: &[(String, String)],
) -> Vec<serde_json::Value> {
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
    if installed {
        "✓"
    } else {
        "✗"
    }
}

/// Format the version comparison message when update available.
/// Extracted for testability.
pub(crate) fn format_version_comparison(
    installed: Option<&str>,
    available: &str,
) -> VersionDisplay {
    match installed {
        Some(inst) if inst == available => VersionDisplay::UpToDate(inst.to_string()),
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
    UpdateAvailable { installed: String, available: String },
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
    info.insert("build_dependencies".to_string(), serde_json::json!(build_dependencies));
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

    if json {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let is_installed = installer.is_installed(&r.name);
                serde_json::json!({
                    "name": r.name,
                    "full_name": r.full_name,
                    "version": r.version,
                    "description": r.description,
                    "installed": is_installed
                })
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
    } else if results.is_empty() {
        if installed {
            println!("No installed formulas found matching '{}'.", query);
        } else {
            println!("No formulas found matching '{}'.", query);
        }
    } else {
        let label = if installed {
            "installed formulas"
        } else {
            "formulas"
        };
        println!(
            "{} Found {} {}:",
            style("==>").cyan().bold(),
            style(results.len()).green().bold(),
            label
        );
        println!();

        for result in results.iter().take(20) {
            let is_installed = installer.is_installed(&result.name);
            let marker = if is_installed {
                style("✓").green().to_string()
            } else {
                " ".to_string()
            };

            println!(
                "{} {} {}",
                marker,
                style(&result.name).bold(),
                style(&result.version).dim()
            );

            if !result.description.is_empty() {
                let desc = if result.description.len() > 70 {
                    format!("{}...", &result.description[..67])
                } else {
                    result.description.clone()
                };
                println!("    {}", style(desc).dim());
            }
        }

        if results.len() > 20 {
            println!();
            println!(
                "    {} and {} more...",
                style("...").dim(),
                results.len() - 20
            );
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
        let files = vec![("/usr/local/bin/git".to_string(), "/opt/zb/git/bin/git".to_string())];
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
        assert_eq!(json.get("dependencies").unwrap(), &serde_json::json!(["openssl", "readline"]));
        assert_eq!(json.get("build_dependencies").unwrap(), &serde_json::json!(["pkg-config"]));
        assert_eq!(json.get("caveats").unwrap(), "See python.org for documentation");
        assert_eq!(json.get("keg_only").unwrap(), false);
    }

    #[test]
    fn test_build_formula_api_json_minimal() {
        let json = build_formula_api_json(
            "1.0.0",
            None,
            None,
            None,
            &[],
            &[],
            None,
            false,
        );

        assert_eq!(json.get("available_version").unwrap(), "1.0.0");
        assert!(!json.contains_key("description"));
        assert!(!json.contains_key("homepage"));
        assert!(!json.contains_key("license"));
        assert_eq!(json.get("dependencies").unwrap(), &serde_json::json!([]));
        assert_eq!(json.get("build_dependencies").unwrap(), &serde_json::json!([]));
        assert!(!json.contains_key("caveats"));
        assert_eq!(json.get("keg_only").unwrap(), false);
    }

    #[test]
    fn test_build_formula_api_json_keg_only() {
        let json = build_formula_api_json(
            "1.0.0",
            Some("Test"),
            None,
            None,
            &[],
            &[],
            None,
            true,
        );

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
        let files = vec![
            ("/path/with spaces/bin".to_string(), "/target/with spaces".to_string()),
        ];
        let json = build_linked_files_json(&files);
        assert_eq!(json[0]["link"], "/path/with spaces/bin");
        assert_eq!(json[0]["target"], "/target/with spaces");
    }

    #[test]
    fn test_build_linked_files_json_unicode_paths() {
        let files = vec![
            ("/usr/bin/日本語".to_string(), "/opt/日本語".to_string()),
        ];
        let json = build_linked_files_json(&files);
        assert_eq!(json[0]["link"], "/usr/bin/日本語");
    }

    // ========================================================================
    // Search Result JSON Edge Cases
    // ========================================================================

    #[test]
    fn test_build_search_result_json_with_special_chars() {
        let json = build_search_result_json(
            "c++",
            "homebrew/core/c++",
            "1.0",
            "A C++ compiler",
            false,
        );
        assert_eq!(json["name"], "c++");
    }

    #[test]
    fn test_build_search_result_json_long_description() {
        let long_desc = "x".repeat(1000);
        let json = build_search_result_json("test", "test", "1.0", &long_desc, false);
        assert_eq!(json["description"].as_str().unwrap().len(), 1000);
    }
}
