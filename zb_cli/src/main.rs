//! Zerobrew CLI - A fast Homebrew-compatible package installer.

use clap::{Parser, Subcommand};
use console::style;
use std::path::{Path, PathBuf};
use std::process::Command;

use zb_io::install::create_installer;

mod commands;
mod display;

use display::{format_bytes, print_shellenv};

#[derive(Parser)]
#[command(name = "zb")]
#[command(about = "Zerobrew - A fast Homebrew-compatible package installer")]
#[command(version)]
struct Cli {
    /// Root directory for zerobrew data
    #[arg(long, default_value = "/opt/zerobrew")]
    root: PathBuf,

    /// Prefix directory for linked binaries
    #[arg(long, default_value = "/opt/zerobrew/prefix")]
    prefix: PathBuf,

    /// Number of parallel downloads
    #[arg(long, default_value = "48")]
    concurrency: usize,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install a formula
    Install {
        /// Formula name to install
        formula: String,

        /// Skip linking executables
        #[arg(long)]
        no_link: bool,

        /// Build from source instead of using a bottle
        #[arg(long, short = 's')]
        build_from_source: bool,

        /// Install the HEAD version (requires building from source)
        #[arg(long, short = 'H')]
        head: bool,
    },

    /// Uninstall a formula (or all formulas if no name given)
    Uninstall {
        /// Formula name to uninstall (omit to uninstall all)
        formula: Option<String>,
    },

    /// List installed formulas
    List {
        /// Show only pinned formulas
        #[arg(long)]
        pinned: bool,
    },

