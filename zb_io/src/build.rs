//! Source build support for Zerobrew.
//!
//! This module provides the ability to build packages from source when bottles
//! are not available. It supports common build systems:
//! - configure/make (autotools)
//! - cmake
//! - meson/ninja
//!
//! The build flow is:
//! 1. Download source tarball
//! 2. Verify checksum
//! 3. Extract to build directory
//! 4. Set up environment (compilers, paths)
//! 5. Run build commands
//! 6. Capture installed files
//! 7. Move to store/cellar

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use zb_core::{Error, Formula};

/// Build system type detected from source
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildSystem {
    /// Autotools (configure && make)
    Autotools,
    /// CMake
    CMake,
    /// Meson + Ninja
    Meson,
    /// Simple Makefile (no configure)
    Make,
    /// Custom (requires explicit commands)
    Custom,
    /// Unknown - cannot auto-detect
    Unknown,
}

/// Build environment for compiling from source
#[derive(Debug, Clone)]
pub struct BuildEnvironment {
    /// Source directory (extracted tarball)
    pub source_dir: PathBuf,
    /// Build directory (where compilation happens)
    pub build_dir: PathBuf,
    /// Installation prefix (where files are installed)
    pub prefix: PathBuf,
    /// Staging directory (temporary install location)
    pub staging_dir: PathBuf,

    /// C compiler
    pub cc: String,
    /// C++ compiler
    pub cxx: String,
    /// C compiler flags
    pub cflags: String,
    /// C++ compiler flags
    pub cxxflags: String,
    /// Linker flags
    pub ldflags: String,
    /// pkg-config search path
    pub pkg_config_path: String,

    /// Additional environment variables
    pub env: HashMap<String, String>,

    /// Number of parallel jobs for make
    pub jobs: usize,
}

impl BuildEnvironment {
    /// Create a new build environment for a formula
    pub fn new(
        formula: &Formula,
        source_dir: PathBuf,
        prefix: &Path,
        opt_dir: &Path,
        staging_dir: PathBuf,
    ) -> Self {
        let build_dir = source_dir.join("build");

        // Determine compiler
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".to_string());

        // Build paths for dependencies
        let mut include_paths = Vec::new();
        let mut lib_paths = Vec::new();
        let mut pkg_config_paths = Vec::new();
        let mut bin_paths = Vec::new();

        // Add all dependencies to the build environment
        let all_deps: Vec<&str> = formula
            .dependencies
            .iter()
            .chain(formula.build_dependencies.iter())
            .map(|s| s.as_str())
            .collect();

        for dep in all_deps {
            let dep_opt = opt_dir.join(dep);
            if dep_opt.exists() {
                let dep_include = dep_opt.join("include");
                let dep_lib = dep_opt.join("lib");
                let dep_pkgconfig = dep_lib.join("pkgconfig");
                let dep_bin = dep_opt.join("bin");

                if dep_include.exists() {
                    include_paths.push(format!("-I{}", dep_include.display()));
                }
                if dep_lib.exists() {
                    lib_paths.push(format!("-L{}", dep_lib.display()));
                }
                if dep_pkgconfig.exists() {
                    pkg_config_paths.push(dep_pkgconfig.to_string_lossy().to_string());
                }
                if dep_bin.exists() {
                    bin_paths.push(dep_bin.to_string_lossy().to_string());
                }
            }
        }

        // Build CFLAGS, LDFLAGS, PKG_CONFIG_PATH
        let cflags = include_paths.join(" ");
        let cxxflags = cflags.clone();
        let ldflags = lib_paths.join(" ");
        let pkg_config_path = pkg_config_paths.join(":");

        // Build PATH
        let mut env = HashMap::new();
        if !bin_paths.is_empty() {
            let existing_path = std::env::var("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", bin_paths.join(":"), existing_path);
            env.insert("PATH".to_string(), new_path);
        }

        // Determine number of parallel jobs
        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        Self {
            source_dir,
            build_dir,
            prefix: prefix.to_path_buf(),
            staging_dir,
            cc,
            cxx,
            cflags,
            cxxflags,
            ldflags,
            pkg_config_path,
            env,
            jobs,
        }
    }

