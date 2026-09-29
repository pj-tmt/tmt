//! The only way a command reaches standard output or standard error. Each
//! stream carries the [`Terminal`] decided for it, so a command never takes a
//! raw handle and never decides color or width on its own.

use crate::palette::Terminal;
use std::io::{self, Write};

/// A locked standard stream and the rendering decided for it.
pub struct Stream<W> {
    terminal: Terminal,
    writer: W,
}

/// Standard output, plain for `--json`.
pub fn stdout(json: bool) -> Stream<io::StdoutLock<'static>> {
    Stream::new(Terminal::stdout(json), io::stdout().lock())
}

/// Standard error, where `error:` and `hint:` lines go.
pub fn stderr() -> Stream<io::StderrLock<'static>> {
    Stream::new(Terminal::stderr(), io::stderr().lock())
}

impl<W: Write> Stream<W> {
    /// Any writer with an explicit decision, such as a buffer in a test.
    pub fn new(terminal: Terminal, writer: W) -> Self {
        Self { terminal, writer }
    }

    pub fn terminal(&self) -> Terminal {
        self.terminal
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> Write for Stream<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writer.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message;

    #[test]
    fn a_stream_renders_with_its_own_decision() {
        let mut stream = Stream::new(Terminal::PLAIN, Vec::new());
        let terminal = stream.terminal();
        message::success(&mut stream, terminal, "Named pane %3 worker.").unwrap();
        assert_eq!(stream.into_inner(), "✓ Named pane %3 worker\n".as_bytes());
        assert_eq!(stdout(true).terminal(), Terminal::PLAIN);
    }
}
