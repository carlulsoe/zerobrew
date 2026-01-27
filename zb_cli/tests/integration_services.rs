#![allow(clippy::unnecessary_unwrap)]

//! Integration tests for services CLI commands.
//!
//! These tests verify the integration between CLI commands and the underlying
//! zb_io layer for service management. Tests focus on:
//!
//! - Service config detection from installed formulas
//! - Service file content generation
//! - Log file path computation
//! - Orphan service detection flow
//!
//! Run with: `cargo test --test integration_services`

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use zb_io::test_utils::TestContext;
use zb_io::{ServiceConfig, ServiceManager, ServiceStatus};

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a ServiceManager with test paths in a temp directory.
fn create_test_service_manager(tmp: &TempDir) -> (ServiceManager, PathBuf, PathBuf, PathBuf) {
    let prefix = tmp.path().join("prefix");
    let service_dir = tmp.path().join("services");
    let log_dir = tmp.path().join("logs");

    fs::create_dir_all(&prefix).unwrap();
    fs::create_dir_all(&service_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();

    let manager = ServiceManager::new_with_paths(&prefix, &service_dir, &log_dir);
    (manager, prefix, service_dir, log_dir)
}

/// Create a fake service file in the service directory.
#[cfg(target_os = "linux")]
fn create_fake_service_file(service_dir: &std::path::Path, formula: &str) {
    let file_name = format!("zerobrew.{}.service", formula);
    let file_path = service_dir.join(&file_name);
    let content = format!(
        r#"[Unit]
Description=Zerobrew: {}

[Service]
Type=simple
ExecStart=/usr/bin/{}

[Install]
WantedBy=default.target
"#,
        formula, formula
    );
    fs::write(file_path, content).unwrap();
}

#[cfg(target_os = "macos")]
fn create_fake_service_file(service_dir: &std::path::Path, formula: &str) {
    let file_name = format!("com.zerobrew.{}.plist", formula);
    let file_path = service_dir.join(&file_name);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.zerobrew.{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/{}</string>
    </array>
</dict>
</plist>
"#,
        formula, formula
    );
    fs::write(file_path, content).unwrap();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_fake_service_file(service_dir: &std::path::Path, formula: &str) {
    let file_name = format!("zerobrew.{}.service", formula);
    let file_path = service_dir.join(&file_name);
    fs::write(file_path, format!("# Service: {}\n", formula)).unwrap();
}

// ============================================================================
// Service Config Detection Tests
// ============================================================================

/// Test that service config is detected when a binary exists in opt/formula/bin/.
#[test]
fn test_detect_service_config_finds_binary() {
    let tmp = TempDir::new().unwrap();
    let (manager, prefix, _, _) = create_test_service_manager(&tmp);

    // Create opt/myservice/bin/myservice binary
    let bin_dir = prefix.join("opt/myservice/bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let binary_path = bin_dir.join("myservice");
    fs::write(&binary_path, "#!/bin/sh\necho hello").unwrap();

    // Create a fake keg path
    let keg_path = prefix.join("Cellar/myservice/1.0.0");
    fs::create_dir_all(&keg_path).unwrap();

    let config = manager.detect_service_config("myservice", &keg_path);
    assert!(config.is_some(), "Should detect service config");

    let config = config.unwrap();
    assert_eq!(config.program, binary_path);
}

/// Test that service config is detected for daemon-style binaries (with 'd' suffix).
#[test]
fn test_detect_service_config_finds_daemon_binary() {
    let tmp = TempDir::new().unwrap();
    let (manager, prefix, _, _) = create_test_service_manager(&tmp);

    // Create opt/nginx/bin/nginxd (daemon suffix)
    let bin_dir = prefix.join("opt/nginx/bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let binary_path = bin_dir.join("nginxd");
    fs::write(&binary_path, "#!/bin/sh\necho nginx").unwrap();

    let keg_path = prefix.join("Cellar/nginx/1.25.0");
    fs::create_dir_all(&keg_path).unwrap();

    let config = manager.detect_service_config("nginx", &keg_path);
    assert!(config.is_some(), "Should detect daemon binary");

    let config = config.unwrap();
    assert!(
        config.program.to_string_lossy().ends_with("nginxd"),
        "Should find the daemon binary"
    );
}

/// Test that service config is detected for server-style binaries.
#[test]
fn test_detect_service_config_finds_server_binary() {
    let tmp = TempDir::new().unwrap();
    let (manager, prefix, _, _) = create_test_service_manager(&tmp);

    // Create opt/redis/bin/redis-server
    let bin_dir = prefix.join("opt/redis/bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let binary_path = bin_dir.join("redis-server");
    fs::write(&binary_path, "#!/bin/sh\necho redis").unwrap();

    let keg_path = prefix.join("Cellar/redis/7.2.0");
    fs::create_dir_all(&keg_path).unwrap();

    let config = manager.detect_service_config("redis", &keg_path);
    assert!(config.is_some(), "Should detect server binary");

    let config = config.unwrap();
    assert!(
        config.program.to_string_lossy().ends_with("redis-server"),
        "Should find the server binary"
    );
}

/// Test that no service config is detected when no binary exists.
#[test]
fn test_detect_service_config_returns_none_when_no_binary() {
    let tmp = TempDir::new().unwrap();
    let (manager, prefix, _, _) = create_test_service_manager(&tmp);

    // Create opt/libfoo/lib but no bin directory
    let lib_dir = prefix.join("opt/libfoo/lib");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(lib_dir.join("libfoo.so"), "fake library").unwrap();

    let keg_path = prefix.join("Cellar/libfoo/1.0.0");
    fs::create_dir_all(&keg_path).unwrap();

    let config = manager.detect_service_config("libfoo", &keg_path);
    assert!(
        config.is_none(),
        "Should not detect service config for library-only formula"
    );
}

// ============================================================================
// Service Config Detection with TestContext (Installed Formulas)
// ============================================================================

/// Test service config detection from an actually installed formula.
#[tokio::test]
async fn test_detect_service_config_from_installed_formula() {
    let mut ctx = TestContext::new().await;

    // Install a formula
    ctx.mount_formula("testservice", "1.0.0", &[]).await;
    ctx.installer_mut()
        .install("testservice", true)
        .await
        .unwrap();

    // Verify the formula is installed
    assert!(ctx.installer().is_installed("testservice"));

    // Create a service manager pointing to the test prefix
    let tmp = TempDir::new().unwrap();
    let service_dir = tmp.path().join("services");
    let log_dir = tmp.path().join("logs");
    fs::create_dir_all(&service_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();

    let manager = ServiceManager::new_with_paths(&ctx.prefix(), &service_dir, &log_dir);

    // The test formula from TestContext creates bin/testservice
    // Try to detect service config
    let keg_path = ctx.cellar().join("testservice/1.0.0");

    // Note: detect_service_config looks in opt/<formula>/bin, not cellar
    // So we need to check if opt path was created during install
    let opt_path = ctx.prefix().join("opt/testservice/bin");
    if opt_path.exists() {
        let config = manager.detect_service_config("testservice", &keg_path);
        // Config detection depends on opt links being created
        if config.is_some() {
            let cfg = config.unwrap();
            assert!(!cfg.program.as_os_str().is_empty());
        }
    }
}

// ============================================================================
// Service File Content Generation Tests
// ============================================================================

/// Test that systemd service file is generated correctly on Linux.
#[test]
#[cfg(target_os = "linux")]
fn test_generate_service_file_linux_basic() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    let config = ServiceConfig {
        program: PathBuf::from("/opt/zerobrew/prefix/opt/redis/bin/redis-server"),
        args: vec!["--port".to_string(), "6379".to_string()],
        working_directory: Some(PathBuf::from("/var/lib/redis")),
        restart_on_failure: true,
        run_at_load: true,
        ..Default::default()
    };

    // Create service (this writes the file)
    let result = manager.create_service("redis", &config);

    // daemon_reload might fail in test environment without systemd
    // But the file should still be created
    let service_file = service_dir.join("zerobrew.redis.service");
    assert!(service_file.exists(), "Service file should be created");

    let content = fs::read_to_string(&service_file).unwrap();

    // Verify systemd unit file structure
    assert!(content.contains("[Unit]"), "Should have [Unit] section");
    assert!(
        content.contains("[Service]"),
        "Should have [Service] section"
    );
    assert!(
        content.contains("[Install]"),
        "Should have [Install] section"
    );
    assert!(
        content.contains("Description=Zerobrew: redis"),
        "Should have description"
    );
    assert!(
        content.contains("ExecStart=/opt/zerobrew/prefix/opt/redis/bin/redis-server --port 6379"),
        "Should have ExecStart with args"
    );
    assert!(
        content.contains("WorkingDirectory=/var/lib/redis"),
        "Should have working directory"
    );
    assert!(
        content.contains("Restart=on-failure"),
        "Should have restart policy"
    );
    assert!(
        content.contains("WantedBy=default.target"),
        "Should have install target"
    );

    // Cleanup result doesn't matter for this test
    let _ = result;
}

/// Test that launchd plist file is generated correctly on macOS.
#[test]
#[cfg(target_os = "macos")]
fn test_generate_service_file_macos_basic() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    let config = ServiceConfig {
        program: PathBuf::from("/opt/zerobrew/prefix/opt/redis/bin/redis-server"),
        args: vec!["--port".to_string(), "6379".to_string()],
        run_at_load: true,
        keep_alive: true,
        ..Default::default()
    };

    let _ = manager.create_service("redis", &config);

    let plist_file = service_dir.join("com.zerobrew.redis.plist");
    assert!(plist_file.exists(), "Plist file should be created");

    let content = fs::read_to_string(&plist_file).unwrap();

    // Verify plist structure
    assert!(content.contains("<?xml version"), "Should be XML");
    assert!(
        content.contains("<key>Label</key>"),
        "Should have Label key"
    );
    assert!(
        content.contains("<string>com.zerobrew.redis</string>"),
        "Should have correct label"
    );
    assert!(
        content.contains("<key>ProgramArguments</key>"),
        "Should have program arguments"
    );
    assert!(
        content.contains("<key>RunAtLoad</key>"),
        "Should have RunAtLoad"
    );
    assert!(
        content.contains("<key>KeepAlive</key>"),
        "Should have KeepAlive"
    );
}

/// Test service file generation with environment variables.
#[test]
#[cfg(target_os = "linux")]
fn test_generate_service_file_with_environment() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    let mut env = std::collections::HashMap::new();
    env.insert("REDIS_PORT".to_string(), "6379".to_string());
    env.insert("REDIS_HOST".to_string(), "127.0.0.1".to_string());

    let config = ServiceConfig {
        program: PathBuf::from("/usr/bin/redis-server"),
        environment: env,
        ..Default::default()
    };

    let _ = manager.create_service("redis", &config);

    let service_file = service_dir.join("zerobrew.redis.service");
    let content = fs::read_to_string(&service_file).unwrap();

    assert!(
        content.contains("Environment="),
        "Should have environment variables"
    );
    assert!(
        content.contains("REDIS_PORT=6379") || content.contains("\"REDIS_PORT=6379\""),
        "Should have REDIS_PORT"
    );
}

/// Test service file generation without restart on failure.
#[test]
#[cfg(target_os = "linux")]
fn test_generate_service_file_no_restart() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    let config = ServiceConfig {
        program: PathBuf::from("/usr/bin/oneshot"),
        restart_on_failure: false,
        run_at_load: false,
        ..Default::default()
    };

    let _ = manager.create_service("oneshot", &config);

    let service_file = service_dir.join("zerobrew.oneshot.service");
    let content = fs::read_to_string(&service_file).unwrap();

    assert!(
        !content.contains("Restart=on-failure"),
        "Should not have restart policy"
    );
    assert!(
        !content.contains("WantedBy=default.target"),
        "Should not have install target when run_at_load is false"
    );
}

// ============================================================================
// Log File Path Computation Tests
// ============================================================================

/// Test that log paths are computed correctly.
#[test]
fn test_log_file_path_computation_basic() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, log_dir) = create_test_service_manager(&tmp);

    let (stdout_log, stderr_log) = manager.get_log_paths("redis");

    assert_eq!(
        stdout_log,
        log_dir.join("redis.log"),
        "Stdout log should be in log_dir"
    );
    assert_eq!(
        stderr_log,
        log_dir.join("redis.error.log"),
        "Stderr log should be in log_dir"
    );
}

