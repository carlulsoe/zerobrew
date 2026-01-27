# Testing Async I/O Functions in Zerobrew

## Executive Summary

This document proposes strategies for adding tests to the remaining untested async functions in zerobrew. The codebase already has strong testing infrastructure (83%+ coverage in `zb_io`), and the remaining untested code is primarily CLI orchestration that combines multiple I/O dependencies.

**Key Recommendation:** Use the existing `TestContext` infrastructure for integration-style testing rather than heavy architectural refactoring. The ROI for trait-based mocking of `Installer`/`ServiceManager` is low compared to the existing patterns.

---

## 1. Current Testing Infrastructure

### Existing Patterns

| Pattern | Location | Usage |
|---------|----------|-------|
| **wiremock** | `zb_io/src/install/tests.rs`, `zb_io/src/api.rs` | HTTP mocking for API calls |
| **mockall** | `zb_io/src/traits.rs` | `MockHttpClient`, `MockFileSystem` |
| **TestContext** | `zb_io/src/test_utils.rs` | Full integration test setup |
| **Pure function extraction** | `zb_cli/src/commands/*.rs` | Testable helpers |
| **tempfile** | Throughout | Isolated filesystem tests |

### Test Statistics (from codebase review)
- `zb_io/src/install/tests.rs`: ~1500+ lines, 40+ async integration tests
- `zb_io/src/traits.rs`: ~300+ lines of mock tests
- `zb_cli/src/commands/info.rs`: 70+ pure function unit tests
- `zb_cli/src/commands/services/control.rs`: 300+ unit tests for helpers

### The `TestContext` Pattern (Already Implemented!)

```rust
// From zb_io/src/test_utils.rs
pub struct TestContext {
    pub tmp: TempDir,
    pub mock_server: MockServer,
    installer: Installer,
}

impl TestContext {
    pub async fn new() -> Self { ... }
    pub async fn mount_formula(&self, name: &str, version: &str, deps: &[&str]) -> String { ... }
    pub fn installer_mut(&mut self) -> &mut Installer { ... }
}
```

This is the **recommended pattern** for testing CLI commands.

---

## 2. Analysis of Untested Code

### A. `zb_cli/src/commands/info.rs`

**Functions needing tests:**
- `run_list(installer: &Installer, pinned: bool)` — Sync, uses DB
- `run_info(installer: &mut Installer, prefix: &Path, formula: String, json: bool)` — Async, API + DB
- `run_search(installer: &Installer, root: &Path, query: String, json: bool, installed: bool)` — Async, API

**Current coverage:** Pure helper functions are 100% covered (70+ tests)

**Gap:** The `run_*` functions that orchestrate I/O are untested

### B. `zb_cli/src/commands/services/control.rs`

**Functions needing tests:**
- `run_start(installer, service_manager, prefix, formula)`
- `run_stop(service_manager, formula)`
- `run_restart(service_manager, formula)`
- `run_enable(service_manager, formula)`
- `run_disable(service_manager, formula)`
- `run_foreground(installer, service_manager, prefix, formula)`
- `run_log(service_manager, formula, lines, follow)`
- `run_cleanup(installer, service_manager, dry_run)`

**Current coverage:** Pure helper functions are 100% covered (300+ tests)

**Gap:** Orchestration functions that interact with systemd/launchctl

### C. `zb_cli/src/main.rs`

**Functions needing tests:**
- `run()` — Main CLI orchestration
- `run_uninstall()`, `run_gc()`, `run_autoremove()`, etc.

**Current coverage:** CLI argument parsing tests exist (50+ tests)

**Gap:** End-to-end command execution

---

## 3. Mocking Strategies

### Strategy A: Use Existing `TestContext` (Recommended)

**Effort:** Low  
**Coverage gain:** Medium-High  
**Risk:** Low

The `TestContext` already sets up a complete `Installer` with wiremock:

```rust
// Example: Testing run_info
#[tokio::test]
async fn test_run_info_installed_package() {
    let ctx = TestContext::new().await;
    
    // Install a package first
    ctx.mount_formula("wget", "1.24.5", &[]).await;
    ctx.installer_mut().install("wget", true).await.unwrap();
    
    // Capture stdout for verification
    let mut output = Vec::new();
    // Note: Would need to refactor run_info to accept a writer
    
    // For now, verify the underlying data is correct
    let keg = ctx.installer().get_installed("wget");
    assert!(keg.is_some());
    assert_eq!(keg.unwrap().version, "1.24.5");
}
```

**Pros:**
- Already implemented and battle-tested
- No architectural changes needed
- Realistic integration testing

**Cons:**
- Tests are slower (real HTTP mocking)
- Can't test specific edge cases in isolation

### Strategy B: Trait-Based Dependency Injection

**Effort:** High  
**Coverage gain:** High  
**Risk:** Medium (breaking changes)

This would require refactoring `Installer` and `ServiceManager`:

