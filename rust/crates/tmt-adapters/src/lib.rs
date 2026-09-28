//! Concrete native adapters. Application policy must not depend on this crate.

#[cfg(unix)]
pub mod api;

#[cfg(unix)]
pub mod bounded_file;
pub mod config;
#[cfg(unix)]
pub mod delivery;
pub mod dispatch;
#[cfg(unix)]
pub mod extension_command;
pub mod extension_hooks;
#[cfg(unix)]
pub mod file_lock;
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
pub mod office_avatar;
#[cfg(unix)]
pub mod office_block;
pub mod office_board;
#[cfg(unix)]
pub mod office_companion;
#[cfg(feature = "office")]
pub mod office_deployment;
#[cfg(feature = "office")]
mod office_http;
#[cfg(feature = "office")]
pub mod office_pairing;
#[cfg(unix)]
pub mod office_profile;
#[cfg(unix)]
pub mod office_prop;
#[cfg(unix)]
pub mod office_service;
pub mod office_whiteboard;
#[cfg(unix)]
pub mod office_world;
#[cfg(unix)]
pub mod pane_badge;
#[cfg(unix)]
pub mod process;
#[cfg(unix)]
mod release_http;
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
pub mod runtime_caller;
#[cfg(unix)]
pub mod setup;
#[cfg(unix)]
pub mod skill_installation;
pub mod storage;
#[cfg(unix)]
pub mod tmux;

#[cfg(test)]
mod test_support;

#[cfg(all(test, unix))]
mod process_tests;