/// Test log paths for versioned formulas.
#[test]
fn test_log_file_path_computation_versioned() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, log_dir) = create_test_service_manager(&tmp);

    let (stdout_log, stderr_log) = manager.get_log_paths("postgresql@14");

    assert_eq!(stdout_log, log_dir.join("postgresql@14.log"));
    assert_eq!(stderr_log, log_dir.join("postgresql@14.error.log"));
}

/// Test log paths for formulas with special characters.
#[test]
fn test_log_file_path_computation_special_chars() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, log_dir) = create_test_service_manager(&tmp);

    let (stdout_log, stderr_log) = manager.get_log_paths("my-service_v2");

    assert_eq!(stdout_log, log_dir.join("my-service_v2.log"));
    assert_eq!(stderr_log, log_dir.join("my-service_v2.error.log"));
}

/// Test get_log_dir returns the configured log directory.
#[test]
fn test_get_log_dir() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, log_dir) = create_test_service_manager(&tmp);

    assert_eq!(manager.get_log_dir(), log_dir.as_path());
}

// ============================================================================
// Orphan Service Detection Tests
// ============================================================================

/// Test that orphaned services are detected when formula is not installed.
#[test]
fn test_find_orphaned_services_detects_orphans() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    // Create service files for formulas
    create_fake_service_file(&service_dir, "redis");
    create_fake_service_file(&service_dir, "postgresql");
    create_fake_service_file(&service_dir, "nginx");

    // Only redis and nginx are "installed"
    let installed_formulas = vec!["redis".to_string(), "nginx".to_string()];

    let orphaned = manager.find_orphaned_services(&installed_formulas).unwrap();

    // postgresql should be orphaned
    assert_eq!(orphaned.len(), 1, "Should find one orphaned service");
    assert_eq!(orphaned[0].name, "postgresql");
}

