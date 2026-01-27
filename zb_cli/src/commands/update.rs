//! Update command - self-update zb to the latest release.

use console::style;
use std::env;
use std::fs;
use std::io::Write;

const GITHUB_RELEASES_API: &str = "https://api.github.com/repos/carlulsoe/zerobrew/releases/latest";

/// Get the binary name for the current platform.
fn get_platform_binary_name() -> Option<&'static str> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    match (os, arch) {
        ("macos", "x86_64") => Some("zb-darwin-x86_64"),
        ("macos", "aarch64") => Some("zb-darwin-aarch64"),
        ("linux", "x86_64") => Some("zb-linux-x86_64"),
        ("linux", "aarch64") => Some("zb-linux-aarch64"),
        _ => None,
    }
}

/// Fetch the latest release info from GitHub.
async fn fetch_latest_release() -> Result<(String, String), zb_core::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get(GITHUB_RELEASES_API)
        .header("User-Agent", "zerobrew")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| zb_core::Error::NetworkFailure {
            message: format!("Failed to fetch release info: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(zb_core::Error::NetworkFailure {
            message: format!("GitHub API returned status: {}", response.status()),
        });
    }

    let json: serde_json::Value =
        response
            .json()
            .await
            .map_err(|e| zb_core::Error::NetworkFailure {
                message: format!("Failed to parse release JSON: {}", e),
            })?;

    let tag_name = json["tag_name"]
        .as_str()
        .ok_or_else(|| zb_core::Error::NetworkFailure {
            message: "Release missing tag_name".to_string(),
        })?
        .to_string();

    let binary_name = get_platform_binary_name().ok_or_else(|| zb_core::Error::NetworkFailure {
        message: format!(
            "Unsupported platform: {}-{}",
            env::consts::OS,
            env::consts::ARCH
        ),
    })?;

    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| zb_core::Error::NetworkFailure {
            message: "Release missing assets".to_string(),
        })?;

    let download_url = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(binary_name))
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or_else(|| zb_core::Error::NetworkFailure {
            message: format!("No binary found for platform: {}", binary_name),
        })?
        .to_string();

    Ok((tag_name, download_url))
}

/// Get the current version from the binary.
fn get_current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Extract version components for comparison.
/// Returns (base_version, date, sha) from "v0.1.0-20260127.abc1234"
fn parse_version(version: &str) -> Option<(&str, &str, &str)> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let parts: Vec<&str> = version.split('-').collect();
    if parts.len() == 2 {
        let base = parts[0];
        let suffix_parts: Vec<&str> = parts[1].split('.').collect();
        if suffix_parts.len() == 2 {
            return Some((base, suffix_parts[0], suffix_parts[1]));
        }
    }
    // Simple version without date/sha suffix
    Some((version, "", ""))
}

/// Compare versions. Returns true if remote is newer.
fn is_newer_version(current: &str, remote: &str) -> bool {
    let current_parsed = parse_version(current);
    let remote_parsed = parse_version(remote);

    match (current_parsed, remote_parsed) {
        (Some((c_base, c_date, _)), Some((r_base, r_date, _))) => {
            // Compare base versions first
            if c_base != r_base {
                return version_cmp(r_base, c_base);
            }
            // Same base version, compare dates
            r_date > c_date
        }
        _ => false,
    }
}

/// Simple semver comparison. Returns true if a > b.
fn version_cmp(a: &str, b: &str) -> bool {
    let a_parts: Vec<u32> = a.split('.').filter_map(|s| s.parse().ok()).collect();
    let b_parts: Vec<u32> = b.split('.').filter_map(|s| s.parse().ok()).collect();

    for i in 0..std::cmp::max(a_parts.len(), b_parts.len()) {
        let a_val = a_parts.get(i).copied().unwrap_or(0);
        let b_val = b_parts.get(i).copied().unwrap_or(0);
        if a_val > b_val {
            return true;
        }
        if a_val < b_val {
            return false;
        }
    }
    false
}

