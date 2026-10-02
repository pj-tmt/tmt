//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
pub mod core;
pub mod http;
pub mod keyring;
pub mod limits;
pub mod store;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
