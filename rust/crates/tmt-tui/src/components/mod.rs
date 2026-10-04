//! Application-neutral components implementing the Full-screen interaction
//! contract in `design/cli-style.md`. Markup is admitted by `surface::compile`.
pub mod collection;
mod key_help;
mod list;
mod modal;
mod outline;
mod picker;
mod scroll;
pub mod strip;
pub mod surface;

pub use key_help::{KeyHelp, KeyHelpEntry, KeyHelpSection, KeyHint, footer};
pub use modal::{Modal, ModalAreas, Placement};
pub use outline::Outline;
pub use scroll::{ScrollState, Step};

pub use collection::Table;
pub use list::{ListEvent, ListFrame, ListRow, ListState, RowGeometry};
pub use picker::{MAX_QUERY_BYTES, Picker, PickerEvent, PickerField, PickerInput};
