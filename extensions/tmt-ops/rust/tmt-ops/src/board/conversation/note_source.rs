//! Descriptive message provenance for the checklist. Held render metadata never grants request authority.
use super::{App, Reply};
use crate::board::{home_leads::Kind, row_detail::Capture};
use ratatui::layout::Position;
use std::ops::Range;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::board) enum Source {
    #[default]
    Unavailable,
    Mixed,
    Message {
        author: String,
        kind: &'static str,
        at_ms: Option<u64>,
        reference: String,
    },
}

impl Source {
    pub(super) fn reply(reply: &Reply) -> Self {
        let Some(key) = &reply.key else {
            return Self::Unavailable;
        };
        Self::Message {
            author: reply.from.clone(),
            kind: if key.kind == Kind::Reply {
                "final"
            } else {
                "message"
            },
            at_ms: reply.at_ms,
            reference: format!("request {}", key.request),
        }
    }

    pub(super) fn request(author: &str, request: &str, at_ms: u64) -> Self {
        Self::Message {
            author: author.into(),
            kind: "message",
            at_ms: Some(at_ms),
            reference: format!("request {request}"),
        }
    }

    pub(super) fn capture(author: &str, request: &str, capture: &Capture) -> Self {
        match capture {
            Capture::Answered { ended_ms, .. } => Self::Message {
                author: author.into(),
                kind: "turn output",
                at_ms: Some(*ended_ms),
                reference: format!("request {request}"),
            },
            _ => Self::Unavailable,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Unavailable => "source unavailable".into(),
            Self::Mixed => "multiple messages / missing source".into(),
            Self::Message {
                author,
                kind,
                at_ms,
                ..
            } => format!(
                "{author} · {kind} · {}",
                at_ms.map_or_else(
                    || "time unavailable".into(),
                    |at| format!("{} ago", crate::requests::age(crate::status::now_ms(), at))
                )
            ),
        }
    }

    /// Stable human-readable bytes, without an age that would become stale on disk.
    pub fn body(&self) -> String {
        match self {
            Self::Unavailable => "Source: unavailable".into(),
            Self::Mixed => "Source: multiple messages / missing source".into(),
            Self::Message {
                author,
                kind,
                at_ms,
                reference,
            } => format!(
                "Source: {} · {kind} · {} · {}",
                crate::board::notes::single_line(author),
                crate::board::notes::single_line(reference),
                at_ms.map_or_else(
                    || "time unavailable".into(),
                    |at| i64::try_from(at)
                        .ok()
                        .and_then(|at| jiff::Timestamp::from_millisecond(at).ok())
                        .map_or_else(|| format!("{at} ms since Unix epoch"), |at| at.to_string())
                )
            ),
        }
    }
}

/// Sources occur once per message; contiguous rows retain only their source's index.
#[derive(Clone, Default)]
pub(super) struct Map {
    sources: Vec<Source>,
    spans: Vec<(Range<usize>, usize)>,
}

impl Map {
    pub fn add(&mut self, source: Source) -> usize {
        if let Some(id) = self.sources.iter().position(|held| held == &source) {
            return id;
        }
        self.sources.push(source);
        self.sources.len() - 1
    }

    pub fn row(&mut self, line: usize, source: usize) {
        if let Some((range, last)) = self.spans.last_mut()
            && *last == source
            && range.end == line
        {
            range.end += 1;
        } else {
            self.spans.push((line..line + 1, source));
        }
    }

    fn source(&self, line: usize) -> &Source {
        self.spans
            .iter()
            .find(|(range, _)| range.contains(&line))
            .map_or(&Source::Unavailable, |(_, id)| &self.sources[*id])
    }

    pub fn at(&self, line: usize) -> Source {
        self.source(line).clone()
    }

    pub fn selected(&self, lines: impl Iterator<Item = usize>) -> Source {
        let mut sources = lines.map(|line| self.source(line));
        let Some(first) = sources.next() else {
            return Source::Unavailable;
        };
        if sources.all(|source| source == first) {
            first.clone()
        } else {
            Source::Mixed
        }
    }
}

impl App {
    pub(in crate::board) fn note_source_at(&self, point: Position) -> Option<Source> {
        let state = self.conversation.borrow();
        let area = state.response_area.filter(|area| area.contains(point))?;
        let rendered = state
            .sessions
            .get(state.active.as_ref()?)?
            .rendered
            .as_ref()?;
        let line =
            self.scrolls.offset(crate::config::Pane::Replies) + usize::from(point.y - area.y);
        let line = *rendered.anchor_sources.get(line)?;
        Some(rendered.note_sources.at(line))
    }
}
