//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
mod app_inventory;
pub mod ask;
pub mod assets;
mod chrome;
pub mod control;
pub mod core;
pub mod decoder;
pub mod export;
pub mod fold;
pub mod inspection;
mod ipc;
pub mod keyring;
pub mod limits;
pub mod management;
pub mod page;
pub mod readers;
pub mod registration;
pub mod settings;
pub mod short_links;
pub mod socket;
pub mod store;
pub mod sync;
mod threads;
pub mod transitions;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Reuse the integration-test decoder budget in owner-local unit fixtures.
#[cfg(test)]
extern crate self as tmt_colab;
