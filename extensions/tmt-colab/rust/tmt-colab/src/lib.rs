//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
mod app_inventory;
pub mod ask;
pub mod assets;
pub mod attachments;
mod chrome;
pub mod control;
pub mod core;
pub mod decoder;
pub mod discussion;
pub mod export;
pub mod fold;
pub mod inspection;
mod ipc;
pub mod keyring;
pub mod limits;
pub mod management;
mod object_channel;
pub mod page;
pub mod publication;
pub mod readers;
pub mod registration;
pub mod serve_release;
pub mod settings;
pub mod short_links;
pub mod socket;
pub mod store;
pub mod sync;
pub mod threads;
pub mod transitions;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Reuse the integration-test decoder budget in owner-local unit fixtures.
#[cfg(test)]
extern crate self as tmt_colab;
