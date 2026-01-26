use clap::{Parser, Subcommand};
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use zb_io::install::create_installer;
use zb_io::search::search_formulas;
use zb_io::{ApiClient, ApiCache, InstallProgress, ProgressCallback, ServiceManager, ServiceStatus};

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
    #[allow(clippy::enum_variant_names)]
    ZbCommands,

    /// External subcommand - runs zb-<cmd> from PATH or ~/.zerobrew/cmd/
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand, Clone)]
enum ServicesAction {
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
enum BundleAction {
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
        #[arg(long, short = 'f')]
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

/// Check if zerobrew directories need initialization
fn needs_init(root: &Path, prefix: &Path) -> bool {
    // Check if directories exist and are writable
    let root_ok = root.exists() && is_writable(root);
    let prefix_ok = prefix.exists() && is_writable(prefix);
    !(root_ok && prefix_ok)
}

fn is_writable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    // Try to check if we can write to this directory
    let test_file = path.join(".zb_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

/// Run initialization - create directories and set permissions
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

    // Check if we need sudo
    let need_sudo = dirs_to_create.iter().any(|d| {
        if d.exists() {
            !is_writable(d)
        } else {
            // Check parent
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

        // Create directories with sudo
        for dir in &dirs_to_create {
            let status = Command::new("sudo")
                .args(["mkdir", "-p", &dir.to_string_lossy()])
                .status()
                .map_err(|e| format!("Failed to run sudo mkdir: {}", e))?;

            if !status.success() {
                return Err(format!("Failed to create directory: {}", dir.display()));
            }
        }

        // Change ownership to current user - use whoami for reliability
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
        // Create directories without sudo
        for dir in &dirs_to_create {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
        }
    }

    // Add to shell config if not already there
    add_to_path(prefix)?;

    println!("{} Initialization complete!", style("==>").cyan().bold());

    Ok(())
}

fn add_to_path(prefix: &Path) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    let config_file = if shell.contains("zsh") {
        format!("{}/.zshrc", home)
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

    // Check if already in config
    let already_added = if let Ok(contents) = std::fs::read_to_string(&config_file) {
        contents.contains(&bin_path.to_string_lossy().to_string())
    } else {
        false
    };

    if !already_added {
        // Append to config
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

    // Always check if PATH is actually set in current shell
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

/// Ensure zerobrew is initialized, prompting user if needed
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
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim();

    if !input.is_empty() && !input.eq_ignore_ascii_case("y") && !input.eq_ignore_ascii_case("yes") {
        return Err(zb_core::Error::StoreCorruption {
            message: "Initialization required. Run 'zb init' first.".to_string(),
        });
    }

    run_init(root, prefix).map_err(|e| zb_core::Error::StoreCorruption { message: e })
}

fn suggest_homebrew(formula: &str, error: &zb_core::Error) {
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
        // Ensure initialized before other commands
        ensure_init(&cli.root, &cli.prefix)?;
    }

    let mut installer = create_installer(&cli.root, &cli.prefix, cli.concurrency)?;

    match cli.command {
        Commands::Init => unreachable!(), // Handled above
        Commands::Shellenv { .. } => unreachable!(), // Handled above
        Commands::Install { formula, no_link, build_from_source, head } => {
            let start = Instant::now();

            // HEAD implies building from source
            let build_from_source = build_from_source || head;

            if build_from_source {
                // Source build path
                let build_type = if head { "HEAD" } else { "source" };
                println!(
                    "{} Building {} from {}...",
                    style("==>").cyan().bold(),
                    style(&formula).bold(),
                    build_type
                );

                println!(
                    "{} Downloading source and dependencies...",
                    style("==>").cyan().bold()
                );

                let result = match installer.install_from_source(&formula, !no_link, head).await {
                    Ok(r) => r,
                    Err(e) => {
                        suggest_homebrew(&formula, &e);
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
                if let Ok(formula_info) = installer.get_formula(&formula).await {
                    // Display keg-only info if applicable
                    if formula_info.keg_only {
                        println!();
                        println!("{}", style("==> Keg-only").yellow().bold());
                        println!(
                            "{} is keg-only, which means it was not symlinked into {}",
                            style(&formula).bold(),
                            cli.prefix.display()
                        );
                        if let Some(ref reason) = formula_info.keg_only_reason
                            && !reason.explanation.is_empty()
                        {
                            println!();
                            println!("{}", reason.explanation);
                        }
                        println!();
                        println!("To use this formula, you can:");
                        println!(
                            "    • Add it to your PATH: {}",
                            style(format!("export PATH=\"{}/opt/{}/bin:$PATH\"", cli.prefix.display(), formula)).cyan()
                        );
                        println!(
                            "    • Link it with: {}",
                            style(format!("zb link {} --force", formula)).cyan()
                        );
                    }

                    // Display caveats
                    if let Some(ref caveats) = formula_info.caveats {
                        println!();
                        println!("{}", style("==> Caveats").yellow().bold());
                        // Replace $HOMEBREW_PREFIX with actual prefix
                        let caveats = caveats.replace("$HOMEBREW_PREFIX", &cli.prefix.to_string_lossy());
                        for line in caveats.lines() {
                            println!("{}", line);
                        }
                    }
                }
            } else {
                // Normal bottle install path
                println!(
                    "{} Installing {}...",
                    style("==>").cyan().bold(),
                    style(&formula).bold()
                );

                let plan = match installer.plan(&formula).await {
                    Ok(p) => p,
                    Err(e) => {
                        suggest_homebrew(&formula, &e);
                        return Err(e);
                    }
                };

                // Extract info from the root formula before executing the plan
                let root_formula = plan.formulas
                    .iter()
                    .find(|f| f.name == plan.root_name);
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

                // Set up progress display
                let multi = MultiProgress::new();
                let bars: Arc<Mutex<HashMap<String, ProgressBar>>> =
                    Arc::new(Mutex::new(HashMap::new()));

                let download_style = ProgressStyle::default_bar()
                    .template(
                        "    {prefix:<16} {bar:25.cyan/dim} {bytes:>10}/{total_bytes:<10} {eta:>6}",
                    )
                    .unwrap()
                    .progress_chars("━━╸");

                let spinner_style = ProgressStyle::default_spinner()
                    .template("    {prefix:<16} {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

                let done_style = ProgressStyle::default_spinner()
                    .template("    {prefix:<16} {msg}")
                    .unwrap();

                println!(
                    "{} Downloading and installing...",
                    style("==>").cyan().bold()
                );

                let bars_clone = bars.clone();
                let multi_clone = multi.clone();
                let download_style_clone = download_style.clone();
                let spinner_style_clone = spinner_style.clone();
                let done_style_clone = done_style.clone();

                let progress_callback: Arc<ProgressCallback> = Arc::new(Box::new(move |event| {
                    let mut bars = bars_clone.lock().unwrap();
                    match event {
                        InstallProgress::DownloadStarted { name, total_bytes } => {
                            let pb = if let Some(total) = total_bytes {
                                let pb = multi_clone.add(ProgressBar::new(total));
                                pb.set_style(download_style_clone.clone());
                                pb
                            } else {
                                let pb = multi_clone.add(ProgressBar::new_spinner());
                                pb.set_style(spinner_style_clone.clone());
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
                                pb.set_style(spinner_style_clone.clone());
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
                                pb.set_style(done_style_clone.clone());
                                pb.set_message(format!("{} installed", style("✓").green()));
                                pb.finish();
                            }
                        }
                    }
                }));

                let result = match installer
                    .execute_with_progress(plan, !no_link, Some(progress_callback))
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        suggest_homebrew(&formula, &e);
                        return Err(e);
                    }
                };

                // Finish any remaining bars
                {
                    let bars = bars.lock().unwrap();
                    for (_, pb) in bars.iter() {
                        if !pb.is_finished() {
                            pb.finish();
                        }
                    }
                }

                let elapsed = start.elapsed();
                println!();
                println!(
                    "{} Installed {} packages in {:.2}s",
                    style("==>").cyan().bold(),
                    style(result.installed).green().bold(),
                    elapsed.as_secs_f64()
                );

                // Display keg-only info if applicable
                if root_keg_only {
                    println!();
                    println!("{}", style("==> Keg-only").yellow().bold());
                    println!(
                        "{} is keg-only, which means it was not symlinked into {}",
                        style(&formula).bold(),
                        cli.prefix.display()
                    );
                    if let Some(ref reason) = root_keg_only_reason
                        && !reason.explanation.is_empty()
                    {
                        println!();
                        println!("{}", reason.explanation);
                    }
                    println!();
                    println!("To use this formula, you can:");
                    println!(
                        "    • Add it to your PATH: {}",
                        style(format!("export PATH=\"{}/opt/{}/bin:$PATH\"", cli.prefix.display(), formula)).cyan()
                    );
                    println!(
                        "    • Link it with: {}",
                        style(format!("zb link {} --force", formula)).cyan()
                    );
                }

                // Display caveats for the root formula if present
                if let Some(ref caveats) = root_caveats {
                    println!();
                    println!("{}", style("==> Caveats").yellow().bold());
                    // Replace $HOMEBREW_PREFIX with actual prefix
                    let caveats = caveats.replace("$HOMEBREW_PREFIX", &cli.prefix.to_string_lossy());
                    for line in caveats.lines() {
                        println!("{}", line);
                    }
                }
            }
        }

        Commands::Uninstall { formula } => match formula {
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
        },

        Commands::List { pinned } => {
            let installed = if pinned {
                installer.list_pinned()?
            } else {
                installer.list_installed()?
            };

            if installed.is_empty() {
                if pinned {
                    println!("No pinned formulas.");
                } else {
                    println!("No formulas installed.");
                }
            } else {
                for keg in installed {
                    let pin_marker = if keg.pinned {
                        format!(" {}", style("(pinned)").yellow())
                    } else {
                        String::new()
                    };
                    println!(
                        "{} {}{}",
                        style(&keg.name).bold(),
                        style(&keg.version).dim(),
                        pin_marker
                    );
                }
            }
        }

        Commands::Info { formula, json } => {
            // First check if installed
            let keg = installer.get_installed(&formula);

            // Try to get formula info from API for additional details
            let api_formula = installer.get_formula(&formula).await.ok();

            if json {
                // JSON output
                let mut info = serde_json::Map::new();

                info.insert("name".to_string(), serde_json::json!(formula));

                if let Some(ref keg) = keg {
                    info.insert("installed".to_string(), serde_json::json!(true));
                    info.insert("installed_version".to_string(), serde_json::json!(keg.version));
                    info.insert("store_key".to_string(), serde_json::json!(keg.store_key));
                    info.insert("installed_at".to_string(), serde_json::json!(keg.installed_at));
                    info.insert("pinned".to_string(), serde_json::json!(keg.pinned));
                    info.insert("explicit".to_string(), serde_json::json!(keg.explicit));

                    // Get linked files
                    if let Ok(linked_files) = installer.get_linked_files(&formula) {
                        let files: Vec<_> = linked_files
                            .iter()
                            .map(|(link, target)| serde_json::json!({"link": link, "target": target}))
                            .collect();
                        info.insert("linked_files".to_string(), serde_json::json!(files));
                    }

                    // Get dependents
                    if let Ok(dependents) = installer.get_dependents(&formula).await {
                        info.insert("dependents".to_string(), serde_json::json!(dependents));
                    }
                } else {
                    info.insert("installed".to_string(), serde_json::json!(false));
                }

                if let Some(ref f) = api_formula {
                    info.insert("available_version".to_string(), serde_json::json!(f.effective_version()));
                    if let Some(ref desc) = f.desc {
                        info.insert("description".to_string(), serde_json::json!(desc));
                    }
                    if let Some(ref homepage) = f.homepage {
                        info.insert("homepage".to_string(), serde_json::json!(homepage));
                    }
                    if let Some(ref license) = f.license {
                        info.insert("license".to_string(), serde_json::json!(license));
                    }
                    info.insert("dependencies".to_string(), serde_json::json!(f.effective_dependencies()));
                    info.insert("build_dependencies".to_string(), serde_json::json!(f.build_dependencies));
                    if let Some(ref caveats) = f.caveats {
                        info.insert("caveats".to_string(), serde_json::json!(caveats));
                    }
                    info.insert("keg_only".to_string(), serde_json::json!(f.keg_only));
                }

                println!("{}", serde_json::to_string_pretty(&info).unwrap());
            } else {
                // Human-readable output
                if keg.is_none() && api_formula.is_none() {
                    println!("Formula '{}' not found.", formula);
                    std::process::exit(1);
                }

                // Header
                println!(
                    "{} {}",
                    style("==>").cyan().bold(),
                    style(&formula).bold()
                );

                // Description from API
                if let Some(ref f) = api_formula {
                    if let Some(ref desc) = f.desc {
                        println!("{}", style(desc).dim());
                    }
                    if let Some(ref homepage) = f.homepage {
                        println!("{}", style(homepage).cyan().underlined());
                    }
                }

                println!();

                // Version info
                if let Some(ref keg) = keg {
                    print!("{} {}", style("Installed:").dim(), style(&keg.version).green());
                    if keg.pinned {
                        print!(" {}", style("(pinned)").yellow());
                    }
                    if !keg.explicit {
                        print!(" {}", style("(installed as dependency)").dim());
                    }
                    println!();
                } else {
                    println!("{} Not installed", style("Installed:").dim());
                }

                if let Some(ref f) = api_formula {
                    let available_version = f.effective_version();
                    if let Some(ref keg) = keg {
                        if keg.version != available_version {
                            println!(
                                "{} {} {}",
                                style("Available:").dim(),
                                style(&available_version).yellow(),
                                style("(update available)").yellow()
                            );
                        }
                    } else {
                        println!("{} {}", style("Available:").dim(), available_version);
                    }
                }

                // License
                if let Some(ref f) = api_formula
                    && let Some(ref license) = f.license
                {
                    println!("{} {}", style("License:").dim(), license);
                }

                // Keg-only status
                if let Some(ref f) = api_formula
                    && f.keg_only
                {
                    print!("{} Yes", style("Keg-only:").dim());
                    if let Some(ref reason) = f.keg_only_reason
                        && !reason.explanation.is_empty()
                    {
                        print!(" ({})", reason.explanation);
                    }
                    println!();
                }

                // Dependencies
                if let Some(ref f) = api_formula {
                    let deps = f.effective_dependencies();
                    if !deps.is_empty() {
                        println!();
                        println!("{}", style("Dependencies:").dim());
                        for dep in &deps {
                            let installed = installer.is_installed(dep);
                            let marker = if installed {
                                style("✓").green()
                            } else {
                                style("✗").red()
                            };
                            println!("  {} {}", marker, dep);
                        }
                    }

                    if !f.build_dependencies.is_empty() {
                        println!();
                        println!("{}", style("Build dependencies:").dim());
                        for dep in &f.build_dependencies {
                            println!("  {}", dep);
                        }
                    }
                }

                // Dependents (what depends on this package)
                if keg.is_some()
                    && let Ok(dependents) = installer.get_dependents(&formula).await
                    && !dependents.is_empty()
                {
                    println!();
                    println!("{}", style("Required by:").dim());
                    for dep in &dependents {
                        println!("  {}", dep);
                    }
                }

                // Linked files
                if let Some(ref keg) = keg
                    && let Ok(linked_files) = installer.get_linked_files(&formula)
                    && !linked_files.is_empty()
                {
                    println!();
                    println!("{} ({} files)", style("Linked files:").dim(), linked_files.len());
                    // Show first few linked files
                    for (link, _target) in linked_files.iter().take(5) {
                        println!("  {}", link);
                    }
                    if linked_files.len() > 5 {
                        println!(
                            "  {} and {} more...",
                            style("...").dim(),
                            linked_files.len() - 5
                        );
                    }

                    // Store info
                    println!();
                    println!("{} {}", style("Store key:").dim(), &keg.store_key[..12]);
                    println!("{} {}", style("Installed:").dim(), chrono_lite_format(keg.installed_at));
                }

                // Caveats
                if let Some(ref f) = api_formula
                    && let Some(ref caveats) = f.caveats
                {
                    println!();
                    println!("{}", style("==> Caveats").yellow().bold());
                    // Replace $HOMEBREW_PREFIX with actual prefix
                    let caveats = caveats.replace("$HOMEBREW_PREFIX", &cli.prefix.to_string_lossy());
                    for line in caveats.lines() {
                        println!("{}", line);
                    }
                }
            }
        }

        Commands::Search {
            query,
            json,
            installed,
        } => {
            if !json {
                println!(
                    "{} Searching for '{}'...",
                    style("==>").cyan().bold(),
                    style(&query).bold()
                );
            }

            // Create API client with cache
            let cache_dir = cli.root.join("cache");
            let cache = ApiCache::open(&cache_dir).ok();
            let api_client = if let Some(c) = cache {
                ApiClient::new().with_cache(c)
            } else {
                ApiClient::new()
            };

            let formulas = api_client.get_all_formulas().await?;
            let mut results = search_formulas(&formulas, &query);

            // Filter to installed-only if requested
            if installed {
                results.retain(|r| installer.is_installed(&r.name));
            }

            if json {
                // JSON output
                let json_results: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        let is_installed = installer.is_installed(&r.name);
                        serde_json::json!({
                            "name": r.name,
                            "full_name": r.full_name,
                            "version": r.version,
                            "description": r.description,
                            "installed": is_installed
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_results).unwrap());
            } else if results.is_empty() {
                if installed {
                    println!("No installed formulas found matching '{}'.", query);
                } else {
                    println!("No formulas found matching '{}'.", query);
                }
            } else {
                let label = if installed { "installed formulas" } else { "formulas" };
                println!(
                    "{} Found {} {}:",
                    style("==>").cyan().bold(),
                    style(results.len()).green().bold(),
                    label
                );
                println!();

                // Limit to top 20 results
                for result in results.iter().take(20) {
                    let is_installed = installer.is_installed(&result.name);
                    let marker = if is_installed {
                        style("✓").green().to_string()
                    } else {
                        " ".to_string()
                    };

                    println!(
                        "{} {} {}",
                        marker,
                        style(&result.name).bold(),
                        style(&result.version).dim()
                    );

                    if !result.description.is_empty() {
                        // Truncate long descriptions
                        let desc = if result.description.len() > 70 {
                            format!("{}...", &result.description[..67])
                        } else {
                            result.description.clone()
                        };
                        println!("    {}", style(desc).dim());
                    }
                }

                if results.len() > 20 {
                    println!();
                    println!(
                        "    {} and {} more...",
                        style("...").dim(),
                        results.len() - 20
                    );
                }
            }
        }

        Commands::Outdated { json } => {
            if !json {
                println!(
                    "{} Checking for outdated packages...",
                    style("==>").cyan().bold()
                );
            }

            let outdated = installer.get_outdated().await?;
            let pinned = installer.list_pinned()?;
            let pinned_count = pinned.len();

            if json {
                // JSON output
                let json_output: Vec<serde_json::Value> = outdated
                    .iter()
                    .map(|pkg| {
                        serde_json::json!({
                            "name": pkg.name,
                            "installed_version": pkg.installed_version,
                            "available_version": pkg.available_version
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_output).unwrap());
            } else if outdated.is_empty() {
                println!("All packages are up to date.");
                if pinned_count > 0 {
                    println!(
                        "    {} {} pinned packages not checked",
                        style("→").dim(),
                        pinned_count
                    );
                }
            } else {
                println!(
                    "{} {} outdated packages:",
                    style("==>").cyan().bold(),
                    style(outdated.len()).yellow().bold()
                );
                println!();

                for pkg in &outdated {
                    println!(
                        "  {} {} → {}",
                        style(&pkg.name).bold(),
                        style(&pkg.installed_version).red(),
                        style(&pkg.available_version).green()
                    );
                }

                println!();
                println!(
                    "    {} Run {} to upgrade all",
                    style("→").cyan(),
                    style("zb upgrade").cyan()
                );
                if pinned_count > 0 {
                    println!(
                        "    {} {} pinned packages not shown (use {} to see them)",
                        style("→").dim(),
                        pinned_count,
                        style("zb list --pinned").dim()
                    );
                }
            }
        }

        Commands::Upgrade { formula, dry_run } => {
            let start = Instant::now();

            // Get list of packages to upgrade
            let to_upgrade = if let Some(ref name) = formula {
                // Single package
                let outdated = installer.get_outdated().await?;
                outdated.into_iter().filter(|p| p.name == *name).collect::<Vec<_>>()
            } else {
                // All packages
                installer.get_outdated().await?
            };

            if to_upgrade.is_empty() {
                if let Some(ref name) = formula {
                    // Check if the formula exists but is up to date
                    if installer.is_installed(name) {
                        println!("{} {} is already up to date.", style("==>").cyan().bold(), style(name).bold());
                    } else {
                        println!("{} {} is not installed.", style("==>").cyan().bold(), style(name).bold());
                    }
                } else {
                    println!("{} All packages are up to date.", style("==>").cyan().bold());
                }
                return Ok(());
            }

            if dry_run {
                println!(
                    "{} Would upgrade {} packages:",
                    style("==>").cyan().bold(),
                    style(to_upgrade.len()).yellow().bold()
                );
                println!();
                for pkg in &to_upgrade {
                    println!(
                        "  {} {} → {}",
                        style(&pkg.name).bold(),
                        style(&pkg.installed_version).red(),
                        style(&pkg.available_version).green()
                    );
                }
                return Ok(());
            }

            println!(
                "{} Upgrading {} packages...",
                style("==>").cyan().bold(),
                style(to_upgrade.len()).yellow().bold()
            );

            // Set up progress display (same as install)
            let multi = MultiProgress::new();
            let bars: Arc<Mutex<HashMap<String, ProgressBar>>> =
                Arc::new(Mutex::new(HashMap::new()));

            let download_style = ProgressStyle::default_bar()
                .template(
                    "    {prefix:<16} {bar:25.cyan/dim} {bytes:>10}/{total_bytes:<10} {eta:>6}",
                )
                .unwrap()
                .progress_chars("━━╸");

            let spinner_style = ProgressStyle::default_spinner()
                .template("    {prefix:<16} {spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

            let done_style = ProgressStyle::default_spinner()
                .template("    {prefix:<16} {msg}")
                .unwrap();

            let bars_clone = bars.clone();
            let multi_clone = multi.clone();
            let download_style_clone = download_style.clone();
            let spinner_style_clone = spinner_style.clone();
            let done_style_clone = done_style.clone();

            let progress_callback: Arc<ProgressCallback> = Arc::new(Box::new(move |event| {
                let mut bars = bars_clone.lock().unwrap();
                match event {
                    InstallProgress::DownloadStarted { name, total_bytes } => {
                        let pb = if let Some(total) = total_bytes {
                            let pb = multi_clone.add(ProgressBar::new(total));
                            pb.set_style(download_style_clone.clone());
                            pb
                        } else {
                            let pb = multi_clone.add(ProgressBar::new_spinner());
                            pb.set_style(spinner_style_clone.clone());
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
                            pb.set_style(spinner_style_clone.clone());
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
                            pb.set_style(done_style_clone.clone());
                            pb.set_message(format!("{} upgraded", style("✓").green()));
                            pb.finish();
                        }
                    }
                }
            }));

            // Perform the upgrades
            let mut upgraded_packages = Vec::new();
            for pkg in &to_upgrade {
                println!();
                println!(
                    "{} Upgrading {} {} → {}...",
                    style("==>").cyan().bold(),
                    style(&pkg.name).bold(),
                    style(&pkg.installed_version).red(),
                    style(&pkg.available_version).green()
                );

                match installer
                    .upgrade_one(&pkg.name, true, Some(progress_callback.clone()))
                    .await
                {
                    Ok(Some((old_ver, new_ver))) => {
                        upgraded_packages.push((pkg.name.clone(), old_ver, new_ver));
                    }
                    Ok(None) => {
                        // Already up to date (shouldn't happen but handle gracefully)
                        println!(
                            "    {} {} is already up to date",
                            style("✓").green(),
                            pkg.name
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "    {} Failed to upgrade {}: {}",
                            style("✗").red(),
                            pkg.name,
                            e
                        );
                        // Continue with other packages
                    }
                }
            }

            // Finish any remaining bars
            {
                let bars = bars.lock().unwrap();
                for (_, pb) in bars.iter() {
                    if !pb.is_finished() {
                        pb.finish();
                    }
                }
            }

            let elapsed = start.elapsed();
            println!();
            if upgraded_packages.is_empty() {
                println!("{} No packages were upgraded.", style("==>").cyan().bold());
            } else {
                println!(
                    "{} Upgraded {} packages in {:.2}s:",
                    style("==>").cyan().bold(),
                    style(upgraded_packages.len()).green().bold(),
                    elapsed.as_secs_f64()
                );
                for (name, old_ver, new_ver) in &upgraded_packages {
                    println!(
                        "    {} {} {} → {}",
                        style("✓").green(),
                        style(name).bold(),
                        style(old_ver).dim(),
                        style(new_ver).green()
                    );
                }
            }
        }

        Commands::Pin { formula } => {
            match installer.pin(&formula) {
                Ok(true) => {
                    println!(
                        "{} Pinned {} - it will not be upgraded",
                        style("==>").cyan().bold(),
                        style(&formula).green().bold()
                    );
                }
                Ok(false) => {
                    // This shouldn't happen since we check if installed first
                    println!("Formula '{}' is not installed.", formula);
                }
                Err(zb_core::Error::NotInstalled { .. }) => {
                    println!("Formula '{}' is not installed.", formula);
                    std::process::exit(1);
                }
                Err(e) => return Err(e),
            }
        }

        Commands::Unpin { formula } => {
            match installer.unpin(&formula) {
                Ok(true) => {
                    println!(
                        "{} Unpinned {} - it will be upgraded when outdated",
                        style("==>").cyan().bold(),
                        style(&formula).green().bold()
                    );
                }
                Ok(false) => {
                    // This shouldn't happen since we check if installed first
                    println!("Formula '{}' is not installed.", formula);
                }
                Err(zb_core::Error::NotInstalled { .. }) => {
                    println!("Formula '{}' is not installed.", formula);
                    std::process::exit(1);
                }
                Err(e) => return Err(e),
            }
        }

        Commands::Gc => {
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
        }

        Commands::Autoremove { dry_run } => {
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
        }

        Commands::Cleanup { dry_run, prune } => {
            if dry_run {
                println!(
                    "{} Checking for files to clean up...",
                    style("==>").cyan().bold()
                );

                let result = installer.cleanup_dry_run(prune)?;

                if result.store_entries_removed == 0 && result.blobs_removed == 0 && result.http_cache_removed == 0 {
                    println!("Nothing to clean up.");
                    return Ok(());
                }

                println!(
                    "{} Would remove:\n",
                    style("==>").cyan().bold()
                );

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
                println!(
                    "{} Cleaning up...",
                    style("==>").cyan().bold()
                );

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
        }

        Commands::Reset { yes } => {
            if !cli.root.exists() && !cli.prefix.exists() {
                println!("Nothing to reset - directories do not exist.");
                return Ok(());
            }

            if !yes {
                println!(
                    "{} This will delete all zerobrew data at:",
                    style("Warning:").yellow().bold()
                );
                println!("      • {}", cli.root.display());
                println!("      • {}", cli.prefix.display());
                print!("Continue? [y/N] ");
                use std::io::{self, Write};
                io::stdout().flush().unwrap();

                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }

            // Remove directories - try without sudo first, then with
            for dir in [&cli.root, &cli.prefix] {
                if !dir.exists() {
                    continue;
                }

                println!(
                    "{} Removing {}...",
                    style("==>").cyan().bold(),
                    dir.display()
                );

                if std::fs::remove_dir_all(dir).is_err() {
                    // Try with sudo
                    let status = Command::new("sudo")
                        .args(["rm", "-rf", &dir.to_string_lossy()])
                        .status();

                    if status.is_err() || !status.unwrap().success() {
                        eprintln!(
                            "{} Failed to remove {}",
                            style("error:").red().bold(),
                            dir.display()
                        );
                        std::process::exit(1);
                    }
                }
            }

            // Re-initialize with correct permissions
            run_init(&cli.root, &cli.prefix)
                .map_err(|e| zb_core::Error::StoreCorruption { message: e })?;

            println!(
                "{} Reset complete. Ready for cold install.",
                style("==>").cyan().bold()
            );
        }

        Commands::Tap { user_repo } => match user_repo {
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
                        message: format!(
                            "invalid tap format '{}': expected user/repo",
                            user_repo
                        ),
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
        },

        Commands::Untap { user_repo } => {
            let parts: Vec<&str> = user_repo.split('/').collect();
            if parts.len() != 2 {
                return Err(zb_core::Error::StoreCorruption {
                    message: format!(
                        "invalid tap format '{}': expected user/repo",
                        user_repo
                    ),
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
        }

        Commands::Link { formula, overwrite, force } => {
            // Check if installed
            if !installer.is_installed(&formula) {
                eprintln!(
                    "{} Formula '{}' is not installed.",
                    style("error:").red().bold(),
                    formula
                );
                std::process::exit(1);
            }

            // Check if it's a keg-only formula (if not force)
            if !force
                && let Ok(api_formula) = installer.get_formula(&formula).await
                && api_formula.keg_only
            {
                eprintln!(
                    "{} {} is keg-only, which means it was not symlinked into {}",
                    style("Warning:").yellow().bold(),
                    formula,
                    cli.prefix.display()
                );
                if let Some(ref reason) = api_formula.keg_only_reason
                    && !reason.explanation.is_empty()
                {
                    eprintln!();
                    eprintln!("{}", reason.explanation);
                }
                eprintln!();
                eprintln!(
                    "If you need to have {} first in your PATH, run:",
                    formula
                );
                eprintln!(
                    "  {} link --force {}",
                    style("zb").cyan(),
                    formula
                );
                std::process::exit(1);
            }

            println!(
                "{} Linking {}...",
                style("==>").cyan().bold(),
                style(&formula).bold()
            );

            match installer.link(&formula, overwrite, force) {
                Ok(result) => {
                    if result.already_linked {
                        println!(
                            "{} {} is already linked",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
                        );
                    } else if result.files_linked == 0 {
                        println!(
                            "{} {} has no files to link",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
                        );
                    } else {
                        println!(
                            "{} {} Linked {} files for {}",
                            style("==>").cyan().bold(),
                            style("✓").green(),
                            result.files_linked,
                            style(&formula).bold()
                        );
                        if result.keg_only_forced {
                            println!(
                                "    {} This is a keg-only formula - it was linked with --force",
                                style("→").dim()
                            );
                        }
                    }
                }
                Err(zb_core::Error::LinkConflict { path }) => {
                    eprintln!(
                        "{} Could not link {}:",
                        style("error:").red().bold(),
                        formula
                    );
                    eprintln!();
                    eprintln!(
                        "  {} already exists",
                        path.display()
                    );
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
        }

        Commands::Unlink { formula } => {
            // Check if installed
            if !installer.is_installed(&formula) {
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
                style(&formula).bold()
            );

            let unlinked = installer.unlink(&formula)?;

            if unlinked == 0 {
                println!(
                    "{} {} has no linked files",
                    style("==>").cyan().bold(),
                    style(&formula).bold()
                );
            } else {
                println!(
                    "{} {} Unlinked {} files for {}",
                    style("==>").cyan().bold(),
                    style("✓").green(),
                    unlinked,
                    style(&formula).bold()
                );
            }
        }

        Commands::Deps { formula, tree, installed, all } => {
            if tree {
                // Tree view
                println!(
                    "{} Dependencies for {} (tree view):",
                    style("==>").cyan().bold(),
                    style(&formula).bold()
                );
                println!();

                let tree = installer.get_deps_tree(&formula, installed).await?;
                print_deps_tree(&tree, "", true);
            } else {
                // Flat list view
                let deps = installer.get_deps(&formula, installed, all).await?;

                if deps.is_empty() {
                    println!(
                        "{} {} has no{}dependencies.",
                        style("==>").cyan().bold(),
                        style(&formula).bold(),
                        if installed { " installed " } else { " " }
                    );
                } else {
                    println!(
                        "{} Dependencies for {}{}:",
                        style("==>").cyan().bold(),
                        style(&formula).bold(),
                        if all { " (all)" } else { "" }
                    );
                    println!();

                    for dep in &deps {
                        let installed_marker = if installer.is_installed(dep) {
                            style("✓").green().to_string()
                        } else {
                            style("✗").red().to_string()
                        };
                        println!("  {} {}", installed_marker, dep);
                    }
                }
            }
        }

        Commands::Uses { formula, installed: _, recursive } => {
            println!(
                "{} Checking what uses {}...",
                style("==>").cyan().bold(),
                style(&formula).bold()
            );

            // Check if the formula exists (either installed or in API)
            let formula_exists = installer.is_installed(&formula)
                || installer.get_formula(&formula).await.is_ok();

            if !formula_exists {
                println!("Formula '{}' not found.", formula);
                std::process::exit(1);
            }

            // uses command defaults to installed-only (installed flag is ignored, always true)
            let uses = installer.get_uses(&formula, true, recursive).await?;

            if uses.is_empty() {
                println!(
                    "{} No installed formulas use {}.",
                    style("==>").cyan().bold(),
                    style(&formula).bold()
                );
            } else {
                println!(
                    "{} {} installed formulas use {}{}:",
                    style("==>").cyan().bold(),
                    style(uses.len()).green().bold(),
                    style(&formula).bold(),
                    if recursive { " (directly or indirectly)" } else { "" }
                );
                println!();

                for name in &uses {
                    println!("  {}", name);
                }
            }
        }

        Commands::Leaves => {
            println!(
                "{} Finding leaf packages...",
                style("==>").cyan().bold()
            );

            let leaves = installer.get_leaves().await?;

            if leaves.is_empty() {
                println!("No installed packages, or all packages are dependencies.");
            } else {
                println!(
                    "{} {} leaf packages (not dependencies of other installed packages):",
                    style("==>").cyan().bold(),
                    style(leaves.len()).green().bold()
                );
                println!();

                for name in &leaves {
                    println!("  {}", name);
                }
            }
        }

        Commands::Doctor => {
            println!(
                "{} Running diagnostics...\n",
                style("==>").cyan().bold()
            );

            let result = installer.doctor().await;

            for check in &result.checks {
                let (marker, color) = match check.status {
                    zb_io::DoctorStatus::Ok => (style("✓").green(), ""),
                    zb_io::DoctorStatus::Warning => (style("!").yellow(), ""),
                    zb_io::DoctorStatus::Error => (style("✗").red(), ""),
                };
                let _ = color; // Suppress unused warning

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
                        if result.errors == 1 { "error" } else { "errors" }
                    );
                }
                if result.warnings > 0 {
                    println!(
                        "{} {} {} found",
                        style("==>").cyan().bold(),
                        style(result.warnings).yellow().bold(),
                        if result.warnings == 1 { "warning" } else { "warnings" }
                    );
                }
            }
        }

        Commands::Services { action } => {
            let service_manager = ServiceManager::new(&cli.prefix);

            match action {
                None | Some(ServicesAction::List { json: false }) => {
                    // List all services (human-readable format)
                    let services = service_manager.list()?;

                    if services.is_empty() {
                        println!("{} No services available.", style("==>").cyan().bold());
                        println!();
                        println!("    To start a service, first install a formula that provides one.");
                        println!("    Then run: {} services start <formula>", style("zb").cyan());
                    } else {
                        println!(
                            "{} {} services:",
                            style("==>").cyan().bold(),
                            services.len()
                        );
                        println!();

                        // Header
                        println!(
                            "{:<20} {:<10} {:<10} {}",
                            style("Name").bold(),
                            style("Status").bold(),
                            style("PID").bold(),
                            style("File").bold()
                        );
                        println!("{}", "-".repeat(60));

                        for service in &services {
                            let status_display = match &service.status {
                                ServiceStatus::Running => style("running").green().to_string(),
                                ServiceStatus::Stopped => style("stopped").dim().to_string(),
                                ServiceStatus::Unknown => style("unknown").yellow().to_string(),
                                ServiceStatus::Error(msg) => {
                                    format!("{} ({})", style("error").red(), msg)
                                }
                            };

                            let pid_display = service
                                .pid
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| "-".to_string());

                            println!(
                                "{:<20} {:<10} {:<10} {}",
                                service.name,
                                status_display,
                                pid_display,
                                service.file_path.display()
                            );
                        }
                    }
                }

                Some(ServicesAction::List { json: true }) => {
                    // List all services (JSON format)
                    let services = service_manager.list()?;

                    let json_services: Vec<serde_json::Value> = services
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "name": s.name,
                                "status": match &s.status {
                                    ServiceStatus::Running => "running",
                                    ServiceStatus::Stopped => "stopped",
                                    ServiceStatus::Unknown => "unknown",
                                    ServiceStatus::Error(_) => "error",
                                },
                                "pid": s.pid,
                                "file": s.file_path.to_string_lossy(),
                                "auto_start": s.auto_start,
                                "error": match &s.status {
                                    ServiceStatus::Error(msg) => Some(msg.clone()),
                                    _ => None,
                                }
                            })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&json_services).unwrap());
                }

                Some(ServicesAction::Start { formula }) => {
                    // Check if formula is installed
                    if !installer.is_installed(&formula) {
                        eprintln!(
                            "{} Formula '{}' is not installed.",
                            style("error:").red().bold(),
                            formula
                        );
                        std::process::exit(1);
                    }

                    // Check if service file exists, if not try to create it
                    let service_info = service_manager.get_service_info(&formula);
                    let needs_setup = match &service_info {
                        Ok(info) => !info.file_path.exists(),
                        Err(_) => true,
                    };

                    if needs_setup {
                        // Try to auto-detect service config from installed files
                        let keg = installer.get_installed(&formula).ok_or_else(|| {
                            zb_core::Error::NotInstalled {
                                name: formula.clone(),
                            }
                        })?;
                        let keg_path = cli.prefix.join("Cellar").join(&formula).join(&keg.version);

                        if let Some(config) =
                            service_manager.detect_service_config(&formula, &keg_path)
                        {
                            println!(
                                "{} Creating service file for {}...",
                                style("==>").cyan().bold(),
                                style(&formula).bold()
                            );
                            service_manager.create_service(&formula, &config)?;
                        } else {
                            eprintln!(
                                "{} Formula '{}' does not have a service definition.",
                                style("error:").red().bold(),
                                formula
                            );
                            eprintln!();
                            eprintln!("    Not all formulas provide services.");
                            eprintln!("    Check the formula's caveats with: {} info {}", style("zb").cyan(), formula);
                            std::process::exit(1);
                        }
                    }

                    println!(
                        "{} Starting {}...",
                        style("==>").cyan().bold(),
                        style(&formula).bold()
                    );

                    service_manager.start(&formula)?;

                    println!(
                        "{} {} Started {}",
                        style("==>").cyan().bold(),
                        style("✓").green(),
                        style(&formula).bold()
                    );
                }

                Some(ServicesAction::Stop { formula }) => {
                    println!(
                        "{} Stopping {}...",
                        style("==>").cyan().bold(),
                        style(&formula).bold()
                    );

                    service_manager.stop(&formula)?;

                    println!(
                        "{} {} Stopped {}",
                        style("==>").cyan().bold(),
                        style("✓").green(),
                        style(&formula).bold()
                    );
                }

                Some(ServicesAction::Restart { formula }) => {
                    println!(
                        "{} Restarting {}...",
                        style("==>").cyan().bold(),
                        style(&formula).bold()
                    );

                    service_manager.restart(&formula)?;

                    println!(
                        "{} {} Restarted {}",
                        style("==>").cyan().bold(),
                        style("✓").green(),
                        style(&formula).bold()
                    );
                }

                Some(ServicesAction::Enable { formula }) => {
                    // Check if service file exists
                    let info = service_manager.get_service_info(&formula)?;
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
                            style(&formula).bold()
                        );
                    } else {
                        println!(
                            "{} Enabling {} to start automatically...",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
                        );

                        service_manager.enable_auto_start(&formula)?;

                        println!(
                            "{} {} Enabled {} - it will start automatically at login",
                            style("==>").cyan().bold(),
                            style("✓").green(),
                            style(&formula).bold()
                        );
                    }
                }

                Some(ServicesAction::Disable { formula }) => {
                    // Check if service file exists
                    let info = service_manager.get_service_info(&formula)?;
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
                            style(&formula).bold()
                        );
                    } else {
                        println!(
                            "{} Disabling {} from starting automatically...",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
                        );

                        service_manager.disable_auto_start(&formula)?;

                        println!(
                            "{} {} Disabled {} - it will no longer start automatically",
                            style("==>").cyan().bold(),
                            style("✓").green(),
                            style(&formula).bold()
                        );
                    }
                }

                Some(ServicesAction::Run { formula }) => {
                    // Check if formula is installed
                    if !installer.is_installed(&formula) {
                        eprintln!(
                            "{} Formula '{}' is not installed.",
                            style("error:").red().bold(),
                            formula
                        );
                        std::process::exit(1);
                    }

                    // Get service config to find the command to run
                    let keg = installer.get_installed(&formula).ok_or_else(|| {
                        zb_core::Error::NotInstalled {
                            name: formula.clone(),
                        }
                    })?;
                    let keg_path = cli.prefix.join("Cellar").join(&formula).join(&keg.version);

                    if let Some(config) =
                        service_manager.detect_service_config(&formula, &keg_path)
                    {
                        println!(
                            "{} Running {} in foreground...",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
                        );
                        println!("    Command: {} {}", config.program.display(), config.args.join(" "));
                        println!("    Press Ctrl+C to stop.");
                        println!();

                        // Execute the command
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
                }

                Some(ServicesAction::Info { formula }) => {
                    let info = service_manager.get_service_info(&formula)?;
                    let (stdout_log, stderr_log) = service_manager.get_log_paths(&formula);

                    println!("{} Service: {}", style("==>").cyan().bold(), style(&formula).bold());
                    println!();

                    // Status
                    let status_display = match &info.status {
                        ServiceStatus::Running => style("running").green().to_string(),
                        ServiceStatus::Stopped => style("stopped").dim().to_string(),
                        ServiceStatus::Unknown => style("unknown").yellow().to_string(),
                        ServiceStatus::Error(msg) => format!("{} ({})", style("error").red(), msg),
                    };
                    println!("Status:        {}", status_display);

                    // PID
                    if let Some(pid) = info.pid {
                        println!("PID:           {}", pid);
                    }

                    // Auto-start
                    let auto_start_display = if info.auto_start {
                        style("yes").green().to_string()
                    } else {
                        style("no").dim().to_string()
                    };
                    println!("Auto-start:    {}", auto_start_display);

                    // Service file
                    println!("Service file:  {}", info.file_path.display());

                    // Log files
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

                    // Check if formula is installed
                    if installer.is_installed(&formula) {
                        println!();
                        println!("{} Formula is installed", style("==>").cyan().bold());
                    } else {
                        println!();
                        println!(
                            "{} {} Formula is not installed",
                            style("==>").cyan().bold(),
                            style("!").yellow()
                        );
                        println!("    This service may be orphaned.");
                        println!("    Run: {} services cleanup", style("zb").cyan());
                    }
                }

                Some(ServicesAction::Log { formula, lines, follow }) => {
                    let (stdout_log, stderr_log) = service_manager.get_log_paths(&formula);

                    // Check if any log exists
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
                        eprintln!("    Start the service first with: {} services start {}", style("zb").cyan(), formula);
                        std::process::exit(1);
                    }

                    // Prefer stdout log, fall back to stderr
                    let log_file = if stdout_log.exists() {
                        &stdout_log
                    } else {
                        &stderr_log
                    };

                    if follow {
                        // Use tail -f for following
                        println!(
                            "{} Following logs for {} (Ctrl+C to stop)...",
                            style("==>").cyan().bold(),
                            style(&formula).bold()
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
                        // Read last N lines
                        println!(
                            "{} Logs for {} (last {} lines):",
                            style("==>").cyan().bold(),
                            style(&formula).bold(),
                            lines
                        );
                        println!("    {}", log_file.display());
                        println!();

                        // Read file and get last N lines
                        let content = std::fs::read_to_string(log_file).map_err(|e| zb_core::Error::StoreCorruption {
                            message: format!("failed to read log file: {}", e),
                        })?;

                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = if all_lines.len() > lines {
                            all_lines.len() - lines
                        } else {
                            0
                        };

                        for line in &all_lines[start..] {
                            println!("{}", line);
                        }

                        if stderr_log.exists() && stderr_log != *log_file {
                            println!();
                            println!("    Note: Error log also exists at {}", stderr_log.display());
                        }
                    }
                }

                Some(ServicesAction::Cleanup { dry_run }) => {
                    // Get list of installed formulas
                    let installed: Vec<String> = installer
                        .list_installed()?
                        .iter()
                        .map(|k| k.name.clone())
                        .collect();

                    // Find orphaned services
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
                        println!("    → Run {} services cleanup to remove", style("zb").cyan());
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
                }
            }
        }

        Commands::Bundle { action } => {
            let cwd = std::env::current_dir().map_err(|e| zb_core::Error::StoreCorruption {
                message: format!("failed to get current directory: {}", e),
            })?;

            match action {
                None | Some(BundleAction::Install { file: None }) => {
                    // Default: install from Brewfile
                    let brewfile_path = installer.find_brewfile(&cwd).ok_or_else(|| {
                        zb_core::Error::StoreCorruption {
                            message: "No Brewfile found in current directory or parent directories".to_string(),
                        }
                    })?;

                    println!(
                        "{} Installing from {}",
                        style("==>").cyan().bold(),
                        brewfile_path.display()
                    );

                    let result = installer.bundle_install(&brewfile_path).await?;

                    // Report results
                    if !result.taps_added.is_empty() {
                        println!();
                        println!("{} Taps added:", style("==>").cyan().bold());
                        for tap in &result.taps_added {
                            println!("    {} {}", style("✓").green(), tap);
                        }
                    }

                    if !result.formulas_installed.is_empty() {
                        println!();
                        println!("{} Formulas installed:", style("==>").cyan().bold());
                        for formula in &result.formulas_installed {
                            println!("    {} {}", style("✓").green(), formula);
                        }
                    }

                    if !result.formulas_skipped.is_empty() {
                        println!();
                        println!("{} Already installed:", style("==>").cyan().bold());
                        for formula in &result.formulas_skipped {
                            println!("    {} {}", style("-").dim(), formula);
                        }
                    }

                    if !result.failed.is_empty() {
                        println!();
                        println!("{} Failed:", style("==>").red().bold());
                        for (name, error) in &result.failed {
                            println!("    {} {}: {}", style("✗").red(), name, error);
                        }
                    }

                    // Summary
                    println!();
                    let total_installed = result.taps_added.len() + result.formulas_installed.len();
                    if result.failed.is_empty() {
                        println!(
                            "{} Bundle complete. {} installed, {} already satisfied.",
                            style("==>").cyan().bold(),
                            total_installed,
                            result.formulas_skipped.len()
                        );
                    } else {
                        println!(
                            "{} Bundle complete with errors. {} installed, {} already satisfied, {} failed.",
                            style("==>").yellow().bold(),
                            total_installed,
                            result.formulas_skipped.len(),
                            result.failed.len()
                        );
                        std::process::exit(1);
                    }
                }

                Some(BundleAction::Install { file: Some(path) }) => {
                    println!(
                        "{} Installing from {}",
                        style("==>").cyan().bold(),
                        path.display()
                    );

                    let result = installer.bundle_install(&path).await?;

                    // Same reporting as above
                    if !result.taps_added.is_empty() {
                        println!();
                        println!("{} Taps added:", style("==>").cyan().bold());
                        for tap in &result.taps_added {
                            println!("    {} {}", style("✓").green(), tap);
                        }
                    }

                    if !result.formulas_installed.is_empty() {
                        println!();
                        println!("{} Formulas installed:", style("==>").cyan().bold());
                        for formula in &result.formulas_installed {
                            println!("    {} {}", style("✓").green(), formula);
                        }
                    }

                    if !result.formulas_skipped.is_empty() {
                        println!();
                        println!("{} Already installed:", style("==>").cyan().bold());
                        for formula in &result.formulas_skipped {
                            println!("    {} {}", style("-").dim(), formula);
                        }
                    }

                    if !result.failed.is_empty() {
                        println!();
                        println!("{} Failed:", style("==>").red().bold());
                        for (name, error) in &result.failed {
                            println!("    {} {}: {}", style("✗").red(), name, error);
                        }
                    }

                    let total_installed = result.taps_added.len() + result.formulas_installed.len();
                    println!();
                    if result.failed.is_empty() {
                        println!(
                            "{} Bundle complete. {} installed, {} already satisfied.",
                            style("==>").cyan().bold(),
                            total_installed,
                            result.formulas_skipped.len()
                        );
                    } else {
                        println!(
                            "{} Bundle complete with errors. {} installed, {} already satisfied, {} failed.",
                            style("==>").yellow().bold(),
                            total_installed,
                            result.formulas_skipped.len(),
                            result.failed.len()
                        );
                        std::process::exit(1);
                    }
                }

                Some(BundleAction::Dump { file, describe, force }) => {
                    let content = installer.bundle_dump(describe)?;

                    if let Some(path) = file {
                        // Check if file exists and force flag not set
                        if path.exists() && !force {
                            eprintln!(
                                "{} File '{}' already exists. Use --force to overwrite.",
                                style("error:").red().bold(),
                                path.display()
                            );
                            std::process::exit(1);
                        }

                        std::fs::write(&path, &content).map_err(|e| zb_core::Error::StoreCorruption {
                            message: format!("failed to write Brewfile: {}", e),
                        })?;

                        println!(
                            "{} Brewfile written to {}",
                            style("==>").cyan().bold(),
                            path.display()
                        );
                    } else {
                        // Print to stdout
                        print!("{}", content);
                        if !content.ends_with('\n') {
                            println!();
                        }
                    }
                }

                Some(BundleAction::Check { file, strict }) => {
                    let brewfile_path = if let Some(path) = file {
                        path
                    } else {
                        installer.find_brewfile(&cwd).ok_or_else(|| {
                            zb_core::Error::StoreCorruption {
                                message: "No Brewfile found in current directory or parent directories".to_string(),
                            }
                        })?
                    };

                    println!(
                        "{} Checking {}",
                        style("==>").cyan().bold(),
                        brewfile_path.display()
                    );

                    let result = installer.bundle_check(&brewfile_path)?;

                    if result.satisfied {
                        println!();
                        println!("{} All entries are satisfied!", style("==>").green().bold());
                    } else {
                        if !result.missing_taps.is_empty() {
                            println!();
                            println!("{} Missing taps:", style("==>").yellow().bold());
                            for tap in &result.missing_taps {
                                println!("    {} {}", style("✗").red(), tap);
                            }
                        }

                        if !result.missing_formulas.is_empty() {
                            println!();
                            println!("{} Missing formulas:", style("==>").yellow().bold());
                            for formula in &result.missing_formulas {
                                println!("    {} {}", style("✗").red(), formula);
                            }
                        }

                        println!();
                        println!(
                            "    → Run {} bundle to install missing entries",
                            style("zb").cyan()
                        );

                        if strict {
                            std::process::exit(1);
                        }
                    }
                }

                Some(BundleAction::List { file }) => {
                    let brewfile_path = if let Some(path) = file {
                        path
                    } else {
                        installer.find_brewfile(&cwd).ok_or_else(|| {
                            zb_core::Error::StoreCorruption {
                                message: "No Brewfile found in current directory or parent directories".to_string(),
                            }
                        })?
                    };

                    let entries = installer.parse_brewfile(&brewfile_path)?;

                    println!(
                        "{} Entries in {}:",
                        style("==>").cyan().bold(),
                        brewfile_path.display()
                    );
                    println!();

                    #[allow(unused_mut)]
                    let mut tap_count = 0;
                    let mut brew_count = 0;

                    for entry in &entries {
                        match entry {
                            zb_io::BrewfileEntry::Tap { name } => {
                                println!("tap  {}", style(name).cyan());
                                tap_count += 1;
                            }
                            zb_io::BrewfileEntry::Brew { name, args } => {
                                if args.is_empty() {
                                    println!("brew {}", style(name).green());
                                } else {
                                    println!(
                                        "brew {} ({})",
                                        style(name).green(),
                                        args.join(", ")
                                    );
                                }
                                brew_count += 1;
                            }
                            zb_io::BrewfileEntry::Comment(_) => {
                                // Skip comments in list output
                            }
                        }
                    }

                    println!();
                    println!(
                        "{} {} taps, {} formulas",
                        style("==>").cyan().bold(),
                        tap_count,
                        brew_count
                    );
                }
            }
        }

        Commands::ZbCommands => {
            // List all available commands (built-in and external)
            let builtin_commands = [
                ("autoremove", "Remove orphaned dependencies"),
                ("bundle", "Install from a Brewfile or manage Brewfile configuration"),
                ("cleanup", "Remove old versions and cache files"),
                ("deps", "Show dependencies for a formula"),
                ("doctor", "Diagnose common issues"),
                ("gc", "Garbage collect unreferenced store entries"),
                ("info", "Show info about an installed formula"),
                ("init", "Initialize zerobrew directories"),
                ("install", "Install a formula"),
                ("leaves", "List installed formulas that are not dependencies"),
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
                ("zb-commands", "List all available commands"),
            ];

            println!("{} Built-in commands:", style("==>").cyan().bold());
            for (name, desc) in &builtin_commands {
                println!("    {} {}", style(name).green().bold(), style(desc).dim());
            }

            // Find external commands
            let external_commands = find_external_commands(&cli.root);
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
        }

        Commands::External(args) => {
            if args.is_empty() {
                eprintln!("{} No command specified", style("error:").red().bold());
                std::process::exit(1);
            }

            let cmd_name = &args[0];
            let cmd_args = &args[1..];

            // Look for zb-<cmd> executable
            if let Some(cmd_path) = find_external_command(cmd_name, &cli.root) {
                let status = Command::new(&cmd_path)
                    .args(cmd_args)
                    .env("ZB_ROOT", &cli.root)
                    .env("ZB_PREFIX", &cli.prefix)
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
                    "{} Unknown command '{}'\n\nRun 'zb zb-commands' to see available commands.",
                    style("error:").red().bold(),
                    cmd_name
                );
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Find all external commands (zb-* executables in PATH and ~/.zerobrew/cmd/)
fn find_external_commands(root: &Path) -> Vec<(String, PathBuf)> {
    let mut commands = Vec::new();

    // Look in ~/.zerobrew/cmd/
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

    // Look in PATH for zb-* commands
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
                        // Don't add duplicates
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

/// Find a specific external command
fn find_external_command(name: &str, root: &Path) -> Option<PathBuf> {
    let cmd_name = format!("zb-{}", name);

    // First look in ~/.zerobrew/cmd/
    let cmd_dir = root.join("cmd");
    let local_cmd = cmd_dir.join(&cmd_name);
    if local_cmd.exists() && is_executable(&local_cmd) {
        return Some(local_cmd);
    }

    // Then look in PATH
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

/// Check if a file is executable
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = path.metadata() {
        meta.permissions().mode() & 0o111 != 0
    } else {
        false
    }
}

/// Detect the current shell from environment
fn detect_shell() -> &'static str {
    // Check SHELL environment variable
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("fish") {
            return "fish";
        } else if shell.contains("csh") || shell.contains("tcsh") {
            return "csh";
        } else if shell.contains("zsh") {
            return "zsh";
        }
    }
    // Default to bash-compatible (works for bash, zsh, sh, etc.)
    "bash"
}

/// Generate shell environment setup commands
fn generate_shellenv(prefix: &Path, shell: &str) -> String {
    let bin_path = prefix.join("bin");
    let sbin_path = prefix.join("sbin");
    let man_path = prefix.join("share").join("man");
    let info_path = prefix.join("share").join("info");
    let cellar_path = prefix.join("Cellar");

    match shell {
        "fish" => {
            // Fish shell syntax
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
            // C shell syntax
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
            // POSIX-compatible shells (bash, zsh, sh, ksh, etc.)
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

/// Print shell environment setup commands
fn print_shellenv(prefix: &Path, shell: Option<&str>) {
    let shell = shell.unwrap_or_else(|| detect_shell());
    println!("{}", generate_shellenv(prefix, shell));
}

fn chrono_lite_format(timestamp: i64) -> String {
    // Simple timestamp formatting without pulling in chrono
    use std::time::{Duration, UNIX_EPOCH};

    let dt = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
    format!("{:?}", dt)
}

/// Format bytes into a human-readable string (e.g., "1.5 GB")
fn format_bytes(bytes: u64) -> String {
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

/// Print a dependency tree with ASCII art formatting
fn print_deps_tree(tree: &zb_io::DepsTree, prefix: &str, is_last: bool) {
    // Print current node
    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let installed_marker = if tree.installed {
        style("✓").green().to_string()
    } else {
        style("✗").red().to_string()
    };

    println!("{}{}{} {}", prefix, connector, installed_marker, tree.name);

    // Prepare prefix for children
    let new_prefix = if prefix.is_empty() {
        "".to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    // Print children
    for (i, child) in tree.children.iter().enumerate() {
        let is_last_child = i == tree.children.len() - 1;
        print_deps_tree(child, &new_prefix, is_last_child);
    }
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
        assert!(output.contains("export PATH=\"/opt/zerobrew/prefix/bin:/opt/zerobrew/prefix/sbin:$PATH\""));
        assert!(output.contains("export MANPATH=\"/opt/zerobrew/prefix/share/man:${MANPATH:-}\""));
        assert!(output.contains("export INFOPATH=\"/opt/zerobrew/prefix/share/info:${INFOPATH:-}\""));
    }

    #[test]
    fn test_generate_shellenv_zsh() {
        // zsh uses the same syntax as bash
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
        assert!(output.contains("set -gx PATH \"/opt/zerobrew/prefix/bin\" \"/opt/zerobrew/prefix/sbin\" $PATH"));
        assert!(output.contains("set -q MANPATH; or set MANPATH ''"));
        assert!(output.contains("set -q INFOPATH; or set INFOPATH ''"));
    }

    #[test]
    fn test_generate_shellenv_csh() {
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let output = generate_shellenv(&prefix, "csh");

        assert!(output.contains("setenv HOMEBREW_PREFIX \"/opt/zerobrew/prefix\""));
        assert!(output.contains("setenv HOMEBREW_CELLAR \"/opt/zerobrew/prefix/Cellar\""));
        assert!(output.contains("setenv PATH \"/opt/zerobrew/prefix/bin:/opt/zerobrew/prefix/sbin:${PATH}\""));
        assert!(output.contains("setenv MANPATH \"/opt/zerobrew/prefix/share/man:${MANPATH}\""));
        assert!(output.contains("setenv INFOPATH \"/opt/zerobrew/prefix/share/info:${INFOPATH}\""));
    }

    #[test]
    fn test_generate_shellenv_tcsh() {
        // tcsh uses the same syntax as csh
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
        // Unknown shells should use POSIX-compatible syntax
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

    #[test]
    fn test_services_list_json_flag_parsing() {
        use clap::Parser;

        // Test that --json flag is parsed correctly for services list
        let cli = Cli::try_parse_from(["zb", "services", "list", "--json"]);
        assert!(cli.is_ok());
        if let Ok(cli) = cli {
            match cli.command {
                Commands::Services { action: Some(ServicesAction::List { json }) } => {
                    assert!(json);
                }
                _ => panic!("Expected Services List command"),
            }
        }

        // Test services list without --json flag
        let cli = Cli::try_parse_from(["zb", "services", "list"]);
        assert!(cli.is_ok());
        if let Ok(cli) = cli {
            match cli.command {
                Commands::Services { action: Some(ServicesAction::List { json }) } => {
                    assert!(!json);
                }
                _ => panic!("Expected Services List command"),
            }
        }
    }

    #[test]
    fn test_services_enable_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "enable", "redis"]);
        assert!(cli.is_ok());
        if let Ok(cli) = cli {
            match cli.command {
                Commands::Services { action: Some(ServicesAction::Enable { formula }) } => {
                    assert_eq!(formula, "redis");
                }
                _ => panic!("Expected Services Enable command"),
            }
        }
    }

    #[test]
    fn test_services_disable_parsing() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["zb", "services", "disable", "postgresql"]);
        assert!(cli.is_ok());
        if let Ok(cli) = cli {
            match cli.command {
                Commands::Services { action: Some(ServicesAction::Disable { formula }) } => {
                    assert_eq!(formula, "postgresql");
                }
                _ => panic!("Expected Services Disable command"),
            }
        }
    }
}
