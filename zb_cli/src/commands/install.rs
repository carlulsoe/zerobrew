//! Install command implementation.

use console::style;
use indicatif::MultiProgress;
use std::path::Path;
use std::time::Instant;

use zb_core::formula::KegOnlyReason;
use zb_io::install::Installer;

use crate::display::{
    create_progress_callback, finish_progress_bars, suggest_homebrew, ProgressStyles,
};

/// Run the install command.
pub async fn run(
    installer: &mut Installer,
    prefix: &Path,
    formula: String,
    no_link: bool,
    build_from_source: bool,
    head: bool,
) -> Result<(), zb_core::Error> {
    let start = Instant::now();

    // HEAD implies building from source
    let build_from_source = build_from_source || head;

    if build_from_source {
        run_source_install(installer, prefix, &formula, no_link, head, start).await
    } else {
        run_bottle_install(installer, prefix, &formula, no_link, start).await
    }
}

async fn run_source_install(
    installer: &mut Installer,
    prefix: &Path,
    formula: &str,
    no_link: bool,
    head: bool,
    start: Instant,
) -> Result<(), zb_core::Error> {
    let build_type = if head { "HEAD" } else { "source" };
    println!(
        "{} Building {} from {}...",
        style("==>").cyan().bold(),
        style(formula).bold(),
        build_type
    );

    println!(
        "{} Downloading source and dependencies...",
        style("==>").cyan().bold()
    );

    let result = match installer
        .install_from_source(formula, !no_link, head)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} Built and installed {} {} ({} files) in {:.2}s",
        style("==>").cyan().bold(),
        style(&result.name).green().bold(),
        style(&result.version).dim(),
        result.files_installed,
        elapsed.as_secs_f64()
    );
    if result.files_linked > 0 {
        println!(
            "    {} Linked {} files",
            style("✓").green(),
            result.files_linked
        );
    }

    // Display keg-only and caveats info if present
    if let Ok(formula_info) = installer.get_formula(formula).await {
        print_keg_only_info(
            formula_info.keg_only,
            formula_info.keg_only_reason.as_ref(),
            prefix,
            formula,
        );
        print_caveats(formula_info.caveats.as_ref(), prefix);
    }

    Ok(())
}

async fn run_bottle_install(
    installer: &mut Installer,
    prefix: &Path,
    formula: &str,
    no_link: bool,
    start: Instant,
) -> Result<(), zb_core::Error> {
    println!(
        "{} Installing {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    let plan = match installer.plan(formula).await {
        Ok(p) => p,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    // Extract info from the root formula before executing the plan
    let root_formula = plan.formulas.iter().find(|f| f.name == plan.root_name);
    let root_caveats = root_formula.and_then(|f| f.caveats.clone());
    let root_keg_only = root_formula.map(|f| f.keg_only).unwrap_or(false);
    let root_keg_only_reason = root_formula.and_then(|f| f.keg_only_reason.clone());

    println!(
        "{} Resolving dependencies ({} packages)...",
        style("==>").cyan().bold(),
        plan.formulas.len()
    );
    for f in &plan.formulas {
        println!(
            "    {} {}",
            style(&f.name).green(),
            style(&f.versions.stable).dim()
        );
    }

    println!(
        "{} Downloading and installing...",
        style("==>").cyan().bold()
    );

    let multi = MultiProgress::new();
    let styles = ProgressStyles::default();
    let (progress_callback, bars) = create_progress_callback(multi, styles, "installed");

    let result = match installer
        .execute_with_progress(plan, !no_link, Some(progress_callback))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            suggest_homebrew(formula, &e);
            return Err(e);
        }
    };

    finish_progress_bars(&bars);

    let elapsed = start.elapsed();
    println!();
    println!(
        "{} Installed {} packages in {:.2}s",
        style("==>").cyan().bold(),
        style(result.installed).green().bold(),
        elapsed.as_secs_f64()
    );

    // Display keg-only and caveats info if present
    print_keg_only_info(root_keg_only, root_keg_only_reason.as_ref(), prefix, formula);
    print_caveats(root_caveats.as_ref(), prefix);

    Ok(())
}