    /// Show info about an installed formula
    Info {
        /// Formula name
        formula: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search for formulas
    Search {
        /// Search query (use /regex/ for regex search)
        query: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Only show installed packages
        #[arg(long)]
        installed: bool,
    },

    /// List outdated formulas
    Outdated {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Upgrade outdated formulas
    Upgrade {
        /// Formula name to upgrade (omit to upgrade all)
        formula: Option<String>,

        /// Show what would be upgraded without doing it
        #[arg(long)]
        dry_run: bool,
    },

    /// Pin a formula to prevent automatic upgrades
    Pin {
        /// Formula name to pin
        formula: String,
    },

    /// Unpin a formula to allow upgrades
    Unpin {
        /// Formula name to unpin
        formula: String,
    },

    /// Garbage collect unreferenced store entries
    Gc,

    /// Remove orphaned dependencies (packages no longer needed by any explicit install)
    Autoremove {
        /// Show what would be removed without doing it
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove old versions and cache files
    Cleanup {
        /// Show what would be removed without doing it
        #[arg(long)]
        dry_run: bool,

        /// Remove cache files older than specified days (default: remove all unused)
        #[arg(long)]
        prune: Option<u32>,
    },

    /// Reset zerobrew (delete all data for cold install testing)
    Reset {
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Initialize zerobrew directories with correct permissions
    Init,

    /// Print shell environment setup commands
    Shellenv {
        /// Shell type (bash, zsh, fish, csh). Auto-detected if not specified.
        #[arg(long, short)]
        shell: Option<String>,
    },

    /// Manage third-party repositories (taps)
    Tap {
        /// Tap to add (in user/repo format). If omitted, lists installed taps.
        user_repo: Option<String>,
    },

    /// Remove a tap repository
    Untap {
        /// Tap to remove (in user/repo format)
        user_repo: String,
    },

    /// Create symlinks for a keg (installed formula)
    Link {
        /// Formula name to link
        formula: String,

        /// Overwrite existing symlinks from other kegs
        #[arg(long)]
        overwrite: bool,

        /// Link keg-only formulas that are normally not linked
        #[arg(long, short)]
        force: bool,
    },

    /// Remove symlinks for a keg (keeps the formula installed)
    Unlink {
        /// Formula name to unlink
        formula: String,
    },

    /// Show dependencies for a formula
    Deps {
        /// Formula name to show dependencies for
        formula: String,

        /// Show dependencies as a tree
        #[arg(long)]
        tree: bool,

        /// Only show installed dependencies
        #[arg(long)]
        installed: bool,

        /// Include all recursive (transitive) dependencies
        #[arg(long, short = '1')]
        all: bool,
    },

    /// Show which installed formulas use (depend on) a given formula
    Uses {
        /// Formula name to check for dependents
        formula: String,

        /// Only show installed packages that use this formula
        #[arg(long)]
        installed: bool,

        /// Include packages that transitively depend on this formula
        #[arg(long)]
        recursive: bool,
    },

    /// List installed formulas that are not dependencies of any other installed formula
    Leaves,

    /// Diagnose common issues with the zerobrew installation
    Doctor,

    /// Manage background services for installed formulas
    Services {
        #[command(subcommand)]
        action: Option<ServicesAction>,
    },

    /// Install from a Brewfile or manage Brewfile configuration
    Bundle {
        #[command(subcommand)]
        action: Option<BundleAction>,
    },

    /// List all available commands (built-in and external)
    #[command(alias = "zb-commands")]
    #[allow(clippy::enum_variant_names)]
    Commands,

    /// External subcommand - runs `zb-<cmd>` from PATH or `~/.zerobrew/cmd/`
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand, Clone)]
pub enum ServicesAction {
    /// List all managed services and their status
    List {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Start a service
    Start {
        /// Formula name to start
        formula: String,
    },

    /// Enable a service to start automatically at login
    Enable {
        /// Formula name to enable
        formula: String,
    },

    /// Disable a service from starting automatically
    Disable {
        /// Formula name to disable
        formula: String,
    },

    /// Stop a service
    Stop {
        /// Formula name to stop
        formula: String,
    },

    /// Restart a service (stop then start)
    Restart {
        /// Formula name to restart
        formula: String,
    },

    /// Run a service in the foreground (useful for debugging)
    Run {
        /// Formula name to run
        formula: String,
    },

    /// Show detailed information about a service
    Info {
        /// Formula name to show info for
        formula: String,
    },

    /// View service logs
    Log {
        /// Formula name to view logs for
        formula: String,

        /// Show only the last N lines (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        lines: usize,

        /// Follow log output in real-time
        #[arg(short, long)]
        follow: bool,
    },

    /// Remove services for uninstalled formulas
    Cleanup {
        /// Show what would be removed without removing
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Clone)]
pub enum BundleAction {
    /// Install all entries from a Brewfile (default when running 'zb bundle')
    Install {
        /// Path to Brewfile (default: ./Brewfile or parent directories)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Generate a Brewfile from installed packages
    Dump {
        /// Path to write Brewfile (default: stdout)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Include comments and formatting
        #[arg(long)]
        describe: bool,

        /// Overwrite existing file without prompting
        #[arg(long, short = 'F')]
        force: bool,
    },

    /// Check if all Brewfile entries are satisfied
    Check {
        /// Path to Brewfile (default: ./Brewfile or parent directories)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Exit with code 1 if any entries are not satisfied
        #[arg(long)]
        strict: bool,
    },

    /// List all entries from a Brewfile
    List {
        /// Path to Brewfile (default: ./Brewfile or parent directories)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("{} {}", style("error:").red().bold(), e);
        std::process::exit(1);
    }
}

/// Check if zerobrew directories need initialization.
fn needs_init(root: &Path, prefix: &Path) -> bool {
    let root_ok = root.exists() && is_writable(root);
    let prefix_ok = prefix.exists() && is_writable(prefix);
    !(root_ok && prefix_ok)
}

fn is_writable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let test_file = path.join(".zb_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

/// Run initialization - create directories and set permissions.
fn run_init(root: &Path, prefix: &Path) -> Result<(), String> {
    println!("{} Initializing zerobrew...", style("==>").cyan().bold());

    let dirs_to_create: Vec<PathBuf> = vec![
        root.to_path_buf(),
        root.join("store"),
        root.join("db"),
        root.join("cache"),
        root.join("locks"),
        prefix.to_path_buf(),
        prefix.join("bin"),
        prefix.join("Cellar"),
    ];

    let need_sudo = dirs_to_create.iter().any(|d| {
        if d.exists() {
            !is_writable(d)
        } else {
            d.parent()
                .map(|p| p.exists() && !is_writable(p))
                .unwrap_or(true)
        }
    });

    if need_sudo {
        println!(
            "{}",
            style("    Creating directories (requires sudo)...").dim()
        );

        for dir in &dirs_to_create {
            let status = Command::new("sudo")
                .args(["mkdir", "-p", &dir.to_string_lossy()])
                .status()
                .map_err(|e| format!("Failed to run sudo mkdir: {}", e))?;

            if !status.success() {
                return Err(format!("Failed to create directory: {}", dir.display()));
            }
        }

        let user = Command::new("whoami")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "root".to_string()));

        let status = Command::new("sudo")
            .args(["chown", "-R", &user, &root.to_string_lossy()])
            .status()
            .map_err(|e| format!("Failed to run sudo chown: {}", e))?;

        if !status.success() {
            return Err(format!("Failed to set ownership on {}", root.display()));
        }

        let status = Command::new("sudo")
            .args(["chown", "-R", &user, &prefix.to_string_lossy()])
            .status()
            .map_err(|e| format!("Failed to run sudo chown: {}", e))?;

        if !status.success() {
            return Err(format!("Failed to set ownership on {}", prefix.display()));
        }
    } else {
        for dir in &dirs_to_create {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
        }
    }

    add_to_path(prefix)?;

    println!("{} Initialization complete!", style("==>").cyan().bold());

    Ok(())
}

fn add_to_path(prefix: &Path) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    let config_file = if shell.contains("zsh") {
        let zdotdir = std::env::var("ZDOTDIR").unwrap_or_else(|_| home.clone());
        let zshenv = format!("{}/.zshenv", zdotdir);

        // Prefer .zshenv (sourced for all shells), fall back to .zshrc
        if std::path::Path::new(&zshenv).exists() {
            zshenv
        } else {
            format!("{}/.zshrc", zdotdir)
        }
    } else if shell.contains("bash") {
        let bash_profile = format!("{}/.bash_profile", home);
        if std::path::Path::new(&bash_profile).exists() {
            bash_profile
        } else {
            format!("{}/.bashrc", home)
        }
    } else {
        format!("{}/.profile", home)
    };

    let bin_path = prefix.join("bin");
    let path_export = format!("export PATH=\"{}:$PATH\"", bin_path.display());

    let already_added = if let Ok(contents) = std::fs::read_to_string(&config_file) {
        contents.contains(&bin_path.to_string_lossy().to_string())
    } else {
        false
    };

    if !already_added {
        let addition = format!("\n# zerobrew\n{}\n", path_export);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config_file)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(addition.as_bytes())
            })
            .map_err(|e| format!("Failed to update {}: {}", config_file, e))?;

        println!(
            "    {} Added {} to PATH in {}",
            style("✓").green(),
            bin_path.display(),
            config_file
        );
    }

    let current_path = std::env::var("PATH").unwrap_or_default();
    if !current_path.contains(&bin_path.to_string_lossy().to_string()) {
        println!(
            "    {} Run {} or restart your terminal",
            style("→").cyan(),
            style(format!("source {}", config_file)).cyan()
        );
    }

    Ok(())
}

/// Ensure zerobrew is initialized, prompting user if needed.
fn ensure_init(root: &Path, prefix: &Path) -> Result<(), zb_core::Error> {
    if !needs_init(root, prefix) {
        return Ok(());
    }

    println!(
        "{} Zerobrew needs to be initialized first.",
        style("Note:").yellow().bold()
    );
    println!("    This will create directories at:");
    println!("      • {}", root.display());
    println!("      • {}", prefix.display());
    println!();

    print!("Initialize now? [Y/n] ");
    use std::io::{self, Write};
    if io::stdout().flush().is_err() {
        return Err(zb_core::Error::StoreCorruption {
            message: "Failed to flush stdout".to_string(),
        });
    }

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return Err(zb_core::Error::StoreCorruption {
            message: "Failed to read user input".to_string(),
        });
    }
    let input = input.trim();

    if !input.is_empty() && !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        return Err(zb_core::Error::StoreCorruption {
            message: "Initialization required. Run 'zb init' first.".to_string(),
        });
    }

    run_init(root, prefix).map_err(|e| zb_core::Error::StoreCorruption { message: e })
}

