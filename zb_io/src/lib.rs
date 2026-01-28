// Allow various clippy lints for pre-existing code patterns
#![allow(clippy::collapsible_if)]
#![allow(clippy::unnecessary_map_or)]
#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::iter_skip_next)]
#![allow(clippy::useless_vec)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::nonminimal_bool)]
#![allow(unused_variables)]
#![allow(unused_imports)]

//! I/O layer for zerobrew - a fast Homebrew-compatible package manager.
//!
//! This crate provides the core I/O and orchestration functionality:
//!
//! - [`Installer`] - Core installation/uninstallation orchestration
//! - [`ApiClient`] - Homebrew API access with caching
//! - [`Database`] - Local SQLite state storage for installed packages
//! - [`Store`] - Content-addressable blob store for package data
//! - [`Downloader`] / [`ParallelDownloader`] - HTTP download handling
//! - [`Linker`] - Symlink management for installed formulas
//! - [`Cellar`] - Package materialization from the store
//! - [`ServiceManager`] - Background service lifecycle management
//! - [`TapManager`] - Third-party tap repository management
//! - [`Builder`] - Source compilation support
//! - [`traits`] - Trait abstractions for mockable I/O operations

pub mod api;
pub mod blob;
pub mod build;
pub mod bundle;
pub mod cache;
pub mod db;
pub mod download;
pub mod extract;
pub mod install;
pub mod link;
pub mod materialize;
#[cfg(target_os = "linux")]
pub mod patchelf;
pub mod progress;
pub mod search;
pub mod services;
pub mod store;
pub mod tap;
pub mod traits;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use api::{ApiClient, FormulaInfo};
pub use blob::BlobCache;
pub use build::{BuildEnvironment, BuildResult, BuildSystem, Builder, detect_build_system};
pub use bundle::{BrewfileEntry, BundleCheckResult, BundleInstallResult};
pub use cache::ApiCache;
pub use db::{Database, InstalledKeg, InstalledTap};
pub use download::{DownloadProgressCallback, DownloadRequest, Downloader, ParallelDownloader};
pub use extract::extract_tarball;
pub use install::{
    CleanupResult, DepsTree, DoctorCheck, DoctorResult, DoctorStatus, Installer, LinkResult,
    SourceBuildResult, UpgradeResult,
};
pub use link::Linker;
pub use materialize::Cellar;
pub use progress::{InstallProgress, ProgressCallback};
pub use services::{ServiceConfig, ServiceInfo, ServiceManager, ServiceStatus};
pub use store::Store;
pub use tap::{TapFormula, TapInfo, TapManager};
pub use traits::{FileSystem, HttpClient, ReqwestHttpClient, StdFileSystem};
