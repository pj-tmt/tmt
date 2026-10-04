//! Pure lead lines; the HOME stream owns placement, scrolling and hits.
use crate::{
    board::home_leads::{Kind, Lead},
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::{Role, breakpoint::MD, table::escape};
use tmt_tui::components::Outline;

/// Outline owns square border glyphs/styles; its rows join the admitted stream.
pub(super) struct Chrome {
    pub top: Line<'static>,
    pub bottom: Line<'static>,
    left: Span<'static>,
    right: Span<'static>,
    width: usize,
}

impl Chrome {
    pub fn new(width: u16, look: Look) -> Self {
        let area = Rect::new(0, 0, width, 3);
        let mut buffer = Buffer::empty(area);
        Outline {
            title: Line::default(),
            border: look.role(Role::Dim),
            title_style: look.role(Role::Dim),
        }
        .paint(area, &mut buffer);
        let row = |y| {
            Line::from(
                (0..width)
                    .map(|x| {
                        let cell = &buffer[(x, y)];
                        Span::styled(cell.symbol().to_owned(), cell.style())
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let side = |x| {
            if width > 0 {
                let cell = &buffer[(x, 1)];
                Span::styled(cell.symbol().to_owned(), cell.style())
            } else {
                Span::raw("")
            }
        };
        Self {
            top: row(0),
            bottom: row(2),
            left: side(0),
            right: side(width.saturating_sub(1)),
            width: usize::from(width),
        }
    }

    pub fn wrap(&self, mut line: Line<'static>, look: Look, selected: bool) -> Line<'static> {
        if selected {
            for span in &mut line.spans {
                span.style = look.selection().patch(span.style);
            }
        }
        let padding = self.width.saturating_sub(2).saturating_sub(line.width());
        line.spans.push(Span::styled(
            " ".repeat(padding),
            if selected {
                look.selection()
            } else {
                look.role(Role::Text)
            },
        ));
        if self.width > 0 {
            line.spans.insert(0, self.left.clone());
        }
        if self.width > 1 {
            line.spans.push(self.right.clone());
        }
        line
    }
}

pub(super) fn heading(
    lead: &Lead,
    width: u16,
    look: Look,
    selected: bool,
    now: u64,
) -> Line<'static> {
    let (mark, role) = match lead.exchange.as_ref().map(|exchange| exchange.kind) {
        Some(Kind::Question) => ("◆", Role::Waiting),
        Some(Kind::Asked) => ("…", Role::Dim),
        Some(Kind::Reply) => ("✓", Role::Working),
        None => (" ", Role::Dim),
    };
    let inner = usize::from(width.saturating_sub(2));
    let age = lead
        .exchange
        .as_ref()
        .and_then(|exchange| exchange.since_ms)
        .map_or_else(|| "–".into(), |at| crate::requests::age(now, at));
    let age = super::super::view::fit(&age, inner.saturating_sub(4))
        .trim_end()
        .to_owned();
    let available = inner.saturating_sub(4 + unicode_width::UnicodeWidthStr::width(age.as_str()));
    let name_width = available.min(24);
    let name = super::super::view::fit(&escape(lead.name()), name_width);
    let squad = if width >= MD.cells && available > name_width + 3 {
        format!(
            "   {}",
            super::super::view::fit(&escape(&lead.squad), available - name_width - 3).trim_end()
        )
    } else {
        String::new()
    };
    let mut spans = vec![
        Span::styled(
            format!(" {mark} "),
            look.row_span(selected, look.role(role), true),
        ),
        Span::styled(
            name,
            look.row_span(
                selected,
                look.role(Role::Text).add_modifier(Modifier::BOLD),
                true,
            ),
        ),
        Span::styled(
            squad,
            look.row_span(selected, look.role(Role::Muted), false),
        ),
    ];
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled(
        " ".repeat(
            inner.saturating_sub(used + unicode_width::UnicodeWidthStr::width(age.as_str()) + 1),
        ),
        if selected {
            look.selection()
        } else {
            look.role(Role::Text)
        },
    ));
    spans.push(Span::styled(
        format!("{age} "),
        look.row_span(selected, look.role(Role::Dim), false),
    ));
    Line::from(spans)
}

pub(super) fn preview(lead: &Lead, width: u16, look: Look) -> Line<'static> {
    let (text, role) = lead.exchange.as_ref().map_or_else(
        || {
            (
                if lead.failure.is_some() {
                    "(exchange unavailable)"
                } else {
                    "–"
                }
                .into(),
                Role::Dim,
            )
        },
        |exchange| match exchange.kind {
            Kind::Question => (format!("asks: {}", exchange.preview), Role::Waiting),
            Kind::Asked => (format!("no reply yet to: {}", exchange.preview), Role::Dim),
            Kind::Reply => (exchange.preview.clone(), Role::Text),
        },
    );
    Line::styled(
        format!(
            "  {}",
            super::super::view::fit(&text, usize::from(width.saturating_sub(5))).trim_end()
        ),
        look.role(role),
    )
}