    /// Get the environment variables for the build
    pub fn get_env(&self) -> HashMap<String, String> {
        let mut env = self.env.clone();
        env.insert("CC".to_string(), self.cc.clone());
        env.insert("CXX".to_string(), self.cxx.clone());

        if !self.cflags.is_empty() {
            env.insert("CFLAGS".to_string(), self.cflags.clone());
        }
        if !self.cxxflags.is_empty() {
            env.insert("CXXFLAGS".to_string(), self.cxxflags.clone());
        }
        if !self.ldflags.is_empty() {
            env.insert("LDFLAGS".to_string(), self.ldflags.clone());
        }
        if !self.pkg_config_path.is_empty() {
            env.insert("PKG_CONFIG_PATH".to_string(), self.pkg_config_path.clone());
        }

        env
    }
}

/// Detect the build system from source directory contents
pub fn detect_build_system(source_dir: &Path) -> BuildSystem {
    // Check for CMakeLists.txt
    if source_dir.join("CMakeLists.txt").exists() {
        return BuildSystem::CMake;
    }

    // Check for meson.build
    if source_dir.join("meson.build").exists() {
        return BuildSystem::Meson;
    }

    // Check for configure script
    if source_dir.join("configure").exists() {
        return BuildSystem::Autotools;
    }

    // Check for autogen.sh or configure.ac (needs autoreconf)
    if source_dir.join("autogen.sh").exists() || source_dir.join("configure.ac").exists() {
        return BuildSystem::Autotools;
    }

    // Check for Makefile
    if source_dir.join("Makefile").exists() || source_dir.join("GNUmakefile").exists() {
        return BuildSystem::Make;
    }

    BuildSystem::Unknown
}

/// Result of a build operation
#[derive(Debug)]
pub struct BuildResult {
    /// Whether the build succeeded
    pub success: bool,
    /// List of installed files (relative to staging_dir)
    pub installed_files: Vec<PathBuf>,
    /// Build output (stdout + stderr)
    pub output: String,
}

/// Builder that executes build commands
pub struct Builder {
    env: BuildEnvironment,
}

impl Builder {
    /// Create a new builder with the given environment
    pub fn new(env: BuildEnvironment) -> Self {
        Self { env }
    }

