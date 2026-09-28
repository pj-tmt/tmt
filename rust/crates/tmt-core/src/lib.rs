//! Shared native-domain contracts for tmux-team.

pub mod binding;
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
pub mod office_art_reference;
pub mod office_block;
pub mod office_board;
pub mod office_extension;
pub mod office_map;
pub mod office_profile;
pub mod office_protocol;
pub mod office_whiteboard;
pub mod office_world;
pub mod operation;
pub mod profile;
pub mod request;
pub mod retention;
pub mod room;
pub mod settings;
pub mod skill_provider;

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod office_block_tests;
#[cfg(test)]
mod settings_tests;
