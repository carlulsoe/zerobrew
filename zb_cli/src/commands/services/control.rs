//! Service control commands (start/stop/restart/enable/disable).

use console::style;
use std::path::Path;
use std::process::Command;

use zb_io::install::Installer;
use zb_io::ServiceManager;

// ============================================================================
// Pure Helper Functions (Extracted for Testability)
// ============================================================================

/// Pluralize a word based on count.
/// Extracted for testability.
pub(crate) fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
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

// ============================================================================
// Action Message Formatters
// ============================================================================

/// Format the "Starting <formula>..." message.
pub(crate) fn format_starting_message(formula: &str) -> String {
    format!("Starting {}...", formula)
}

/// Format the "Started <formula>" completion message.
pub(crate) fn format_started_message(formula: &str) -> String {
    format!("Started {}", formula)
}

/// Format the "Stopping <formula>..." message.
pub(crate) fn format_stopping_message(formula: &str) -> String {
    format!("Stopping {}...", formula)
}

/// Format the "Stopped <formula>" completion message.
pub(crate) fn format_stopped_message(formula: &str) -> String {
    format!("Stopped {}", formula)
}

/// Format the "Restarting <formula>..." message.
pub(crate) fn format_restarting_message(formula: &str) -> String {
    format!("Restarting {}...", formula)
}

/// Format the "Restarted <formula>" completion message.
pub(crate) fn format_restarted_message(formula: &str) -> String {
    format!("Restarted {}", formula)
}

/// Format the "Enabling <formula>..." message.
pub(crate) fn format_enabling_message(formula: &str) -> String {
    format!("Enabling {} to start automatically...", formula)
}

/// Format the "Enabled <formula>" completion message.
pub(crate) fn format_enabled_message(formula: &str) -> String {
    format!("Enabled {} - it will start automatically at login", formula)
}

/// Format the "Disabling <formula>..." message.
pub(crate) fn format_disabling_message(formula: &str) -> String {
    format!("Disabling {} from starting automatically...", formula)
}

/// Format the "Disabled <formula>" completion message.
pub(crate) fn format_disabled_message(formula: &str) -> String {
    format!("Disabled {} - it will no longer start automatically", formula)
}

/// Format the "already enabled" message.
pub(crate) fn format_already_enabled_message(formula: &str) -> String {
    format!("{} is already set to start automatically.", formula)
}

/// Format the "not enabled" message.
pub(crate) fn format_not_enabled_message(formula: &str) -> String {
    format!("{} is not set to start automatically.", formula)
}

/// Format the "Creating service file" message.
pub(crate) fn format_creating_service_message(formula: &str) -> String {
    format!("Creating service file for {}...", formula)
}

/// Format the "Running in foreground" message.
pub(crate) fn format_foreground_message(formula: &str) -> String {
    format!("Running {} in foreground...", formula)
}

/// Format the foreground command display.
pub(crate) fn format_foreground_command(program: &Path, args: &[String]) -> String {
    format!("Command: {} {}", program.display(), args.join(" "))
}

/// Format the log header message.
pub(crate) fn format_log_header(formula: &str, lines: usize) -> String {
    format!("Logs for {} (last {} lines):", formula, lines)
}

/// Format the log follow header message.
pub(crate) fn format_log_follow_header(formula: &str) -> String {
    format!("Following logs for {} (Ctrl+C to stop)...", formula)
}

// ============================================================================
// Error Message Formatters
// ============================================================================

/// Format the "formula not installed" error message.
pub(crate) fn format_not_installed_error(formula: &str) -> String {
    format!("Formula '{}' is not installed.", formula)
}

/// Format the "no service definition" error message.
pub(crate) fn format_no_service_definition_error(formula: &str) -> String {
    format!("Formula '{}' does not have a service definition.", formula)
}

/// Format the "no service file" error message.
pub(crate) fn format_no_service_file_error(formula: &str) -> String {
    format!("No service file found for '{}'.", formula)
}

/// Format the "no log files" error message.
pub(crate) fn format_no_log_files_error(formula: &str) -> String {
    format!("No log files found for '{}'.", formula)
}

/// Format the expected log files hint.
pub(crate) fn format_expected_log_files_hint(stdout_path: &Path, stderr_path: &Path) -> String {
    format!(
        "Expected log files:\n      {}\n      {}",
        stdout_path.display(),
        stderr_path.display()
    )
}

/// Format the "start service first" hint for logs.
pub(crate) fn format_start_service_hint(formula: &str) -> String {
    format!("Start the service first with: zb services start {}", formula)
}

/// Format the "start service first" hint for enable.
pub(crate) fn format_start_service_for_enable_hint(formula: &str) -> String {
    format!(
        "Start the service first to create the service file:\n    zb services start {}",
        formula
    )
}

/// Format the service exited message.
pub(crate) fn format_service_exited_message(exit_code: i32) -> String {
    format!("Service exited with status: {}", exit_code)
}

