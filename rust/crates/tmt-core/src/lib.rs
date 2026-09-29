//! Shared native-domain contracts for tmux-team.

pub mod binding;
pub mod content_digest;
pub mod dispatch;
pub mod driver;
pub mod endpoint;
pub mod exact_text;
pub mod extension_command;
pub mod identity;
pub mod identity_hooks;
pub mod identity_metadata;
pub mod identity_status;
pub mod limits;
pub mod names;
pub mod native_install;
pub mod operation;
pub mod profile;
pub mod repository_id;
pub mod request;
pub mod retention;
pub mod room;
pub mod settings;

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod settings_tests;
