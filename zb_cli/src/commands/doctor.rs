//! Doctor command implementation.

use console::style;

use zb_io::install::Installer;
use zb_io::{DoctorCheck, DoctorResult, DoctorStatus};

/// Format the marker symbol for a given doctor status.
pub fn format_status_marker(status: &DoctorStatus) -> String {
    match status {
        DoctorStatus::Ok => style("✓").green().to_string(),
        DoctorStatus::Warning => style("!").yellow().to_string(),
        DoctorStatus::Error => style("✗").red().to_string(),
    }
}

/// Get the plain (unstyled) marker for a given doctor status.
pub fn plain_status_marker(status: &DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Ok => "✓",
        DoctorStatus::Warning => "!",
        DoctorStatus::Error => "✗",
    }
}

/// Pluralize issue type based on count.
pub fn pluralize_issue(count: usize, singular: &str) -> String {
    if count == 1 {
        singular.to_string()
    } else {
        format!("{}s", singular)
    }
}

/// Format the issue count for display (plain text, no styling).
pub fn format_issue_count_plain(count: usize, issue_type: &str) -> String {
    let noun = pluralize_issue(count, issue_type);
    format!("{} {} found", count, noun)
}

/// Format the summary message based on the doctor result.
pub fn format_summary_message(result: &DoctorResult) -> String {
    if result.is_healthy() {
        "Your system is ready to brew!".to_string()
    } else {
        let mut parts = Vec::new();
        if result.errors > 0 {
            parts.push(format_issue_count_plain(result.errors, "error"));
        }
        if result.warnings > 0 {
            parts.push(format_issue_count_plain(result.warnings, "warning"));
        }
        parts.join(", ")
    }
}

/// Format a single check result line (plain text).
pub fn format_check_line(check: &DoctorCheck) -> String {
    let marker = plain_status_marker(&check.status);
    let mut line = format!("{} {}", marker, check.message);
    if let Some(ref fix) = check.fix {
        line.push_str(&format!("\n    → {}", fix));
    }
    line
}

/// Format the complete doctor output (plain text, for testing).
/// Used by unit tests to verify output structure without ANSI styling.
#[allow(dead_code)]
pub fn format_doctor_output_plain(result: &DoctorResult) -> String {
    let mut output = String::from("==> Running diagnostics...\n\n");

    for check in &result.checks {
        output.push_str(&format_check_line(check));
        output.push('\n');
    }

    output.push('\n');
    output.push_str(&format!("==> {}", format_summary_message(result)));

    output
}

/// Format a single check result line (styled for terminal).
/// Uses plain_status_marker internally for the symbol, then applies styling.
pub fn format_check_line_styled(check: &DoctorCheck) -> String {
    let marker = format_status_marker(&check.status);
    let mut line = format!("{} {}", marker, check.message);
    if let Some(ref fix) = check.fix {
        line.push_str(&format!("\n    {} {}", style("→").dim(), style(fix).dim()));
    }
    line
}

/// Format the issue count with styling for terminal output.
/// Wraps format_issue_count_plain with appropriate colors.
pub fn format_issue_count_styled(count: usize, issue_type: &str) -> String {
    let color_fn = match issue_type {
        "error" => |s: String| style(s).red().bold().to_string(),
        "warning" => |s: String| style(s).yellow().bold().to_string(),
        _ => |s: String| s,
    };
    let noun = pluralize_issue(count, issue_type);
    format!(
        "{} {} {} found",
        style("==>").cyan().bold(),
        color_fn(count.to_string()),
        noun
    )
}

/// Format the summary message with styling for terminal output.
/// Uses format_summary_message internally for the text content.
pub fn format_summary_styled(result: &DoctorResult) -> Vec<String> {
    if result.is_healthy() {
        vec![format!(
            "{} {}",
            style("==>").cyan().bold(),
            format_summary_message(result)
        )]
    } else {
        let mut lines = Vec::new();
        if result.errors > 0 {
            lines.push(format_issue_count_styled(result.errors, "error"));
        }
        if result.warnings > 0 {
            lines.push(format_issue_count_styled(result.warnings, "warning"));
        }
        lines
    }
}