```rust
// Current (struct with concrete types)
pub struct Installer {
    api_client: ApiClient,
    db: Database,
    // ...
}

// Proposed (trait objects)
pub struct Installer<A: ApiClientTrait, D: DatabaseTrait> {
    api_client: A,
    db: D,
    // ...
}

// Or using dynamic dispatch
pub struct Installer {
    api_client: Box<dyn ApiClientTrait>,
    db: Box<dyn DatabaseTrait>,
}
```

**Required traits:**
```rust
#[async_trait]
pub trait ApiClientTrait: Send + Sync {
    async fn get_formula(&self, name: &str) -> Result<Formula, Error>;
    async fn get_all_formulas(&self) -> Result<Vec<FormulaInfo>, Error>;
}

pub trait DatabaseTrait: Send + Sync {
    fn get_installed(&self, name: &str) -> Option<InstalledKeg>;
    fn list_installed(&self) -> Result<Vec<InstalledKeg>, Error>;
    // ...20+ more methods
}

pub trait ServiceManagerTrait: Send + Sync {
    fn get_status(&self, formula: &str) -> Result<ServiceStatus, Error>;
    fn start(&self, formula: &str) -> Result<(), Error>;
    // ...10+ more methods
}
```

**Pros:**
- Fine-grained mocking for edge cases
- Faster unit tests
- Better isolation

**Cons:**
- Significant refactoring effort (10+ files)
- Risk of introducing bugs
- Adds complexity for maintenance

### Strategy C: System Command Mocking (For ServiceManager)

**Effort:** Medium  
**Coverage gain:** Medium  
**Risk:** Low

For `ServiceManager`, the challenge is mocking `systemctl`/`launchctl` commands:

```rust
// Option 1: Extract command execution to trait
pub trait CommandExecutor {
    fn execute(&self, cmd: &str, args: &[&str]) -> std::io::Result<Output>;
}

// Option 2: Use environment variable to switch behavior
impl ServiceManager {
    fn run_systemctl(&self, args: &[&str]) -> Result<Output, Error> {
        if cfg!(test) && std::env::var("ZB_MOCK_SYSTEMCTL").is_ok() {
            return self.mock_systemctl(args);
        }
        Command::new("systemctl").args(args).output()
    }
}

// Option 3: Create fake systemd unit files in test tempdir
// (The ServiceManager already uses paths from $HOME)
```

### Strategy D: Output Capture Testing

**Effort:** Low  
**Coverage gain:** Low-Medium  
**Risk:** Low

Refactor `run_*` functions to accept writers:

```rust
// Current
pub fn run_list(installer: &Installer, pinned: bool) -> Result<(), Error> {
    println!("...");
}

// Proposed
pub fn run_list<W: Write>(
    installer: &Installer, 
    pinned: bool,
    writer: &mut W
) -> Result<(), Error> {
    writeln!(writer, "...")?;
}

// Test
#[test]
fn test_run_list_empty() {
    let mut output = Vec::new();
    run_list(&mock_installer, false, &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("No formulas installed"));
}
```

---

## 4. Recommended Implementation Plan

### Phase 1: Low-Hanging Fruit (1-2 days)

1. **Add integration tests using `TestContext`** for:
   - `run_info` (basic case, JSON output, not found)
   - `run_search` (with results, empty, installed filter)
   - `run_list` (empty, with packages, pinned filter)

2. **Test file:** `zb_cli/tests/integration_info.rs`

```rust
use zb_io::test_utils::TestContext;

#[tokio::test]
async fn test_info_installed_formula() {
    let mut ctx = TestContext::new().await;
    ctx.mount_formula("ripgrep", "14.1.0", &[]).await;
    ctx.installer_mut().install("ripgrep", true).await.unwrap();
    
    // Verify run_info preconditions
    assert!(ctx.installer().is_installed("ripgrep"));
    let keg = ctx.installer().get_installed("ripgrep").unwrap();
    assert_eq!(keg.version, "14.1.0");
}

#[tokio::test]
async fn test_search_returns_results() {
    let ctx = TestContext::new().await;
    // Mount a formula list endpoint...
}
```

### Phase 2: ServiceManager Testing (2-3 days)

1. **Add filesystem-based testing** for service files:
   - Create service files in tempdir
   - Verify file content generation
   - Test orphan detection

2. **Skip actual systemctl/launchctl calls** in tests:
   - Check `cfg!(test)` flag
   - Or use `#[cfg(test)]` mock implementations

```rust
// In services.rs
impl ServiceManager {
    #[cfg(test)]
    pub fn new_test(prefix: &Path, service_dir: &Path, log_dir: &Path) -> Self {
        Self {
            prefix: prefix.to_path_buf(),
            service_dir: service_dir.to_path_buf(),
            log_dir: log_dir.to_path_buf(),
        }
    }
}

// In tests
#[test]
fn test_service_file_generation() {
    let tmp = TempDir::new().unwrap();
    let manager = ServiceManager::new_test(
        tmp.path().join("prefix").as_ref(),
        tmp.path().join("services").as_ref(),
        tmp.path().join("logs").as_ref(),
    );
    
    let config = ServiceConfig {
        program: PathBuf::from("/usr/bin/redis-server"),
        ..Default::default()
    };
    
    manager.create_service("redis", &config).unwrap();
    
    // Verify file was created with correct content
    let content = fs::read_to_string(manager.service_file_path("redis")).unwrap();
    assert!(content.contains("ExecStart=/usr/bin/redis-server"));
}
```