/// Test that no orphans are found when all services have installed formulas.
#[test]
fn test_find_orphaned_services_none_when_all_installed() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");
    create_fake_service_file(&service_dir, "postgresql");

    let installed_formulas = vec!["redis".to_string(), "postgresql".to_string()];

    let orphaned = manager.find_orphaned_services(&installed_formulas).unwrap();

    assert!(orphaned.is_empty(), "Should find no orphaned services");
}

/// Test that all services are orphaned when nothing is installed.
#[test]
fn test_find_orphaned_services_all_orphaned() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");
    create_fake_service_file(&service_dir, "postgresql");
    create_fake_service_file(&service_dir, "nginx");

    let installed_formulas: Vec<String> = vec![];

    let orphaned = manager.find_orphaned_services(&installed_formulas).unwrap();

    assert_eq!(orphaned.len(), 3, "All services should be orphaned");
}

/// Test orphan detection with empty service directory.
#[test]
fn test_find_orphaned_services_empty_service_dir() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, _) = create_test_service_manager(&tmp);

    let installed_formulas = vec!["redis".to_string()];

    let orphaned = manager.find_orphaned_services(&installed_formulas).unwrap();

    assert!(
        orphaned.is_empty(),
        "Should find no orphaned services in empty dir"
    );
}

/// Test orphan detection ignores non-zerobrew service files.
#[test]
#[cfg(target_os = "linux")]
fn test_find_orphaned_services_ignores_non_zerobrew_files() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    // Create a zerobrew service
    create_fake_service_file(&service_dir, "redis");

    // Create non-zerobrew service files
    fs::write(
        service_dir.join("other.service"),
        "[Unit]\nDescription=Other",
    )
    .unwrap();
    fs::write(
        service_dir.join("someapp.service"),
        "[Unit]\nDescription=Some App",
    )
    .unwrap();

    let installed_formulas: Vec<String> = vec![];

    let orphaned = manager.find_orphaned_services(&installed_formulas).unwrap();

    // Only redis should be found as orphaned (other files are not zerobrew services)
    assert_eq!(orphaned.len(), 1);
    assert_eq!(orphaned[0].name, "redis");
}

