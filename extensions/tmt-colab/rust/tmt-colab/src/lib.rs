//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
pub mod core;
pub mod decoder;
pub mod fold;
pub mod keyring;
pub mod limits;
pub mod registration;
pub mod socket;
pub mod store;
pub mod sync;
pub mod transitions;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
