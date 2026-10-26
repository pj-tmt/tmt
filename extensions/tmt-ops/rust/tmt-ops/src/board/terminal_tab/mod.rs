//! The Terminal tab's model: where a built-in-host member's stream is and the thread that reads
//! it. The tab itself, its keys and its painting come after this.
//!
//! The wire is `contracts/terminal-stream-v1.md`; the frames are `tmt-pty-protocol`'s and the
//! screen is `tmt-tui`'s terminal model, so nothing here parses an escape sequence.
#![expect(
    dead_code,
    reason = "the Terminal tab is wired to this by the next slice of #4606; until then only tests reach it"
)]

pub(in crate::board) mod discovery;
pub(in crate::board) mod stream;
