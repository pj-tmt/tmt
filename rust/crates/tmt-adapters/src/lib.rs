//! Concrete native adapters. Application policy must not depend on this crate.

#[cfg(unix)]
pub mod api;

#[cfg(unix)]
pub mod bounded_file;
pub mod config;
#[cfg(unix)]
pub mod core_executable;
#[cfg(unix)]
pub mod delivery;
pub mod dispatch;
#[cfg(unix)]
pub mod drivers;
pub mod executable_trust;
pub mod extension_command;
pub mod extension_hooks;
#[cfg(unix)]
pub mod file_lock;
#[cfg(unix)]
#[cfg(unix)]
pub mod host;
pub mod identity_projection;
pub mod identity_status;
#[cfg(unix)]
pub mod interrupt;
mod json_document;
#[cfg(unix)]
pub mod native_install;
#[cfg(unix)]
pub mod notes;
#[cfg(unix)]
pub mod pane_badge;
pub mod private_file;
#[cfg(unix)]
pub mod process;
#[cfg(unix)]
mod release_http;
#[cfg(unix)]
pub mod reply_notice;
pub mod reply_receipt;
#[cfg(unix)]
pub mod repository_remote;
pub mod request_history;
pub mod request_runtime;
#[cfg(unix)]
pub mod response_input;
pub mod room;
#[cfg(unix)]
pub mod runtime;
#[cfg(unix)]
#[cfg(unix)]
pub mod setup;
#[cfg(unix)]
pub mod skill_installation;
pub mod storage;
#[cfg(unix)]
pub mod tmux;

#[cfg(all(test, unix))]
mod scripted_runner;
#[cfg(test)]
mod test_support;

#[cfg(all(test, unix))]
mod process_tests;
