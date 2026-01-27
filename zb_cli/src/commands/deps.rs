//! Deps, uses, and leaves command implementations.

use console::style;

use zb_io::install::Installer;

use crate::display::print_deps_tree;

/// Run the deps command.
pub async fn run_deps(
    installer: &mut Installer,
    formula: String,
    tree: bool,
    installed: bool,
    all: bool,
) -> Result<(), zb_core::Error> {
    if tree {
        println!(
            "{} Dependencies for {} (tree view):",
            style("==>").cyan().bold(),
            style(&formula).bold()
        );
        println!();

        let tree_data = installer.get_deps_tree(&formula, installed).await?;
        print_deps_tree(&tree_data, "", true);
    } else {
        let deps = installer.get_deps(&formula, installed, all).await?;

        if deps.is_empty() {
            println!(
                "{} {} has no{}dependencies.",
                style("==>").cyan().bold(),
                style(&formula).bold(),
                if installed { " installed " } else { " " }
            );
        } else {
            println!(
                "{} Dependencies for {}{}:",
                style("==>").cyan().bold(),
                style(&formula).bold(),
                if all { " (all)" } else { "" }
            );
            println!();

            for dep in &deps {
                let installed_marker = if installer.is_installed(dep) {
                    style("✓").green().to_string()
                } else {
                    style("✗").red().to_string()
                };
                println!("  {} {}", installed_marker, dep);
            }
        }
    }

    Ok(())
}

/// Run the uses command.
pub async fn run_uses(
    installer: &mut Installer,
    formula: String,
    recursive: bool,
) -> Result<(), zb_core::Error> {
    println!(
        "{} Checking what uses {}...",
        style("==>").cyan().bold(),
        style(&formula).bold()
    );

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
        println!(
            "{} No installed formulas use {}.",
            style("==>").cyan().bold(),
            style(&formula).bold()
        );
    } else {
        println!(
            "{} {} installed formulas use {}{}:",
            style("==>").cyan().bold(),
            style(uses.len()).green().bold(),
            style(&formula).bold(),
            if recursive {
                " (directly or indirectly)"
            } else {
                ""
            }
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
    println!("{} Finding leaf packages...", style("==>").cyan().bold());

    let leaves = installer.get_leaves().await?;

    if leaves.is_empty() {
        println!("No installed packages, or all packages are dependencies.");
    } else {
        println!(
            "{} {} leaf packages (not dependencies of other installed packages):",
            style("==>").cyan().bold(),
            style(leaves.len()).green().bold()
        );
        println!();

        for name in &leaves {
            println!("  {}", name);
        }
    }

    Ok(())
}
