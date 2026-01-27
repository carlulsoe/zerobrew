//! Service control commands (start/stop/restart/enable/disable).

use console::style;
use std::path::Path;
use std::process::Command;

use zb_io::install::Installer;
use zb_io::ServiceManager;

/// Pluralize a word based on count.
/// Extracted for testability.
pub(crate) fn pluralize(count: usize, singular: &str, plural: &str) -> &str {
    if count == 1 { singular } else { plural }
}

/// Format the count suffix for orphaned services.
/// Extracted for testability.
pub(crate) fn format_orphan_count_message(count: usize, dry_run: bool) -> String {
    let suffix = pluralize(count, "", "s");
    if dry_run {
        format!("Would remove {} orphaned service{}", count, suffix)
    } else {
        format!("Removing {} orphaned service{}", count, suffix)
    }
}

/// Select the appropriate log file to display.
/// Returns the log file path to use (prefers stdout if it exists).
/// Extracted for testability.
pub(crate) fn select_log_file<'a>(
    stdout_log: &'a Path,
    stderr_log: &'a Path,
) -> Option<&'a Path> {
    if stdout_log.exists() {
        Some(stdout_log)
    } else if stderr_log.exists() {
        Some(stderr_log)
    } else {
        None
    }
}

/// Get the last N lines from content.
/// Extracted for testability.
pub(crate) fn get_last_lines(content: &str, lines: usize) -> Vec<&str> {
    let all_lines: Vec<&str> = content.lines().collect();
    let start = if all_lines.len() > lines {
        all_lines.len() - lines
    } else {
        0
    };
    all_lines[start..].to_vec()
}

/// Format cleanup completion message.
/// Extracted for testability.
pub(crate) fn format_cleanup_complete_message(count: usize) -> String {
    let suffix = pluralize(count, "", "s");
    format!("Removed {} orphaned service{}", count, suffix)
}

