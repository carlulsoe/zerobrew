//! Tap and untap command implementations.

use console::style;

use zb_io::install::Installer;

/// Parse a tap name in user/repo format.
///
/// Returns `Ok((user, repo))` if the format is valid, or an error message if not.
pub fn parse_tap_name(user_repo: &str) -> Result<(&str, &str), String> {
    let parts: Vec<&str> = user_repo.split('/').collect();

    if parts.len() != 2 {
        return Err(format!(
            "invalid tap format '{}': expected user/repo",
            user_repo
        ));
    }

    let (user, repo) = (parts[0], parts[1]);

    if user.is_empty() {
        return Err(format!(
            "invalid tap format '{}': user cannot be empty",
            user_repo
        ));
    }

    if repo.is_empty() {
        return Err(format!(
            "invalid tap format '{}': repo cannot be empty",
            user_repo
        ));
    }

    Ok((user, repo))
}

/// Format tap list output for display.
///
/// Returns a vector of formatted lines.
pub fn format_tap_list(tap_names: &[String]) -> Vec<String> {
    if tap_names.is_empty() {
        vec![
            format!("{} No taps installed", style("==>").cyan().bold()),
            format!(
                "\n    → Add a tap with: {} tap user/repo",
                style("zb").cyan()
            ),
        ]
    } else {
        let mut lines = vec![format!(
            "{} {} installed taps:",
            style("==>").cyan().bold(),
            tap_names.len()
        )];

        for name in tap_names {
            lines.push(format!("    {}", name));
        }

        lines
    }
}

/// Run the tap command.
pub async fn run_tap(
    installer: &mut Installer,
    user_repo: Option<String>,
) -> Result<(), zb_core::Error> {
    match user_repo {
        None => {
            // List taps
            let taps = installer.list_taps()?;
            let tap_names: Vec<String> = taps.iter().map(|t| t.name.clone()).collect();

            for line in format_tap_list(&tap_names) {
                println!("{}", line);
            }
        }
        Some(user_repo) => {
            // Add tap
            let (user, repo) =
                parse_tap_name(&user_repo).map_err(|message| zb_core::Error::StoreCorruption {
                    message,
                })?;

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
    let (user, repo) =
        parse_tap_name(&user_repo).map_err(|message| zb_core::Error::StoreCorruption {
            message,
        })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    mod parse_tap_name {
        use super::*;

        #[test]
        fn valid_user_repo_format() {
            let result = parse_tap_name("homebrew/core");
            assert_eq!(result, Ok(("homebrew", "core")));
        }

        #[test]
        fn valid_with_hyphens_and_underscores() {
            let result = parse_tap_name("my-user/my_repo");
            assert_eq!(result, Ok(("my-user", "my_repo")));
        }

        #[test]
        fn valid_with_numbers() {
            let result = parse_tap_name("user123/repo456");
            assert_eq!(result, Ok(("user123", "repo456")));
        }

        #[test]
        fn missing_slash() {
            let result = parse_tap_name("homebrew-core");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("expected user/repo"));
        }

        #[test]
        fn too_many_slashes() {
            let result = parse_tap_name("homebrew/core/extra");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("expected user/repo"));
        }

        #[test]
        fn empty_user() {
            let result = parse_tap_name("/repo");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("user cannot be empty"));
        }

        #[test]
        fn empty_repo() {
            let result = parse_tap_name("user/");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("repo cannot be empty"));
        }

        #[test]
        fn both_empty() {
            let result = parse_tap_name("/");
            assert!(result.is_err());
            // Should hit user empty check first
            assert!(result.unwrap_err().contains("user cannot be empty"));
        }

        #[test]
        fn empty_string() {
            let result = parse_tap_name("");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("expected user/repo"));
        }

        #[test]
        fn just_slash() {
            let result = parse_tap_name("/");
            assert!(result.is_err());
        }

        #[test]
        fn url_like_input_rejected() {
            // GitHub URLs should not be accepted, only user/repo format
            let result = parse_tap_name("https://github.com/homebrew/core");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("expected user/repo"));
        }
    }

    mod format_tap_list {
        use super::*;

        #[test]
        fn empty_list_shows_hint() {
            let result = format_tap_list(&[]);
            assert_eq!(result.len(), 2);
            // Check content without ANSI codes
            let plain: String = result[0].chars().filter(|c| !c.is_control()).collect();
            assert!(plain.contains("No taps installed"));
        }

        #[test]
        fn single_tap_shows_count() {
            let taps = vec!["homebrew/core".to_string()];
            let result = format_tap_list(&taps);
            assert_eq!(result.len(), 2); // header + 1 tap
            let plain: String = result[0].chars().filter(|c| !c.is_control()).collect();
            assert!(plain.contains("1 installed taps"));
            assert!(result[1].contains("homebrew/core"));
        }

        #[test]
        fn multiple_taps_shows_all() {
            let taps = vec![
                "homebrew/core".to_string(),
                "homebrew/cask".to_string(),
                "my-user/my-tap".to_string(),
            ];
            let result = format_tap_list(&taps);
            assert_eq!(result.len(), 4); // header + 3 taps
            let plain: String = result[0].chars().filter(|c| !c.is_control()).collect();
            assert!(plain.contains("3 installed taps"));
            assert!(result[1].contains("homebrew/core"));
            assert!(result[2].contains("homebrew/cask"));
            assert!(result[3].contains("my-user/my-tap"));
        }

        #[test]
        fn taps_are_indented() {
            let taps = vec!["test/tap".to_string()];
            let result = format_tap_list(&taps);
            assert!(result[1].starts_with("    ")); // 4 spaces
        }
    }

    mod error_messages {
        use super::*;

        #[test]
        fn error_includes_original_input() {
            let result = parse_tap_name("bad-input");
            let err = result.unwrap_err();
            assert!(err.contains("bad-input"));
        }

        #[test]
        fn error_for_url_is_descriptive() {
            let result = parse_tap_name("https://github.com/user/repo");
            let err = result.unwrap_err();
            assert!(err.contains("https://github.com/user/repo"));
            assert!(err.contains("expected user/repo"));
        }
    }
}
