//! List and info commands for services.

use console::style;
use std::path::Path;

use zb_io::{ServiceManager, ServiceStatus};

/// Format a service status for display.
/// Extracted for testability.
pub(crate) fn format_status_display(status: &ServiceStatus) -> String {
    match status {
        ServiceStatus::Running => "running".to_string(),
        ServiceStatus::Stopped => "stopped".to_string(),
        ServiceStatus::Unknown => "unknown".to_string(),
        ServiceStatus::Error(msg) => format!("error ({})", msg),
    }
}

/// Format a PID for display (number or dash if None).
/// Extracted for testability.
pub(crate) fn format_pid_display(pid: Option<u32>) -> String {
    pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string())
}

/// Convert a ServiceStatus to its JSON string representation.
/// Extracted for testability.
pub(crate) fn status_to_json_string(status: &ServiceStatus) -> &'static str {
    match status {
        ServiceStatus::Running => "running",
        ServiceStatus::Stopped => "stopped",
        ServiceStatus::Unknown => "unknown",
        ServiceStatus::Error(_) => "error",
    }
}

/// Extract the error message from a ServiceStatus, if any.
/// Extracted for testability.
pub(crate) fn extract_status_error(status: &ServiceStatus) -> Option<String> {
    match status {
        ServiceStatus::Error(msg) => Some(msg.clone()),
        _ => None,
    }
}

/// Format auto-start status for display ("yes" or "no").
/// Extracted for testability.
pub(crate) fn format_auto_start_display(auto_start: bool) -> &'static str {
    if auto_start { "yes" } else { "no" }
}

/// Build JSON representation of a service.
/// Extracted for testability.
pub(crate) fn service_to_json(
    name: &str,
    status: &ServiceStatus,
    pid: Option<u32>,
    file_path: &Path,
    auto_start: bool,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "status": status_to_json_string(status),
        "pid": pid,
        "file": file_path.to_string_lossy(),
        "auto_start": auto_start,
        "error": extract_status_error(status)
    })
}