    /// Run a command in the build environment
    fn run_command(&self, cmd: &str, args: &[&str], work_dir: &Path) -> Result<String, Error> {
        let mut command = Command::new(cmd);
        command.args(args);
        command.current_dir(work_dir);

        // Set environment
        for (key, value) in self.env.get_env() {
            command.env(&key, &value);
        }

        let output = command.output().map_err(|e| Error::StoreCorruption {
            message: format!("failed to run {}: {}", cmd, e),
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        if !output.status.success() {
            return Err(Error::StoreCorruption {
                message: format!(
                    "command '{}' failed with exit code {}: {}",
                    cmd,
                    output.status.code().unwrap_or(-1),
                    combined
                ),
            });
        }

        Ok(combined)
    }

    /// Build using autotools (configure && make)
    pub fn build_autotools(&self, configure_args: &[String]) -> Result<BuildResult, Error> {
        let source_dir = &self.env.source_dir;
        let staging_dir = &self.env.staging_dir;

        // Check if we need to run autoreconf
        if !source_dir.join("configure").exists() {
            if source_dir.join("autogen.sh").exists() {
                self.run_command("./autogen.sh", &[], source_dir)?;
            } else if source_dir.join("configure.ac").exists() {
                self.run_command("autoreconf", &["-fiv"], source_dir)?;
            }
        }

        // Run configure
        let mut args = vec![format!("--prefix={}", staging_dir.display())];
        args.extend(configure_args.iter().cloned());
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let mut output = String::new();
        output.push_str(&self.run_command("./configure", &args_refs, source_dir)?);

        // Run make
        let jobs_arg = format!("-j{}", self.env.jobs);
        output.push_str(&self.run_command("make", &[&jobs_arg], source_dir)?);

        // Run make install
        output.push_str(&self.run_command("make", &["install"], source_dir)?);

        // Collect installed files
        let installed_files = collect_installed_files(staging_dir)?;

        Ok(BuildResult {
            success: true,
            installed_files,
            output,
        })
    }

    /// Build using cmake
    pub fn build_cmake(&self, cmake_args: &[String]) -> Result<BuildResult, Error> {
        let source_dir = &self.env.source_dir;
        let build_dir = &self.env.build_dir;
        let staging_dir = &self.env.staging_dir;

        // Create build directory
        std::fs::create_dir_all(build_dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to create build directory: {}", e),
        })?;

        // Run cmake configure
        let mut args = vec![
            format!("-S{}", source_dir.display()),
            format!("-B{}", build_dir.display()),
            format!("-DCMAKE_INSTALL_PREFIX={}", staging_dir.display()),
            "-DCMAKE_BUILD_TYPE=Release".to_string(),
        ];
        args.extend(cmake_args.iter().cloned());
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let mut output = String::new();
        output.push_str(&self.run_command("cmake", &args_refs, source_dir)?);

        // Run cmake build
        let jobs_arg = format!("-j{}", self.env.jobs);
        output.push_str(&self.run_command(
            "cmake",
            &["--build", &build_dir.to_string_lossy(), &jobs_arg],
            source_dir,
        )?);

        // Run cmake install
        output.push_str(&self.run_command(
            "cmake",
            &["--install", &build_dir.to_string_lossy()],
            source_dir,
        )?);

        // Collect installed files
        let installed_files = collect_installed_files(staging_dir)?;

        Ok(BuildResult {
            success: true,
            installed_files,
            output,
        })
    }

    /// Build using meson
    pub fn build_meson(&self, meson_args: &[String]) -> Result<BuildResult, Error> {
        let source_dir = &self.env.source_dir;
        let build_dir = &self.env.build_dir;
        let staging_dir = &self.env.staging_dir;

        // Run meson setup
        let mut args = vec![
            "setup".to_string(),
            build_dir.to_string_lossy().to_string(),
            format!("--prefix={}", staging_dir.display()),
            "--buildtype=release".to_string(),
        ];
        args.extend(meson_args.iter().cloned());
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let mut output = String::new();
        output.push_str(&self.run_command("meson", &args_refs, source_dir)?);

        // Run ninja
        let jobs_arg = format!("-j{}", self.env.jobs);
        output.push_str(&self.run_command(
            "ninja",
            &["-C", &build_dir.to_string_lossy(), &jobs_arg],
            source_dir,
        )?);

        // Run ninja install
        output.push_str(&self.run_command(
            "ninja",
            &["-C", &build_dir.to_string_lossy(), "install"],
            source_dir,
        )?);

        // Collect installed files
        let installed_files = collect_installed_files(staging_dir)?;

        Ok(BuildResult {
            success: true,
            installed_files,
            output,
        })
    }

    /// Build using plain make (no configure)
    pub fn build_make(&self, make_args: &[String]) -> Result<BuildResult, Error> {
        let source_dir = &self.env.source_dir;
        let staging_dir = &self.env.staging_dir;

        // Run make
        let jobs_arg = format!("-j{}", self.env.jobs);
        let prefix_arg = format!("PREFIX={}", staging_dir.display());
        let mut args = vec![jobs_arg.as_str(), prefix_arg.as_str()];
        let make_args_refs: Vec<&str> = make_args.iter().map(|s| s.as_str()).collect();
        args.extend(make_args_refs);

        let mut output = String::new();
        output.push_str(&self.run_command("make", &args, source_dir)?);

        // Run make install
        let install_args = vec!["install", prefix_arg.as_str()];
        output.push_str(&self.run_command("make", &install_args, source_dir)?);

        // Collect installed files
        let installed_files = collect_installed_files(staging_dir)?;

        Ok(BuildResult {
            success: true,
            installed_files,
            output,
        })
    }

    /// Auto-detect build system and build
    pub fn build_auto(&self, extra_args: &[String]) -> Result<BuildResult, Error> {
        let build_system = detect_build_system(&self.env.source_dir);

        match build_system {
            BuildSystem::CMake => self.build_cmake(extra_args),
            BuildSystem::Meson => self.build_meson(extra_args),
            BuildSystem::Autotools => self.build_autotools(extra_args),
            BuildSystem::Make => self.build_make(extra_args),
            BuildSystem::Custom | BuildSystem::Unknown => Err(Error::StoreCorruption {
                message: format!(
                    "could not detect build system for {}",
                    self.env.source_dir.display()
                ),
            }),
        }
    }
}

/// Collect all files installed to the staging directory
fn collect_installed_files(staging_dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut files = Vec::new();

    fn walk_dir(dir: &Path, base: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
        if !dir.exists() {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read directory {}: {}", dir.display(), e),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_dir(&path, base, files)?;
            } else if let Ok(rel_path) = path.strip_prefix(base) {
                files.push(rel_path.to_path_buf());
            }
        }

        Ok(())
    }

