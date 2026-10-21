//! Exchange headers, semantic sender/status colours and a shared grouped body gutter.
use crate::look::Look;
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) enum Sender<'a> {
    You(&'a str),
    Agent(&'a str),
}
impl Sender<'_> {
    fn name(&self) -> &str {
        match self {
            Self::You(name) | Self::Agent(name) => name,
        }
    }
    fn style(&self, look: Look) -> ratatui::style::Style {
        look.role(match self {
            Self::You(_) => Role::Accent,
            Self::Agent(_) => Role::Text,
        })
        .add_modifier(Modifier::BOLD)
    }
}

pub(in crate::board) fn header(
    sender: Sender<'_>,
    recipient: &str,
    stamp: &str,
    read_only: bool,
    width: usize,
    look: Look,
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let stamp = format!(
        "{}{}",
        super::notes::single_line(stamp),
        if read_only { " · read-only" } else { "" }
    );
    let stamp = if stamp.width() + 7 > width && read_only {
        "read-only".into()
    } else {
        stamp
    };
    let right = stamp.width().saturating_add(2).min(width);
    if right < 2 {
        return Line::styled(super::view::fit(&stamp, width), look.role(Role::Dim));
    }
    let left = width.saturating_sub(right);
    let names = left.saturating_sub(4);
    let sender_style = sender.style(look);
    let sender = super::notes::single_line(sender.name());
    let recipient = super::notes::single_line(recipient);
    let sender_cells = sender.width().min(
        names
            .div_ceil(2)
            .max(names.saturating_sub(recipient.width())),
    );
    let recipient_cells = names.saturating_sub(sender_cells);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(super::view::fit(&sender, sender_cells), sender_style),
        Span::styled(" → ", look.role(Role::Dim)),
        Span::styled(
            super::view::fit(&recipient, recipient_cells),
            look.role(Role::Text),
        ),
    ];
    if left < 4 {
        spans.clear();
        spans.push(Span::raw(" ".repeat(left)));
    }
    let used = spans.iter().map(Span::width).sum::<usize>();
    spans.push(Span::raw(" ".repeat(left.saturating_sub(used))));
    spans.push(Span::styled(
        format!(" {} ", super::view::fit(&stamp, right.saturating_sub(2))),
        look.role(Role::Dim),
    ));
    Line::from(spans)
}

pub(in crate::board) fn feedback(text: Line<'static>, look: Look) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("│", look.role(Role::Dim)),
        Span::raw(" "),
    ];
    spans.extend(text.spans);
    Line::from(spans)
}

/// Style the whole authored status before wrapping, including split words.
pub(in crate::board) fn feedback_content(text: &str, look: Look) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(rest) = text.strip_prefix('✗') {
        spans.push(Span::styled("✗", look.role(Role::Blocked)));
        spans.push(Span::styled(rest.to_owned(), look.role(Role::Dim)));
    } else if let Some(rest) = text.strip_prefix('✓') {
        spans.push(Span::styled("✓", look.role(Role::Working)));
        spans.push(Span::styled(rest.to_owned(), look.role(Role::Dim)));
    } else if let Some((glyph, rest)) = text.split_once(' ')
        && glyph.chars().count() == 1
        && matches!(glyph.chars().next(), Some('\u{2800}'..='\u{28ff}'))
        && let Some((name, elapsed)) = rest.rsplit_once(" is working · ")
    {
        spans.push(Span::styled(glyph.to_owned(), look.role(Role::Working)));
        spans.push(Span::styled(format!(" {name} "), look.role(Role::Dim)));
        spans.push(Span::styled("is working", look.role(Role::Working)));
        spans.push(Span::styled(format!(" · {elapsed}"), look.role(Role::Dim)));
    } else {
        spans.push(Span::styled(text.to_owned(), look.role(Role::Dim)));
    }
    Line::from(spans)
}