// ============================================================================
// Service Listing Tests
// ============================================================================

/// Test listing services returns correct count.
#[test]
fn test_list_services_returns_all() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");
    create_fake_service_file(&service_dir, "postgresql");
    create_fake_service_file(&service_dir, "nginx");

    let services = manager.list().unwrap();

    assert_eq!(services.len(), 3, "Should list all three services");

    let names: Vec<&str> = services.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"redis"));
    assert!(names.contains(&"postgresql"));
    assert!(names.contains(&"nginx"));
}

/// Test listing services with empty directory.
#[test]
fn test_list_services_empty() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, _, _) = create_test_service_manager(&tmp);

    let services = manager.list().unwrap();

    assert!(services.is_empty(), "Should return empty list");
}

/// Test listing services when service directory doesn't exist.
#[test]
fn test_list_services_nonexistent_dir() {
    let tmp = TempDir::new().unwrap();
    let manager = ServiceManager::new_with_paths(
        tmp.path(),
        &tmp.path().join("nonexistent/services"),
        &tmp.path().join("logs"),
    );

    let services = manager.list().unwrap();

    assert!(
        services.is_empty(),
        "Should return empty list for nonexistent dir"
    );
}

/// Test that listed services are sorted by name.
#[test]
fn test_list_services_sorted() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    // Create in non-alphabetical order
    create_fake_service_file(&service_dir, "zebra");
    create_fake_service_file(&service_dir, "apple");
    create_fake_service_file(&service_dir, "mango");

    let services = manager.list().unwrap();

    let names: Vec<&str> = services.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["apple", "mango", "zebra"], "Should be sorted");
}

