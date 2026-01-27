//! Tap and untap command implementations.

use console::style;

use zb_io::install::Installer;

/// Run the tap command.
pub async fn run_tap(
    installer: &mut Installer,
    user_repo: Option<String>,
) -> Result<(), zb_core::Error> {
    match user_repo {
        None => {
            // List taps
            let taps = installer.list_taps()?;

            if taps.is_empty() {
                println!("{} No taps installed", style("==>").cyan().bold());
                println!(
                    "\n    → Add a tap with: {} tap user/repo",
                    style("zb").cyan()
                );
            } else {
                println!(
                    "{} {} installed taps:",
                    style("==>").cyan().bold(),
                    taps.len()
                );

                for tap in taps {
                    println!("    {}", tap.name);
                }
            }
        }
        Some(user_repo) => {
            // Add tap
            let parts: Vec<&str> = user_repo.split('/').collect();
            if parts.len() != 2 {
                return Err(zb_core::Error::StoreCorruption {
                    message: format!("invalid tap format '{}': expected user/repo", user_repo),
                });
            }

            let (user, repo) = (parts[0], parts[1]);

            println!(
                "{} Tapping {}...",
                style("==>").cyan().bold(),
                style(&user_repo).bold()
            );

            installer.add_tap(user, repo).await?;

            println!(
                "\n{} {} Tapped {}",
                style("==>").cyan().bold(),
                style("✓").green().bold(),
                style(&user_repo).bold()
            );
        }
    }

    Ok(())
}

/// Run the untap command.
pub fn run_untap(installer: &mut Installer, user_repo: String) -> Result<(), zb_core::Error> {
    let parts: Vec<&str> = user_repo.split('/').collect();
    if parts.len() != 2 {
        return Err(zb_core::Error::StoreCorruption {
            message: format!("invalid tap format '{}': expected user/repo", user_repo),
        });
    }

    let (user, repo) = (parts[0], parts[1]);

    println!(
        "{} Untapping {}...",
        style("==>").cyan().bold(),
        style(&user_repo).bold()
    );

    installer.remove_tap(user, repo)?;

    println!(
        "\n{} {} Untapped {}",
        style("==>").cyan().bold(),
        style("✓").green().bold(),
        style(&user_repo).bold()
    );

    Ok(())
}
