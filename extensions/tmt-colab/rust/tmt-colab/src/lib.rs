//! Local-build colab persistence slice: opaque ciphertext, never decoded Yjs.
pub mod keyring;
pub mod limits;
pub mod store;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