/// Run the doctor command.
pub async fn run(installer: &mut Installer) -> Result<(), zb_core::Error> {
    println!("{} Running diagnostics...\n", style("==>").cyan().bold());

    let result = installer.doctor().await;

    for check in &result.checks {
        println!("{}", format_check_line_styled(check));
    }

    println!();
    for line in format_summary_styled(&result) {
        println!("{}", line);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a DoctorCheck
    fn make_check(message: &str, status: DoctorStatus, fix: Option<&str>) -> DoctorCheck {
        DoctorCheck {
            name: "test_check".to_string(),
            message: message.to_string(),
            status,
            fix: fix.map(|s| s.to_string()),
        }
    }

    // Helper to create a DoctorResult
    fn make_result(checks: Vec<DoctorCheck>, errors: usize, warnings: usize) -> DoctorResult {
        DoctorResult {
            checks,
            errors,
            warnings,
        }
    }

    // ========== Status Marker Tests ==========

    #[test]
    fn test_plain_status_marker_ok() {
        assert_eq!(plain_status_marker(&DoctorStatus::Ok), "✓");
    }

    #[test]
    fn test_plain_status_marker_warning() {
        assert_eq!(plain_status_marker(&DoctorStatus::Warning), "!");
    }

    #[test]
    fn test_plain_status_marker_error() {
        assert_eq!(plain_status_marker(&DoctorStatus::Error), "✗");
    }

    #[test]
    fn test_format_status_marker_contains_symbol() {
        // Styled markers should still contain the base symbol
        let ok_marker = format_status_marker(&DoctorStatus::Ok);
        let warn_marker = format_status_marker(&DoctorStatus::Warning);
        let err_marker = format_status_marker(&DoctorStatus::Error);

        assert!(ok_marker.contains("✓") || ok_marker.contains("\u{2713}"));
        assert!(warn_marker.contains("!"));
        assert!(err_marker.contains("✗") || err_marker.contains("\u{2717}"));
    }

    // ========== Pluralization Tests ==========

    #[test]
    fn test_pluralize_issue_singular() {
        assert_eq!(pluralize_issue(1, "error"), "error");
        assert_eq!(pluralize_issue(1, "warning"), "warning");
    }

    #[test]
    fn test_pluralize_issue_plural() {
        assert_eq!(pluralize_issue(0, "error"), "errors");
        assert_eq!(pluralize_issue(2, "error"), "errors");
        assert_eq!(pluralize_issue(5, "warning"), "warnings");
        assert_eq!(pluralize_issue(100, "issue"), "issues");
    }

    // ========== Issue Count Formatting Tests ==========

    #[test]
    fn test_format_issue_count_plain_singular() {
        assert_eq!(format_issue_count_plain(1, "error"), "1 error found");
        assert_eq!(format_issue_count_plain(1, "warning"), "1 warning found");
    }

    #[test]
    fn test_format_issue_count_plain_plural() {
        assert_eq!(format_issue_count_plain(0, "error"), "0 errors found");
        assert_eq!(format_issue_count_plain(2, "error"), "2 errors found");
        assert_eq!(format_issue_count_plain(5, "warning"), "5 warnings found");
    }

    // ========== Summary Message Tests ==========

    #[test]
    fn test_format_summary_message_healthy() {
        let result = make_result(vec![], 0, 0);
        assert_eq!(
            format_summary_message(&result),
            "Your system is ready to brew!"
        );
    }

    #[test]
    fn test_format_summary_message_with_errors_only() {
        let result = make_result(vec![], 3, 0);
        assert_eq!(format_summary_message(&result), "3 errors found");
    }

    #[test]
    fn test_format_summary_message_with_warnings_only() {
        let result = make_result(vec![], 0, 2);
        assert_eq!(format_summary_message(&result), "2 warnings found");
    }

    #[test]
    fn test_format_summary_message_with_both() {
        let result = make_result(vec![], 1, 1);
        assert_eq!(
            format_summary_message(&result),
            "1 error found, 1 warning found"
        );
    }

    #[test]
    fn test_format_summary_message_single_error() {
        let result = make_result(vec![], 1, 0);
        assert_eq!(format_summary_message(&result), "1 error found");
    }

    #[test]
    fn test_format_summary_message_single_warning() {
        let result = make_result(vec![], 0, 1);
        assert_eq!(format_summary_message(&result), "1 warning found");
    }

    // ========== Check Line Formatting Tests ==========

    #[test]
    fn test_format_check_line_ok_no_fix() {
        let check = make_check("All dependencies installed", DoctorStatus::Ok, None);
        assert_eq!(format_check_line(&check), "✓ All dependencies installed");
    }

    #[test]
    fn test_format_check_line_warning_with_fix() {
        let check = make_check(
            "Outdated packages detected",
            DoctorStatus::Warning,
            Some("Run 'zb upgrade' to update"),
        );
        let output = format_check_line(&check);
        assert!(output.contains("! Outdated packages detected"));
        assert!(output.contains("→ Run 'zb upgrade' to update"));
    }

    #[test]
    fn test_format_check_line_error_with_fix() {
        let check = make_check(
            "Prefix not writable",
            DoctorStatus::Error,
            Some("Run 'sudo chown -R $USER /opt/zerobrew'"),
        );
        let output = format_check_line(&check);
        assert!(output.contains("✗ Prefix not writable"));
        assert!(output.contains("→ Run 'sudo chown -R $USER /opt/zerobrew'"));
    }

    #[test]
    fn test_format_check_line_preserves_fix_newline() {
        let check = make_check("Problem found", DoctorStatus::Warning, Some("Fix it"));
        let output = format_check_line(&check);
        assert!(output.contains("\n    →")); // Fix should be on its own line, indented
    }

    // ========== Complete Output Formatting Tests ==========

    #[test]
    fn test_format_doctor_output_plain_healthy() {
        let checks = vec![
            make_check("Prefix is writable", DoctorStatus::Ok, None),
            make_check("Cellar structure valid", DoctorStatus::Ok, None),
        ];
        let result = make_result(checks, 0, 0);

        let output = format_doctor_output_plain(&result);

        assert!(output.starts_with("==> Running diagnostics..."));
        assert!(output.contains("✓ Prefix is writable"));
        assert!(output.contains("✓ Cellar structure valid"));
        assert!(output.contains("Your system is ready to brew!"));
    }

    #[test]
    fn test_format_doctor_output_plain_with_issues() {
        let checks = vec![
            make_check("Prefix is writable", DoctorStatus::Ok, None),
            make_check(
                "Broken symlinks found",
                DoctorStatus::Warning,
                Some("Run 'zb cleanup' to fix"),
            ),
            make_check(
                "Database corrupted",
                DoctorStatus::Error,
                Some("Run 'zb repair'"),
            ),
        ];
        let result = make_result(checks, 1, 1);

        let output = format_doctor_output_plain(&result);

        assert!(output.contains("✓ Prefix is writable"));
        assert!(output.contains("! Broken symlinks found"));
        assert!(output.contains("→ Run 'zb cleanup' to fix"));
        assert!(output.contains("✗ Database corrupted"));
        assert!(output.contains("1 error found, 1 warning found"));
    }

    #[test]
    fn test_format_doctor_output_plain_empty_checks() {
        let result = make_result(vec![], 0, 0);
        let output = format_doctor_output_plain(&result);

        assert!(output.contains("Running diagnostics"));
        assert!(output.contains("Your system is ready to brew!"));
    }

    // ========== Edge Case Tests ==========

    #[test]
    fn test_format_check_line_empty_message() {
        let check = make_check("", DoctorStatus::Ok, None);
        assert_eq!(format_check_line(&check), "✓ ");
    }

    #[test]
    fn test_format_check_line_empty_fix() {
        let check = make_check("Test", DoctorStatus::Warning, Some(""));
        let output = format_check_line(&check);
        assert!(output.contains("→ ")); // Empty fix still shows arrow
    }

    #[test]
    fn test_large_issue_counts() {
        assert_eq!(format_issue_count_plain(999, "error"), "999 errors found");
        assert_eq!(
            format_issue_count_plain(1000000, "warning"),
            "1000000 warnings found"
        );
    }

    #[test]
    fn test_summary_with_many_issues() {
        let result = make_result(vec![], 42, 17);
        assert_eq!(
            format_summary_message(&result),
            "42 errors found, 17 warnings found"
        );
    }

    // ========== Unicode and Special Characters ==========

    #[test]
    fn test_format_check_line_with_unicode() {
        let check = make_check("检查通过 ✨", DoctorStatus::Ok, None);
        assert_eq!(format_check_line(&check), "✓ 检查通过 ✨");
    }

    #[test]
    fn test_format_check_line_with_special_chars() {
        let check = make_check(
            "Path contains $HOME/bin",
            DoctorStatus::Ok,
            Some("export PATH=\"$HOME/bin:$PATH\""),
        );
        let output = format_check_line(&check);
        assert!(output.contains("$HOME/bin"));
        assert!(output.contains("export PATH"));
    }
}
