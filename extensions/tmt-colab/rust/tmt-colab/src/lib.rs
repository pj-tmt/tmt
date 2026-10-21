//! Local-build colab pilot. The server stores ciphertext and never decodes Yjs.
mod app_inventory;
pub mod ask;
pub mod assets;
pub mod attachments;
mod browser_policy;
mod chrome;
pub mod closed;
pub mod control;
pub mod core;
pub mod decoder;
pub mod deploy_declaration;
pub mod discussion;
pub mod edit_request;
pub mod export;
pub mod fold;
pub mod hosting;
pub mod inspection;
mod ipc;
pub mod keyring;
pub mod limits;
pub mod management;
mod object_channel;
pub use object_channel::{Client as SealedClient, Error as ChannelError};
pub mod page;
pub mod page_check;
pub mod page_expiry;
pub mod projects;
pub mod publication;
pub mod readers;
pub mod registration;
mod rekeyer;
pub use object_channel::cloud_sync;
pub use object_channel::documents as sealed_documents;
pub mod serve_release;
pub mod settings;
pub mod shell;
pub mod short_links;
pub mod socket;
pub mod store;
pub mod summary;
pub mod summary_lines;
pub mod summary_passages;
mod summary_pending;
pub mod summary_readings;
pub mod sync;
pub mod threads;
pub mod transitions;
pub mod wip;
pub mod work;
pub mod work_github;
pub mod work_index;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Reuse the integration-test decoder budget in owner-local unit fixtures.
#[cfg(test)]
extern crate self as tmt_colab;
#[cfg(test)]
mod fold_vectors;