    walk_dir(staging_dir, staging_dir, &mut files)?;
    files.sort();
    Ok(files)
}

/// Download a source tarball and verify its checksum
pub fn download_source(url: &str, dest: &Path, expected_sha256: Option<&str>) -> Result<(), Error> {
    // Use curl to download
    let output = Command::new("curl")
        .args(["-fsSL", "-o", &dest.to_string_lossy(), url])
        .output()
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to download {}: {}", url, e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::StoreCorruption {
            message: format!("failed to download {}: {}", url, stderr),
        });
    }

    // Verify checksum if provided
    if let Some(expected) = expected_sha256 {
        let actual = compute_sha256(dest)?;
        if actual != expected {
            return Err(Error::StoreCorruption {
                message: format!(
                    "checksum mismatch for {}: expected {}, got {}",
                    dest.display(),
                    expected,
                    actual
                ),
            });
        }
    }

    Ok(())
}

/// Compute SHA256 hash of a file
fn compute_sha256(path: &Path) -> Result<String, Error> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| Error::StoreCorruption {
        message: format!("failed to open {}: {}", path.display(), e),
    })?;

    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|e| Error::StoreCorruption {
            message: format!("failed to read {}: {}", path.display(), e),
        })?;

        if bytes_read == 0 {
            break;
        }

        use sha2::Digest;
        hasher.update(&buffer[..bytes_read]);
    }

    use sha2::Digest;
    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

/// Clone a git repository
pub fn clone_git_repo(url: &str, branch: Option<&str>, dest: &Path) -> Result<(), Error> {
    let dest_str = dest.to_string_lossy().to_string();
    let mut args = vec!["clone", "--depth", "1"];

    if let Some(b) = branch {
        args.push("-b");
        args.push(b);
    }

    args.push(url);
    args.push(&dest_str);

    let output = Command::new("git")
        .args(&args)
        .output()
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to clone {}: {}", url, e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::StoreCorruption {
            message: format!("failed to clone {}: {}", url, stderr),
        });
    }

    Ok(())
}

