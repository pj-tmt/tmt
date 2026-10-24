//! Observed step progress. Callers own clocks, effects and terminal eligibility.

use crate::{Terminal, Token, grid, mark::Mark, table::escape};
use std::{
    io::{self, Write},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Download,
    Verify,
    Activate,
}

impl Step {
    fn index(self) -> usize {
        match self {
            Self::Download => 0,
            Self::Verify => 1,
            Self::Activate => 2,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Download => "Download",
            Self::Verify => "Verify",
            Self::Activate => "Activate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Waiting,
    Running,
    Done,
    Failed,
    Warning,
}

#[derive(Debug, Clone, Copy)]
pub struct Transfer {
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    /// Observed by the caller, never sampled by the renderer.
    pub elapsed: Duration,
}

pub struct StepProgress<'a> {
    pub step: Step,
    pub state: State,
    pub transfer: Option<Transfer>,
    pub detail: &'a str,
}

struct Row {
    state: State,
    transfer: Option<Transfer>,
    detail: String,
}

pub struct Progress {
    title: String,
    rows: [Row; 3],
    live: bool,
    drawn: bool,
}

impl Progress {
    /// `live` permits replacement of this component's own four lines. Pass false
    /// for pipes, dumb terminals and static captures, regardless of color.
    pub fn new(title: &str, live: bool) -> Self {
        Self {
            title: escape(title),
            rows: std::array::from_fn(|_| Row {
                state: State::Waiting,
                transfer: None,
                detail: String::new(),
            }),
            live,
            drawn: false,
        }
    }

    pub fn title(&mut self, title: &str) {
        self.title = escape(title);
    }

    pub fn update(&mut self, update: StepProgress<'_>) {
        self.rows[update.step.index()] = Row {
            state: update.state,
            transfer: update.transfer,
            detail: escape(update.detail),
        };
    }

    pub fn write(&mut self, out: &mut impl Write, terminal: Terminal) -> io::Result<()> {
        if self.live && self.drawn {
            write!(out, "\x1b[4A")?;
        }
        self.line(out, terminal, &self.title, Token::Title)?;
        for step in [Step::Download, Step::Verify, Step::Activate] {
            let row = &self.rows[step.index()];
            let (mark, token) = match row.state {
                State::Waiting => ("·", Token::Dim),
                State::Running => ("⠹", Token::Accent),
                State::Done => (Mark::Done.symbol(), Mark::Done.token()),
                State::Failed => (Mark::Failed.symbol(), Mark::Failed.token()),
                State::Warning => (Mark::Warning.symbol(), Mark::Warning.token()),
            };
            let detail = if let Some(transfer) = row.transfer {
                let received = bytes(transfer.received_bytes);
                let size = match transfer.total_bytes {
                    Some(total) if total > 0 => {
                        let filled = ((u128::from(transfer.received_bytes.min(total)) * 24)
                            / u128::from(total)) as usize;
                        format!(
                            "{}{}  {received} of {}",
                            "━".repeat(filled),
                            "─".repeat(24 - filled),
                            bytes(total)
                        )
                    }
                    _ => format!("{received} downloaded"),
                };
                let rate = if row.state == State::Running && !transfer.elapsed.is_zero() {
                    format!(
                        " · {}/s",
                        bytes(
                            (transfer.received_bytes as f64 / transfer.elapsed.as_secs_f64())
                                as u64
                        )
                    )
                } else {
                    String::new()
                };
                format!("{size}{rate}")
            } else {
                row.detail.clone()
            };
            let text = format!("{mark} {:12} {detail}", step.label());
            self.line(out, terminal, &text, token)?;
        }
        self.drawn = true;
        out.flush()
    }