/// Download and replace the current binary.
async fn download_and_replace(download_url: &str) -> Result<(), zb_core::Error> {
    let current_exe = env::current_exe().map_err(|e| zb_core::Error::StoreCorruption {
        message: format!("Failed to get current executable path: {}", e),
    })?;

    println!(
        "    {} Downloading from {}",
        style("→").cyan(),
        style(&download_url).dim()
    );

    let client = reqwest::Client::new();
    let response = client
        .get(download_url)
        .header("User-Agent", "zerobrew")
        .send()
        .await
        .map_err(|e| zb_core::Error::NetworkFailure {
            message: format!("Failed to download binary: {}", e),
        })?;

    if !response.status().is_success() {
        return Err(zb_core::Error::NetworkFailure {
            message: format!("Download failed with status: {}", response.status()),
        });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| zb_core::Error::NetworkFailure {
            message: format!("Failed to read download: {}", e),
        })?;

    // Write to a temp file first
    let temp_path = current_exe.with_extension("new");
    let mut temp_file =
        fs::File::create(&temp_path).map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("Failed to create temp file: {}", e),
        })?;
    temp_file
        .write_all(&bytes)
        .map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("Failed to write temp file: {}", e),
        })?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_path)
            .map_err(|e| zb_core::Error::StoreCorruption {
                message: format!("Failed to get temp file metadata: {}", e),
            })?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_path, perms).map_err(|e| zb_core::Error::StoreCorruption {
            message: format!("Failed to set executable permissions: {}", e),
        })?;
    }

    // Atomic replace
    let backup_path = current_exe.with_extension("old");
    if backup_path.exists() {
        fs::remove_file(&backup_path).ok();
    }

    // Rename current to backup, then new to current
    fs::rename(&current_exe, &backup_path).map_err(|e| zb_core::Error::StoreCorruption {
        message: format!("Failed to backup current binary: {}", e),
    })?;

    if let Err(e) = fs::rename(&temp_path, &current_exe) {
        // Try to restore backup
        let _ = fs::rename(&backup_path, &current_exe);
        return Err(zb_core::Error::StoreCorruption {
            message: format!("Failed to replace binary: {}", e),
        });
    }

    // Clean up backup
    fs::remove_file(&backup_path).ok();

    Ok(())
}

/// Run the update command.
pub async fn run(dry_run: bool, force: bool) -> Result<(), zb_core::Error> {
    println!("{} Checking for updates...", style("==>").cyan().bold());

    let current_version = get_current_version();
    println!(
        "    {} Current version: {}",
        style("→").dim(),
        style(current_version).cyan()
    );

    let (latest_version, download_url) = fetch_latest_release().await?;
    println!(
        "    {} Latest version:  {}",
        style("→").dim(),
        style(&latest_version).cyan()
    );

    let needs_update = force || is_newer_version(current_version, &latest_version);

    if !needs_update {
        println!(
            "\n{} {} is already up to date.",
            style("==>").cyan().bold(),
            style("zb").green()
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "\n{} Would update {} → {}",
            style("==>").cyan().bold(),
            style(current_version).yellow(),
            style(&latest_version).green()
        );
        println!(
            "    {} Run {} to install the update",
            style("→").dim(),
            style("zb update").cyan()
        );
        return Ok(());
    }

    println!(
        "\n{} Updating {} → {}",
        style("==>").cyan().bold(),
        style(current_version).yellow(),
        style(&latest_version).green()
    );

    download_and_replace(&download_url).await?;

    println!(
        "\n{} {} Updated successfully!",
        style("==>").cyan().bold(),
        style("✓").green()
    );
    println!(
        "    {} Run {} to verify",
        style("→").dim(),
        style("zb --version").cyan()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_platform_binary_name() {
        // This will return a value on supported platforms
        let result = get_platform_binary_name();
        // On CI, this should return Some value for the supported platforms
        if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
            assert_eq!(result, Some("zb-darwin-x86_64"));
        } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            assert_eq!(result, Some("zb-darwin-aarch64"));
        } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            assert_eq!(result, Some("zb-linux-x86_64"));
        } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
            assert_eq!(result, Some("zb-linux-aarch64"));
        }
    }

    #[test]
    fn test_parse_version_with_suffix() {
        let result = parse_version("v0.1.0-20260127.abc1234");
        assert_eq!(result, Some(("0.1.0", "20260127", "abc1234")));
    }

    #[test]
    fn test_parse_version_without_prefix() {
        let result = parse_version("0.1.0-20260127.abc1234");
        assert_eq!(result, Some(("0.1.0", "20260127", "abc1234")));
    }

    #[test]
    fn test_parse_version_simple() {
        let result = parse_version("0.1.0");
        assert_eq!(result, Some(("0.1.0", "", "")));
    }

    #[test]
    fn test_is_newer_version_newer_date() {
        assert!(is_newer_version(
            "0.1.0-20260126.abc1234",
            "v0.1.0-20260127.def5678"
        ));
    }

    #[test]
    fn test_is_newer_version_same() {
        assert!(!is_newer_version(
            "0.1.0-20260127.abc1234",
            "v0.1.0-20260127.abc1234"
        ));
    }

    #[test]
    fn test_is_newer_version_older() {
        assert!(!is_newer_version(
            "0.1.0-20260127.abc1234",
            "v0.1.0-20260126.def5678"
        ));
    }

    #[test]
    fn test_is_newer_version_newer_base() {
        assert!(is_newer_version(
            "0.1.0-20260127.abc1234",
            "v0.2.0-20260126.def5678"
        ));
    }

    #[test]
    fn test_version_cmp_greater() {
        assert!(version_cmp("0.2.0", "0.1.0"));
        assert!(version_cmp("1.0.0", "0.9.9"));
        assert!(version_cmp("0.1.1", "0.1.0"));
    }

    #[test]
    fn test_version_cmp_equal() {
        assert!(!version_cmp("0.1.0", "0.1.0"));
    }

    #[test]
    fn test_version_cmp_less() {
        assert!(!version_cmp("0.1.0", "0.2.0"));
    }
}
