//! The Terminal tab: a built-in-host member's live terminal in the Talk pane. The wire is
//! `contracts/terminal-stream-v1.md`; the frames are `tmt-pty-protocol`'s and the screen is
//! `tmt-tui`'s terminal model, so nothing here parses an escape sequence.
//!
//! [`discovery`] asks core where the stream is, [`stream`] is the thread that reads it, [`tab`]
//! holds the state, [`paint`] draws it and `board` ties it to the board loop.
#![expect(
    dead_code,
    reason = "typing (Stream::input, resize and stop_typing) is wired by the next slice of #4606"
)]

mod board;
pub(in crate::board) mod discovery;
pub(in crate::board) mod paint;
pub(in crate::board) mod stream;
pub(in crate::board) mod tab;
#[cfg(test)]
mod tests;