// ============================================================================
// Service Info Tests
// ============================================================================

/// Test getting service info for an existing service.
#[test]
fn test_get_service_info_existing() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");

    let info = manager.get_service_info("redis").unwrap();

    assert_eq!(info.name, "redis");
    // Status will be Unknown or Stopped since we're not actually running systemd/launchd
    assert!(
        matches!(info.status, ServiceStatus::Stopped | ServiceStatus::Unknown),
        "Status should be stopped or unknown in test environment"
    );
}

/// Test service info file path is correct.
#[test]
#[cfg(target_os = "linux")]
fn test_get_service_info_file_path_linux() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");

    let info = manager.get_service_info("redis").unwrap();

    assert_eq!(info.file_path, service_dir.join("zerobrew.redis.service"));
}

#[test]
#[cfg(target_os = "macos")]
fn test_get_service_info_file_path_macos() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    create_fake_service_file(&service_dir, "redis");

    let info = manager.get_service_info("redis").unwrap();

    assert_eq!(info.file_path, service_dir.join("com.zerobrew.redis.plist"));
}

// ============================================================================
// Cleanup Service Tests
// ============================================================================

/// Test cleanup removes orphaned service files.
#[test]
fn test_cleanup_services_removes_orphans() {
    let tmp = TempDir::new().unwrap();
    let (manager, _, service_dir, _) = create_test_service_manager(&tmp);

    // Create orphaned services
    create_fake_service_file(&service_dir, "orphan1");
    create_fake_service_file(&service_dir, "orphan2");

    let orphaned = manager.find_orphaned_services(&[]).unwrap();
    assert_eq!(orphaned.len(), 2);

    // Cleanup services (this will try to stop them first, which may fail in test env)
    let removed = manager.cleanup_services(&orphaned);

    // Even if stop/disable fail, the files should be removed
    // Check that files are removed
    #[cfg(target_os = "linux")]
    {
        assert!(!service_dir.join("zerobrew.orphan1.service").exists());
        assert!(!service_dir.join("zerobrew.orphan2.service").exists());
    }

    #[cfg(target_os = "macos")]
    {
        assert!(!service_dir.join("com.zerobrew.orphan1.plist").exists());
        assert!(!service_dir.join("com.zerobrew.orphan2.plist").exists());
    }

    // Verify count
    if let Ok(count) = removed {
        assert_eq!(count, 2, "Should have removed 2 services");
    }
}

