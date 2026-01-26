//! Service management for Zerobrew.
//!
//! This module provides cross-platform service management for formulas that run
//! as background services. It supports:
//! - Linux: systemd user services
//! - macOS: launchd LaunchAgents
//!
//! Services are managed using the native service management system on each platform.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use zb_core::Error;

/// Status of a service
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    /// Service is running
    Running,
    /// Service is stopped
    Stopped,
    /// Service status is unknown or errored
    Unknown,
    /// Service has an error (with message)
    Error(String),
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::Running => write!(f, "running"),
            ServiceStatus::Stopped => write!(f, "stopped"),
            ServiceStatus::Unknown => write!(f, "unknown"),
            ServiceStatus::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// Information about a service
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    /// Formula name
    pub name: String,
    /// Current status
    pub status: ServiceStatus,
    /// PID if running
    pub pid: Option<u32>,
    /// Path to the service file
    pub file_path: PathBuf,
    /// Whether the service is set to start at login
    pub auto_start: bool,
}

/// Configuration for a service
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// The command to run
    pub program: PathBuf,
    /// Arguments to pass to the program
    pub args: Vec<String>,
    /// Working directory
    pub working_directory: Option<PathBuf>,
    /// Environment variables
    pub environment: HashMap<String, String>,
    /// Whether to restart on failure
    pub restart_on_failure: bool,
    /// Whether to start at login/boot
    pub run_at_load: bool,
    /// Keep the service alive
    pub keep_alive: bool,
    /// Log file for stdout
    pub stdout_log: Option<PathBuf>,
    /// Log file for stderr
    pub stderr_log: Option<PathBuf>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            program: PathBuf::new(),
            args: Vec::new(),
            working_directory: None,
            environment: HashMap::new(),
            restart_on_failure: true,
            run_at_load: true,
            keep_alive: false,
            stdout_log: None,
            stderr_log: None,
        }
    }
}

/// Service manager that handles platform-specific service operations
pub struct ServiceManager {
    /// Path to the Zerobrew prefix
    prefix: PathBuf,
    /// Path for service files
    service_dir: PathBuf,
    /// Path for log files
    log_dir: PathBuf,
}

impl ServiceManager {
    /// Create a new service manager
    pub fn new(prefix: &Path) -> Self {
        let (service_dir, log_dir) = Self::get_service_paths();
        Self {
            prefix: prefix.to_path_buf(),
            service_dir,
            log_dir,
        }
    }