/// List all available services.
pub fn run_list(service_manager: &ServiceManager, json: bool) -> Result<(), zb_core::Error> {
    let services = service_manager.list()?;

    if json {
        let json_services: Vec<serde_json::Value> = services
            .iter()
            .map(|s| service_to_json(&s.name, &s.status, s.pid, &s.file_path, s.auto_start))
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
            let status_str = format_status_display(&service.status);
            let status_display = match &service.status {
                ServiceStatus::Running => style(&status_str).green().to_string(),
                ServiceStatus::Stopped => style(&status_str).dim().to_string(),
                ServiceStatus::Unknown => style(&status_str).yellow().to_string(),
                ServiceStatus::Error(_) => style(&status_str).red().to_string(),
            };

            let pid_display = format_pid_display(service.pid);

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

    let status_str = format_status_display(&info.status);
    let status_display = match &info.status {
        ServiceStatus::Running => style(&status_str).green().to_string(),
        ServiceStatus::Stopped => style(&status_str).dim().to_string(),
        ServiceStatus::Unknown => style(&status_str).yellow().to_string(),
        ServiceStatus::Error(_) => style(&status_str).red().to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ==================== format_status_display Tests ====================

    #[test]
    fn test_format_status_display_running() {
        let status = ServiceStatus::Running;
        assert_eq!(format_status_display(&status), "running");
    }

    #[test]
    fn test_format_status_display_stopped() {
        let status = ServiceStatus::Stopped;
        assert_eq!(format_status_display(&status), "stopped");
    }

    #[test]
    fn test_format_status_display_unknown() {
        let status = ServiceStatus::Unknown;
        assert_eq!(format_status_display(&status), "unknown");
    }

    #[test]
    fn test_format_status_display_error() {
        let status = ServiceStatus::Error("connection refused".to_string());
        assert_eq!(format_status_display(&status), "error (connection refused)");
    }

    #[test]
    fn test_format_status_display_error_empty() {
        let status = ServiceStatus::Error(String::new());
        assert_eq!(format_status_display(&status), "error ()");
    }

    #[test]
    fn test_format_status_display_error_multiline() {
        let status = ServiceStatus::Error("line 1\nline 2".to_string());
        assert_eq!(format_status_display(&status), "error (line 1\nline 2)");
    }

    // ==================== format_pid_display Tests ====================

    #[test]
    fn test_format_pid_display_some() {
        assert_eq!(format_pid_display(Some(12345)), "12345");
    }

    #[test]
    fn test_format_pid_display_none() {
        assert_eq!(format_pid_display(None), "-");
    }

    #[test]
    fn test_format_pid_display_zero() {
        // PID 0 is a valid (though unusual) display case
        assert_eq!(format_pid_display(Some(0)), "0");
    }

    #[test]
    fn test_format_pid_display_max() {
        assert_eq!(format_pid_display(Some(u32::MAX)), "4294967295");
    }

    // ==================== status_to_json_string Tests ====================

    #[test]
    fn test_status_to_json_string_running() {
        assert_eq!(status_to_json_string(&ServiceStatus::Running), "running");
    }

    #[test]
    fn test_status_to_json_string_stopped() {
        assert_eq!(status_to_json_string(&ServiceStatus::Stopped), "stopped");
    }

    #[test]
    fn test_status_to_json_string_unknown() {
        assert_eq!(status_to_json_string(&ServiceStatus::Unknown), "unknown");
    }

    #[test]
    fn test_status_to_json_string_error() {
        // Error message is NOT included in the status string, just "error"
        assert_eq!(
            status_to_json_string(&ServiceStatus::Error("some error".to_string())),
            "error"
        );
    }

    // ==================== extract_status_error Tests ====================

    #[test]
    fn test_extract_status_error_running() {
        assert_eq!(extract_status_error(&ServiceStatus::Running), None);
    }

    #[test]
    fn test_extract_status_error_stopped() {
        assert_eq!(extract_status_error(&ServiceStatus::Stopped), None);
    }

    #[test]
    fn test_extract_status_error_unknown() {
        assert_eq!(extract_status_error(&ServiceStatus::Unknown), None);
    }

    #[test]
    fn test_extract_status_error_error() {
        let status = ServiceStatus::Error("failed to start".to_string());
        assert_eq!(extract_status_error(&status), Some("failed to start".to_string()));
    }

    #[test]
    fn test_extract_status_error_empty() {
        let status = ServiceStatus::Error(String::new());
        assert_eq!(extract_status_error(&status), Some(String::new()));
    }

    // ==================== format_auto_start_display Tests ====================

    #[test]
    fn test_format_auto_start_display_true() {
        assert_eq!(format_auto_start_display(true), "yes");
    }

    #[test]
    fn test_format_auto_start_display_false() {
        assert_eq!(format_auto_start_display(false), "no");
    }

    // ==================== service_to_json Tests ====================

    #[test]
    fn test_service_to_json_running() {
        let json = service_to_json(
            "redis",
            &ServiceStatus::Running,
            Some(12345),
            &PathBuf::from("/home/user/.config/systemd/user/zerobrew.redis.service"),
            true,
        );

        assert_eq!(json["name"], "redis");
        assert_eq!(json["status"], "running");
        assert_eq!(json["pid"], 12345);
        assert_eq!(json["file"], "/home/user/.config/systemd/user/zerobrew.redis.service");
        assert_eq!(json["auto_start"], true);
        assert!(json["error"].is_null());
    }

    #[test]
    fn test_service_to_json_stopped_no_pid() {
        let json = service_to_json(
            "postgresql",
            &ServiceStatus::Stopped,
            None,
            &PathBuf::from("/path/to/service"),
            false,
        );

        assert_eq!(json["name"], "postgresql");
        assert_eq!(json["status"], "stopped");
        assert!(json["pid"].is_null());
        assert_eq!(json["auto_start"], false);
        assert!(json["error"].is_null());
    }

    #[test]
    fn test_service_to_json_error_with_message() {
        let json = service_to_json(
            "nginx",
            &ServiceStatus::Error("port 80 already in use".to_string()),
            None,
            &PathBuf::from("/path"),
            true,
        );

        assert_eq!(json["name"], "nginx");
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"], "port 80 already in use");
    }

    #[test]
    fn test_service_to_json_unknown_status() {
        let json = service_to_json(
            "mystery",
            &ServiceStatus::Unknown,
            None,
            &PathBuf::from("/unknown/path"),
            false,
        );

        assert_eq!(json["status"], "unknown");
        assert!(json["error"].is_null());
    }

    #[test]
    fn test_service_to_json_versioned_formula() {
        let json = service_to_json(
            "postgresql@14",
            &ServiceStatus::Running,
            Some(9999),
            &PathBuf::from("/path/zerobrew.postgresql@14.service"),
            true,
        );

        assert_eq!(json["name"], "postgresql@14");
        assert!(json["file"].as_str().unwrap().contains("postgresql@14"));
    }

    #[test]
    fn test_service_to_json_special_characters_in_path() {
        let json = service_to_json(
            "test",
            &ServiceStatus::Running,
            Some(1),
            &PathBuf::from("/path with spaces/service.service"),
            false,
        );

        assert_eq!(json["file"], "/path with spaces/service.service");
    }

    // ==================== Integration-style Tests ====================

    #[test]
    fn test_json_serialization_roundtrip() {
        let json = service_to_json(
            "redis",
            &ServiceStatus::Running,
            Some(42),
            &PathBuf::from("/test/path"),
            true,
        );

        // Should serialize without error
        let serialized = serde_json::to_string(&json).unwrap();
        
        // Should contain expected fields
        assert!(serialized.contains("\"name\":\"redis\""));
        assert!(serialized.contains("\"status\":\"running\""));
        assert!(serialized.contains("\"pid\":42"));
        assert!(serialized.contains("\"auto_start\":true"));
    }

    #[test]
    fn test_multiple_services_json_array() {
        let services = vec![
            service_to_json("redis", &ServiceStatus::Running, Some(100), &PathBuf::from("/a"), true),
            service_to_json("postgres", &ServiceStatus::Stopped, None, &PathBuf::from("/b"), false),
            service_to_json("nginx", &ServiceStatus::Error("fail".to_string()), None, &PathBuf::from("/c"), true),
        ];

        let json_str = serde_json::to_string_pretty(&services).unwrap();
        
        // Verify it's valid JSON array
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.len(), 3);
        
        // Verify order is preserved
        assert_eq!(parsed[0]["name"], "redis");
        assert_eq!(parsed[1]["name"], "postgres");
        assert_eq!(parsed[2]["name"], "nginx");
    }
}
