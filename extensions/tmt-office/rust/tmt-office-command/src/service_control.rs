//! The local Office service's lifecycle for core's uninstall: whether it is
//! running, and stopping it through the same path as `tmt office stop`.

use std::io;
use tmt_adapters::config::ConfigPaths;
use tmt_office_service::{self as office_service, ServiceError};

fn error(error: ServiceError) -> io::Error {
    io::Error::other(error.to_string())
}

/// Whether a healthy service is recorded. An unconfirmable process is an
/// error, so a caller never removes files under a live service by guessing.
pub fn running(paths: &ConfigPaths) -> io::Result<bool> {
    // The installed version only decides whether a restart is needed.
    office_service::status(paths, "")
        .map(|status| status.running)
        .map_err(error)
}

/// Stops the service; `true` when it was running.
pub fn stop(paths: &ConfigPaths) -> io::Result<bool> {
    office_service::stop(paths).map_err(error)
}
