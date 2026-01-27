//! Display utilities for progress bars and formatting helpers.

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zb_io::{DepsTree, InstallProgress, ProgressCallback};

/// Create progress styles used by install/upgrade operations.
pub struct ProgressStyles {
    pub download: ProgressStyle,
    pub spinner: ProgressStyle,
    pub done: ProgressStyle,
}

impl Default for ProgressStyles {
    fn default() -> Self {
        Self {
            download: ProgressStyle::default_bar()
                .template(
                    "    {prefix:<16} {bar:25.cyan/dim} {bytes:>10}/{total_bytes:<10} {eta:>6}",
                )
                .unwrap()
                .progress_chars("━━╸"),
            spinner: ProgressStyle::default_spinner()
                .template("    {prefix:<16} {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
            done: ProgressStyle::default_spinner()
                .template("    {prefix:<16} {msg}")
                .unwrap(),
        }
    }
}

/// Create a progress callback for install/upgrade operations.
pub fn create_progress_callback(
    multi: MultiProgress,
    styles: ProgressStyles,
    completion_message: &'static str,
) -> (Arc<ProgressCallback>, Arc<Mutex<HashMap<String, ProgressBar>>>) {
    let bars: Arc<Mutex<HashMap<String, ProgressBar>>> = Arc::new(Mutex::new(HashMap::new()));

    let bars_clone = bars.clone();
    let download_style = styles.download;
    let spinner_style = styles.spinner;
    let done_style = styles.done;

    let callback: Arc<ProgressCallback> = Arc::new(Box::new(move |event| {
        let mut bars = bars_clone.lock().unwrap();
        match event {
            InstallProgress::DownloadStarted { name, total_bytes } => {
                let pb = if let Some(total) = total_bytes {
                    let pb = multi.add(ProgressBar::new(total));
                    pb.set_style(download_style.clone());
                    pb
                } else {
                    let pb = multi.add(ProgressBar::new_spinner());
                    pb.set_style(spinner_style.clone());
                    pb.set_message("downloading...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(80));
                    pb
                };
                pb.set_prefix(name.clone());
                bars.insert(name, pb);
            }
            InstallProgress::DownloadProgress {
                name,
                downloaded,
                total_bytes,
            } => {
                if let Some(pb) = bars.get(&name)
                    && total_bytes.is_some()
                {
                    pb.set_position(downloaded);
                }
            }
            InstallProgress::DownloadCompleted { name, total_bytes } => {
                if let Some(pb) = bars.get(&name) {
                    if total_bytes > 0 {
                        pb.set_position(total_bytes);
                    }
                    pb.set_style(spinner_style.clone());
                    pb.set_message("unpacking...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(80));
                }
            }
            InstallProgress::UnpackStarted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("unpacking...");
                }
            }
            InstallProgress::UnpackCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("linking...");
                }
            }
            InstallProgress::LinkStarted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("linking...");
                }
            }
            InstallProgress::LinkCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_message("linked");
                }
            }
            InstallProgress::InstallCompleted { name } => {
                if let Some(pb) = bars.get(&name) {
                    pb.set_style(done_style.clone());
                    pb.set_message(format!("{} {}", style("✓").green(), completion_message));
                    pb.finish();
                }
            }
        }
    }));

    (callback, bars)
}

/// Finish any remaining progress bars.
pub fn finish_progress_bars(bars: &Arc<Mutex<HashMap<String, ProgressBar>>>) {
    let bars = bars.lock().unwrap();
    for (_, pb) in bars.iter() {
        if !pb.is_finished() {
            pb.finish();
        }
    }
}

/// Format bytes into a human-readable string (e.g., "1.5 GB").
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Format a Unix timestamp in a simple human-readable way.
pub fn chrono_lite_format(timestamp: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let dt = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    format!("{:?}", dt)
}

/// Calculate the tree connector string based on position.
pub fn tree_connector(prefix: &str, is_last: bool) -> &'static str {
    if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    }
}

/// Calculate the prefix for child nodes.
pub fn tree_child_prefix(prefix: &str, is_last: bool) -> String {
    if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    }
}

