//! OSC 8 hyperlinks for the web links the conversation paints.
//!
//! A terminal that supports them makes the label itself clickable; the link
//! popover, Enter and selection stay the way every terminal gets them. The board
//! writes the escape in its own backend, around the cells it already draws, so
//! nothing changes for a terminal that does not support them: no byte differs.
//!
//! The writer accepts only http and https addresses. Agent text can carry
//! `file:`, `javascript:` or custom schemes, and a terminal opens whatever it is
//! handed; anything else, and anything with a control byte, renders as plain
//! text.
use ratatui::layout::{Position, Rect};
use std::{cell::RefCell, ops::Range, rc::Rc};

/// A longer address is refused outright; terminals cap the sequence (VTE at 2083).
const LONGEST: usize = 2000;

/// Whether this terminal makes OSC 8 links clickable. `FORCE_HYPERLINK` (read by
/// `supports-hyperlinks` and other CLIs) decides when set: `0` is off, any other
/// non-empty value on. Unset, only a positive signal turns links on, because an
/// emulator that ignores the sequence may print its payload.
pub fn supported(var: impl Fn(&str) -> Option<String>, tmux: impl FnOnce() -> bool) -> bool {
    let var = |name: &str| var(name).filter(|value| !value.is_empty());
    match var("FORCE_HYPERLINK").as_deref() {
        Some("0") => return false,
        Some(_) => return true,
        None => {}
    }
    // The multiplexers decide what reaches the terminal, whatever it is.
    if var("TMUX").is_some() {
        return tmux();
    }
    if var("STY").is_some() {
        return false;
    }
    let term = var("TERM").unwrap_or_default();
    let program = var("TERM_PROGRAM").unwrap_or_default();
    matches!(
        program.as_str(),
        "iTerm.app" | "WezTerm" | "ghostty" | "vscode"
    ) || matches!(term.as_str(), "xterm-kitty" | "xterm-ghostty" | "alacritty")
        || term.starts_with("foot")
        || var("KITTY_WINDOW_ID").is_some()
        || var("WT_SESSION").is_some()
        || var("KONSOLE_VERSION").is_some()
        || var("VTE_VERSION")
            .and_then(|version| version.parse::<u32>().ok())
            .is_some_and(|version| version >= 5000)
}

/// Reads the process environment and, inside tmux, the server.
pub fn detect() -> bool {
    supported(
        |name| std::env::var(name).ok(),
        || {
            crate::effects::tmux_socket()
                .is_some_and(|socket| crate::effects::tmux_passes_hyperlinks(&socket))
        },
    )
}

/// The address as it goes into the escape, or `None` when it must not: only an
/// http(s) address with no whitespace or control character (C0, DEL or C1, so no
/// ESC, BEL or ST can end the sequence early), within the length cap. Non-ASCII is
/// percent-encoded, as a URI requires.
pub fn address(url: &str) -> Option<String> {
    let url = crate::effects::web_link(url).ok()?;
    let mut out = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii() {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    (out.len() <= LONGEST).then_some(out)
}

/// `OSC 8 ; id=<n> ; URI ST` opens a link and the same with no parameters and an
/// empty URI ends it. ST (`ESC \`) rather than BEL: both terminate, and ST is what
/// the spec shows. The only place the board writes the escape.
fn sequence(link: Option<(u32, &str)>) -> Vec<u8> {
    match link {
        Some((id, address)) => format!("\x1b]8;id={id};{address}\x1b\\"),
        None => "\x1b]8;;\x1b\\".to_owned(),
    }
    .into_bytes()
}

/// Ends the open link.
pub fn close() -> Vec<u8> {
    sequence(None)
}

/// A link's painted cells, ready to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub area: Rect,
    open: Vec<u8>,
}

impl Span {
    #[cfg(test)]
    pub fn open(&self) -> &[u8] {
        &self.open
    }

    /// `None` when the address may not be written. `id` joins the pieces of one
    /// link (a wrapped label) so a terminal underlines them together.
    pub fn new(area: Rect, url: &str, id: u32) -> Option<Self> {
        let address = address(url)?;
        Some(Self {
            area,
            open: sequence(Some((id, &address))),
        })
    }
}

/// The spans of the frame being drawn, shared between the view that paints the
/// links and the backend that writes them. Empty unless the terminal supports them.
#[derive(Clone, Default)]
pub struct Links(Rc<RefCell<Inner>>);

#[derive(Default)]
struct Inner {
    enabled: bool,
    spans: Vec<Span>,
}

impl Links {
    pub fn enable(&self, on: bool) {
        let mut inner = self.0.borrow_mut();
        inner.enabled = on;
        inner.spans.clear();
    }

    pub fn enabled(&self) -> bool {
        self.0.borrow().enabled
    }