pub(in crate::board) fn body(line: Line<'static>, look: Look) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("│", look.role(Role::Dim)),
        Span::raw(" "),
    ];
    // The Markdown owner supplies neutral links, filled code and muted quotes.
    // Add the shared gutter without flattening those authored block styles.
    let line_style = line.style;
    spans.extend(line.spans.into_iter().map(|mut span| {
        span.style = look.role(Role::Text).patch(line_style).patch(span.style);
        span
    }));
    Line::from(spans)
}
pub(in crate::board) fn replier(sender: Sender<'_>, look: Look) -> Line<'static> {
    let mut line = body(Line::default(), look);
    line.spans.push(Span::styled(
        super::notes::single_line(sender.name()),
        sender.style(look),
    ));
    line
}
pub(in crate::board) fn waiting(look: Look) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled("│", look.role(Role::Dim)),
        Span::raw(" "),
        Span::styled("Needs you", look.role(Role::Dim)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_cli_style::{Base, Depth, Theme};

    #[test]
    fn exchange_colours_follow_identity_and_leave_metadata_neutral() {
        for base in [Base::Tmt, Base::TmtLight, Base::Terminal] {
            for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
                let look = Look {
                    theme: Theme::new(base),
                    depth,
                };
                let you = header(Sender::You("Ben"), "worker", "2m ago", false, 60, look);
                let agent = header(Sender::Agent("you"), "Ben", "2m ago", false, 60, look);
                assert_eq!(
                    you.spans[1].style,
                    look.role(Role::Accent).add_modifier(Modifier::BOLD)
                );
                assert_eq!(
                    agent.spans[1].style,
                    look.role(Role::Text).add_modifier(Modifier::BOLD)
                );
                for line in [&you, &agent] {
                    assert_eq!(line.spans[2].style, look.role(Role::Dim));
                    assert_eq!(line.spans[3].style, look.role(Role::Text));
                    assert_eq!(line.spans.last().unwrap().style, look.role(Role::Dim));
                }
                assert_eq!(
                    replier(Sender::You("Ben"), look).spans[3].style,
                    you.spans[1].style
                );
                assert_eq!(
                    replier(Sender::Agent("you"), look).spans[3].style,
                    agent.spans[1].style
                );
                let working = feedback_content("⠹ worker is working · 0:42", look);
                assert_eq!(working.to_string(), "⠹ worker is working · 0:42");
                assert_eq!(working.spans[0].style, look.role(Role::Working));
                assert_eq!(working.spans[1].style, look.role(Role::Dim));
                assert_eq!(working.spans[2].style, look.role(Role::Working));
                assert_eq!(working.spans[3].style, look.role(Role::Dim));
                for text in [
                    "✓ sent · worker's activity is not reported",
                    "✓ worker finished without replying · 2m ago",
                ] {
                    let line = feedback_content(text, look);
                    assert_eq!(line.to_string(), text);
                    assert_eq!(line.spans[0].style, look.role(Role::Working));
                    assert_eq!(line.spans[1].style, look.role(Role::Dim));
                }
                let earlier = feedback_content("◌ worker is finishing an earlier turn · 3m", look);
                assert_eq!(earlier.spans[0].style, look.role(Role::Dim));
                if depth == Depth::Ansi16 {
                    assert_eq!(you.spans[1].style.fg, Some(ratatui::style::Color::Blue));
                    assert_eq!(
                        working.spans[0].style.fg,
                        Some(ratatui::style::Color::Green)
                    );
                }
                if depth == Depth::None {
                    assert!(you.spans[1].style.fg.is_none());
                    assert_eq!(you.spans[1].style.add_modifier, Modifier::BOLD);
                    assert_eq!(working.spans[0].style, ratatui::style::Style::new());
                }
            }
        }
    }

    #[test]
    fn feedback_colours_survive_wrapping_without_changing_text_or_geometry() {
        let look = Look::default();
        for text in [
            "⠹ very-long-worker-name is working · 0:42",
            "✓ worker finished without replying · 2m ago",
            "◌ worker is finishing an earlier turn · 3m",
        ] {
            for width in 1..=80 {
                let plain = super::super::markdown::wrap_styled(Line::raw(text.to_owned()), width);
                let styled =
                    super::super::markdown::wrap_styled(feedback_content(text, look), width);
                assert_eq!(
                    plain.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    styled.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "{width}: {text}"
                );
                let coloured: String = styled
                    .iter()
                    .flat_map(|line| &line.spans)
                    .filter(|span| span.style == look.role(Role::Working))
                    .map(|span| span.content.as_ref())
                    .collect();
                if text.starts_with('⠹') {
                    assert_eq!(
                        coloured.split_whitespace().collect::<Vec<_>>().join(""),
                        "⠹isworking"
                    );
                } else if text.starts_with('✓') {
                    assert_eq!(coloured, "✓");
                } else {
                    assert!(coloured.is_empty());
                }
            }
        }
    }
    #[test]
    fn replies_share_the_gutter_without_an_arrow_or_extra_indent() {
        let look = Look::default();
        let reply = replier(Sender::You("you"), look);
        assert_eq!(reply.to_string(), " │ you");
        assert!(!reply.to_string().contains('→'));
        assert_eq!(
            reply.spans[3].style,
            look.role(Role::Accent).add_modifier(Modifier::BOLD)
        );
        let title = header(Sender::Agent("member"), "you", "6h ago", false, 59, look);
        assert!(title.to_string().ends_with("6h ago "));
        let narrow = header(
            Sender::Agent("an unreasonably long sender"),
            "another recipient",
            "now",
            true,
            32,
            look,
        );
        assert!(narrow.to_string().contains("read-only"));
        assert!(narrow.width() <= 32);
    }
}

#[cfg(test)]
mod fitting_tests {
    use super::*;
    #[test]
    fn metadata_fits_without_changing_effect_targets() {
        for width in [0, 1, 2, 8, 20, 59] {
            assert!(
                header(
                    Sender::Agent("very long sender"),
                    "very long recipient",
                    "6h ago",
                    true,
                    width,
                    Look::default()
                )
                .width()
                    <= width
            );
        }
    }
}

#[cfg(test)]
mod markdown_tests {
    use super::*;
    #[test]
    fn grouped_body_preserves_markdown_fill_and_muted_quote_styles() {
        let look = Look::default();
        let track = look.role(Role::Text).patch(look.role(Role::Track));
        let code = body(Line::styled("code", track), look);
        assert_eq!(code.spans[3].style, track);
        let quote = body(
            Line::from(vec![
                Span::styled("▎ ", look.role(Role::Dim)),
                Span::styled("quote", look.role(Role::Muted)),
            ]),
            look,
        );
        assert_eq!(quote.spans[3].style, look.role(Role::Dim));
        assert_eq!(quote.spans[4].style, look.role(Role::Muted));
    }
}