/// Format the installed marker.
pub fn installed_marker(installed: bool) -> String {
    if installed {
        style("✓").green().to_string()
    } else {
        style("✗").red().to_string()
    }
}

/// Format a single tree line (without children).
pub fn format_tree_line(name: &str, installed: bool, prefix: &str, is_last: bool) -> String {
    let connector = tree_connector(prefix, is_last);
    let marker = installed_marker(installed);
    format!("{}{}{} {}", prefix, connector, marker, name)
}

/// Format a dependency tree as a string (for testing).
/// Returns a vector of lines representing the tree.
pub fn format_deps_tree_lines(tree: &DepsTree, prefix: &str, is_last: bool) -> Vec<String> {
    let mut lines = Vec::new();
    format_deps_tree_recursive(tree, prefix, is_last, &mut lines);
    lines
}

fn format_deps_tree_recursive(tree: &DepsTree, prefix: &str, is_last: bool, lines: &mut Vec<String>) {
    lines.push(format_tree_line(&tree.name, tree.installed, prefix, is_last));
    
    let new_prefix = tree_child_prefix(prefix, is_last);
    
    for (i, child) in tree.children.iter().enumerate() {
        let is_last_child = i == tree.children.len() - 1;
        format_deps_tree_recursive(child, &new_prefix, is_last_child, lines);
    }
}

/// Print a dependency tree with ASCII art formatting.
pub fn print_deps_tree(tree: &DepsTree, prefix: &str, is_last: bool) {
    for line in format_deps_tree_lines(tree, prefix, is_last) {
        println!("{}", line);
    }
}

/// Detect the current shell from environment.
pub fn detect_shell() -> &'static str {
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("fish") {
            return "fish";
        } else if shell.contains("csh") || shell.contains("tcsh") {
            return "csh";
        } else if shell.contains("zsh") {
            return "zsh";
        }
    }
    "bash"
}

/// Generate shell environment setup commands.
pub fn generate_shellenv(prefix: &std::path::Path, shell: &str) -> String {
    let bin_path = prefix.join("bin");
    let sbin_path = prefix.join("sbin");
    let man_path = prefix.join("share").join("man");
    let info_path = prefix.join("share").join("info");
    let cellar_path = prefix.join("Cellar");

    match shell {
        "fish" => {
            format!(
                r#"set -gx HOMEBREW_PREFIX "{}";
set -gx HOMEBREW_CELLAR "{}";
set -gx PATH "{}" "{}" $PATH;
set -q MANPATH; or set MANPATH ''; set -gx MANPATH "{}" $MANPATH;
set -q INFOPATH; or set INFOPATH ''; set -gx INFOPATH "{}" $INFOPATH;"#,
                prefix.display(),
                cellar_path.display(),
                bin_path.display(),
                sbin_path.display(),
                man_path.display(),
                info_path.display()
            )
        }
        "csh" | "tcsh" => {
            format!(
                r#"setenv HOMEBREW_PREFIX "{}";
setenv HOMEBREW_CELLAR "{}";
setenv PATH "{}:{}:${{PATH}}";
setenv MANPATH "{}:${{MANPATH}}";
setenv INFOPATH "{}:${{INFOPATH}}";"#,
                prefix.display(),
                cellar_path.display(),
                bin_path.display(),
                sbin_path.display(),
                man_path.display(),
                info_path.display()
            )
        }
        _ => {
            format!(
                r#"export HOMEBREW_PREFIX="{}";
export HOMEBREW_CELLAR="{}";
export PATH="{}:{}:$PATH";
export MANPATH="{}:${{MANPATH:-}}";
export INFOPATH="{}:${{INFOPATH:-}}";"#,
                prefix.display(),
                cellar_path.display(),
                bin_path.display(),
                sbin_path.display(),
                man_path.display(),
                info_path.display()
            )
        }
    }
}

/// Print shell environment setup commands.
pub fn print_shellenv(prefix: &std::path::Path, shell: Option<&str>) {
    let shell = match shell {
        Some(s) => s,
        None => detect_shell(),
    };
    println!("{}", generate_shellenv(prefix, shell));
}

