//! Services command implementations.
//!
//! This module handles service management for installed formulas:
//! - Listing and inspecting services
//! - Starting, stopping, and restarting services
//! - Enabling/disabling auto-start at login
//! - Viewing logs and running in foreground

mod control;
mod list;

use std::path::Path;

use zb_io::install::Installer;
use zb_io::ServiceManager;

use crate::ServicesAction;

// Re-export submodule functions for use in dispatch
pub use control::{
    run_cleanup, run_disable, run_enable, run_foreground, run_log, run_restart, run_start,
    run_stop,
};
pub use list::{run_info, run_list};

/// Run the services command.
pub fn run(
    installer: &mut Installer,
    prefix: &Path,
    action: Option<ServicesAction>,
) -> Result<(), zb_core::Error> {
    let service_manager = ServiceManager::new(prefix);

    match action {
        None | Some(ServicesAction::List { json: false }) => run_list(&service_manager, false),
        Some(ServicesAction::List { json: true }) => run_list(&service_manager, true),
        Some(ServicesAction::Start { formula }) => {
            run_start(installer, &service_manager, prefix, &formula)
        }
        Some(ServicesAction::Stop { formula }) => run_stop(&service_manager, &formula),
        Some(ServicesAction::Restart { formula }) => run_restart(&service_manager, &formula),
        Some(ServicesAction::Enable { formula }) => run_enable(&service_manager, &formula),
        Some(ServicesAction::Disable { formula }) => run_disable(&service_manager, &formula),
        Some(ServicesAction::Run { formula }) => {
            run_foreground(installer, &service_manager, prefix, &formula)
        }
        Some(ServicesAction::Info { formula }) => run_info(&service_manager, &formula),
        Some(ServicesAction::Log {
            formula,
            lines,
            follow,
        }) => run_log(&service_manager, &formula, lines, follow),
        Some(ServicesAction::Cleanup { dry_run }) => {
            run_cleanup(installer, &service_manager, dry_run)
        }
    }
}