async fn run(cli: Cli) -> Result<(), zb_core::Error> {
    // Handle init separately - it doesn't need the installer
    if matches!(cli.command, Commands::Init) {
        return run_init(&cli.root, &cli.prefix)
            .map_err(|e| zb_core::Error::StoreCorruption { message: e });
    }

    // Handle shellenv separately - it only outputs environment setup
    if let Commands::Shellenv { ref shell } = cli.command {
        print_shellenv(&cli.prefix, shell.as_deref());
        return Ok(());
    }

    // For reset, handle specially since directories may not be writable
    if matches!(cli.command, Commands::Reset { .. }) {
        // Skip init check for reset
    } else {
        ensure_init(&cli.root, &cli.prefix)?;
    }

    let mut installer = create_installer(&cli.root, &cli.prefix, cli.concurrency)?;

    match cli.command {
        Commands::Init => unreachable!(),
        Commands::Shellenv { .. } => unreachable!(),

        Commands::Install {
            formula,
            no_link,
            build_from_source,
            head,
        } => {
            commands::install::run(
                &mut installer,
                &cli.prefix,
                formula,
                no_link,
                build_from_source,
                head,
            )
            .await
        }

        Commands::Uninstall { formula } => run_uninstall(&mut installer, formula),

        Commands::List { pinned } => commands::info::run_list(&installer, pinned),

        Commands::Info { formula, json } => {
            commands::info::run_info(&mut installer, &cli.prefix, formula, json).await
        }

        Commands::Search {
            query,
            json,
            installed,
        } => commands::info::run_search(&installer, &cli.root, query, json, installed).await,

        Commands::Outdated { json } => commands::upgrade::run_outdated(&mut installer, json).await,

        Commands::Upgrade { formula, dry_run } => {
            commands::upgrade::run_upgrade(&mut installer, formula, dry_run).await
        }

        Commands::Pin { formula } => commands::upgrade::run_pin(&mut installer, &formula),

        Commands::Unpin { formula } => commands::upgrade::run_unpin(&mut installer, &formula),

        Commands::Gc => run_gc(&mut installer),

        Commands::Autoremove { dry_run } => run_autoremove(&mut installer, dry_run).await,

        Commands::Cleanup { dry_run, prune } => run_cleanup(&mut installer, dry_run, prune),

        Commands::Reset { yes } => run_reset(&cli.root, &cli.prefix, yes),

        Commands::Tap { user_repo } => commands::tap::run_tap(&mut installer, user_repo).await,

        Commands::Untap { user_repo } => commands::tap::run_untap(&mut installer, user_repo),

        Commands::Link {
            formula,
            overwrite,
            force,
        } => run_link(&mut installer, &cli.prefix, &formula, overwrite, force).await,

        Commands::Unlink { formula } => run_unlink(&mut installer, &formula),

        Commands::Deps {
            formula,
            tree,
            installed,
            all,
        } => commands::deps::run_deps(&mut installer, formula, tree, installed, all).await,

        Commands::Uses {
            formula,
            installed: _,
            recursive,
        } => commands::deps::run_uses(&mut installer, formula, recursive).await,

        Commands::Leaves => commands::deps::run_leaves(&mut installer).await,

        Commands::Doctor => commands::doctor::run(&mut installer).await,

        Commands::Services { action } => {
            commands::services::run(&mut installer, &cli.prefix, action)
        }

        Commands::Bundle { action } => commands::bundle::run(&mut installer, action).await,

        Commands::Commands => run_commands(&cli.root),

        Commands::External(args) => run_external(&cli.root, &cli.prefix, args),
    }
}

// ============================================================================
// Inline command implementations (not worth extracting to separate modules)
// ============================================================================

fn run_uninstall(
    installer: &mut zb_io::install::Installer,
    formula: Option<String>,
) -> Result<(), zb_core::Error> {
    match formula {
        Some(name) => {
            println!(
                "{} Uninstalling {}...",
                style("==>").cyan().bold(),
                style(&name).bold()
            );
            installer.uninstall(&name)?;
            println!(
                "{} Uninstalled {}",
                style("==>").cyan().bold(),
                style(&name).green()
            );
        }
        None => {
            let installed = installer.list_installed()?;
            if installed.is_empty() {
                println!("No formulas installed.");
                return Ok(());
            }

            println!(
                "{} Uninstalling {} packages...",
                style("==>").cyan().bold(),
                installed.len()
            );

            for keg in installed {
                print!("    {} {}...", style("○").dim(), keg.name);
                installer.uninstall(&keg.name)?;
                println!(" {}", style("✓").green());
            }

            println!("{} Uninstalled all packages", style("==>").cyan().bold());
        }
    }
    Ok(())
}

fn run_gc(installer: &mut zb_io::install::Installer) -> Result<(), zb_core::Error> {
    println!(
        "{} Running garbage collection...",
        style("==>").cyan().bold()
    );
    let removed = installer.gc()?;

    if removed.is_empty() {
        println!("No unreferenced store entries to remove.");
    } else {
        for key in &removed {
            println!("    {} Removed {}", style("✓").green(), &key[..12]);
        }
        println!(
            "{} Removed {} store entries",
            style("==>").cyan().bold(),
            style(removed.len()).green().bold()
        );
    }
    Ok(())
}