    /// Get platform-specific paths for service files and logs
    #[cfg(target_os = "linux")]
    fn get_service_paths() -> (PathBuf, PathBuf) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        let config_dir = PathBuf::from(&home).join(".config/systemd/user");
        let log_dir = PathBuf::from(&home).join(".local/share/zerobrew/logs");
        (config_dir, log_dir)
    }

    #[cfg(target_os = "macos")]
    fn get_service_paths() -> (PathBuf, PathBuf) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".to_string());
        let service_dir = PathBuf::from(&home).join("Library/LaunchAgents");
        let log_dir = PathBuf::from(&home).join("Library/Logs/zerobrew");
        (service_dir, log_dir)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn get_service_paths() -> (PathBuf, PathBuf) {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        (
            PathBuf::from(&home).join(".zerobrew/services"),
            PathBuf::from(&home).join(".zerobrew/logs"),
        )
    }

    /// Get the service file path for a formula
    #[cfg(target_os = "linux")]
    fn service_file_path(&self, formula: &str) -> PathBuf {
        self.service_dir.join(format!("zerobrew.{}.service", formula))
    }

    #[cfg(target_os = "macos")]
    fn service_file_path(&self, formula: &str) -> PathBuf {
        self.service_dir.join(format!("com.zerobrew.{}.plist", formula))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn service_file_path(&self, formula: &str) -> PathBuf {
        self.service_dir.join(format!("zerobrew.{}.service", formula))
    }

    /// Get the service label/name for a formula
    #[cfg(target_os = "linux")]
    fn service_label(&self, formula: &str) -> String {
        format!("zerobrew.{}.service", formula)
    }

    #[cfg(target_os = "macos")]
    fn service_label(&self, formula: &str) -> String {
        format!("com.zerobrew.{}", formula)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn service_label(&self, formula: &str) -> String {
        format!("zerobrew.{}", formula)
    }

    /// List all managed services
    pub fn list(&self) -> Result<Vec<ServiceInfo>, Error> {
        let mut services = Vec::new();

        // Ensure service directory exists
        if !self.service_dir.exists() {
            return Ok(services);
        }

        // Read service files
        let entries = std::fs::read_dir(&self.service_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read service directory {}: {}", self.service_dir.display(), e),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Filter to zerobrew services only
            #[cfg(target_os = "linux")]
            let is_zerobrew_service = file_name.starts_with("zerobrew.") && file_name.ends_with(".service");
            #[cfg(target_os = "macos")]
            let is_zerobrew_service = file_name.starts_with("com.zerobrew.") && file_name.ends_with(".plist");
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            let is_zerobrew_service = file_name.starts_with("zerobrew.");

            if is_zerobrew_service {
                // Extract formula name from service file name
                let formula = self.extract_formula_name(file_name);
                if let Some(name) = formula {
                    let info = self.get_service_info(&name)?;
                    services.push(info);
                }
            }
        }

        // Sort by name
        services.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(services)
    }

    /// Extract formula name from service file name
    fn extract_formula_name(&self, file_name: &str) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            // zerobrew.formula.service -> formula
            file_name
                .strip_prefix("zerobrew.")
                .and_then(|s| s.strip_suffix(".service"))
                .map(String::from)
        }
        #[cfg(target_os = "macos")]
        {
            // com.zerobrew.formula.plist -> formula
            file_name
                .strip_prefix("com.zerobrew.")
                .and_then(|s| s.strip_suffix(".plist"))
                .map(String::from)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            file_name
                .strip_prefix("zerobrew.")
                .and_then(|s| s.strip_suffix(".service"))
                .map(String::from)
        }
    }

    /// Get information about a specific service
    pub fn get_service_info(&self, formula: &str) -> Result<ServiceInfo, Error> {
        let file_path = self.service_file_path(formula);
        let status = self.get_status(formula)?;
        let pid = self.get_pid(formula).ok().flatten();
        let auto_start = self.is_auto_start_enabled(formula);

        Ok(ServiceInfo {
            name: formula.to_string(),
            status,
            pid,
            file_path,
            auto_start,
        })
    }

    /// Get the status of a service
    #[cfg(target_os = "linux")]
    pub fn get_status(&self, formula: &str) -> Result<ServiceStatus, Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "is-active", &label])
            .output();

        match output {
            Ok(out) => {
                let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
                match status.as_str() {
                    "active" => Ok(ServiceStatus::Running),
                    "inactive" | "dead" => Ok(ServiceStatus::Stopped),
                    "failed" => {
                        // Get more details about the failure
                        let detail = self.get_status_detail(formula);
                        Ok(ServiceStatus::Error(detail))
                    }
                    _ => Ok(ServiceStatus::Unknown),
                }
            }
            Err(_) => Ok(ServiceStatus::Unknown),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn get_status(&self, formula: &str) -> Result<ServiceStatus, Error> {
        let label = self.service_label(formula);

        // First check if service is loaded
        let output = Command::new("launchctl")
            .args(["list"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                // Parse launchctl list output to find our service
                for line in stdout.lines() {
                    if line.contains(&label) {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            // First column is PID (- if not running), second is exit status
                            let pid = parts[0];
                            let exit_status = parts[1];

                            if pid == "-" {
                                // Not running - check exit status
                                if exit_status == "0" {
                                    return Ok(ServiceStatus::Stopped);
                                } else {
                                    return Ok(ServiceStatus::Error(format!("exited with status {}", exit_status)));
                                }
                            } else {
                                return Ok(ServiceStatus::Running);
                            }
                        }
                    }
                }
                Ok(ServiceStatus::Stopped)
            }
            Err(_) => Ok(ServiceStatus::Unknown),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn get_status(&self, _formula: &str) -> Result<ServiceStatus, Error> {
        Ok(ServiceStatus::Unknown)
    }

    /// Get more details about a service status (systemd only)
    #[cfg(target_os = "linux")]
    fn get_status_detail(&self, formula: &str) -> String {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "status", &label, "--no-pager"])
            .output();

        if let Ok(out) = output {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Look for relevant status lines
            for line in stdout.lines().chain(stderr.lines()) {
                if line.contains("Active:") && line.contains("failed") {
                    return line.trim().to_string();
                }
            }
        }
        "unknown error".to_string()
    }

    /// Get PID of a running service
    #[cfg(target_os = "linux")]
    pub fn get_pid(&self, formula: &str) -> Result<Option<u32>, Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "show", &label, "--property=MainPID"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                // Output is like: MainPID=12345
                for line in stdout.lines() {
                    if let Some(pid_str) = line.strip_prefix("MainPID=") {
                        if let Ok(pid) = pid_str.trim().parse::<u32>() {
                            if pid > 0 {
                                return Ok(Some(pid));
                            }
                        }
                    }
                }
                Ok(None)
            }
            Err(_) => Ok(None),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn get_pid(&self, formula: &str) -> Result<Option<u32>, Error> {
        let label = self.service_label(formula);
        let output = Command::new("launchctl")
            .args(["list", &label])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                // First line has: PID Status Label
                if let Some(first_line) = stdout.lines().skip(1).next() {
                    let parts: Vec<&str> = first_line.split_whitespace().collect();
                    if !parts.is_empty() && parts[0] != "-" {
                        if let Ok(pid) = parts[0].parse::<u32>() {
                            return Ok(Some(pid));
                        }
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn get_pid(&self, _formula: &str) -> Result<Option<u32>, Error> {
        Ok(None)
    }

    /// Check if auto-start is enabled
    #[cfg(target_os = "linux")]
    fn is_auto_start_enabled(&self, formula: &str) -> bool {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "is-enabled", &label])
            .output();

        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim() == "enabled",
            Err(_) => false,
        }
    }

    #[cfg(target_os = "macos")]
    fn is_auto_start_enabled(&self, formula: &str) -> bool {
        // On macOS, if the plist exists and is loaded, it's auto-start
        let file_path = self.service_file_path(formula);
        file_path.exists()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn is_auto_start_enabled(&self, _formula: &str) -> bool {
        false
    }

    /// Create a service file for a formula
    pub fn create_service(&self, formula: &str, config: &ServiceConfig) -> Result<(), Error> {
        // Ensure service directory exists
        std::fs::create_dir_all(&self.service_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create service directory {}: {}", self.service_dir.display(), e),
        })?;

        // Ensure log directory exists
        std::fs::create_dir_all(&self.log_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create log directory {}: {}", self.log_dir.display(), e),
        })?;

        let file_path = self.service_file_path(formula);
        let content = self.generate_service_file(formula, config);

        std::fs::write(&file_path, content).map_err(|e| Error::StoreCorruption {
            message: format!("failed to write service file {}: {}", file_path.display(), e),
        })?;

        // Reload daemon
        self.daemon_reload()?;

        Ok(())
    }

    /// Generate service file content
    #[cfg(target_os = "linux")]
    fn generate_service_file(&self, formula: &str, config: &ServiceConfig) -> String {
        let mut unit = format!(
            r#"[Unit]
Description=Zerobrew: {formula}
After=network.target

[Service]
Type=simple
ExecStart={program}"#,
            formula = formula,
            program = config.program.display(),
        );

        // Add arguments if any
        if !config.args.is_empty() {
            for arg in &config.args {
                unit.push_str(&format!(" {}", arg));
            }
        }
        unit.push('\n');

        // Working directory
        if let Some(ref wd) = config.working_directory {
            unit.push_str(&format!("WorkingDirectory={}\n", wd.display()));
        }

        // Environment variables
        for (key, value) in &config.environment {
            unit.push_str(&format!("Environment=\"{}={}\"\n", key, value));
        }

        // Restart policy
        if config.restart_on_failure {
            unit.push_str("Restart=on-failure\n");
            unit.push_str("RestartSec=3\n");
        }

        // Logging
        let stdout_log = config.stdout_log.clone().unwrap_or_else(|| {
            self.log_dir.join(format!("{}.log", formula))
        });
        let stderr_log = config.stderr_log.clone().unwrap_or_else(|| {
            self.log_dir.join(format!("{}.error.log", formula))
        });
        unit.push_str(&format!("StandardOutput=append:{}\n", stdout_log.display()));
        unit.push_str(&format!("StandardError=append:{}\n", stderr_log.display()));

        // Install section
        unit.push_str("\n[Install]\n");
        if config.run_at_load {
            unit.push_str("WantedBy=default.target\n");
        }

        unit
    }

    #[cfg(target_os = "macos")]
    fn generate_service_file(&self, formula: &str, config: &ServiceConfig) -> String {
        let label = self.service_label(formula);
        let stdout_log = config.stdout_log.clone().unwrap_or_else(|| {
            self.log_dir.join(format!("{}.log", formula))
        });
        let stderr_log = config.stderr_log.clone().unwrap_or_else(|| {
            self.log_dir.join(format!("{}.error.log", formula))
        });

        let mut plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>"#,
            label = label,
            program = config.program.display(),
        );

        // Add arguments
        for arg in &config.args {
            plist.push_str(&format!("\n        <string>{}</string>", arg));
        }
        plist.push_str("\n    </array>\n");

        // Working directory
        if let Some(ref wd) = config.working_directory {
            plist.push_str(&format!(
                "    <key>WorkingDirectory</key>\n    <string>{}</string>\n",
                wd.display()
            ));
        }

        // Environment variables
        if !config.environment.is_empty() {
            plist.push_str("    <key>EnvironmentVariables</key>\n    <dict>\n");
            for (key, value) in &config.environment {
                plist.push_str(&format!(
                    "        <key>{}</key>\n        <string>{}</string>\n",
                    key, value
                ));
            }
            plist.push_str("    </dict>\n");
        }

        // Run at load
        plist.push_str(&format!(
            "    <key>RunAtLoad</key>\n    <{}/>\\n",
            if config.run_at_load { "true" } else { "false" }
        ));

        // Keep alive
        if config.keep_alive {
            plist.push_str("    <key>KeepAlive</key>\n    <true/>\n");
        }

        // Logging
        plist.push_str(&format!(
            "    <key>StandardOutPath</key>\n    <string>{}</string>\n",
            stdout_log.display()
        ));
        plist.push_str(&format!(
            "    <key>StandardErrorPath</key>\n    <string>{}</string>\n",
            stderr_log.display()
        ));

        plist.push_str("</dict>\n</plist>\n");

        plist
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn generate_service_file(&self, formula: &str, config: &ServiceConfig) -> String {
        // Generic format for unsupported platforms
        format!(
            "# Service: {}\n# Program: {}\n",
            formula,
            config.program.display()
        )
    }

    /// Remove a service
    pub fn remove_service(&self, formula: &str) -> Result<(), Error> {
        // Stop the service first
        let _ = self.stop(formula);

        // Remove from auto-start
        let _ = self.disable_auto_start(formula);

        // Remove the service file
        let file_path = self.service_file_path(formula);
        if file_path.exists() {
            std::fs::remove_file(&file_path).map_err(|e| Error::StoreCorruption {
                message: format!("failed to remove service file {}: {}", file_path.display(), e),
            })?;
        }

        // Reload daemon
        self.daemon_reload()?;

        Ok(())
    }

    /// Start a service
    #[cfg(target_os = "linux")]
    pub fn start(&self, formula: &str) -> Result<(), Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "start", &label])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to start service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::StoreCorruption {
                message: format!("failed to start service: {}", stderr),
            });
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn start(&self, formula: &str) -> Result<(), Error> {
        let file_path = self.service_file_path(formula);

        // Load the service if not already loaded
        let output = Command::new("launchctl")
            .args(["load", "-w", &file_path.to_string_lossy()])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to start service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "already loaded" error
            if !stderr.contains("already loaded") {
                return Err(Error::StoreCorruption {
                    message: format!("failed to start service: {}", stderr),
                });
            }
        }

        // Start the service
        let label = self.service_label(formula);
        let _ = Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{}/{}", self.get_uid(), label)])
            .output();

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn start(&self, _formula: &str) -> Result<(), Error> {
        Err(Error::StoreCorruption {
            message: "service management not supported on this platform".to_string(),
        })
    }

    /// Stop a service
    #[cfg(target_os = "linux")]
    pub fn stop(&self, formula: &str) -> Result<(), Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "stop", &label])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to stop service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::StoreCorruption {
                message: format!("failed to stop service: {}", stderr),
            });
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn stop(&self, formula: &str) -> Result<(), Error> {
        let file_path = self.service_file_path(formula);

        let output = Command::new("launchctl")
            .args(["unload", &file_path.to_string_lossy()])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to stop service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "not loaded" error
            if !stderr.contains("Could not find") && !stderr.contains("not loaded") {
                return Err(Error::StoreCorruption {
                    message: format!("failed to stop service: {}", stderr),
                });
            }
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn stop(&self, _formula: &str) -> Result<(), Error> {
        Err(Error::StoreCorruption {
            message: "service management not supported on this platform".to_string(),
        })
    }

    /// Restart a service
    pub fn restart(&self, formula: &str) -> Result<(), Error> {
        self.stop(formula)?;
        self.start(formula)
    }

    /// Enable auto-start for a service
    #[cfg(target_os = "linux")]
    pub fn enable_auto_start(&self, formula: &str) -> Result<(), Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "enable", &label])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to enable service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::StoreCorruption {
                message: format!("failed to enable service: {}", stderr),
            });
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn enable_auto_start(&self, formula: &str) -> Result<(), Error> {
        // On macOS, loading with -w enables auto-start
        let file_path = self.service_file_path(formula);
        let output = Command::new("launchctl")
            .args(["load", "-w", &file_path.to_string_lossy()])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to enable service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("already loaded") {
                return Err(Error::StoreCorruption {
                    message: format!("failed to enable service: {}", stderr),
                });
            }
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn enable_auto_start(&self, _formula: &str) -> Result<(), Error> {
        Err(Error::StoreCorruption {
            message: "service management not supported on this platform".to_string(),
        })
    }

    /// Disable auto-start for a service
    #[cfg(target_os = "linux")]
    pub fn disable_auto_start(&self, formula: &str) -> Result<(), Error> {
        let label = self.service_label(formula);
        let output = Command::new("systemctl")
            .args(["--user", "disable", &label])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to disable service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::StoreCorruption {
                message: format!("failed to disable service: {}", stderr),
            });
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn disable_auto_start(&self, formula: &str) -> Result<(), Error> {
        let file_path = self.service_file_path(formula);
        let output = Command::new("launchctl")
            .args(["unload", "-w", &file_path.to_string_lossy()])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to disable service: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("Could not find") && !stderr.contains("not loaded") {
                return Err(Error::StoreCorruption {
                    message: format!("failed to disable service: {}", stderr),
                });
            }
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn disable_auto_start(&self, _formula: &str) -> Result<(), Error> {
        Err(Error::StoreCorruption {
            message: "service management not supported on this platform".to_string(),
        })
    }

    /// Reload systemd daemon
    #[cfg(target_os = "linux")]
    fn daemon_reload(&self) -> Result<(), Error> {
        let output = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output()
            .map_err(|e| Error::StoreCorruption {
                message: format!("failed to reload daemon: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::StoreCorruption {
                message: format!("failed to reload daemon: {}", stderr),
            });
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn daemon_reload(&self) -> Result<(), Error> {
        // No-op on non-Linux platforms
        Ok(())
    }

    /// Get current user ID (macOS only)
    #[cfg(target_os = "macos")]
    fn get_uid(&self) -> u32 {
        unsafe { libc::getuid() }
    }

    /// Try to detect service configuration from installed formula files
    pub fn detect_service_config(&self, formula: &str, keg_path: &Path) -> Option<ServiceConfig> {
        // Look for common service-related files in the keg
        let opt_path = self.prefix.join("opt").join(formula);
        let bin_path = opt_path.join("bin");

        // First, check if there's a standard service binary
        let possible_binaries = vec![
            bin_path.join(formula),
            bin_path.join(format!("{}d", formula)),  // daemon suffix
            bin_path.join(&format!("{}-server", formula)),
        ];

        for binary in possible_binaries {
            if binary.exists() {
                return Some(ServiceConfig {
                    program: binary,
                    working_directory: Some(self.prefix.join("var")),
                    ..Default::default()
                });
            }
        }

        // Check for existing homebrew service files in the keg
        #[cfg(target_os = "macos")]
        {
            let plist_path = keg_path.join("homebrew.mxcl.").join(format!("{}.plist", formula));
            if plist_path.exists() {
                return self.parse_homebrew_plist(&plist_path);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let service_path = keg_path.join("systemd").join(format!("{}.service", formula));
            if service_path.exists() {
                return self.parse_homebrew_systemd(&service_path);
            }
        }

        None
    }

    /// Parse a Homebrew plist file to extract service config
    #[cfg(target_os = "macos")]
    fn parse_homebrew_plist(&self, path: &Path) -> Option<ServiceConfig> {
        let content = std::fs::read_to_string(path).ok()?;

        // Simple XML parsing to extract program arguments
        let mut config = ServiceConfig::default();

        // Find ProgramArguments
        if let Some(start) = content.find("<key>ProgramArguments</key>") {
            if let Some(array_start) = content[start..].find("<array>") {
                let array_content = &content[start + array_start..];
                if let Some(array_end) = array_content.find("</array>") {
                    let array = &array_content[7..array_end];
                    let mut args = Vec::new();
                    for line in array.lines() {
                        if let Some(s) = line.trim().strip_prefix("<string>") {
                            if let Some(value) = s.strip_suffix("</string>") {
                                args.push(value.to_string());
                            }
                        }
                    }
                    if !args.is_empty() {
                        config.program = PathBuf::from(&args[0]);
                        config.args = args[1..].to_vec();
                    }
                }
            }
        }

        if config.program.as_os_str().is_empty() {
            return None;
        }

        // Check for RunAtLoad
        config.run_at_load = content.contains("<key>RunAtLoad</key>")
            && content.contains("<true/>");

        // Check for KeepAlive
        config.keep_alive = content.contains("<key>KeepAlive</key>")
            && content.contains("<true/>");

        Some(config)
    }

    /// Parse a Homebrew systemd service file
    #[cfg(target_os = "linux")]
    fn parse_homebrew_systemd(&self, path: &Path) -> Option<ServiceConfig> {
        let content = std::fs::read_to_string(path).ok()?;

        let mut config = ServiceConfig::default();

        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("ExecStart=") {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if !parts.is_empty() {
                    config.program = PathBuf::from(parts[0]);
                    config.args = parts[1..].iter().map(|s| s.to_string()).collect();
                }
            } else if let Some(value) = line.strip_prefix("WorkingDirectory=") {
                config.working_directory = Some(PathBuf::from(value));
            } else if line.starts_with("Restart=") && line != "Restart=no" {
                config.restart_on_failure = true;
            }
        }

        if config.program.as_os_str().is_empty() {
            return None;
        }

        Some(config)
    }

    /// Get the log file paths for a formula's service
    pub fn get_log_paths(&self, formula: &str) -> (PathBuf, PathBuf) {
        let stdout_log = self.log_dir.join(format!("{}.log", formula));
        let stderr_log = self.log_dir.join(format!("{}.error.log", formula));
        (stdout_log, stderr_log)
    }

    /// Get the log directory path
    pub fn get_log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Find services whose formulas are no longer installed
    pub fn find_orphaned_services(&self, installed_formulas: &[String]) -> Result<Vec<ServiceInfo>, Error> {
        let all_services = self.list()?;
        let orphaned: Vec<ServiceInfo> = all_services
            .into_iter()
            .filter(|s| !installed_formulas.contains(&s.name))
            .collect();
        Ok(orphaned)
    }

    /// Remove multiple services (cleanup)
    pub fn cleanup_services(&self, services: &[ServiceInfo]) -> Result<usize, Error> {
        let mut removed = 0;
        for service in services {
            self.remove_service(&service.name)?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_service_status_display() {
        assert_eq!(format!("{}", ServiceStatus::Running), "running");
        assert_eq!(format!("{}", ServiceStatus::Stopped), "stopped");
        assert_eq!(format!("{}", ServiceStatus::Unknown), "unknown");
        assert_eq!(format!("{}", ServiceStatus::Error("test".to_string())), "error: test");
    }

    #[test]
    fn test_service_config_default() {
        let config = ServiceConfig::default();
        assert!(config.program.as_os_str().is_empty());
        assert!(config.args.is_empty());
        assert!(config.working_directory.is_none());
        assert!(config.restart_on_failure);
        assert!(config.run_at_load);
        assert!(!config.keep_alive);
    }

    #[test]
    fn test_service_manager_new() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        assert_eq!(manager.prefix, PathBuf::from("/opt/zerobrew/prefix"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_service_file_path_linux() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        let path = manager.service_file_path("redis");
        assert!(path.to_string_lossy().ends_with("zerobrew.redis.service"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_service_label_linux() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        assert_eq!(manager.service_label("redis"), "zerobrew.redis.service");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_extract_formula_name_linux() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        assert_eq!(
            manager.extract_formula_name("zerobrew.redis.service"),
            Some("redis".to_string())
        );
        assert_eq!(
            manager.extract_formula_name("zerobrew.postgresql@14.service"),
            Some("postgresql@14".to_string())
        );
        assert_eq!(manager.extract_formula_name("other.service"), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_generate_service_file_linux() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        let config = ServiceConfig {
            program: PathBuf::from("/opt/zerobrew/prefix/opt/redis/bin/redis-server"),
            args: vec!["/opt/zerobrew/prefix/etc/redis.conf".to_string()],
            working_directory: Some(PathBuf::from("/opt/zerobrew/prefix/var")),
            restart_on_failure: true,
            run_at_load: true,
            ..Default::default()
        };

        let content = manager.generate_service_file("redis", &config);
        assert!(content.contains("[Unit]"));
        assert!(content.contains("Description=Zerobrew: redis"));
        assert!(content.contains("ExecStart=/opt/zerobrew/prefix/opt/redis/bin/redis-server"));
        assert!(content.contains("/opt/zerobrew/prefix/etc/redis.conf"));
        assert!(content.contains("WorkingDirectory=/opt/zerobrew/prefix/var"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("[Install]"));
        assert!(content.contains("WantedBy=default.target"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_service_file_path_macos() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        let path = manager.service_file_path("redis");
        assert!(path.to_string_lossy().ends_with("com.zerobrew.redis.plist"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_service_label_macos() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        assert_eq!(manager.service_label("redis"), "com.zerobrew.redis");
    }

    #[test]
    fn test_get_log_paths() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        let (stdout, stderr) = manager.get_log_paths("redis");
        assert!(stdout.to_string_lossy().contains("redis.log"));
        assert!(stderr.to_string_lossy().contains("redis.error.log"));
    }

    #[test]
    fn test_find_orphaned_services_empty_when_all_installed() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        // When service dir doesn't exist, should return empty vec
        let installed = vec!["redis".to_string(), "postgresql".to_string()];
        let orphaned = manager.find_orphaned_services(&installed).unwrap();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_get_log_dir() {
        let manager = ServiceManager::new(Path::new("/opt/zerobrew/prefix"));
        let log_dir = manager.get_log_dir();
        // Should return a path that includes "logs" or "Logs"
        let path_str = log_dir.to_string_lossy().to_lowercase();
        assert!(path_str.contains("log"));
    }
}