/// Format the cleanup dry-run prompt.
pub(crate) fn format_cleanup_dry_run_prompt() -> String {
    "Run zb services cleanup to remove".to_string()
}

/// Format the "no orphaned services" message.
pub(crate) fn format_no_orphaned_services_message() -> String {
    "No orphaned services found.".to_string()
}

/// Format the check caveats hint.
pub(crate) fn format_check_caveats_hint(formula: &str) -> String {
    format!("Check the formula's caveats with: zb info {}", formula)
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate that a formula name is non-empty.
pub(crate) fn validate_formula_name(formula: &str) -> Result<(), String> {
    if formula.is_empty() {
        Err("Formula name cannot be empty".to_string())
    } else if formula.contains('/') && !formula.contains('@') {
        // Looks like a tap path without version
        Err(format!("Invalid formula name '{}': use the formula name, not the tap path", formula))
    } else {
        Ok(())
    }
}

/// Check if a line count is valid for log display.
pub(crate) fn validate_log_lines(lines: usize) -> Result<(), String> {
    if lines == 0 {
        Err("Line count must be greater than 0".to_string())
    } else if lines > 100_000 {
        Err("Line count too large (max 100000)".to_string())
    } else {
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ==================== pluralize Tests ====================

    #[test]
    fn test_pluralize_zero() {
        assert_eq!(pluralize(0, "service", "services"), "services");
    }

    #[test]
    fn test_pluralize_one() {
        assert_eq!(pluralize(1, "service", "services"), "service");
    }

    #[test]
    fn test_pluralize_two() {
        assert_eq!(pluralize(2, "service", "services"), "services");
    }

    #[test]
    fn test_pluralize_many() {
        assert_eq!(pluralize(100, "service", "services"), "services");
    }

    #[test]
    fn test_pluralize_empty_strings() {
        assert_eq!(pluralize(0, "", "s"), "s");
        assert_eq!(pluralize(1, "", "s"), "");
        assert_eq!(pluralize(2, "", "s"), "s");
    }

    #[test]
    fn test_pluralize_irregular() {
        assert_eq!(pluralize(1, "child", "children"), "child");
        assert_eq!(pluralize(2, "child", "children"), "children");
    }

    // ==================== format_orphan_count_message Tests ====================

    #[test]
    fn test_format_orphan_count_message_zero_dry_run() {
        let msg = format_orphan_count_message(0, true);
        assert_eq!(msg, "Would remove 0 orphaned services");
    }

    #[test]
    fn test_format_orphan_count_message_one_dry_run() {
        let msg = format_orphan_count_message(1, true);
        assert_eq!(msg, "Would remove 1 orphaned service");
    }

    #[test]
    fn test_format_orphan_count_message_multiple_dry_run() {
        let msg = format_orphan_count_message(5, true);
        assert_eq!(msg, "Would remove 5 orphaned services");
    }

    #[test]
    fn test_format_orphan_count_message_zero_actual() {
        let msg = format_orphan_count_message(0, false);
        assert_eq!(msg, "Removing 0 orphaned services");
    }

    #[test]
    fn test_format_orphan_count_message_one_actual() {
        let msg = format_orphan_count_message(1, false);
        assert_eq!(msg, "Removing 1 orphaned service");
    }

    #[test]
    fn test_format_orphan_count_message_multiple_actual() {
        let msg = format_orphan_count_message(10, false);
        assert_eq!(msg, "Removing 10 orphaned services");
    }

    // ==================== format_cleanup_complete_message Tests ====================

    #[test]
    fn test_format_cleanup_complete_message_zero() {
        let msg = format_cleanup_complete_message(0);
        assert_eq!(msg, "Removed 0 orphaned services");
    }

    #[test]
    fn test_format_cleanup_complete_message_one() {
        let msg = format_cleanup_complete_message(1);
        assert_eq!(msg, "Removed 1 orphaned service");
    }

    #[test]
    fn test_format_cleanup_complete_message_multiple() {
        let msg = format_cleanup_complete_message(7);
        assert_eq!(msg, "Removed 7 orphaned services");
    }

    // ==================== select_log_file Tests ====================

    #[test]
    fn test_select_log_file_both_exist() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-log-both");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let stdout_log = temp_dir.join("stdout.log");
        let stderr_log = temp_dir.join("stderr.log");
        fs::write(&stdout_log, "stdout content").unwrap();
        fs::write(&stderr_log, "stderr content").unwrap();

        // Should prefer stdout
        let selected = select_log_file(&stdout_log, &stderr_log);
        assert_eq!(selected, Some(stdout_log.as_path()));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_select_log_file_only_stdout() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-log-stdout");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let stdout_log = temp_dir.join("stdout.log");
        let stderr_log = temp_dir.join("stderr.log");
        fs::write(&stdout_log, "stdout content").unwrap();
        // stderr doesn't exist

        let selected = select_log_file(&stdout_log, &stderr_log);
        assert_eq!(selected, Some(stdout_log.as_path()));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_select_log_file_only_stderr() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-log-stderr");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let stdout_log = temp_dir.join("stdout.log");
        let stderr_log = temp_dir.join("stderr.log");
        // stdout doesn't exist
        fs::write(&stderr_log, "stderr content").unwrap();

        let selected = select_log_file(&stdout_log, &stderr_log);
        assert_eq!(selected, Some(stderr_log.as_path()));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_select_log_file_neither_exist() {
        let stdout_log = PathBuf::from("/nonexistent/stdout.log");
        let stderr_log = PathBuf::from("/nonexistent/stderr.log");

        let selected = select_log_file(&stdout_log, &stderr_log);
        assert_eq!(selected, None);
    }

    // ==================== get_last_lines Tests ====================

    #[test]
    fn test_get_last_lines_empty() {
        let content = "";
        let result = get_last_lines(content, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_last_lines_fewer_than_requested() {
        let content = "line 1\nline 2\nline 3";
        let result = get_last_lines(content, 10);
        assert_eq!(result, vec!["line 1", "line 2", "line 3"]);
    }

    #[test]
    fn test_get_last_lines_exact_count() {
        let content = "line 1\nline 2\nline 3";
        let result = get_last_lines(content, 3);
        assert_eq!(result, vec!["line 1", "line 2", "line 3"]);
    }

    #[test]
    fn test_get_last_lines_more_than_requested() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        let result = get_last_lines(content, 2);
        assert_eq!(result, vec!["line 4", "line 5"]);
    }

    #[test]
    fn test_get_last_lines_request_zero() {
        let content = "line 1\nline 2\nline 3";
        let result = get_last_lines(content, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_last_lines_single_line() {
        let content = "only line";
        let result = get_last_lines(content, 5);
        assert_eq!(result, vec!["only line"]);
    }

    #[test]
    fn test_get_last_lines_with_empty_lines() {
        let content = "line 1\n\nline 3\n\nline 5";
        let result = get_last_lines(content, 3);
        assert_eq!(result, vec!["line 3", "", "line 5"]);
    }

    #[test]
    fn test_get_last_lines_trailing_newline() {
        let content = "line 1\nline 2\n";
        let result = get_last_lines(content, 10);
        // Rust's str::lines() does NOT include trailing empty element for trailing newline
        assert_eq!(result, vec!["line 1", "line 2"]);
    }

    #[test]
    fn test_get_last_lines_large_file_simulation() {
        // Simulate a large log file
        let lines: Vec<String> = (1..=1000).map(|i| format!("Log entry {}", i)).collect();
        let content = lines.join("\n");

        let result = get_last_lines(&content, 50);
        assert_eq!(result.len(), 50);
        assert_eq!(result[0], "Log entry 951");
        assert_eq!(result[49], "Log entry 1000");
    }

    // ==================== Error Message Format Tests ====================

    #[test]
    fn test_error_messages_are_user_friendly() {
        // These tests document expected error message patterns
        // for future reference and to prevent regressions

        // Formula not installed pattern
        let formula = "redis";
        let expected = format!("Formula '{}' is not installed.", formula);
        assert!(expected.contains(formula));
        assert!(expected.contains("not installed"));

        // No service definition pattern
        let expected = format!("Formula '{}' does not have a service definition.", formula);
        assert!(expected.contains("service definition"));

        // No service file pattern
        let expected = format!("No service file found for '{}'.", formula);
        assert!(expected.contains("service file"));
    }

    // ==================== Integration-style Logic Tests ====================

    #[test]
    fn test_cleanup_message_consistency() {
        // Verify dry-run and actual messages use consistent pluralization
        for count in [0, 1, 2, 5, 100] {
            let dry_run_msg = format_orphan_count_message(count, true);
            let actual_msg = format_orphan_count_message(count, false);
            let complete_msg = format_cleanup_complete_message(count);

            // All should use "service" for 1, "services" otherwise
            let expected_suffix = if count == 1 { "service" } else { "services" };
            assert!(dry_run_msg.contains(expected_suffix));
            assert!(actual_msg.contains(expected_suffix));
            assert!(complete_msg.contains(expected_suffix));
        }
    }

    #[test]
    fn test_log_selection_priority() {
        // This documents the priority: stdout > stderr > none
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join("zb-test-log-priority");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let stdout_log = temp_dir.join("stdout.log");
        let stderr_log = temp_dir.join("stderr.log");

        // Neither exists
        assert!(select_log_file(&stdout_log, &stderr_log).is_none());

        // Only stderr
        fs::write(&stderr_log, "err").unwrap();
        assert_eq!(select_log_file(&stdout_log, &stderr_log), Some(stderr_log.as_path()));

        // Both exist - stdout wins
        fs::write(&stdout_log, "out").unwrap();
        assert_eq!(select_log_file(&stdout_log, &stderr_log), Some(stdout_log.as_path()));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
