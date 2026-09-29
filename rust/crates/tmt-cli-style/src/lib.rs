//! The one TMT command-line style: palette, marks, values, messages, lists,
//! detail views, tables and help. `docs/cli-style.md` owns the
//! rules; this crate is their only implementation.
//!
//! It depends on no TMT crate, so core and every extension CLI render through
//! the same code. Rendering never decides policy: callers pass a [`Terminal`]
//! that was decided once per stream.

pub mod audit;
pub mod detail;
pub mod help;
pub mod interaction;
pub mod list;
pub mod mark;
pub mod message;
pub mod palette;
pub mod stream;
pub mod table;
pub mod value;

pub use anstyle::{AnsiColor, Effects};
pub use help::{
    CommandSpec, Example, HelpSection, OutputModes, Route, ShownExample, apply, command,
    command_with_sections, examples, frame, help_text, route, version_arg,
};
pub use interaction::{Interaction, Mode};
pub use palette::{Terminal, Token};
