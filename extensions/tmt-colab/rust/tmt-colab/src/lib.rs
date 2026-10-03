//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
mod app_inventory;
pub mod assets;
pub mod core;
pub mod decoder;
pub mod export;
pub mod fold;
pub mod inspection;
pub mod keyring;
pub mod limits;
pub mod management;
pub mod page;

pub mod readers;
pub mod registration;
pub mod socket;
pub mod store;
pub mod sync;
pub mod transitions;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