/// Print keg-only information for a formula.
fn print_keg_only_info(
    keg_only: bool,
    keg_only_reason: Option<&KegOnlyReason>,
    prefix: &Path,
    formula: &str,
) {
    if !keg_only {
        return;
    }

    println!();
    println!("{}", style("==> Keg-only").yellow().bold());
    println!(
        "{} is keg-only, which means it was not symlinked into {}",
        style(formula).bold(),
        prefix.display()
    );
    if let Some(reason) = keg_only_reason
        && !reason.explanation.is_empty()
    {
        println!();
        println!("{}", reason.explanation);
    }
    println!();
    println!("To use this formula, you can:");
    println!(
        "    • Add it to your PATH: {}",
        style(format!(
            "export PATH=\"{}/opt/{}/bin:$PATH\"",
            prefix.display(),
            formula
        ))
        .cyan()
    );
    println!(
        "    • Link it with: {}",
        style(format!("zb link {} --force", formula)).cyan()
    );
}

/// Print caveats for a formula.
fn print_caveats(caveats: Option<&String>, prefix: &Path) {
    let Some(caveats) = caveats else { return };

    println!();
    println!("{}", style("==> Caveats").yellow().bold());
    let caveats = caveats.replace("$HOMEBREW_PREFIX", &prefix.to_string_lossy());
    for line in caveats.lines() {
        println!("{}", line);
    }
}

/// Substitute $HOMEBREW_PREFIX in caveats text.
/// Extracted for testability.
pub(crate) fn substitute_prefix(text: &str, prefix: &Path) -> String {
    text.replace("$HOMEBREW_PREFIX", &prefix.to_string_lossy())
}

/// Build keg-only PATH suggestion.
/// Extracted for testability.
pub(crate) fn build_keg_only_path_suggestion(prefix: &Path, formula: &str) -> String {
    format!(
        "export PATH=\"{}/opt/{}/bin:$PATH\"",
        prefix.display(),
        formula
    )
}

/// Build keg-only link suggestion.
/// Extracted for testability.
pub(crate) fn build_keg_only_link_suggestion(formula: &str) -> String {
    format!("zb link {} --force", formula)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_substitute_prefix_basic() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let text = "Add $HOMEBREW_PREFIX/bin to your PATH";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "Add /opt/zerobrew/prefix/bin to your PATH");
    }

    #[test]
    fn test_substitute_prefix_multiple_occurrences() {
        let prefix = PathBuf::from("/usr/local");
        let text = "$HOMEBREW_PREFIX/bin and $HOMEBREW_PREFIX/sbin";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "/usr/local/bin and /usr/local/sbin");
    }

    #[test]
    fn test_substitute_prefix_no_placeholder() {
        let prefix = PathBuf::from("/opt/zerobrew");
        let text = "No placeholder here";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "No placeholder here");
    }

    #[test]
    fn test_substitute_prefix_empty_string() {
        let prefix = PathBuf::from("/opt/zerobrew");
        let text = "";
        let result = substitute_prefix(text, &prefix);
        assert_eq!(result, "");
    }

    #[test]
    fn test_build_keg_only_path_suggestion() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let result = build_keg_only_path_suggestion(&prefix, "openssl@3");
        assert_eq!(
            result,
            "export PATH=\"/opt/zerobrew/prefix/opt/openssl@3/bin:$PATH\""
        );
    }

    #[test]
    fn test_build_keg_only_path_suggestion_versioned_formula() {
        let prefix = PathBuf::from("/usr/local");
        let result = build_keg_only_path_suggestion(&prefix, "python@3.11");
        assert_eq!(
            result,
            "export PATH=\"/usr/local/opt/python@3.11/bin:$PATH\""
        );
    }

    #[test]
    fn test_build_keg_only_link_suggestion() {
        let result = build_keg_only_link_suggestion("openssl@3");
        assert_eq!(result, "zb link openssl@3 --force");
    }

    #[test]
    fn test_build_keg_only_link_suggestion_simple_formula() {
        let result = build_keg_only_link_suggestion("readline");
        assert_eq!(result, "zb link readline --force");
    }
}