    /// Replaces the frame's spans; ignored while links are off.
    pub fn set(&self, spans: Vec<Span>) {
        let mut inner = self.0.borrow_mut();
        if inner.enabled {
            inner.spans = spans;
        }
    }

    /// Splits the cells about to be drawn into runs that lie in one span (with that
    /// span's open) or in none. The caller writes the open, the run, then `close()`.
    pub fn runs(
        &self,
        cells: &[(u16, u16, &ratatui::buffer::Cell)],
    ) -> Vec<(Range<usize>, Option<Vec<u8>>)> {
        let inner = self.0.borrow();
        let span_at = |(x, y, _): &(u16, u16, &ratatui::buffer::Cell)| {
            inner
                .spans
                .iter()
                .position(|span| span.area.contains(Position::new(*x, *y)))
        };
        let mut runs = Vec::new();
        let mut start = 0;
        while start < cells.len() {
            let span = span_at(&cells[start]);
            let mut end = start + 1;
            while end < cells.len() && span_at(&cells[end]) == span {
                end += 1;
            }
            runs.push((
                start..end,
                span.map(|index| inner.spans[index].open.clone()),
            ));
            start = end;
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn force_hyperlink_decides_and_unset_needs_a_positive_signal() {
        let yes = || true;
        let no = || false;
        assert!(supported(env(&[("FORCE_HYPERLINK", "1")]), no));
        assert!(!supported(
            env(&[("FORCE_HYPERLINK", "0"), ("TERM_PROGRAM", "iTerm.app")]),
            yes
        ));
        // Empty counts as unset.
        assert!(!supported(env(&[("FORCE_HYPERLINK", "")]), yes));
        assert!(!supported(env(&[]), yes));
        assert!(!supported(env(&[("TERM", "xterm-256color")]), yes));
        assert!(!supported(env(&[("TERM_PROGRAM", "Apple_Terminal")]), yes));
        for signal in [
            ("TERM_PROGRAM", "iTerm.app"),
            ("TERM_PROGRAM", "WezTerm"),
            ("TERM_PROGRAM", "ghostty"),
            ("TERM_PROGRAM", "vscode"),
            ("TERM", "xterm-kitty"),
            ("TERM", "alacritty"),
            ("TERM", "foot-extra"),
            ("KITTY_WINDOW_ID", "3"),
            ("WT_SESSION", "x"),
            ("KONSOLE_VERSION", "230400"),
            ("VTE_VERSION", "7600"),
        ] {
            assert!(supported(env(&[signal]), no), "{signal:?}");
        }
        assert!(!supported(env(&[("VTE_VERSION", "4800")]), yes));
        assert!(!supported(env(&[("VTE_VERSION", "garbage")]), yes));
    }

    #[test]
    fn a_multiplexer_decides_whatever_the_outer_terminal_is() {
        let outer = ("TERM_PROGRAM", "iTerm.app");
        assert!(!supported(env(&[("TMUX", "/s,1,0"), outer]), || false));
        assert!(supported(env(&[("TMUX", "/s,1,0")]), || true));
        assert!(!supported(env(&[("STY", "1.pts"), outer]), || true));
        // The question to tmux is asked only inside tmux.
        assert!(!supported(env(&[]), || panic!("not inside tmux")));
        assert!(supported(
            env(&[("TMUX", "/s,1,0"), ("FORCE_HYPERLINK", "1")]),
            || panic!("forced")
        ));
    }

    #[test]
    fn only_clean_http_addresses_are_written_and_the_rest_stay_plain() {
        assert_eq!(
            address("https://example.com/a?b=1#c").as_deref(),
            Some("https://example.com/a?b=1#c")
        );
        assert_eq!(
            address("http://example.com/é").as_deref(),
            Some("http://example.com/%C3%A9")
        );
        for refused in [
            "mailto:a@example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com/",
            "https://",
            "https://exa mple.com",
            "https://example.com/\x1b\\x",
            "https://example.com/\x07",
            "https://example.com/\u{9c}",
            "https://example.com/\u{7f}",
            "https://example.com/\n",
        ] {
            assert_eq!(address(refused), None, "{refused:?}");
            assert_eq!(Span::new(Rect::new(0, 0, 3, 1), refused, 1), None);
        }
        let long = format!("https://example.com/{}", "a".repeat(LONGEST));
        assert_eq!(address(&long), None);
    }

    #[test]
    fn a_span_opens_with_its_id_and_the_close_is_the_empty_address() {
        let span = Span::new(Rect::new(0, 0, 3, 1), "https://example.com/", 7).unwrap();
        assert_eq!(span.open, b"\x1b]8;id=7;https://example.com/\x1b\\");
        assert_eq!(close(), b"\x1b]8;;\x1b\\");
    }
}