/// Start a service.
pub fn run_start(
    installer: &mut Installer,
    service_manager: &ServiceManager,
    prefix: &Path,
    formula: &str,
) -> Result<(), zb_core::Error> {
    if !installer.is_installed(formula) {
        eprintln!(
            "{} Formula '{}' is not installed.",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    let service_info = service_manager.get_service_info(formula);
    let needs_setup = match &service_info {
        Ok(info) => !info.file_path.exists(),
        Err(_) => true,
    };

    if needs_setup {
        let keg = installer.get_installed(formula).ok_or_else(|| {
            zb_core::Error::NotInstalled {
                name: formula.to_string(),
            }
        })?;
        let keg_path = prefix.join("Cellar").join(formula).join(&keg.version);

        if let Some(config) = service_manager.detect_service_config(formula, &keg_path) {
            println!(
                "{} Creating service file for {}...",
                style("==>").cyan().bold(),
                style(formula).bold()
            );
            service_manager.create_service(formula, &config)?;
        } else {
            eprintln!(
                "{} Formula '{}' does not have a service definition.",
                style("error:").red().bold(),
                formula
            );
            eprintln!();
            eprintln!("    Not all formulas provide services.");
            eprintln!(
                "    Check the formula's caveats with: {} info {}",
                style("zb").cyan(),
                formula
            );
            std::process::exit(1);
        }
    }

    println!(
        "{} Starting {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    service_manager.start(formula)?;

    println!(
        "{} {} Started {}",
        style("==>").cyan().bold(),
        style("✓").green(),
        style(formula).bold()
    );

    Ok(())
}

/// Stop a service.
pub fn run_stop(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
    println!(
        "{} Stopping {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    service_manager.stop(formula)?;

    println!(
        "{} {} Stopped {}",
        style("==>").cyan().bold(),
        style("✓").green(),
        style(formula).bold()
    );

    Ok(())
}

/// Restart a service.
pub fn run_restart(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
    println!(
        "{} Restarting {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    service_manager.restart(formula)?;

    println!(
        "{} {} Restarted {}",
        style("==>").cyan().bold(),
        style("✓").green(),
        style(formula).bold()
    );

    Ok(())
}

/// Enable a service to start automatically.
pub fn run_enable(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
    let info = service_manager.get_service_info(formula)?;
    if !info.file_path.exists() {
        eprintln!(
            "{} No service file found for '{}'.",
            style("error:").red().bold(),
            formula
        );
        eprintln!();
        eprintln!("    Start the service first to create the service file:");
        eprintln!("    {} services start {}", style("zb").cyan(), formula);
        std::process::exit(1);
    }

    if info.auto_start {
        println!(
            "{} {} is already set to start automatically.",
            style("==>").cyan().bold(),
            style(formula).bold()
        );
    } else {
        println!(
            "{} Enabling {} to start automatically...",
            style("==>").cyan().bold(),
            style(formula).bold()
        );

        service_manager.enable_auto_start(formula)?;

        println!(
            "{} {} Enabled {} - it will start automatically at login",
            style("==>").cyan().bold(),
            style("✓").green(),
            style(formula).bold()
        );
    }

    Ok(())
}

/// Disable a service from starting automatically.
pub fn run_disable(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
    let info = service_manager.get_service_info(formula)?;
    if !info.file_path.exists() {
        eprintln!(
            "{} No service file found for '{}'.",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    if !info.auto_start {
        println!(
            "{} {} is not set to start automatically.",
            style("==>").cyan().bold(),
            style(formula).bold()
        );
    } else {
        println!(
            "{} Disabling {} from starting automatically...",
            style("==>").cyan().bold(),
            style(formula).bold()
        );

        service_manager.disable_auto_start(formula)?;

        println!(
            "{} {} Disabled {} - it will no longer start automatically",
            style("==>").cyan().bold(),
            style("✓").green(),
            style(formula).bold()
        );
    }

    Ok(())
}

/// Run a service in foreground mode.
pub fn run_foreground(
    installer: &mut Installer,
    service_manager: &ServiceManager,
    prefix: &Path,
    formula: &str,
) -> Result<(), zb_core::Error> {
    if !installer.is_installed(formula) {
        eprintln!(
            "{} Formula '{}' is not installed.",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    let keg = installer.get_installed(formula).ok_or_else(|| {
        zb_core::Error::NotInstalled {
            name: formula.to_string(),
        }
    })?;
    let keg_path = prefix.join("Cellar").join(formula).join(&keg.version);

    if let Some(config) = service_manager.detect_service_config(formula, &keg_path) {
        println!(
            "{} Running {} in foreground...",
            style("==>").cyan().bold(),
            style(formula).bold()
        );
        println!(
            "    Command: {} {}",
            config.program.display(),
            config.args.join(" ")
        );
        println!("    Press Ctrl+C to stop.");
        println!();

        let mut cmd = Command::new(&config.program);
        cmd.args(&config.args);

        if let Some(wd) = &config.working_directory {
            cmd.current_dir(wd);
        }

        for (key, value) in &config.environment {
            cmd.env(key, value);
        }

        let status = cmd.status().map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("failed to run service: {}", e),
        })?;

        if !status.success() {
            eprintln!(
                "\n{} Service exited with status: {}",
                style("==>").cyan().bold(),
                status.code().unwrap_or(-1)
            );
        }
    } else {
        eprintln!(
            "{} Formula '{}' does not have a service definition.",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    Ok(())
}

/// View service logs.
pub fn run_log(
    service_manager: &ServiceManager,
    formula: &str,
    lines: usize,
    follow: bool,
) -> Result<(), zb_core::Error> {
    let (stdout_log, stderr_log) = service_manager.get_log_paths(formula);

    if !stdout_log.exists() && !stderr_log.exists() {
        eprintln!(
            "{} No log files found for '{}'.",
            style("error:").red().bold(),
            formula
        );
        eprintln!();
        eprintln!("    Expected log files:");
        eprintln!("      {}", stdout_log.display());
        eprintln!("      {}", stderr_log.display());
        eprintln!();
        eprintln!(
            "    Start the service first with: {} services start {}",
            style("zb").cyan(),
            formula
        );
        std::process::exit(1);
    }

    let log_file = if stdout_log.exists() {
        &stdout_log
    } else {
        &stderr_log
    };

    if follow {
        println!(
            "{} Following logs for {} (Ctrl+C to stop)...",
            style("==>").cyan().bold(),
            style(formula).bold()
        );
        println!("    {}", log_file.display());
        println!();

        let mut cmd = Command::new("tail");
        cmd.args(["-f", "-n", &lines.to_string()]);
        cmd.arg(log_file);

        let status = cmd.status().map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("failed to tail log: {}", e),
        })?;

        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    } else {
        println!(
            "{} Logs for {} (last {} lines):",
            style("==>").cyan().bold(),
            style(formula).bold(),
            lines
        );
        println!("    {}", log_file.display());
        println!();

        let content =
            std::fs::read_to_string(log_file).map_err(|e| zb_core::Error::StoreCorruption {
                message: format!("failed to read log file: {}", e),
            })?;

        for line in get_last_lines(&content, lines) {
            println!("{}", line);
        }

        if stderr_log.exists() && stderr_log != *log_file {
            println!();
            println!(
                "    Note: Error log also exists at {}",
                stderr_log.display()
            );
        }
    }

    Ok(())
}

/// Clean up orphaned services.
pub fn run_cleanup(
    installer: &mut Installer,
    service_manager: &ServiceManager,
    dry_run: bool,
) -> Result<(), zb_core::Error> {
    let installed: Vec<String> = installer
        .list_installed()?
        .iter()
        .map(|k| k.name.clone())
        .collect();

    let orphaned = service_manager.find_orphaned_services(&installed)?;

    if orphaned.is_empty() {
        println!("{} No orphaned services found.", style("==>").cyan().bold());
        return Ok(());
    }

    if dry_run {
        println!(
            "{} Would remove {} orphaned service{}:",
            style("==>").cyan().bold(),
            orphaned.len(),
            if orphaned.len() == 1 { "" } else { "s" }
        );
        println!();

        for service in &orphaned {
            println!("    {}", service.name);
            println!("        {}", style(service.file_path.display()).dim());
        }

        println!();
        println!(
            "    → Run {} services cleanup to remove",
            style("zb").cyan()
        );
    } else {
        println!(
            "{} Removing {} orphaned service{}...",
            style("==>").cyan().bold(),
            orphaned.len(),
            if orphaned.len() == 1 { "" } else { "s" }
        );
        println!();

        let count = service_manager.cleanup_services(&orphaned)?;

        for service in &orphaned {
            println!("    {} Removed {}", style("✓").green(), service.name);
        }

        println!();
        println!(
            "{} Removed {} orphaned service{}",
            style("==>").cyan().bold(),
            count,
            if count == 1 { "" } else { "s" }
        );
    }

    Ok(())
}
