//! Bundle command implementations.

use console::style;
use std::path::PathBuf;

use zb_io::install::Installer;

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
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    println!(
        "{} Installing from {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_install(&brewfile_path).await?;

    // Report results
    if !result.taps_added.is_empty() {
        println!();
        println!("{} Taps added:", style("==>").cyan().bold());
        for tap in &result.taps_added {
            println!("    {} {}", style("✓").green(), tap);
        }
    }

    if !result.formulas_installed.is_empty() {
        println!();
        println!("{} Formulas installed:", style("==>").cyan().bold());
        for formula in &result.formulas_installed {
            println!("    {} {}", style("✓").green(), formula);
        }
    }

    if !result.formulas_skipped.is_empty() {
        println!();
        println!("{} Already installed:", style("==>").cyan().bold());
        for formula in &result.formulas_skipped {
            println!("    {} {}", style("-").dim(), formula);
        }
    }

    if !result.failed.is_empty() {
        println!();
        println!("{} Failed:", style("==>").red().bold());
        for (name, error) in &result.failed {
            println!("    {} {}: {}", style("✗").red(), name, error);
        }
    }

    // Summary
    println!();
    let total_installed = result.taps_added.len() + result.formulas_installed.len();
    if result.failed.is_empty() {
        println!(
            "{} Bundle complete. {} installed, {} already satisfied.",
            style("==>").cyan().bold(),
            total_installed,
            result.formulas_skipped.len()
        );
    } else {
        println!(
            "{} Bundle complete with errors. {} installed, {} already satisfied, {} failed.",
            style("==>").yellow().bold(),
            total_installed,
            result.formulas_skipped.len(),
            result.failed.len()
        );
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
            eprintln!(
                "{} File '{}' already exists. Use --force to overwrite.",
                style("error:").red().bold(),
                path.display()
            );
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
        print!("{}", content);
        if !content.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}

fn run_check(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
    strict: bool,
) -> Result<(), zb_core::Error> {
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    println!(
        "{} Checking {}",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );

    let result = installer.bundle_check(&brewfile_path)?;

    if result.satisfied {
        println!();
        println!("{} All entries are satisfied!", style("==>").green().bold());
    } else {
        if !result.missing_taps.is_empty() {
            println!();
            println!("{} Missing taps:", style("==>").yellow().bold());
            for tap in &result.missing_taps {
                println!("    {} {}", style("✗").red(), tap);
            }
        }

        if !result.missing_formulas.is_empty() {
            println!();
            println!("{} Missing formulas:", style("==>").yellow().bold());
            for formula in &result.missing_formulas {
                println!("    {} {}", style("✗").red(), formula);
            }
        }

        println!();
        println!(
            "    → Run {} bundle to install missing entries",
            style("zb").cyan()
        );

        if strict {
            std::process::exit(1);
        }
    }

    Ok(())
}

fn run_list(
    installer: &mut Installer,
    cwd: &std::path::Path,
    file: Option<PathBuf>,
) -> Result<(), zb_core::Error> {
    let brewfile_path = if let Some(path) = file {
        path
    } else {
        installer.find_brewfile(cwd).ok_or_else(|| {
            zb_core::Error::StoreCorruption {
                message: "No Brewfile found in current directory or parent directories".to_string(),
            }
        })?
    };

    let entries = installer.parse_brewfile(&brewfile_path)?;

    println!(
        "{} Entries in {}:",
        style("==>").cyan().bold(),
        brewfile_path.display()
    );
    println!();

    #[allow(unused_mut)]
    let mut tap_count = 0;
    let mut brew_count = 0;

    for entry in &entries {
        match entry {
            zb_io::BrewfileEntry::Tap { name } => {
                println!("tap  {}", style(name).cyan());
                tap_count += 1;
            }
            zb_io::BrewfileEntry::Brew { name, args } => {
                if args.is_empty() {
                    println!("brew {}", style(name).green());
                } else {
                    println!("brew {} ({})", style(name).green(), args.join(", "));
                }
                brew_count += 1;
            }
            zb_io::BrewfileEntry::Comment(_) => {
                // Skip comments in list output
            }
        }
    }

    println!();
    println!(
        "{} {} taps, {} formulas",
        style("==>").cyan().bold(),
        tap_count,
        brew_count
    );

    Ok(())
}