async fn run_autoremove(
    installer: &mut zb_io::install::Installer,
    dry_run: bool,
) -> Result<(), zb_core::Error> {
    println!(
        "{} Finding orphaned dependencies...",
        style("==>").cyan().bold()
    );

    let orphans = installer.find_orphans().await?;

    if orphans.is_empty() {
        println!("No orphaned dependencies to remove.");
        return Ok(());
    }

    if dry_run {
        println!(
            "{} Would remove {} orphaned packages:\n",
            style("==>").cyan().bold(),
            style(orphans.len()).yellow().bold()
        );
        for name in &orphans {
            println!("  {}", name);
        }
        println!(
            "\n    {} Run {} to remove",
            style("→").dim(),
            style("zb autoremove").cyan()
        );
    } else {
        println!(
            "{} Removing {} orphaned packages...\n",
            style("==>").cyan().bold(),
            style(orphans.len()).yellow().bold()
        );

        let removed = installer.autoremove().await?;

        if removed.is_empty() {
            println!("No packages were removed.");
        } else {
            for name in &removed {
                println!("    {} Removed {}", style("✓").green(), name);
            }
            println!(
                "\n{} Removed {} orphaned packages",
                style("==>").cyan().bold(),
                style(removed.len()).green().bold()
            );
        }
    }
    Ok(())
}

fn run_cleanup(
    installer: &mut zb_io::install::Installer,
    dry_run: bool,
    prune: Option<u32>,
) -> Result<(), zb_core::Error> {
    if dry_run {
        println!(
            "{} Checking for files to clean up...",
            style("==>").cyan().bold()
        );

        let result = installer.cleanup_dry_run(prune)?;

        if result.store_entries_removed == 0
            && result.blobs_removed == 0
            && result.http_cache_removed == 0
        {
            println!("Nothing to clean up.");
            return Ok(());
        }

        println!("{} Would remove:\n", style("==>").cyan().bold());

        if result.store_entries_removed > 0 {
            println!(
                "  {} unreferenced store entries",
                style(result.store_entries_removed).yellow()
            );
        }

        if result.blobs_removed > 0 {
            println!(
                "  {} cached bottle downloads",
                style(result.blobs_removed).yellow()
            );
        }

        if result.http_cache_removed > 0 {
            println!(
                "  {} cached API responses",
                style(result.http_cache_removed).yellow()
            );
        }

        if result.bytes_freed > 0 {
            println!(
                "\n  Total: {}",
                style(format_bytes(result.bytes_freed)).yellow()
            );
        }

        println!(
            "\n    {} Run {} to clean up",
            style("→").dim(),
            style("zb cleanup").cyan()
        );
    } else {
        println!("{} Cleaning up...", style("==>").cyan().bold());

        let result = installer.cleanup(prune)?;

        if result.store_entries_removed == 0
            && result.blobs_removed == 0
            && result.temp_files_removed == 0
            && result.locks_removed == 0
            && result.http_cache_removed == 0
        {
            println!("Nothing to clean up.");
            return Ok(());
        }

        println!();

        if result.store_entries_removed > 0 {
            println!(
                "    {} Removed {} unreferenced store entries",
                style("✓").green(),
                result.store_entries_removed
            );
        }

        if result.blobs_removed > 0 {
            println!(
                "    {} Removed {} cached bottle downloads",
                style("✓").green(),
                result.blobs_removed
            );
        }

        if result.http_cache_removed > 0 {
            println!(
                "    {} Removed {} cached API responses",
                style("✓").green(),
                result.http_cache_removed
            );
        }

        if result.temp_files_removed > 0 {
            println!(
                "    {} Removed {} temp files/directories",
                style("✓").green(),
                result.temp_files_removed
            );
        }

        if result.locks_removed > 0 {
            println!(
                "    {} Removed {} stale lock files",
                style("✓").green(),
                result.locks_removed
            );
        }

        if result.bytes_freed > 0 {
            println!(
                "\n{} Freed {}",
                style("==>").cyan().bold(),
                style(format_bytes(result.bytes_freed)).green().bold()
            );
        }
    }
    Ok(())
}

fn run_reset(root: &Path, prefix: &Path, yes: bool) -> Result<(), zb_core::Error> {
    if !root.exists() && !prefix.exists() {
        println!("Nothing to reset - directories do not exist.");
        return Ok(());
    }

    if !yes {
        println!(
            "{} This will delete all zerobrew data at:",
            style("Warning:").yellow().bold()
        );
        println!("      • {}", root.display());
        println!("      • {}", prefix.display());
        print!("Continue? [y/N] ");
        use std::io::{self, Write};
        if io::stdout().flush().is_err() {
            eprintln!("{} Failed to flush stdout", style("error:").red().bold());
            std::process::exit(1);
        }

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("{} Failed to read user input", style("error:").red().bold());
            std::process::exit(1);
        }
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    for dir in [root, prefix] {
        if !dir.exists() {
            continue;
        }

        println!(
            "{} Removing {}...",
            style("==>").cyan().bold(),
            dir.display()
        );

        if std::fs::remove_dir_all(dir).is_err() {
            let status = Command::new("sudo")
                .args(["rm", "-rf", &dir.to_string_lossy()])
                .status();

            let success = match status {
                Ok(s) => s.success(),
                Err(_) => false,
            };
            if !success {
                eprintln!(
                    "{} Failed to remove {}",
                    style("error:").red().bold(),
                    dir.display()
                );
                std::process::exit(1);
            }
        }
    }

    run_init(root, prefix).map_err(|e| zb_core::Error::StoreCorruption { message: e })?;

    println!(
        "{} Reset complete. Ready for cold install.",
        style("==>").cyan().bold()
    );

    Ok(())
}