    fn line(
        &self,
        out: &mut impl Write,
        terminal: Terminal,
        text: &str,
        token: Token,
    ) -> io::Result<()> {
        if self.live {
            write!(out, "\r\x1b[2K")?;
        }
        let text = match terminal.width {
            Some(width) => grid::fit(
                text,
                usize::from(width).saturating_sub(2),
                grid::Align::Left,
                grid::Truncate::End,
            ),
            None => text.to_owned(),
        };
        let text = text.trim_end();
        write!(out, "  ")?;
        if token == Token::Title {
            write!(out, "{}", terminal.paint(token, text))?;
        } else if let Some((mark, rest)) = text.split_once(' ') {
            write!(out, "{} ", terminal.paint(token, mark))?;
            if let Some((label, detail)) = rest.split_once("  ") {
                write!(out, "{label}  ")?;
                // Only marks and completed transfer cells carry status color;
                // labels stay neutral and supporting details stay muted.
                let mut remaining = detail;
                while let Some(ch) = remaining
                    .chars()
                    .next()
                    .filter(|ch| ch.is_whitespace() || matches!(ch, '━' | '─'))
                {
                    let role = if ch == '━' {
                        Token::Accent
                    } else {
                        Token::Dim
                    };
                    write!(out, "{}", terminal.paint(role, &ch.to_string()))?;
                    remaining = &remaining[ch.len_utf8()..];
                }
                write!(out, "{}", terminal.paint(Token::Dim, remaining))?;
            } else {
                write!(out, "{rest}")?;
            }
        } else {
            write!(out, "{}", terminal.paint(token, text))?;
        }

        writeln!(out)
    }
}

pub fn bytes(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} kB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_progress_matches_the_three_step_screen() {
        let mut progress = Progress::new("Installing example 0.1.0-alpha.2", false);
        progress.update(StepProgress {
            step: Step::Download,
            state: State::Done,
            transfer: Some(Transfer {
                received_bytes: 6_400_000,
                total_bytes: Some(6_400_000),
                elapsed: Duration::from_secs(2),
            }),
            detail: "",
        });
        progress.update(StepProgress {
            step: Step::Verify,
            state: State::Done,
            transfer: None,
            detail: "checksum matches the release record; archive checked",
        });
        progress.update(StepProgress {
            step: Step::Activate,
            state: State::Running,
            transfer: None,
            detail: "settling bundled agent skills",
        });
        let mut output = Vec::new();
        progress
            .write(
                &mut output,
                Terminal {
                    width: Some(96),
                    ..Terminal::PLAIN
                },
            )
            .unwrap();
        insta::assert_snapshot!(String::from_utf8(output).unwrap());
    }

    #[test]
    fn real_counts_unknown_totals_and_plain_output_remain_truthful() {
        let mut progress = Progress::new("Installing example", false);
        progress.update(StepProgress {
            step: Step::Download,
            state: State::Running,
            transfer: Some(Transfer {
                received_bytes: 4_000_000,
                total_bytes: Some(6_400_000),
                elapsed: Duration::from_secs(2),
            }),
            detail: "",
        });
        let mut output = Vec::new();
        progress.write(&mut output, Terminal::PLAIN).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("4.0 MB of 6.4 MB · 2.0 MB/s"));
        assert!(!text.contains('\x1b'));
        progress.update(StepProgress {
            step: Step::Download,
            state: State::Running,
            transfer: Some(Transfer {
                received_bytes: 42,
                total_bytes: None,
                elapsed: Duration::ZERO,
            }),
            detail: "",
        });
        let mut output = Vec::new();
        progress.write(&mut output, Terminal::PLAIN).unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("42 B downloaded")
        );
    }
    #[test]
    fn narrow_rows_fit_and_untrusted_details_cannot_control_the_terminal() {
        let mut progress = Progress::new("Installing\x1b[2J", false);
        progress.update(StepProgress {
            step: Step::Activate,
            state: State::Warning,
            transfer: None,
            detail: "skills warning\nretry",
        });
        let mut output = Vec::new();
        progress
            .write(
                &mut output,
                Terminal {
                    width: Some(26),
                    ..Terminal::PLAIN
                },
            )
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(!text.contains('\x1b'));
        assert_eq!(text.lines().count(), 4);
        assert!(
            text.lines()
                .all(|line| unicode_width::UnicodeWidthStr::width(line) <= 26)
        );
    }
    #[test]
    fn live_repaint_replaces_only_its_own_rows_even_without_color() {
        let mut progress = Progress::new("Installing example", true);
        let mut output = Vec::new();
        progress.write(&mut output, Terminal::PLAIN).unwrap();
        let first = output.len();
        progress.write(&mut output, Terminal::PLAIN).unwrap();
        assert!(output[first..].starts_with(b"\x1b[4A"));
        assert!(!output[..first].windows(4).any(|part| part == b"\x1b[4A"));
    }
}
