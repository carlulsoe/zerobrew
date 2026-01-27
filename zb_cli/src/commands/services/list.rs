//! List and info commands for services.

use console::style;

use zb_io::{ServiceManager, ServiceStatus};

/// List all available services.
pub fn run_list(service_manager: &ServiceManager, json: bool) -> Result<(), zb_core::Error> {
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

/// Show detailed info for a specific service.
pub fn run_info(service_manager: &ServiceManager, formula: &str) -> Result<(), zb_core::Error> {
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