### Phase 3: Output Refactoring (Optional, 1-2 days)

If higher coverage is needed:
1. Add `Write` trait bounds to `run_*` functions
2. Create test doubles that capture output
3. Assert on output strings

---

## 5. Effort Estimate & ROI

| Approach | Effort | Coverage Δ | Recommended |
|----------|--------|------------|-------------|
| TestContext integration tests | 1-2 days | +10-15% | ✅ Yes |
| ServiceManager filesystem tests | 2-3 days | +5-10% | ✅ Yes |
| Full trait-based DI refactor | 5-7 days | +15-20% | ❌ Not now |
| Output capture refactoring | 1-2 days | +5% | 🔶 Maybe |

**Recommended total effort:** 3-5 days for meaningful coverage gain

**Coverage target:** From current ~83% to ~93%

---

## 6. Code Examples

### Example 1: Info Command Integration Test

```rust
// zb_cli/tests/integration_info.rs
use zb_io::test_utils::TestContext;

mod info_tests {
    use super::*;

    #[tokio::test]
    async fn test_list_empty() {
        let ctx = TestContext::new().await;
        let installed = ctx.installer().list_installed().unwrap();
        assert!(installed.is_empty());
    }

    #[tokio::test]
    async fn test_list_with_packages() {
        let mut ctx = TestContext::new().await;
        ctx.mount_formula("git", "2.44.0", &[]).await;
        ctx.mount_formula("curl", "8.6.0", &["openssl"]).await;
        
        ctx.installer_mut().install("git", true).await.unwrap();
        ctx.installer_mut().install("curl", true).await.unwrap();
        
        let installed = ctx.installer().list_installed().unwrap();
        assert_eq!(installed.len(), 3); // git, curl, openssl
    }

    #[tokio::test]
    async fn test_info_not_installed() {
        let ctx = TestContext::new().await;
        let keg = ctx.installer().get_installed("nonexistent");
        assert!(keg.is_none());
    }
}
```

### Example 2: ServiceManager Test with Tempdir

```rust
// zb_io/src/services.rs (add to existing tests module)
#[cfg(test)]
mod service_integration_tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_manager() -> (ServiceManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let prefix = tmp.path().join("prefix");
        let service_dir = tmp.path().join("services");
        let log_dir = tmp.path().join("logs");
        
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::create_dir_all(&service_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();
        
        // Create a ServiceManager that uses our test directories
        let mut manager = ServiceManager::new(&prefix);
        // Override service_dir and log_dir for testing
        manager.service_dir = service_dir;
        manager.log_dir = log_dir;
        
        (manager, tmp)
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_generate_systemd_service_file() {
        let (manager, _tmp) = create_test_manager();
        
        let config = ServiceConfig {
            program: PathBuf::from("/opt/zerobrew/prefix/opt/redis/bin/redis-server"),
            args: vec!["--port".to_string(), "6379".to_string()],
            working_directory: Some(PathBuf::from("/var/lib/redis")),
            restart_on_failure: true,
            run_at_load: true,
            ..Default::default()
        };
        
        let content = manager.generate_service_file("redis", &config);
        
        // Verify systemd unit file structure
        assert!(content.contains("[Unit]"));
        assert!(content.contains("[Service]"));
        assert!(content.contains("[Install]"));
        assert!(content.contains("ExecStart=/opt/zerobrew/prefix/opt/redis/bin/redis-server --port 6379"));
        assert!(content.contains("WorkingDirectory=/var/lib/redis"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("WantedBy=default.target"));
    }

    #[test]
    fn test_list_empty_service_dir() {
        let (manager, _tmp) = create_test_manager();
        let services = manager.list().unwrap();
        assert!(services.is_empty());
    }

    #[test]
    fn test_get_log_paths() {
        let (manager, _tmp) = create_test_manager();
        let (stdout, stderr) = manager.get_log_paths("redis");
        
        assert!(stdout.to_string_lossy().contains("redis.log"));
        assert!(stderr.to_string_lossy().contains("redis.error.log"));
    }
}
```

---

## 7. Files to Modify

| File | Changes |
|------|---------|
| `zb_io/src/services.rs` | Add test helpers, make fields accessible for testing |
| `zb_cli/tests/integration_info.rs` | New file for info command tests |
| `zb_cli/tests/integration_services.rs` | New file for services tests |
| `zb_io/Cargo.toml` | Possibly add `tempfile` to dev-deps (already there) |

---

## 8. Conclusion

The zerobrew codebase already has excellent testing infrastructure. The recommended approach is:

1. **Use `TestContext`** for integration-style testing of async commands
2. **Add filesystem-based tests** for `ServiceManager` without mocking systemd
3. **Avoid heavy refactoring** — the pure function extraction pattern is working well
4. **Target ~93% coverage** with 3-5 days of effort

The existing pattern of extracting pure functions for unit tests + integration tests with `TestContext` is the most pragmatic approach for this codebase.
