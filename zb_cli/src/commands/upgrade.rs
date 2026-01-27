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
