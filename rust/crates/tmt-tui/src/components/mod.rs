//! Application-neutral components implementing the Full-screen interaction
//! contract in `design/cli-style.md`. Markup is admitted by `surface::compile`.
mod key_help;
mod modal;
mod scroll;
pub mod surface;

pub use key_help::{KeyHelp, KeyHelpEntry, KeyHelpSection, KeyHint, footer};
pub use modal::{Modal, ModalAreas, Placement};
pub use scroll::{ScrollState, Step};