async fn run_link(
    installer: &mut zb_io::install::Installer,
    prefix: &Path,
    formula: &str,
    overwrite: bool,
    force: bool,
) -> Result<(), zb_core::Error> {
    if !installer.is_installed(formula) {
        eprintln!(
            "{} Formula '{}' is not installed.",
            style("error:").red().bold(),
            formula
        );
        std::process::exit(1);
    }

    if !force
        && let Ok(api_formula) = installer.get_formula(formula).await
        && api_formula.keg_only
    {
        eprintln!(
            "{} {} is keg-only, which means it was not symlinked into {}",
            style("Warning:").yellow().bold(),
            formula,
            prefix.display()
        );
        if let Some(ref reason) = api_formula.keg_only_reason
            && !reason.explanation.is_empty()
        {
            eprintln!();
            eprintln!("{}", reason.explanation);
        }
        eprintln!();
        eprintln!("If you need to have {} first in your PATH, run:", formula);
        eprintln!("  {} link --force {}", style("zb").cyan(), formula);
        std::process::exit(1);
    }

    println!(
        "{} Linking {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    match installer.link(formula, overwrite, force) {
        Ok(result) => {
            if result.already_linked {
                println!(
                    "{} {} is already linked",
                    style("==>").cyan().bold(),
                    style(formula).bold()
                );
            } else if result.files_linked == 0 {
                println!(
                    "{} {} has no files to link",
                    style("==>").cyan().bold(),
                    style(formula).bold()
                );
            } else {
                println!(
                    "{} {} Linked {} files for {}",
                    style("==>").cyan().bold(),
                    style("✓").green(),
                    result.files_linked,
                    style(formula).bold()
                );
                if result.keg_only_forced {
                    println!(
                        "    {} This is a keg-only formula - it was linked with --force",
                        style("→").dim()
                    );
                }
            }
        }
        Err(zb_core::Error::LinkConflict { path, .. }) => {
            eprintln!(
                "{} Could not link {}:",
                style("error:").red().bold(),
                formula
            );
            eprintln!();
            eprintln!("  {} already exists", path.display());
            eprintln!();
            eprintln!(
                "To overwrite existing files, run:\n  {} link --overwrite {}",
                style("zb").cyan(),
                formula
            );
            std::process::exit(1);
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

fn run_unlink(
    installer: &mut zb_io::install::Installer,
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

    println!(
        "{} Unlinking {}...",
        style("==>").cyan().bold(),
        style(formula).bold()
    );

    let unlinked = installer.unlink(formula)?;

    if unlinked == 0 {
        println!(
            "{} {} has no linked files",
            style("==>").cyan().bold(),
            style(formula).bold()
        );
    } else {
        println!(
            "{} {} Unlinked {} files for {}",
            style("==>").cyan().bold(),
            style("✓").green(),
            unlinked,
            style(formula).bold()
        );
    }

    Ok(())
}

fn run_commands(root: &Path) -> Result<(), zb_core::Error> {
    let builtin_commands = [
        ("autoremove", "Remove orphaned dependencies"),
        (
            "bundle",
            "Install from a Brewfile or manage Brewfile configuration",
        ),
        ("cleanup", "Remove old versions and cache files"),
        ("deps", "Show dependencies for a formula"),
        ("doctor", "Diagnose common issues"),
        ("gc", "Garbage collect unreferenced store entries"),
        ("info", "Show info about an installed formula"),
        ("init", "Initialize zerobrew directories"),
        ("install", "Install a formula"),
        (
            "leaves",
            "List installed formulas that are not dependencies",
        ),
        ("link", "Create symlinks for a keg"),
        ("list", "List installed formulas"),
        ("outdated", "List outdated formulas"),
        ("pin", "Pin a formula to prevent upgrades"),
        ("reset", "Reset zerobrew (delete all data)"),
        ("search", "Search for formulas"),
        ("services", "Manage background services"),
        ("shellenv", "Print shell environment setup"),
        ("tap", "Manage third-party repositories"),
        ("uninstall", "Uninstall a formula"),
        ("unlink", "Remove symlinks for a keg"),
        ("unpin", "Unpin a formula"),
        ("untap", "Remove a tap repository"),
        ("upgrade", "Upgrade outdated formulas"),
        ("uses", "Show which formulas use a given formula"),
        ("commands", "List all available commands"),
    ];

    println!("{} Built-in commands:", style("==>").cyan().bold());
    for (name, desc) in &builtin_commands {
        println!("    {} {}", style(name).green().bold(), style(desc).dim());
    }

    let external_commands = find_external_commands(root);
    if !external_commands.is_empty() {
        println!();
        println!("{} External commands:", style("==>").cyan().bold());
        for (name, path) in &external_commands {
            println!(
                "    {} ({})",
                style(name).green().bold(),
                style(path.display()).dim()
            );
        }
    }

    Ok(())
}

fn run_external(root: &Path, prefix: &Path, args: Vec<String>) -> Result<(), zb_core::Error> {
    if args.is_empty() {
        eprintln!("{} No command specified", style("error:").red().bold());
        std::process::exit(1);
    }

    let cmd_name = &args[0];
    let cmd_args = &args[1..];

    if let Some(cmd_path) = find_external_command(cmd_name, root) {
        let status = Command::new(&cmd_path)
            .args(cmd_args)
            .env("ZB_ROOT", root)
            .env("ZB_PREFIX", prefix)
            .status();

        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                std::process::exit(s.code().unwrap_or(1));
            }
            Err(e) => {
                eprintln!(
                    "{} Failed to run external command '{}': {}",
                    style("error:").red().bold(),
                    cmd_name,
                    e
                );
                std::process::exit(1);
            }
        }
    } else {
        eprintln!(
            "{} Unknown command '{}'\n\nRun 'zb commands' to see available commands.",
            style("error:").red().bold(),
            cmd_name
        );
        std::process::exit(1);
    }

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

/// Find all external commands (zb-* executables in PATH and ~/.zerobrew/cmd/).
fn find_external_commands(root: &Path) -> Vec<(String, PathBuf)> {
    let mut commands = Vec::new();

    let cmd_dir = root.join("cmd");
    if cmd_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&cmd_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && is_executable(&path)
                && let Some(name) = path.file_name().and_then(|s| s.to_str())
                && let Some(cmd_name) = name.strip_prefix("zb-")
            {
                commands.push((cmd_name.to_string(), path));
            }
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let dir_path = Path::new(dir);
            if dir_path.exists()
                && let Ok(entries) = std::fs::read_dir(dir_path)
            {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && is_executable(&path)
                        && let Some(name) = path.file_name().and_then(|s| s.to_str())
                        && let Some(cmd_name) = name.strip_prefix("zb-")
                        && !commands.iter().any(|(n, _)| n == cmd_name)
                    {
                        commands.push((cmd_name.to_string(), path));
                    }
                }
            }
        }
    }

    commands.sort_by(|a, b| a.0.cmp(&b.0));
    commands
}