/// Extract a tarball to a directory
pub fn extract_tarball(tarball: &Path, dest: &Path) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dest).map_err(|e| Error::StoreCorruption {
        message: format!("failed to create directory {}: {}", dest.display(), e),
    })?;

    // Use tar to extract
    let output = Command::new("tar")
        .args(["-xf", &tarball.to_string_lossy(), "-C", &dest.to_string_lossy()])
        .output()
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to extract {}: {}", tarball.display(), e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::StoreCorruption {
            message: format!("failed to extract {}: {}", tarball.display(), stderr),
        });
    }

    // Find the extracted directory (usually there's one top-level dir)
    let entries: Vec<_> = std::fs::read_dir(dest)
        .map_err(|e| Error::StoreCorruption {
            message: format!("failed to read directory {}: {}", dest.display(), e),
        })?
        .filter_map(|e| e.ok())
        .collect();

    if entries.len() == 1 && entries[0].path().is_dir() {
        // Single directory extracted - return it
        Ok(entries[0].path())
    } else {
        // Multiple entries or files - return dest itself
        Ok(dest.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_build_system_cmake() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("CMakeLists.txt"), "cmake_minimum_required(VERSION 3.0)").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::CMake);
    }

    #[test]
    fn test_detect_build_system_meson() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("meson.build"), "project('test')").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::Meson);
    }

    #[test]
    fn test_detect_build_system_autotools() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("configure"), "#!/bin/sh").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
    }

    #[test]
    fn test_detect_build_system_autotools_configure_ac() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("configure.ac"), "AC_INIT([test], [1.0])").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
    }

    #[test]
    fn test_detect_build_system_make() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Makefile"), "all:").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::Make);
    }

    #[test]
    fn test_detect_build_system_unknown() {
        let tmp = TempDir::new().unwrap();
        // Empty directory - no build system detected
        assert_eq!(detect_build_system(tmp.path()), BuildSystem::Unknown);
    }

    #[test]
    fn test_detect_build_system_priority_cmake_over_make() {
        // CMake should take priority over Makefile
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("CMakeLists.txt"), "").unwrap();
        std::fs::write(tmp.path().join("Makefile"), "").unwrap();

        assert_eq!(detect_build_system(tmp.path()), BuildSystem::CMake);
    }

    #[test]
    fn test_collect_installed_files() {
        let tmp = TempDir::new().unwrap();

        // Create a directory structure
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
        std::fs::write(tmp.path().join("bin/foo"), "binary").unwrap();
        std::fs::write(tmp.path().join("lib/libfoo.so"), "library").unwrap();

        let files = collect_installed_files(tmp.path()).unwrap();

        assert!(files.contains(&PathBuf::from("bin/foo")));
        assert!(files.contains(&PathBuf::from("lib/libfoo.so")));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_build_environment_creation() {
        let formula = Formula {
            name: "test".to_string(),
            dependencies: vec!["dep1".to_string()],
            build_dependencies: vec!["dep2".to_string()],
            ..Default::default()
        };

        let source_dir = PathBuf::from("/tmp/test-source");
        let prefix = PathBuf::from("/opt/zerobrew/prefix");
        let opt_dir = PathBuf::from("/opt/zerobrew/prefix/opt");
        let staging_dir = PathBuf::from("/tmp/test-staging");

        let env = BuildEnvironment::new(&formula, source_dir.clone(), &prefix, &opt_dir, staging_dir.clone());

        assert_eq!(env.source_dir, source_dir);
        assert_eq!(env.build_dir, source_dir.join("build"));
        assert_eq!(env.prefix, prefix);
        assert_eq!(env.staging_dir, staging_dir);
        assert!(env.jobs > 0);
    }

    #[test]
    fn test_build_environment_get_env() {
        let formula = Formula::default();
        let env = BuildEnvironment::new(
            &formula,
            PathBuf::from("/tmp/source"),
            &PathBuf::from("/prefix"),
            &PathBuf::from("/opt"),
            PathBuf::from("/staging"),
        );

        let env_vars = env.get_env();
        assert!(env_vars.contains_key("CC"));
        assert!(env_vars.contains_key("CXX"));
    }
}
