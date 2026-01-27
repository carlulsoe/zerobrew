//! Services command implementations.

use console::style;
use std::path::Path;
use std::process::Command;

use zb_io::install::Installer;
use zb_io::{ServiceManager, ServiceStatus};

use crate::ServicesAction;

/// Run the services command.
pub fn run(
    installer: &mut Installer,
    prefix: &Path,
    action: Option<ServicesAction>,
) -> Result<(), zb_core::Error> {
    let service_manager = ServiceManager::new(prefix);

    match action {
        None | Some(ServicesAction::List { json: false }) => {
            run_list(&service_manager, false)
        }
        Some(ServicesAction::List { json: true }) => {
            run_list(&service_manager, true)
        }
        Some(ServicesAction::Start { formula }) => {
            run_start(installer, &service_manager, prefix, &formula)
        }
        Some(ServicesAction::Stop { formula }) => {
            run_stop(&service_manager, &formula)
        }
        Some(ServicesAction::Restart { formula }) => {
            run_restart(&service_manager, &formula)
        }
        Some(ServicesAction::Enable { formula }) => {
            run_enable(&service_manager, &formula)
        }
        Some(ServicesAction::Disable { formula }) => {
            run_disable(&service_manager, &formula)
        }
        Some(ServicesAction::Run { formula }) => {
            run_foreground(installer, &service_manager, prefix, &formula)
        }
        Some(ServicesAction::Info { formula }) => {
            run_info(&service_manager, &formula)
        }
        Some(ServicesAction::Log { formula, lines, follow }) => {
            run_log(&service_manager, &formula, lines, follow)
        }
        Some(ServicesAction::Cleanup { dry_run }) => {
            run_cleanup(installer, &service_manager, dry_run)
        }
    }
}

fn run_list(service_manager: &ServiceManager, json: bool) -> Result<(), zb_core::Error> {
    let services = service_manager.list()?;

    if json {
        let json_services: Vec<serde_json::Value> = services
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "status": match &s.status {
                        ServiceStatus::Running => "running",
                        ServiceStatus::Stopped => "stopped",
                        ServiceStatus::Unknown => "unknown",
                        ServiceStatus::Error(_) => "error",
                    },
                    "pid": s.pid,
                    "file": s.file_path.to_string_lossy(),
                    "auto_start": s.auto_start,
                    "error": match &s.status {
                        ServiceStatus::Error(msg) => Some(msg.clone()),
                        _ => None,
                    }
                })
            })
            .collect();
        match serde_json::to_string_pretty(&json_services) {
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
    } else if services.is_empty() {
        println!("{} No services available.", style("==>").cyan().bold());
        println!();
        println!("    To start a service, first install a formula that provides one.");
        println!(
            "    Then run: {} services start <formula>",
            style("zb").cyan()
        );
    } else {
        println!(
            "{} {} services:",
            style("==>").cyan().bold(),
            services.len()
        );
        println!();

        println!(
            "{:<20} {:<10} {:<10} {}",
            style("Name").bold(),
            style("Status").bold(),
            style("PID").bold(),
            style("File").bold()
        );
        println!("{}", "-".repeat(60));

        for service in &services {
            let status_display = match &service.status {
                ServiceStatus::Running => style("running").green().to_string(),
                ServiceStatus::Stopped => style("stopped").dim().to_string(),
                ServiceStatus::Unknown => style("unknown").yellow().to_string(),
                ServiceStatus::Error(msg) => {
                    format!("{} ({})", style("error").red(), msg)
                }
            };

            let pid_display = service
                .pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());

            println!(
                "{:<20} {:<10} {:<10} {}",
                service.name,
                status_display,
                pid_display,
                service.file_path.display()
            );
        }
    }

    Ok(())
}

fn run_start(
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

fn run_stop(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
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

fn run_restart(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
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

fn run_enable(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
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

fn run_disable(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
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

fn run_foreground(
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

fn run_info(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
    let info = service_manager.get_service_info(formula)?;
    let (stdout_log, stderr_log) = service_manager.get_log_paths(formula);

    println!(
        "{} Service: {}",
        style("==>").cyan().bold(),
        style(formula).bold()
    );
    println!();

    let status_display = match &info.status {
        ServiceStatus::Running => style("running").green().to_string(),
        ServiceStatus::Stopped => style("stopped").dim().to_string(),
        ServiceStatus::Unknown => style("unknown").yellow().to_string(),
        ServiceStatus::Error(msg) => format!("{} ({})", style("error").red(), msg),
    };
    println!("Status:        {}", status_display);

    if let Some(pid) = info.pid {
        println!("PID:           {}", pid);
    }

    let auto_start_display = if info.auto_start {
        style("yes").green().to_string()
    } else {
        style("no").dim().to_string()
    };
    println!("Auto-start:    {}", auto_start_display);

    println!("Service file:  {}", info.file_path.display());

    println!();
    println!("{} Log files:", style("==>").cyan().bold());
    println!("Stdout:        {}", stdout_log.display());
    if stdout_log.exists()
        && let Ok(m) = std::fs::metadata(&stdout_log)
    {
        println!("               ({} bytes)", m.len());
    } else {
        println!("               (not yet created)");
    }

    println!("Stderr:        {}", stderr_log.display());
    if stderr_log.exists()
        && let Ok(m) = std::fs::metadata(&stderr_log)
    {
        println!("               ({} bytes)", m.len());
    } else {
        println!("               (not yet created)");
    }

    Ok(())
}

fn run_log(
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

        let all_lines: Vec<&str> = content.lines().collect();
        let start = if all_lines.len() > lines {
            all_lines.len() - lines
        } else {
            0
        };

        for line in &all_lines[start..] {
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

fn run_cleanup(
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
