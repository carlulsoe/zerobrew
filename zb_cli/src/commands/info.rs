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
}
