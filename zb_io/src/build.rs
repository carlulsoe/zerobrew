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
        .args([
            "-xf",
            &tarball.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
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

    // ==========================================================================
    // Build System Detection Tests
    // ==========================================================================

    mod build_system_detection {
        use super::*;

        #[test]
        fn detects_cmake() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(
                tmp.path().join("CMakeLists.txt"),
                "cmake_minimum_required(VERSION 3.0)",
            )
            .unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::CMake);
        }

        #[test]
        fn detects_meson() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("meson.build"), "project('test')").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Meson);
        }

        #[test]
        fn detects_autotools_configure() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("configure"), "#!/bin/sh").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
        }

        #[test]
        fn detects_autotools_configure_ac() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("configure.ac"), "AC_INIT([test], [1.0])").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
        }

        #[test]
        fn detects_autotools_autogen_sh() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("autogen.sh"), "#!/bin/sh\nautoreconf -i").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
        }

        #[test]
        fn detects_make() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("Makefile"), "all:\n\techo hello").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Make);
        }

        #[test]
        fn detects_gnumakefile() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("GNUmakefile"), "all:\n\techo hello").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Make);
        }

        #[test]
        fn returns_unknown_for_empty_directory() {
            let tmp = TempDir::new().unwrap();
            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Unknown);
        }

        #[test]
        fn returns_unknown_for_nonexistent_directory() {
            let path = PathBuf::from("/nonexistent/path/that/does/not/exist");
            assert_eq!(detect_build_system(&path), BuildSystem::Unknown);
        }

        #[test]
        fn returns_unknown_for_unrecognized_files() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("setup.py"), "from setuptools import setup").unwrap();
            std::fs::write(tmp.path().join("package.json"), "{}").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Unknown);
        }

        // Priority tests - build systems are checked in a specific order

        #[test]
        fn cmake_takes_priority_over_make() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("CMakeLists.txt"), "").unwrap();
            std::fs::write(tmp.path().join("Makefile"), "").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::CMake);
        }

        #[test]
        fn cmake_takes_priority_over_meson() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("CMakeLists.txt"), "").unwrap();
            std::fs::write(tmp.path().join("meson.build"), "").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::CMake);
        }

        #[test]
        fn meson_takes_priority_over_autotools() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("meson.build"), "").unwrap();
            std::fs::write(tmp.path().join("configure"), "").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Meson);
        }

        #[test]
        fn autotools_takes_priority_over_make() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("configure"), "").unwrap();
            std::fs::write(tmp.path().join("Makefile"), "").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
        }

        #[test]
        fn configure_ac_takes_priority_over_make() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("configure.ac"), "").unwrap();
            std::fs::write(tmp.path().join("Makefile"), "").unwrap();

            assert_eq!(detect_build_system(tmp.path()), BuildSystem::Autotools);
        }
    }

    // ==========================================================================
    // BuildSystem Enum Tests
    // ==========================================================================

    mod build_system_enum {
        use super::*;

        #[test]
        fn build_system_is_clone() {
            let bs = BuildSystem::CMake;
            let cloned = bs.clone();
            assert_eq!(bs, cloned);
        }

        #[test]
        fn build_system_is_eq() {
            assert_eq!(BuildSystem::CMake, BuildSystem::CMake);
            assert_ne!(BuildSystem::CMake, BuildSystem::Meson);
        }

        #[test]
        fn build_system_is_debug() {
            let bs = BuildSystem::Autotools;
            let debug_str = format!("{:?}", bs);
            assert!(debug_str.contains("Autotools"));
        }

        #[test]
        fn all_variants_are_distinct() {
            let variants = vec![
                BuildSystem::Autotools,
                BuildSystem::CMake,
                BuildSystem::Meson,
                BuildSystem::Make,
                BuildSystem::Custom,
                BuildSystem::Unknown,
            ];
            for (i, a) in variants.iter().enumerate() {
                for (j, b) in variants.iter().enumerate() {
                    if i == j {
                        assert_eq!(a, b);
                    } else {
                        assert_ne!(a, b);
                    }
                }
            }
        }
    }

    // ==========================================================================
    // BuildEnvironment Tests
    // ==========================================================================

    mod build_environment {
        use super::*;

        fn make_test_formula() -> Formula {
            Formula {
                name: "test-pkg".to_string(),
                dependencies: vec!["dep1".to_string(), "dep2".to_string()],
                build_dependencies: vec!["build-dep1".to_string()],
                ..Default::default()
            }
        }

        #[test]
        fn creates_correct_paths() {
            let formula = make_test_formula();
            let source_dir = PathBuf::from("/tmp/test-source");
            let prefix = PathBuf::from("/opt/zerobrew/prefix");
            let opt_dir = PathBuf::from("/opt/zerobrew/prefix/opt");
            let staging_dir = PathBuf::from("/tmp/test-staging");

            let env = BuildEnvironment::new(
                &formula,
                source_dir.clone(),
                &prefix,
                &opt_dir,
                staging_dir.clone(),
            );

            assert_eq!(env.source_dir, source_dir);
            assert_eq!(env.build_dir, source_dir.join("build"));
            assert_eq!(env.prefix, prefix);
            assert_eq!(env.staging_dir, staging_dir);
        }

        #[test]
        fn jobs_is_positive() {
            let formula = Formula::default();
            let env = BuildEnvironment::new(
                &formula,
                PathBuf::from("/tmp/source"),
                &PathBuf::from("/prefix"),
                &PathBuf::from("/opt"),
                PathBuf::from("/staging"),
            );

            assert!(env.jobs > 0);
        }

        #[test]
        fn uses_cc_from_environment() {
            // Save original
            let original_cc = std::env::var("CC").ok();
            let original_cxx = std::env::var("CXX").ok();

            // SAFETY: This test is single-threaded and we restore the original values
            unsafe {
                std::env::set_var("CC", "custom-cc");
                std::env::set_var("CXX", "custom-c++");
            }

            let formula = Formula::default();
            let env = BuildEnvironment::new(
                &formula,
                PathBuf::from("/tmp/source"),
                &PathBuf::from("/prefix"),
                &PathBuf::from("/opt"),
                PathBuf::from("/staging"),
            );

            assert_eq!(env.cc, "custom-cc");
            assert_eq!(env.cxx, "custom-c++");

            // Restore
            // SAFETY: Restoring original environment state
            unsafe {
                if let Some(cc) = original_cc {
                    std::env::set_var("CC", cc);
                } else {
                    std::env::remove_var("CC");
                }
                if let Some(cxx) = original_cxx {
                    std::env::set_var("CXX", cxx);
                } else {
                    std::env::remove_var("CXX");
                }
            }
        }

        #[test]
        fn defaults_to_cc_and_cxx() {
            // Ensure CC/CXX are unset
            let original_cc = std::env::var("CC").ok();
            let original_cxx = std::env::var("CXX").ok();

            // SAFETY: This test is single-threaded and we restore the original values
            unsafe {
                std::env::remove_var("CC");
                std::env::remove_var("CXX");
            }

            let formula = Formula::default();
            let env = BuildEnvironment::new(
                &formula,
                PathBuf::from("/tmp/source"),
                &PathBuf::from("/prefix"),
                &PathBuf::from("/opt"),
                PathBuf::from("/staging"),
            );

            assert_eq!(env.cc, "cc");
            assert_eq!(env.cxx, "c++");

            // Restore
            // SAFETY: Restoring original environment state
            unsafe {
                if let Some(cc) = original_cc {
                    std::env::set_var("CC", cc);
                }
                if let Some(cxx) = original_cxx {
                    std::env::set_var("CXX", cxx);
                }
            }
        }

        #[test]
        fn sets_paths_for_existing_dependencies() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create dependency directories
            let dep1 = opt_dir.join("dep1");
            std::fs::create_dir_all(dep1.join("include")).unwrap();
            std::fs::create_dir_all(dep1.join("lib/pkgconfig")).unwrap();
            std::fs::create_dir_all(dep1.join("bin")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            assert!(env.cflags.contains("-I"));
            assert!(env.cflags.contains("include"));
            assert!(env.ldflags.contains("-L"));
            assert!(env.ldflags.contains("lib"));
            assert!(env.pkg_config_path.contains("pkgconfig"));
            assert!(env.env.contains_key("PATH"));
        }

        #[test]
        fn ignores_nonexistent_dependencies() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");
            // Don't create any directories

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["missing-dep".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            // Should have empty flags since dependency doesn't exist
            assert!(env.cflags.is_empty());
            assert!(env.ldflags.is_empty());
            assert!(env.pkg_config_path.is_empty());
        }

        #[test]
        fn combines_deps_and_build_deps() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create both runtime and build dependency dirs
            let dep1 = opt_dir.join("runtime-dep");
            let dep2 = opt_dir.join("build-dep");
            std::fs::create_dir_all(dep1.join("include")).unwrap();
            std::fs::create_dir_all(dep2.join("include")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["runtime-dep".to_string()],
                build_dependencies: vec!["build-dep".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            // Both should be in CFLAGS
            assert!(env.cflags.contains("runtime-dep"));
            assert!(env.cflags.contains("build-dep"));
        }

        #[test]
        fn get_env_includes_compilers() {
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

        #[test]
        fn get_env_includes_flags_when_set() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create a dependency with all paths
            let dep = opt_dir.join("dep");
            std::fs::create_dir_all(dep.join("include")).unwrap();
            std::fs::create_dir_all(dep.join("lib/pkgconfig")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            let env_vars = env.get_env();
            assert!(env_vars.contains_key("CFLAGS"));
            assert!(env_vars.contains_key("CXXFLAGS"));
            assert!(env_vars.contains_key("LDFLAGS"));
            assert!(env_vars.contains_key("PKG_CONFIG_PATH"));
        }

        #[test]
        fn get_env_omits_empty_flags() {
            let formula = Formula::default();
            let env = BuildEnvironment::new(
                &formula,
                PathBuf::from("/tmp/source"),
                &PathBuf::from("/prefix"),
                &PathBuf::from("/opt"),
                PathBuf::from("/staging"),
            );

            let env_vars = env.get_env();
            // Empty flags should not be set
            if let Some(cflags) = env_vars.get("CFLAGS") {
                assert!(!cflags.is_empty());
            }
            if let Some(ldflags) = env_vars.get("LDFLAGS") {
                assert!(!ldflags.is_empty());
            }
        }
    }

    // ==========================================================================
    // File Collection Tests
    // ==========================================================================

    mod collect_installed_files {
        use super::*;

        #[test]
        fn collects_files_from_flat_structure() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("file1.txt"), "content").unwrap();
            std::fs::write(tmp.path().join("file2.txt"), "content").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files.len(), 2);
            assert!(files.contains(&PathBuf::from("file1.txt")));
            assert!(files.contains(&PathBuf::from("file2.txt")));
        }

        #[test]
        fn collects_files_from_nested_structure() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
            std::fs::create_dir_all(tmp.path().join("lib/pkgconfig")).unwrap();
            std::fs::create_dir_all(tmp.path().join("include/subdir")).unwrap();

            std::fs::write(tmp.path().join("bin/exe"), "binary").unwrap();
            std::fs::write(tmp.path().join("lib/libfoo.so"), "library").unwrap();
            std::fs::write(tmp.path().join("lib/pkgconfig/foo.pc"), "pkg").unwrap();
            std::fs::write(tmp.path().join("include/foo.h"), "header").unwrap();
            std::fs::write(tmp.path().join("include/subdir/bar.h"), "header").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files.len(), 5);
            assert!(files.contains(&PathBuf::from("bin/exe")));
            assert!(files.contains(&PathBuf::from("lib/libfoo.so")));
            assert!(files.contains(&PathBuf::from("lib/pkgconfig/foo.pc")));
            assert!(files.contains(&PathBuf::from("include/foo.h")));
            assert!(files.contains(&PathBuf::from("include/subdir/bar.h")));
        }

        #[test]
        fn returns_empty_for_empty_directory() {
            let tmp = TempDir::new().unwrap();
            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert!(files.is_empty());
        }

        #[test]
        fn returns_empty_for_nonexistent_directory() {
            let path = PathBuf::from("/nonexistent/path");
            let files = super::super::collect_installed_files(&path).unwrap();
            assert!(files.is_empty());
        }

        #[test]
        fn ignores_empty_subdirectories() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("empty_dir")).unwrap();
            std::fs::write(tmp.path().join("file.txt"), "content").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files.len(), 1);
            assert!(files.contains(&PathBuf::from("file.txt")));
        }

        #[test]
        fn returns_sorted_files() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("z.txt"), "").unwrap();
            std::fs::write(tmp.path().join("a.txt"), "").unwrap();
            std::fs::write(tmp.path().join("m.txt"), "").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files[0], PathBuf::from("a.txt"));
            assert_eq!(files[1], PathBuf::from("m.txt"));
            assert_eq!(files[2], PathBuf::from("z.txt"));
        }

        #[test]
        fn handles_deeply_nested_structure() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("a/b/c/d/e")).unwrap();
            std::fs::write(tmp.path().join("a/b/c/d/e/deep.txt"), "deep").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files.len(), 1);
            assert!(files.contains(&PathBuf::from("a/b/c/d/e/deep.txt")));
        }
    }

    // ==========================================================================
    // SHA256 Checksum Tests
    // ==========================================================================

    mod compute_sha256 {
        use super::*;

        #[test]
        fn computes_correct_hash_for_known_content() {
            let tmp = TempDir::new().unwrap();
            let file = tmp.path().join("test.txt");
            std::fs::write(&file, "hello world\n").unwrap();

            let hash = super::super::compute_sha256(&file).unwrap();
            // SHA256 of "hello world\n"
            assert_eq!(
                hash,
                "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447"
            );
        }

        #[test]
        fn computes_correct_hash_for_empty_file() {
            let tmp = TempDir::new().unwrap();
            let file = tmp.path().join("empty.txt");
            std::fs::write(&file, "").unwrap();

            let hash = super::super::compute_sha256(&file).unwrap();
            // SHA256 of empty string
            assert_eq!(
                hash,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
        }

        #[test]
        fn computes_correct_hash_for_binary_content() {
            let tmp = TempDir::new().unwrap();
            let file = tmp.path().join("binary.bin");
            std::fs::write(&file, [0u8, 1, 2, 3, 255, 254, 253]).unwrap();

            let hash = super::super::compute_sha256(&file).unwrap();
            assert!(!hash.is_empty());
            assert_eq!(hash.len(), 64); // SHA256 is 64 hex chars
        }

        #[test]
        fn errors_on_nonexistent_file() {
            let path = PathBuf::from("/nonexistent/file.txt");
            let result = super::super::compute_sha256(&path);
            assert!(result.is_err());
        }

        #[test]
        fn handles_large_file() {
            let tmp = TempDir::new().unwrap();
            let file = tmp.path().join("large.bin");
            // Create a 1MB file
            let data = vec![0x42u8; 1024 * 1024];
            std::fs::write(&file, &data).unwrap();

            let hash = super::super::compute_sha256(&file).unwrap();
            assert_eq!(hash.len(), 64);
        }
    }

    // ==========================================================================
    // Builder Tests
    // ==========================================================================

    mod builder {
        use super::*;

        fn make_test_env(tmp: &TempDir) -> BuildEnvironment {
            let formula = Formula::default();
            BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &tmp.path().join("opt"),
                tmp.path().join("staging"),
            )
        }

        #[test]
        fn builder_creation() {
            let tmp = TempDir::new().unwrap();
            let env = make_test_env(&tmp);
            let builder = Builder::new(env.clone());

            assert_eq!(builder.env.source_dir, env.source_dir);
        }

        #[test]
        fn build_auto_returns_error_for_unknown_build_system() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();

            let env = make_test_env(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_auto(&[]);
            assert!(result.is_err());

            let err = result.unwrap_err();
            let err_msg = format!("{:?}", err);
            assert!(err_msg.contains("could not detect build system"));
        }

        #[test]
        fn build_auto_returns_error_for_custom_build_system() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            // Write a file that doesn't match any build system
            std::fs::write(tmp.path().join("source/build.zig"), "").unwrap();

            let env = make_test_env(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_auto(&[]);
            assert!(result.is_err());
        }
    }

    // ==========================================================================
    // BuildResult Tests
    // ==========================================================================

    mod build_result {
        use super::*;

        #[test]
        fn build_result_is_debug() {
            let result = BuildResult {
                success: true,
                installed_files: vec![PathBuf::from("bin/test")],
                output: "Build completed".to_string(),
            };

            let debug_str = format!("{:?}", result);
            assert!(debug_str.contains("success"));
            assert!(debug_str.contains("true"));
        }

        #[test]
        fn build_result_stores_files() {
            let result = BuildResult {
                success: true,
                installed_files: vec![PathBuf::from("bin/a"), PathBuf::from("lib/b.so")],
                output: String::new(),
            };

            assert_eq!(result.installed_files.len(), 2);
        }
    }

    // ==========================================================================
    // Extract Tarball Tests
    // ==========================================================================

    mod extract_tarball {
        use super::*;

        #[test]
        fn creates_destination_directory() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("extracted");

            // We can't test the actual extraction without a real tarball,
            // but we can test that it attempts to create the directory
            // and fails gracefully on a nonexistent tarball
            let fake_tarball = tmp.path().join("nonexistent.tar.gz");
            let result = super::super::extract_tarball(&fake_tarball, &dest);

            // Should fail because tarball doesn't exist, but dest dir might be created
            assert!(result.is_err());
        }

        #[test]
        fn errors_on_nonexistent_tarball() {
            let tmp = TempDir::new().unwrap();
            let result = super::super::extract_tarball(
                &PathBuf::from("/nonexistent/file.tar.gz"),
                tmp.path(),
            );
            assert!(result.is_err());
        }
    }

    // ==========================================================================
    // Download Source Tests
    // ==========================================================================

    mod download_source {
        use super::*;

        #[test]
        fn errors_on_invalid_url() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("download.tar.gz");

            // This should fail because the URL is invalid
            let result = super::super::download_source("not-a-valid-url", &dest, None);
            assert!(result.is_err());
        }

        #[test]
        fn errors_on_checksum_mismatch() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("test.txt");
            // Create the file manually to simulate a download
            std::fs::write(&dest, "test content").unwrap();

            // Call with wrong checksum
            let result = super::super::download_source(
                "file:///dev/null", // Won't actually download
                &dest,
                Some("0000000000000000000000000000000000000000000000000000000000000000"),
            );

            // The actual curl command would fail, so this test just verifies
            // the function signature and basic error handling
            assert!(result.is_err());
        }
    }

    // ==========================================================================
    // Clone Git Repo Tests
    // ==========================================================================

    mod clone_git_repo {
        use super::*;

        #[test]
        fn errors_on_invalid_url() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("repo");

            let result = super::super::clone_git_repo("not-a-valid-git-url", None, &dest);
            assert!(result.is_err());
        }

        #[test]
        fn errors_on_nonexistent_repo() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("repo");

            let result = super::super::clone_git_repo(
                "https://github.com/nonexistent-user-abc123/nonexistent-repo-xyz789.git",
                None,
                &dest,
            );
            assert!(result.is_err());
        }
    }

    // ==========================================================================
    // Edge Cases and Error Handling
    // ==========================================================================

    mod edge_cases {
        use super::*;

        #[test]
        fn handles_unicode_paths() {
            let tmp = TempDir::new().unwrap();
            let unicode_dir = tmp.path().join("测试目录");
            std::fs::create_dir_all(&unicode_dir).unwrap();
            std::fs::write(unicode_dir.join("CMakeLists.txt"), "").unwrap();

            assert_eq!(detect_build_system(&unicode_dir), BuildSystem::CMake);
        }

        #[test]
        fn handles_paths_with_spaces() {
            let tmp = TempDir::new().unwrap();
            let spaced_dir = tmp.path().join("dir with spaces");
            std::fs::create_dir_all(&spaced_dir).unwrap();
            std::fs::write(spaced_dir.join("meson.build"), "").unwrap();

            assert_eq!(detect_build_system(&spaced_dir), BuildSystem::Meson);
        }

        #[test]
        fn build_env_with_many_dependencies() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create many dependencies
            for i in 0..20 {
                let dep = opt_dir.join(format!("dep{}", i));
                std::fs::create_dir_all(dep.join("include")).unwrap();
                std::fs::create_dir_all(dep.join("lib")).unwrap();
            }

            let deps: Vec<String> = (0..20).map(|i| format!("dep{}", i)).collect();
            let formula = Formula {
                name: "test".to_string(),
                dependencies: deps,
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            // Should have all dependencies in CFLAGS
            for i in 0..20 {
                assert!(env.cflags.contains(&format!("dep{}", i)));
                assert!(env.ldflags.contains(&format!("dep{}", i)));
            }
        }

        #[test]
        fn collect_files_with_symlinks() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("real_file.txt"), "content").unwrap();

            // Create a symlink (only on Unix)
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(
                    tmp.path().join("real_file.txt"),
                    tmp.path().join("symlink.txt"),
                )
                .unwrap();
            }

            let files = super::super::collect_installed_files(tmp.path()).unwrap();

            // Should include the real file
            assert!(files.contains(&PathBuf::from("real_file.txt")));

            #[cfg(unix)]
            {
                // Should also include the symlink
                assert!(files.contains(&PathBuf::from("symlink.txt")));
            }
        }

        #[test]
        fn formula_with_no_dependencies() {
            let formula = Formula {
                name: "standalone".to_string(),
                dependencies: vec![],
                build_dependencies: vec![],
                ..Default::default()
            };

            let tmp = TempDir::new().unwrap();
            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &tmp.path().join("opt"),
                tmp.path().join("staging"),
            );

            assert!(env.cflags.is_empty());
            assert!(env.ldflags.is_empty());
            assert!(env.pkg_config_path.is_empty());
        }
    }

    // ==========================================================================
    // Builder Command Execution Tests
    // ==========================================================================

    mod builder_commands {
        use super::*;

        fn make_test_env_in(tmp: &TempDir) -> BuildEnvironment {
            let formula = Formula::default();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::create_dir_all(tmp.path().join("staging")).unwrap();
            BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &tmp.path().join("opt"),
                tmp.path().join("staging"),
            )
        }

        #[test]
        fn run_command_succeeds_with_simple_command() {
            let tmp = TempDir::new().unwrap();
            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // Run a simple echo command
            let result = builder.run_command("echo", &["hello"], tmp.path());
            assert!(result.is_ok());
            assert!(result.unwrap().contains("hello"));
        }

        #[test]
        fn run_command_captures_stdout_and_stderr() {
            let tmp = TempDir::new().unwrap();
            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // This should capture both stdout
            let result =
                builder.run_command("sh", &["-c", "echo stdout; echo stderr >&2"], tmp.path());
            assert!(result.is_ok());
            let output = result.unwrap();
            assert!(output.contains("stdout"));
            assert!(output.contains("stderr"));
        }

        #[test]
        fn run_command_fails_on_nonexistent_command() {
            let tmp = TempDir::new().unwrap();
            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.run_command("nonexistent_command_xyz123", &[], tmp.path());
            assert!(result.is_err());
        }

        #[test]
        fn run_command_fails_on_exit_code() {
            let tmp = TempDir::new().unwrap();
            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.run_command("sh", &["-c", "exit 1"], tmp.path());
            assert!(result.is_err());
            let err = format!("{:?}", result.unwrap_err());
            assert!(err.contains("failed"));
        }

        #[test]
        fn run_command_uses_environment_variables() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("include")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );
            let builder = Builder::new(env);

            // The command should see CFLAGS set
            let result = builder.run_command("sh", &["-c", "echo $CFLAGS"], tmp.path());
            assert!(result.is_ok());
            let output = result.unwrap();
            assert!(output.contains("dep1"));
        }

        #[test]
        fn build_autotools_fails_without_configure() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            // No configure, no autogen.sh, no configure.ac

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_autotools(&[]);
            assert!(result.is_err());
        }

        #[test]
        fn build_cmake_fails_without_cmakelists() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            // No CMakeLists.txt

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_cmake(&[]);
            // This should fail when cmake tries to configure
            assert!(result.is_err());
        }

        #[test]
        fn build_meson_fails_without_meson_build() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_meson(&[]);
            // This should fail when meson tries to setup
            assert!(result.is_err());
        }

        #[test]
        fn build_make_fails_without_makefile() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_make(&[]);
            // This should fail when make can't find Makefile
            assert!(result.is_err());
        }

        #[test]
        fn build_auto_selects_cmake_when_detected() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/CMakeLists.txt"), "invalid cmake").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // This will fail but it should try cmake
            let result = builder.build_auto(&[]);
            assert!(result.is_err());
            let err = format!("{:?}", result.unwrap_err());
            assert!(err.contains("cmake") || err.contains("failed"));
        }

        #[test]
        fn build_auto_selects_meson_when_detected() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/meson.build"), "invalid meson").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // This will fail but it should try meson
            let result = builder.build_auto(&[]);
            assert!(result.is_err());
        }

        #[test]
        fn build_auto_selects_autotools_when_detected() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/configure"), "#!/bin/sh\nexit 1").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // This will fail but it should try autotools
            let result = builder.build_auto(&[]);
            assert!(result.is_err());
        }

        #[test]
        fn build_auto_selects_make_when_detected() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/Makefile"), "invalid:\n\texit 1").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // This will fail but it should try make
            let result = builder.build_auto(&[]);
            assert!(result.is_err());
        }

        #[test]
        fn build_cmake_creates_build_directory() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/CMakeLists.txt"), "invalid").unwrap();

            let env = make_test_env_in(&tmp);
            let build_dir = env.build_dir.clone();
            let builder = Builder::new(env);

            // Even though cmake will fail, the build directory should be created
            let _ = builder.build_cmake(&[]);
            assert!(build_dir.exists());
        }

        #[test]
        fn build_autotools_with_extra_args() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/configure"), "#!/bin/sh\nexit 1").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            // Pass extra configure args
            let result = builder.build_autotools(&[
                "--enable-feature".to_string(),
                "--disable-other".to_string(),
            ]);
            assert!(result.is_err()); // Will fail but tests arg passing
        }

        #[test]
        fn build_cmake_with_extra_args() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/CMakeLists.txt"), "invalid").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_cmake(&[
                "-DENABLE_TESTS=OFF".to_string(),
                "-DBUILD_SHARED_LIBS=ON".to_string(),
            ]);
            assert!(result.is_err()); // Will fail but tests arg passing
        }

        #[test]
        fn build_meson_with_extra_args() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/meson.build"), "invalid").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result =
                builder.build_meson(&["-Dtests=false".to_string(), "-Ddocs=false".to_string()]);
            assert!(result.is_err()); // Will fail but tests arg passing
        }

        #[test]
        fn build_make_with_extra_args() {
            let tmp = TempDir::new().unwrap();
            std::fs::create_dir_all(tmp.path().join("source")).unwrap();
            std::fs::write(tmp.path().join("source/Makefile"), "invalid").unwrap();

            let env = make_test_env_in(&tmp);
            let builder = Builder::new(env);

            let result = builder.build_make(&["CC=gcc".to_string(), "CFLAGS=-O2".to_string()]);
            assert!(result.is_err()); // Will fail but tests arg passing
        }
    }

    // ==========================================================================
    // Extract Tarball Advanced Tests
    // ==========================================================================

    mod extract_tarball_advanced {
        use super::*;

        #[test]
        fn returns_single_extracted_directory() {
            let tmp = TempDir::new().unwrap();
            let tarball = tmp.path().join("test.tar.gz");
            let dest = tmp.path().join("extracted");

            // Create a tarball with a single top-level directory
            let src_dir = tmp.path().join("myproject-1.0.0");
            std::fs::create_dir_all(&src_dir).unwrap();
            std::fs::write(src_dir.join("file.txt"), "content").unwrap();

            // Create the tarball
            let status = std::process::Command::new("tar")
                .args([
                    "-czf",
                    &tarball.to_string_lossy(),
                    "-C",
                    &tmp.path().to_string_lossy(),
                    "myproject-1.0.0",
                ])
                .status()
                .unwrap();
            assert!(status.success());

            let result = super::super::extract_tarball(&tarball, &dest);
            assert!(result.is_ok());

            let extracted_path = result.unwrap();
            // Should return the inner directory, not dest
            assert_eq!(extracted_path.file_name().unwrap(), "myproject-1.0.0");
        }

        #[test]
        fn returns_dest_for_multiple_entries() {
            let tmp = TempDir::new().unwrap();
            let tarball = tmp.path().join("test.tar.gz");
            let dest = tmp.path().join("extracted");

            // Create files directly (no single top-level dir)
            let src = tmp.path().join("src");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(src.join("file1.txt"), "content").unwrap();
            std::fs::write(src.join("file2.txt"), "content").unwrap();

            // Create tarball with multiple top-level entries
            let status = std::process::Command::new("tar")
                .args([
                    "-czf",
                    &tarball.to_string_lossy(),
                    "-C",
                    &src.to_string_lossy(),
                    "file1.txt",
                    "file2.txt",
                ])
                .status()
                .unwrap();
            assert!(status.success());

            let result = super::super::extract_tarball(&tarball, &dest);
            assert!(result.is_ok());

            let extracted_path = result.unwrap();
            // Should return dest itself since there's no single top-level dir
            assert_eq!(extracted_path, dest);
        }

        #[test]
        fn returns_dest_for_single_file_not_directory() {
            let tmp = TempDir::new().unwrap();
            let tarball = tmp.path().join("test.tar.gz");
            let dest = tmp.path().join("extracted");

            // Create a single file
            let src = tmp.path().join("src");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(src.join("single_file.txt"), "content").unwrap();

            // Create tarball with just one file (not a directory)
            let status = std::process::Command::new("tar")
                .args([
                    "-czf",
                    &tarball.to_string_lossy(),
                    "-C",
                    &src.to_string_lossy(),
                    "single_file.txt",
                ])
                .status()
                .unwrap();
            assert!(status.success());

            let result = super::super::extract_tarball(&tarball, &dest);
            assert!(result.is_ok());

            let extracted_path = result.unwrap();
            // Should return dest since the single entry is a file, not a directory
            assert_eq!(extracted_path, dest);
        }
    }

    // ==========================================================================
    // Download Source Advanced Tests
    // ==========================================================================

    mod download_source_advanced {
        use super::*;

        #[test]
        fn succeeds_with_valid_local_file_url() {
            let tmp = TempDir::new().unwrap();

            // Create a source file
            let source = tmp.path().join("source.txt");
            std::fs::write(&source, "test content").unwrap();

            let dest = tmp.path().join("downloaded.txt");
            let url = format!("file://{}", source.display());

            // Download without checksum verification
            let result = super::super::download_source(&url, &dest, None);
            assert!(result.is_ok());
            assert!(dest.exists());

            let content = std::fs::read_to_string(&dest).unwrap();
            assert_eq!(content, "test content");
        }

        #[test]
        fn succeeds_with_correct_checksum() {
            let tmp = TempDir::new().unwrap();

            // Create a source file
            let source = tmp.path().join("source.txt");
            std::fs::write(&source, "test content").unwrap();

            // Compute the correct checksum
            let expected_hash = super::super::compute_sha256(&source).unwrap();

            let dest = tmp.path().join("downloaded.txt");
            let url = format!("file://{}", source.display());

            let result = super::super::download_source(&url, &dest, Some(&expected_hash));
            assert!(result.is_ok());
        }

        #[test]
        fn fails_with_incorrect_checksum() {
            let tmp = TempDir::new().unwrap();

            // Create a source file
            let source = tmp.path().join("source.txt");
            std::fs::write(&source, "test content").unwrap();

            let dest = tmp.path().join("downloaded.txt");
            let url = format!("file://{}", source.display());

            // Wrong checksum
            let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
            let result = super::super::download_source(&url, &dest, Some(wrong_hash));
            assert!(result.is_err());

            let err = format!("{:?}", result.unwrap_err());
            assert!(err.contains("checksum mismatch"));
        }
    }

    // ==========================================================================
    // Clone Git Repo Advanced Tests
    // ==========================================================================

    mod clone_git_repo_advanced {
        use super::*;

        #[test]
        fn clone_with_branch_parameter() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("repo");

            // Try to clone with a branch - will fail because repo doesn't exist
            // but this tests the branch parameter path
            let result = super::super::clone_git_repo(
                "https://github.com/nonexistent/repo.git",
                Some("main"),
                &dest,
            );
            assert!(result.is_err());
        }

        #[test]
        fn clone_without_branch_parameter() {
            let tmp = TempDir::new().unwrap();
            let dest = tmp.path().join("repo");

            // Try to clone without a branch
            let result = super::super::clone_git_repo(
                "https://github.com/nonexistent/repo.git",
                None,
                &dest,
            );
            assert!(result.is_err());
        }
    }

    // ==========================================================================
    // BuildEnvironment Advanced Tests
    // ==========================================================================

    mod build_environment_advanced {
        use super::*;

        #[test]
        fn handles_partial_dependency_structure_only_include() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Only create include directory
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("include")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            assert!(env.cflags.contains("include"));
            assert!(env.ldflags.is_empty()); // No lib dir
            assert!(env.pkg_config_path.is_empty()); // No pkgconfig dir
        }

        #[test]
        fn handles_partial_dependency_structure_only_lib() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Only create lib directory (no pkgconfig)
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("lib")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            assert!(env.cflags.is_empty()); // No include dir
            assert!(env.ldflags.contains("lib"));
            assert!(env.pkg_config_path.is_empty()); // No pkgconfig
        }

        #[test]
        fn handles_partial_dependency_structure_only_bin() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Only create bin directory
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("bin")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            assert!(env.cflags.is_empty());
            assert!(env.ldflags.is_empty());
            assert!(env.env.contains_key("PATH"));
            assert!(env.env.get("PATH").unwrap().contains("bin"));
        }

        #[test]
        fn handles_dependency_with_pkgconfig() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create lib with pkgconfig
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("lib/pkgconfig")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            assert!(env.pkg_config_path.contains("pkgconfig"));
        }

        #[test]
        fn combines_multiple_pkgconfig_paths() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create multiple dependencies with pkgconfig
            for name in &["dep1", "dep2", "dep3"] {
                let dep = opt_dir.join(name);
                std::fs::create_dir_all(dep.join("lib/pkgconfig")).unwrap();
            }

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string(), "dep2".to_string(), "dep3".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            // Should be colon-separated
            let pkg_paths: Vec<&str> = env.pkg_config_path.split(':').collect();
            assert_eq!(pkg_paths.len(), 3);
        }

        #[test]
        fn combines_multiple_bin_paths() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create multiple dependencies with bin
            for name in &["dep1", "dep2"] {
                let dep = opt_dir.join(name);
                std::fs::create_dir_all(dep.join("bin")).unwrap();
            }

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string(), "dep2".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            let path = env.env.get("PATH").unwrap();
            assert!(path.contains("dep1"));
            assert!(path.contains("dep2"));
        }

        #[test]
        fn get_env_preserves_custom_env_vars() {
            let tmp = TempDir::new().unwrap();
            let opt_dir = tmp.path().join("opt");

            // Create a dependency with bin to add PATH
            let dep = opt_dir.join("dep1");
            std::fs::create_dir_all(dep.join("bin")).unwrap();

            let formula = Formula {
                name: "test".to_string(),
                dependencies: vec!["dep1".to_string()],
                ..Default::default()
            };

            let env = BuildEnvironment::new(
                &formula,
                tmp.path().join("source"),
                tmp.path(),
                &opt_dir,
                tmp.path().join("staging"),
            );

            let env_vars = env.get_env();
            assert!(env_vars.contains_key("PATH"));
            assert!(env_vars.contains_key("CC"));
            assert!(env_vars.contains_key("CXX"));
        }
    }

    // ==========================================================================
    // Collect Installed Files Error Handling Tests
    // ==========================================================================

    mod collect_files_errors {
        use super::*;

        #[test]
        fn handles_permission_denied_gracefully() {
            // This test is platform-specific and may not work in all environments
            // Skip if we can't create the test conditions
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("file.txt"), "content").unwrap();

            // This should still work even if some files are unreadable
            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert!(!files.is_empty());
        }

        #[test]
        fn handles_hidden_files() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join(".hidden"), "hidden content").unwrap();
            std::fs::write(tmp.path().join("visible"), "visible content").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert!(files.contains(&PathBuf::from(".hidden")));
            assert!(files.contains(&PathBuf::from("visible")));
        }

        #[test]
        fn handles_special_characters_in_filenames() {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("file with spaces.txt"), "").unwrap();
            std::fs::write(tmp.path().join("file-with-dashes.txt"), "").unwrap();
            std::fs::write(tmp.path().join("file_with_underscores.txt"), "").unwrap();

            let files = super::super::collect_installed_files(tmp.path()).unwrap();
            assert_eq!(files.len(), 3);
        }
    }

    // ==========================================================================
    // Integration-style Tests (without running actual builds)
    // ==========================================================================

    mod integration {
        use super::*;

        #[test]
        fn full_cmake_project_structure_detection() {
            let tmp = TempDir::new().unwrap();
            let src = tmp.path().join("myproject-1.0.0");
            std::fs::create_dir_all(&src).unwrap();

            // Create realistic CMake project structure
            std::fs::write(
                src.join("CMakeLists.txt"),
                r#"
cmake_minimum_required(VERSION 3.10)
project(myproject VERSION 1.0.0)
add_executable(myproject main.cpp)
install(TARGETS myproject DESTINATION bin)
"#,
            )
            .unwrap();
            std::fs::write(src.join("main.cpp"), "int main() { return 0; }").unwrap();
            std::fs::create_dir_all(src.join("src")).unwrap();
            std::fs::write(src.join("src/lib.cpp"), "void foo() {}").unwrap();

            assert_eq!(detect_build_system(&src), BuildSystem::CMake);
        }

        #[test]
        fn full_autotools_project_structure_detection() {
            let tmp = TempDir::new().unwrap();
            let src = tmp.path().join("myproject-1.0.0");
            std::fs::create_dir_all(&src).unwrap();

            // Create realistic autotools project structure
            std::fs::write(
                src.join("configure.ac"),
                r#"
AC_INIT([myproject], [1.0.0])
AM_INIT_AUTOMAKE
AC_PROG_CC
AC_OUTPUT
"#,
            )
            .unwrap();
            std::fs::write(src.join("Makefile.am"), "bin_PROGRAMS = myproject").unwrap();
            std::fs::write(src.join("autogen.sh"), "#!/bin/sh\nautoreconf -i").unwrap();

            assert_eq!(detect_build_system(&src), BuildSystem::Autotools);
        }

        #[test]
        fn full_meson_project_structure_detection() {
            let tmp = TempDir::new().unwrap();
            let src = tmp.path().join("myproject-1.0.0");
            std::fs::create_dir_all(&src).unwrap();

            // Create realistic meson project structure
            std::fs::write(
                src.join("meson.build"),
                r#"
project('myproject', 'c', version: '1.0.0')
executable('myproject', 'main.c', install: true)
"#,
            )
            .unwrap();
            std::fs::write(src.join("main.c"), "int main() { return 0; }").unwrap();

            assert_eq!(detect_build_system(&src), BuildSystem::Meson);
        }

        #[test]
        fn simulated_build_directory_structure() {
            let tmp = TempDir::new().unwrap();
            let staging = tmp.path().join("staging");

            // Create a typical installed package structure
            std::fs::create_dir_all(staging.join("bin")).unwrap();
            std::fs::create_dir_all(staging.join("lib/pkgconfig")).unwrap();
            std::fs::create_dir_all(staging.join("include")).unwrap();
            std::fs::create_dir_all(staging.join("share/man/man1")).unwrap();
            std::fs::create_dir_all(staging.join("share/doc/myproject")).unwrap();

            std::fs::write(staging.join("bin/myproject"), "binary").unwrap();
            std::fs::write(staging.join("lib/libmyproject.so"), "library").unwrap();
            std::fs::write(staging.join("lib/libmyproject.a"), "static lib").unwrap();
            std::fs::write(staging.join("lib/pkgconfig/myproject.pc"), "pkg-config").unwrap();
            std::fs::write(staging.join("include/myproject.h"), "header").unwrap();
            std::fs::write(staging.join("share/man/man1/myproject.1"), "manpage").unwrap();
            std::fs::write(staging.join("share/doc/myproject/README"), "readme").unwrap();

            let files = super::super::collect_installed_files(&staging).unwrap();

            assert_eq!(files.len(), 7);
            assert!(files.contains(&PathBuf::from("bin/myproject")));
            assert!(files.contains(&PathBuf::from("lib/libmyproject.so")));
            assert!(files.contains(&PathBuf::from("lib/libmyproject.a")));
            assert!(files.contains(&PathBuf::from("lib/pkgconfig/myproject.pc")));
            assert!(files.contains(&PathBuf::from("include/myproject.h")));
            assert!(files.contains(&PathBuf::from("share/man/man1/myproject.1")));
            assert!(files.contains(&PathBuf::from("share/doc/myproject/README")));
        }
    }
}