/// Suggest using Homebrew for unsupported packages.
pub fn suggest_homebrew(formula: &str, error: &zb_core::Error) {
    eprintln!();
    eprintln!(
        "{} This package can't be installed with zerobrew.",
        style("Note:").yellow().bold()
    );
    eprintln!("      Error: {}", error);
    eprintln!();
    eprintln!("      Try installing with Homebrew instead:");
    eprintln!(
        "      {}",
        style(format!("brew install {}", formula)).cyan()
    );
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_generate_shellenv_bash() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "bash");

        assert!(output.contains("export HOMEBREW_PREFIX=\"/opt/zerobrew/prefix\""));
        assert!(output.contains("export HOMEBREW_CELLAR=\"/opt/zerobrew/prefix/Cellar\""));
        assert!(output.contains(
            "export PATH=\"/opt/zerobrew/prefix/bin:/opt/zerobrew/prefix/sbin:$PATH\""
        ));
        assert!(output.contains("export MANPATH=\"/opt/zerobrew/prefix/share/man:${MANPATH:-}\""));
        assert!(
            output.contains("export INFOPATH=\"/opt/zerobrew/prefix/share/info:${INFOPATH:-}\"")
        );
    }

    #[test]
    fn test_generate_shellenv_zsh() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "zsh");

        assert!(output.contains("export HOMEBREW_PREFIX="));
        assert!(output.contains("export PATH="));
    }

    #[test]
    fn test_generate_shellenv_fish() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "fish");

        assert!(output.contains("set -gx HOMEBREW_PREFIX \"/opt/zerobrew/prefix\""));
        assert!(output.contains("set -gx HOMEBREW_CELLAR \"/opt/zerobrew/prefix/Cellar\""));
        assert!(output.contains(
            "set -gx PATH \"/opt/zerobrew/prefix/bin\" \"/opt/zerobrew/prefix/sbin\" $PATH"
        ));
        assert!(output.contains("set -q MANPATH; or set MANPATH ''"));
        assert!(output.contains("set -q INFOPATH; or set INFOPATH ''"));
    }

    #[test]
    fn test_generate_shellenv_csh() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "csh");

        assert!(output.contains("setenv HOMEBREW_PREFIX \"/opt/zerobrew/prefix\""));
        assert!(output.contains("setenv HOMEBREW_CELLAR \"/opt/zerobrew/prefix/Cellar\""));
        assert!(output.contains(
            "setenv PATH \"/opt/zerobrew/prefix/bin:/opt/zerobrew/prefix/sbin:${PATH}\""
        ));
        assert!(output.contains("setenv MANPATH \"/opt/zerobrew/prefix/share/man:${MANPATH}\""));
        assert!(output.contains("setenv INFOPATH \"/opt/zerobrew/prefix/share/info:${INFOPATH}\""));
    }

    #[test]
    fn test_generate_shellenv_tcsh() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "tcsh");

        assert!(output.contains("setenv HOMEBREW_PREFIX"));
        assert!(output.contains("setenv PATH"));
    }

    #[test]
    fn test_generate_shellenv_custom_prefix() {
        let prefix = PathBuf::from("/usr/local/homebrew");
        let output = generate_shellenv(&prefix, "bash");

        assert!(output.contains("/usr/local/homebrew"));
        assert!(output.contains("/usr/local/homebrew/bin"));
        assert!(output.contains("/usr/local/homebrew/sbin"));
        assert!(output.contains("/usr/local/homebrew/Cellar"));
        assert!(output.contains("/usr/local/homebrew/share/man"));
        assert!(output.contains("/usr/local/homebrew/share/info"));
    }

    #[test]
    fn test_generate_shellenv_unknown_shell_defaults_to_posix() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "unknown");

        assert!(output.contains("export HOMEBREW_PREFIX="));
        assert!(output.contains("export PATH="));
    }

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(1), "1 bytes");
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(1023), "1023 bytes");
    }

    #[test]
    fn test_format_bytes_kilobytes() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(10240), "10.0 KB");
    }

    #[test]
    fn test_format_bytes_megabytes() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 + 512 * 1024), "1.5 MB");
        assert_eq!(format_bytes(100 * 1024 * 1024), "100.0 MB");
    }

    #[test]
    fn test_format_bytes_gigabytes() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    // ========================================================================
    // Tree Formatting Tests
    // ========================================================================

    #[test]
    fn test_tree_connector_root() {
        assert_eq!(tree_connector("", true), "");
        assert_eq!(tree_connector("", false), "");
    }

    #[test]
    fn test_tree_connector_last_child() {
        assert_eq!(tree_connector("  ", true), "└── ");
    }

    #[test]
    fn test_tree_connector_middle_child() {
        assert_eq!(tree_connector("  ", false), "├── ");
    }

    #[test]
    fn test_tree_child_prefix_root() {
        assert_eq!(tree_child_prefix("", true), "");
        assert_eq!(tree_child_prefix("", false), "");
    }

    #[test]
    fn test_tree_child_prefix_last() {
        assert_eq!(tree_child_prefix("  ", true), "      ");
    }

    #[test]
    fn test_tree_child_prefix_middle() {
        assert_eq!(tree_child_prefix("  ", false), "  │   ");
    }

    #[test]
    fn test_installed_marker_installed() {
        let marker = installed_marker(true);
        assert!(marker.contains("✓"));
    }

    #[test]
    fn test_installed_marker_not_installed() {
        let marker = installed_marker(false);
        assert!(marker.contains("✗"));
    }

    #[test]
    fn test_format_tree_line_root_installed() {
        let line = format_tree_line("openssl", true, "", true);
        assert!(line.contains("openssl"));
        assert!(line.contains("✓"));
        // Root should not have connector
        assert!(!line.contains("└"));
        assert!(!line.contains("├"));
    }

    #[test]
    fn test_format_tree_line_child_installed() {
        let line = format_tree_line("zlib", true, "  ", true);
        assert!(line.contains("zlib"));
        assert!(line.contains("✓"));
        assert!(line.contains("└── "));
    }

    #[test]
    fn test_format_tree_line_child_not_installed() {
        let line = format_tree_line("libpng", false, "  ", false);
        assert!(line.contains("libpng"));
        assert!(line.contains("✗"));
        assert!(line.contains("├── "));
    }

    #[test]
    fn test_format_deps_tree_lines_single_node_no_deps() {
        let tree = DepsTree {
            name: "ripgrep".to_string(),
            installed: true,
            children: vec![],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("ripgrep"));
        assert!(lines[0].contains("✓"));
    }

    #[test]
    fn test_format_deps_tree_lines_with_children() {
        let tree = DepsTree {
            name: "neovim".to_string(),
            installed: true,
            children: vec![
                DepsTree {
                    name: "luajit".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "libuv".to_string(),
                    installed: false,
                    children: vec![],
                },
            ],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("neovim"));
        assert!(lines[1].contains("luajit"));
        // First-level children don't have tree connectors (root prefix is empty)
        assert!(!lines[1].contains("├── "));
        assert!(lines[2].contains("libuv"));
        assert!(!lines[2].contains("└── "));
    }

    #[test]
    fn test_format_deps_tree_lines_deep_tree() {
        // git -> openssl -> zlib
        let tree = DepsTree {
            name: "git".to_string(),
            installed: true,
            children: vec![DepsTree {
                name: "openssl".to_string(),
                installed: true,
                children: vec![DepsTree {
                    name: "zlib".to_string(),
                    installed: true,
                    children: vec![],
                }],
            }],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("git"));
        assert!(lines[1].contains("openssl"));
        assert!(lines[2].contains("zlib"));
    }

    #[test]
    fn test_format_deps_tree_lines_mixed_install_status() {
        let tree = DepsTree {
            name: "python@3.11".to_string(),
            installed: true,
            children: vec![
                DepsTree {
                    name: "openssl".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "readline".to_string(),
                    installed: false,
                    children: vec![],
                },
                DepsTree {
                    name: "sqlite".to_string(),
                    installed: true,
                    children: vec![],
                },
            ],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 4);
        
        // Check install markers
        assert!(lines[0].contains("✓")); // python installed
        assert!(lines[1].contains("✓")); // openssl installed
        assert!(lines[2].contains("✗")); // readline not installed
        assert!(lines[3].contains("✓")); // sqlite installed
    }

    #[test]
    fn test_format_deps_tree_lines_complex_structure() {
        // Complex tree: neovim with nested deps
        // When root prefix is empty, all descendants also have empty prefix
        // (this is the current behavior - flat list without tree connectors)
        let tree = DepsTree {
            name: "neovim".to_string(),
            installed: true,
            children: vec![
                DepsTree {
                    name: "luajit".to_string(),
                    installed: true,
                    children: vec![DepsTree {
                        name: "libgit2".to_string(),
                        installed: false,
                        children: vec![],
                    }],
                },
                DepsTree {
                    name: "libuv".to_string(),
                    installed: true,
                    children: vec![],
                },
            ],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 4);
        
        // Verify all names are present
        assert!(lines[0].contains("neovim"));
        assert!(lines[1].contains("luajit"));
        assert!(lines[2].contains("libgit2"));
        assert!(lines[3].contains("libuv"));
        
        // Verify install markers
        assert!(lines[0].contains("✓")); // neovim installed
        assert!(lines[1].contains("✓")); // luajit installed
        assert!(lines[2].contains("✗")); // libgit2 not installed
        assert!(lines[3].contains("✓")); // libuv installed
    }

    #[test]
    fn test_format_deps_tree_lines_wide_tree() {
        // Tree with many siblings - all at first level (no connectors with empty root prefix)
        let tree = DepsTree {
            name: "cmake".to_string(),
            installed: true,
            children: vec![
                DepsTree {
                    name: "curl".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "expat".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "libuv".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "ncurses".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "zlib".to_string(),
                    installed: true,
                    children: vec![],
                },
            ],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert_eq!(lines.len(), 6);
        
        // Verify all names present in order
        assert!(lines[0].contains("cmake"));
        assert!(lines[1].contains("curl"));
        assert!(lines[2].contains("expat"));
        assert!(lines[3].contains("libuv"));
        assert!(lines[4].contains("ncurses"));
        assert!(lines[5].contains("zlib"));
        
        // All should be marked as installed
        for line in &lines {
            assert!(line.contains("✓"));
        }
    }

    #[test]
    fn test_format_deps_tree_lines_with_prefix() {
        // Test tree formatting when called with non-empty prefix (subtree case)
        // This is how tree connectors actually appear
        let tree = DepsTree {
            name: "openssl".to_string(),
            installed: true,
            children: vec![
                DepsTree {
                    name: "zlib".to_string(),
                    installed: true,
                    children: vec![],
                },
                DepsTree {
                    name: "ca-certificates".to_string(),
                    installed: false,
                    children: vec![],
                },
            ],
        };
        // Call with non-empty prefix to simulate being a child node
        let lines = format_deps_tree_lines(&tree, "  ", false);
        
        // First line should have middle-child connector
        assert!(lines[0].contains("├── "));
        assert!(lines[0].contains("openssl"));
        
        // Children should have proper tree indentation
        assert!(lines[1].contains("│   ")); // continuation from non-last parent
        assert!(lines[1].contains("├── ")); // first child (not last)
        assert!(lines[2].contains("│   "));
        assert!(lines[2].contains("└── ")); // last child
    }

    #[test]
    fn test_format_deps_tree_preserves_name_special_chars() {
        let tree = DepsTree {
            name: "python@3.11".to_string(),
            installed: true,
            children: vec![DepsTree {
                name: "node@20".to_string(),
                installed: false,
                children: vec![],
            }],
        };
        let lines = format_deps_tree_lines(&tree, "", true);
        assert!(lines[0].contains("python@3.11"));
        assert!(lines[1].contains("node@20"));
    }
}