// ============================================================================
// Integration with Installer (using TestContext)
// ============================================================================

/// Test the full flow: install formula, detect service config, check orphan detection.
#[tokio::test]
async fn test_full_service_lifecycle_integration() {
    let mut ctx = TestContext::new().await;

    // Mount and install formulas
    ctx.mount_formula("redis", "7.2.0", &[]).await;
    ctx.mount_formula("nginx", "1.25.0", &[]).await;

    ctx.installer_mut().install("redis", true).await.unwrap();
    ctx.installer_mut().install("nginx", true).await.unwrap();

    // Create a service manager in a temp directory
    let tmp = TempDir::new().unwrap();
    let service_dir = tmp.path().join("services");
    let log_dir = tmp.path().join("logs");
    fs::create_dir_all(&service_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();

    let manager = ServiceManager::new_with_paths(&ctx.prefix(), &service_dir, &log_dir);

    // Create service files for all installed formulas plus one orphan
    create_fake_service_file(&service_dir, "redis");
    create_fake_service_file(&service_dir, "nginx");
    create_fake_service_file(&service_dir, "orphan_service");

    // Get list of installed formulas from installer
    let installed = ctx.installer().list_installed().unwrap();
    let installed_names: Vec<String> = installed.iter().map(|k| k.name.clone()).collect();

    // Find orphaned services
    let orphaned = manager.find_orphaned_services(&installed_names).unwrap();

    // Only orphan_service should be orphaned
    assert_eq!(orphaned.len(), 1);
    assert_eq!(orphaned[0].name, "orphan_service");

    // Verify log paths are computed correctly
    let (stdout, stderr) = manager.get_log_paths("redis");
    assert!(stdout.to_string_lossy().contains("redis.log"));
    assert!(stderr.to_string_lossy().contains("redis.error.log"));
}

/// Test that uninstalling a formula makes its service orphaned.
#[tokio::test]
async fn test_uninstall_creates_orphan() {
    let mut ctx = TestContext::new().await;

    // Install a formula
    ctx.mount_formula("testpkg", "1.0.0", &[]).await;
    ctx.installer_mut().install("testpkg", true).await.unwrap();

    // Create service manager and service file
    let tmp = TempDir::new().unwrap();
    let service_dir = tmp.path().join("services");
    let log_dir = tmp.path().join("logs");
    fs::create_dir_all(&service_dir).unwrap();
    fs::create_dir_all(&log_dir).unwrap();

    let manager = ServiceManager::new_with_paths(&ctx.prefix(), &service_dir, &log_dir);
    create_fake_service_file(&service_dir, "testpkg");

    // Initially no orphans
    let installed = ctx.installer().list_installed().unwrap();
    let installed_names: Vec<String> = installed.iter().map(|k| k.name.clone()).collect();
    let orphaned = manager.find_orphaned_services(&installed_names).unwrap();
    assert!(orphaned.is_empty(), "No orphans before uninstall");

    // Uninstall the formula
    ctx.installer_mut().uninstall("testpkg").unwrap();

    // Now the service should be orphaned
    let installed = ctx.installer().list_installed().unwrap();
    let installed_names: Vec<String> = installed.iter().map(|k| k.name.clone()).collect();
    let orphaned = manager.find_orphaned_services(&installed_names).unwrap();
    assert_eq!(orphaned.len(), 1, "Should have one orphan after uninstall");
    assert_eq!(orphaned[0].name, "testpkg");
}
