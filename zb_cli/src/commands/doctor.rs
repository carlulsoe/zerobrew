//! Doctor command implementation.

use console::style;

use zb_io::install::Installer;

/// Run the doctor command.
pub async fn run(installer: &mut Installer) -> Result<(), zb_core::Error> {
    println!("{} Running diagnostics...\n", style("==>").cyan().bold());

    let result = installer.doctor().await;

    for check in &result.checks {
        let (marker, _color) = match check.status {
            zb_io::DoctorStatus::Ok => (style("✓").green(), ""),
            zb_io::DoctorStatus::Warning => (style("!").yellow(), ""),
            zb_io::DoctorStatus::Error => (style("✗").red(), ""),
        };

        println!("{} {}", marker, check.message);

        if let Some(ref fix) = check.fix {
            println!("    {} {}", style("→").dim(), style(fix).dim());
        }
    }

    println!();
    if result.is_healthy() {
        println!(
            "{} Your system is ready to brew!",
            style("==>").cyan().bold()
        );
    } else {
        if result.errors > 0 {
            println!(
                "{} {} {} found",
                style("==>").cyan().bold(),
                style(result.errors).red().bold(),
                if result.errors == 1 {
                    "error"
                } else {
                    "errors"
                }
            );
        }
        if result.warnings > 0 {
            println!(
                "{} {} {} found",
                style("==>").cyan().bold(),
                style(result.warnings).yellow().bold(),
                if result.warnings == 1 {
                    "warning"
                } else {
                    "warnings"
                }
            );
        }
    }

    Ok(())
}