/// Find a specific external command.
fn find_external_command(name: &str, root: &Path) -> Option<PathBuf> {
    let cmd_name = format!("zb-{}", name);

    let cmd_dir = root.join("cmd");
    let local_cmd = cmd_dir.join(&cmd_name);
    if local_cmd.exists() && is_executable(&local_cmd) {
        return Some(local_cmd);
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let cmd_path = Path::new(dir).join(&cmd_name);
            if cmd_path.exists() && is_executable(&cmd_path) {
                return Some(cmd_path);
            }
        }
    }

    None
}

/// Check if a file is executable.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = path.metadata() {
        meta.permissions().mode() & 0o111 != 0
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // CLI Argument Parsing Tests
    // ========================================================================

    #[test]
    fn test_services_list_json_flag_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "list", "--json"]).unwrap();
        assert!(
            matches!(
                cli.command,
                Commands::Services {
                    action: Some(ServicesAction::List { json: true }),
                }
            ),
            "Expected Services List command with json=true"
        );

        let cli = Cli::try_parse_from(["zb", "services", "list"]).unwrap();
        assert!(
            matches!(
                cli.command,
                Commands::Services {
                    action: Some(ServicesAction::List { json: false }),
                }
            ),
            "Expected Services List command with json=false"
        );
    }

    #[test]
    fn test_services_enable_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "enable", "redis"]).unwrap();
        match cli.command {
            Commands::Services {
                action: Some(ServicesAction::Enable { formula }),
            } => {
                assert_eq!(formula, "redis");
            }
            _ => panic!("Expected Services Enable command"),
        }
    }

    #[test]
    fn test_services_disable_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "disable", "postgresql"]).unwrap();
        match cli.command {
            Commands::Services {
                action: Some(ServicesAction::Disable { formula }),
            } => {
                assert_eq!(formula, "postgresql");
            }
            _ => panic!("Expected Services Disable command"),
        }
    }

    #[test]
    fn test_commands_subcommand_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "commands"]).unwrap();
        assert!(
            matches!(cli.command, Commands::Commands),
            "Expected Commands subcommand"
        );
    }

    #[test]
    fn test_external_subcommand_parsing() {
        use clap::Parser;

        // External commands are parsed as External(Vec<String>)
        let cli = Cli::try_parse_from(["zb", "my-custom-cmd", "arg1", "arg2"]).unwrap();
        match cli.command {
            Commands::External(args) => {
                assert_eq!(args, vec!["my-custom-cmd", "arg1", "arg2"]);
            }
            _ => panic!("Expected External subcommand"),
        }
    }

    #[test]
    fn test_versioned_formula_install_parsing() {
        use clap::Parser;

        // Test versioned formula name parsing (e.g., python@3.11)
        let cli = Cli::try_parse_from(["zb", "install", "python@3.11"]).unwrap();
        match cli.command {
            Commands::Install { formula, .. } => {
                assert_eq!(formula, "python@3.11");
            }
            _ => panic!("Expected Install command"),
        }
    }

    #[test]
    fn test_versioned_formula_link_with_force() {
        use clap::Parser;

        // Test link --force for keg-only versioned formulas
        let cli = Cli::try_parse_from(["zb", "link", "openssl@3", "--force"]).unwrap();
        match cli.command {
            Commands::Link { formula, force, .. } => {
                assert_eq!(formula, "openssl@3");
                assert!(force);
            }
            _ => panic!("Expected Link command"),
        }
    }

    #[test]
    fn test_info_shows_versioned_formula() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "info", "node@20"]).unwrap();
        match cli.command {
            Commands::Info { formula, .. } => {
                assert_eq!(formula, "node@20");
            }
            _ => panic!("Expected Info command"),
        }
    }

    #[test]
    fn test_is_executable_returns_false_for_nonexistent() {
        let path = Path::new("/nonexistent/path/to/file");
        assert!(!is_executable(path));
    }

    #[test]
    fn test_find_external_commands_handles_empty_cmd_dir() {
        let temp_dir = std::env::temp_dir().join("zb-test-empty-cmd");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let commands = find_external_commands(&temp_dir);
        // Should return empty or only PATH commands, no panic
        assert!(commands.iter().all(|(name, _)| !name.is_empty()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_external_command_returns_none_for_nonexistent() {
        let temp_dir = std::env::temp_dir().join("zb-test-no-cmd");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let result = find_external_command("nonexistent-command", &temp_dir);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ========================================================================
    // Install Command Tests
    // ========================================================================

    #[test]
    fn test_install_no_link_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "install", "git", "--no-link"]).unwrap();
        match cli.command {
            Commands::Install { formula, no_link, .. } => {
                assert_eq!(formula, "git");
                assert!(no_link);
            }
            _ => panic!("Expected Install command"),
        }
    }

    #[test]
    fn test_install_build_from_source_short_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "install", "git", "-s"]).unwrap();
        match cli.command {
            Commands::Install { formula, build_from_source, .. } => {
                assert_eq!(formula, "git");
                assert!(build_from_source);
            }
            _ => panic!("Expected Install command"),
        }
    }

    #[test]
    fn test_install_head_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "install", "neovim", "--head"]).unwrap();
        match cli.command {
            Commands::Install { formula, head, .. } => {
                assert_eq!(formula, "neovim");
                assert!(head);
            }
            _ => panic!("Expected Install command"),
        }
    }

    #[test]
    fn test_install_head_short_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "install", "neovim", "-H"]).unwrap();
        match cli.command {
            Commands::Install { formula, head, .. } => {
                assert_eq!(formula, "neovim");
                assert!(head);
            }
            _ => panic!("Expected Install command"),
        }
    }

    // ========================================================================
    // Upgrade Command Tests
    // ========================================================================

    #[test]
    fn test_upgrade_all_packages() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "upgrade"]).unwrap();
        match cli.command {
            Commands::Upgrade { formula, dry_run } => {
                assert!(formula.is_none());
                assert!(!dry_run);
            }
            _ => panic!("Expected Upgrade command"),
        }
    }

    #[test]
    fn test_upgrade_specific_formula() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "upgrade", "git"]).unwrap();
        match cli.command {
            Commands::Upgrade { formula, dry_run } => {
                assert_eq!(formula, Some("git".to_string()));
                assert!(!dry_run);
            }
            _ => panic!("Expected Upgrade command"),
        }
    }

    #[test]
    fn test_upgrade_dry_run() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "upgrade", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Upgrade { formula, dry_run } => {
                assert!(formula.is_none());
                assert!(dry_run);
            }
            _ => panic!("Expected Upgrade command"),
        }
    }

    #[test]
    fn test_outdated_json_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "outdated", "--json"]).unwrap();
        match cli.command {
            Commands::Outdated { json } => {
                assert!(json);
            }
            _ => panic!("Expected Outdated command"),
        }
    }

    // ========================================================================
    // Info Command Tests
    // ========================================================================

    #[test]
    fn test_info_json_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "info", "git", "--json"]).unwrap();
        match cli.command {
            Commands::Info { formula, json } => {
                assert_eq!(formula, "git");
                assert!(json);
            }
            _ => panic!("Expected Info command"),
        }
    }

    #[test]
    fn test_list_pinned_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "list", "--pinned"]).unwrap();
        match cli.command {
            Commands::List { pinned } => {
                assert!(pinned);
            }
            _ => panic!("Expected List command"),
        }
    }

    // ========================================================================
    // Bundle Command Tests
    // ========================================================================

    #[test]
    fn test_bundle_default_install() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle"]).unwrap();
        match cli.command {
            Commands::Bundle { action } => {
                assert!(action.is_none());
            }
            _ => panic!("Expected Bundle command"),
        }
    }

    #[test]
    fn test_bundle_install_with_file() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle", "install", "--file", "MyBrewfile"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::Install { file }) } => {
                assert_eq!(file, Some(PathBuf::from("MyBrewfile")));
            }
            _ => panic!("Expected Bundle Install command"),
        }
    }

    #[test]
    fn test_bundle_dump_describe() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle", "dump", "--describe"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::Dump { describe, force, file }) } => {
                assert!(describe);
                assert!(!force);
                assert!(file.is_none());
            }
            _ => panic!("Expected Bundle Dump command"),
        }
    }

    #[test]
    fn test_bundle_dump_force_file() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle", "dump", "--force", "--file", "out.txt"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::Dump { describe, force, file }) } => {
                assert!(!describe);
                assert!(force);
                assert_eq!(file, Some(PathBuf::from("out.txt")));
            }
            _ => panic!("Expected Bundle Dump command"),
        }
    }

    #[test]
    fn test_bundle_dump_short_flags() {
        use clap::Parser;

        // -F for force (uppercase to avoid conflict with -f for file)
        let cli = Cli::try_parse_from(["zb", "bundle", "dump", "-F", "-f", "Brewfile"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::Dump { describe, force, file }) } => {
                assert!(!describe);
                assert!(force);
                assert_eq!(file, Some(PathBuf::from("Brewfile")));
            }
            _ => panic!("Expected Bundle Dump command"),
        }
    }

    #[test]
    fn test_bundle_check_strict() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle", "check", "--strict"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::Check { strict, file }) } => {
                assert!(strict);
                assert!(file.is_none());
            }
            _ => panic!("Expected Bundle Check command"),
        }
    }

    #[test]
    fn test_bundle_list() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "bundle", "list"]).unwrap();
        match cli.command {
            Commands::Bundle { action: Some(BundleAction::List { file }) } => {
                assert!(file.is_none());
            }
            _ => panic!("Expected Bundle List command"),
        }
    }

    // ========================================================================
    // Search Command Tests
    // ========================================================================

    #[test]
    fn test_search_basic() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "search", "git"]).unwrap();
        match cli.command {
            Commands::Search { query, json, installed } => {
                assert_eq!(query, "git");
                assert!(!json);
                assert!(!installed);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_search_json_installed() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "search", "python", "--json", "--installed"]).unwrap();
        match cli.command {
            Commands::Search { query, json, installed } => {
                assert_eq!(query, "python");
                assert!(json);
                assert!(installed);
            }
            _ => panic!("Expected Search command"),
        }
    }

    // ========================================================================
    // Deps Command Tests
    // ========================================================================

    #[test]
    fn test_deps_tree_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "deps", "git", "--tree"]).unwrap();
        match cli.command {
            Commands::Deps { formula, tree, installed, all } => {
                assert_eq!(formula, "git");
                assert!(tree);
                assert!(!installed);
                assert!(!all);
            }
            _ => panic!("Expected Deps command"),
        }
    }

    #[test]
    fn test_deps_all_installed() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "deps", "neovim", "--installed", "-1"]).unwrap();
        match cli.command {
            Commands::Deps { formula, tree, installed, all } => {
                assert_eq!(formula, "neovim");
                assert!(!tree);
                assert!(installed);
                assert!(all);
            }
            _ => panic!("Expected Deps command"),
        }
    }

    // ========================================================================
    // Link/Unlink Command Tests
    // ========================================================================

    #[test]
    fn test_link_overwrite_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "link", "openssl", "--overwrite"]).unwrap();
        match cli.command {
            Commands::Link { formula, overwrite, force } => {
                assert_eq!(formula, "openssl");
                assert!(overwrite);
                assert!(!force);
            }
            _ => panic!("Expected Link command"),
        }
    }

    #[test]
    fn test_unlink_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "unlink", "python@3.11"]).unwrap();
        match cli.command {
            Commands::Unlink { formula } => {
                assert_eq!(formula, "python@3.11");
            }
            _ => panic!("Expected Unlink command"),
        }
    }

    // ========================================================================
    // Cleanup Command Tests
    // ========================================================================

    #[test]
    fn test_cleanup_prune_days() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "cleanup", "--prune", "30"]).unwrap();
        match cli.command {
            Commands::Cleanup { dry_run, prune } => {
                assert!(!dry_run);
                assert_eq!(prune, Some(30));
            }
            _ => panic!("Expected Cleanup command"),
        }
    }

    #[test]
    fn test_cleanup_dry_run() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "cleanup", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Cleanup { dry_run, prune } => {
                assert!(dry_run);
                assert!(prune.is_none());
            }
            _ => panic!("Expected Cleanup command"),
        }
    }

    // ========================================================================
    // Global Options Tests
    // ========================================================================

    #[test]
    fn test_custom_root_and_prefix() {
        use clap::Parser;

        let cli = Cli::try_parse_from([
            "zb",
            "--root", "/custom/root",
            "--prefix", "/custom/prefix",
            "list"
        ]).unwrap();
        assert_eq!(cli.root, PathBuf::from("/custom/root"));
        assert_eq!(cli.prefix, PathBuf::from("/custom/prefix"));
    }

    #[test]
    fn test_custom_concurrency() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "--concurrency", "16", "install", "git"]).unwrap();
        assert_eq!(cli.concurrency, 16);
    }

    #[test]
    fn test_default_values() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "list"]).unwrap();
        assert_eq!(cli.root, PathBuf::from("/opt/zerobrew"));
        assert_eq!(cli.prefix, PathBuf::from("/opt/zerobrew/prefix"));
        assert_eq!(cli.concurrency, 48);
    }

    // ========================================================================
    // Services Log Command Tests
    // ========================================================================

    #[test]
    fn test_services_log_with_lines() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "log", "redis", "-n", "50"]).unwrap();
        match cli.command {
            Commands::Services {
                action: Some(ServicesAction::Log { formula, lines, follow }),
            } => {
                assert_eq!(formula, "redis");
                assert_eq!(lines, 50);
                assert!(!follow);
            }
            _ => panic!("Expected Services Log command"),
        }
    }

    #[test]
    fn test_services_log_follow() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "log", "postgresql", "--follow"]).unwrap();
        match cli.command {
            Commands::Services {
                action: Some(ServicesAction::Log { formula, lines, follow }),
            } => {
                assert_eq!(formula, "postgresql");
                assert_eq!(lines, 20); // default
                assert!(follow);
            }
            _ => panic!("Expected Services Log command"),
        }
    }

    // ========================================================================
    // Pin/Unpin Command Tests
    // ========================================================================

    #[test]
    fn test_pin_versioned_formula() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "pin", "node@20"]).unwrap();
        match cli.command {
            Commands::Pin { formula } => {
                assert_eq!(formula, "node@20");
            }
            _ => panic!("Expected Pin command"),
        }
    }

    #[test]
    fn test_unpin_formula() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "unpin", "git"]).unwrap();
        match cli.command {
            Commands::Unpin { formula } => {
                assert_eq!(formula, "git");
            }
            _ => panic!("Expected Unpin command"),
        }
    }

    // ========================================================================
    // Tap/Untap Command Tests
    // ========================================================================

    #[test]
    fn test_tap_list() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "tap"]).unwrap();
        match cli.command {
            Commands::Tap { user_repo } => {
                assert!(user_repo.is_none());
            }
            _ => panic!("Expected Tap command"),
        }
    }

    #[test]
    fn test_tap_add() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "tap", "homebrew/cask"]).unwrap();
        match cli.command {
            Commands::Tap { user_repo } => {
                assert_eq!(user_repo, Some("homebrew/cask".to_string()));
            }
            _ => panic!("Expected Tap command"),
        }
    }

    #[test]
    fn test_untap() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "untap", "homebrew/cask"]).unwrap();
        match cli.command {
            Commands::Untap { user_repo } => {
                assert_eq!(user_repo, "homebrew/cask");
            }
            _ => panic!("Expected Untap command"),
        }
    }

    // ========================================================================
    // Reset Command Tests
    // ========================================================================

    #[test]
    fn test_reset_with_yes_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "reset", "--yes"]).unwrap();
        match cli.command {
            Commands::Reset { yes } => {
                assert!(yes);
            }
            _ => panic!("Expected Reset command"),
        }
    }

    #[test]
    fn test_reset_short_flag() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "reset", "-y"]).unwrap();
        match cli.command {
            Commands::Reset { yes } => {
                assert!(yes);
            }
            _ => panic!("Expected Reset command"),
        }
    }

    // ========================================================================
    // Autoremove Command Tests
    // ========================================================================

    #[test]
    fn test_autoremove_dry_run() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "autoremove", "--dry-run"]).unwrap();
        match cli.command {
            Commands::Autoremove { dry_run } => {
                assert!(dry_run);
            }
            _ => panic!("Expected Autoremove command"),
        }
    }

    // ========================================================================
    // Uses Command Tests  
    // ========================================================================

    #[test]
    fn test_uses_recursive() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "uses", "openssl", "--recursive"]).unwrap();
        match cli.command {
            Commands::Uses { formula, recursive, .. } => {
                assert_eq!(formula, "openssl");
                assert!(recursive);
            }
            _ => panic!("Expected Uses command"),
        }
    }

    // ========================================================================
    // Shellenv Command Tests
    // ========================================================================

    #[test]
    fn test_shellenv_default() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "shellenv"]).unwrap();
        match cli.command {
            Commands::Shellenv { shell } => {
                assert!(shell.is_none());
            }
            _ => panic!("Expected Shellenv command"),
        }
    }

    #[test]
    fn test_shellenv_fish() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "shellenv", "--shell", "fish"]).unwrap();
        match cli.command {
            Commands::Shellenv { shell } => {
                assert_eq!(shell, Some("fish".to_string()));
            }
            _ => panic!("Expected Shellenv command"),
        }
    }
}
