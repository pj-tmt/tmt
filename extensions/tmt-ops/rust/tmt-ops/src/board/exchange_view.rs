//! M8 ExchangeView: neutral exchange headers and a shared grouped body gutter.
use crate::look::Look;
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) fn header(
    sender: &str,
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
    let sender = super::notes::single_line(sender);
    let recipient = super::notes::single_line(recipient);
    let sender_cells = sender.width().min(
        names
            .div_ceil(2)
            .max(names.saturating_sub(recipient.width())),
    );
    let recipient_cells = names.saturating_sub(sender_cells);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            super::view::fit(&sender, sender_cells),
            look.role(Role::Text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" → ", look.role(Role::Dim)),
        Span::styled(
            super::view::fit(&recipient, recipient_cells),
            look.role(Role::Muted),
        ),
    ];
    if left < 4 {
        spans.clear();
        spans.push(Span::raw(" ".repeat(left)));
    }
    spans.push(Span::styled(
        format!(" {} ", super::view::fit(&stamp, right.saturating_sub(2))),
        look.role(Role::Dim),
    ));
    Line::from(spans)
}

/// Only the display label is shortened; effects retain the exact resolved value.
fn link_label(value: &str) -> String {
    let value = super::notes::single_line(value);
    if let Some(path) = value.strip_prefix("https://github.com/") {
        let parts: Vec<_> = path.split('/').collect();
        if let [owner, repo, kind, number] = parts.as_slice()
            && !owner.is_empty()
            && !repo.is_empty()
            && matches!(*kind, "issues" | "pull")
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            return format!("{owner}/{repo}#{number}");
        }
    }
    value
}
pub(in crate::board) fn link(
    value: &str,
    key: &str,
    wording: &str,
    width: usize,
    look: Look,
) -> Line<'static> {
    let action = if wording == "open $link" {
        "open"
    } else {
        wording
    };
    let hint = format!(
        "{} {}",
        super::view::footer::key_label(key),
        super::notes::single_line(action)
    );
    let prefix = link_label(value);
    if key.is_empty() {
        return Line::styled(
            format!(" {}", super::view::fit(&prefix, width.saturating_sub(2))),
            look.role(Role::Muted),
        );
    }
    let available = width.saturating_sub(2);
    if hint.width() + 3 > available {
        return Line::styled(super::view::fit(&hint, width), look.role(Role::Muted));
    }
    let left = available - hint.width() - 3;
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            super::view::fit(&prefix, left.min(prefix.width())),
            look.role(Role::Muted),
        ),
        Span::styled(" · ", look.role(Role::Dim)),
    ];
    spans.extend(super::m12_key_hints::line(&hint, look, look.role(Role::Text), " · ").spans);
    spans.push(Span::raw(" "));
    Line::from(spans)
}

pub(in crate::board) fn feedback(text: &str, look: Look) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("│", look.role(Role::Dim)),
        Span::raw(" "),
    ];
    if let Some(rest) = text.strip_prefix('✗') {
        spans.push(Span::styled("✗", look.role(Role::Blocked)));
        spans.push(Span::styled(rest.to_owned(), look.role(Role::Dim)));
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
    // Markdown's authored emphasis survives; semantic link/heading colours do not.
    let line_style = line.style;
    spans.extend(line.spans.into_iter().map(|mut span| {
        let authored = line_style.patch(span.style);
        span.style = look
            .role(Role::Text)
            .add_modifier(authored.add_modifier)
            .remove_modifier(authored.sub_modifier);
        span
    }));
    Line::from(spans)
}
pub(in crate::board) fn replier(name: &str, look: Look) -> Line<'static> {
    body(
        Line::styled(
            super::notes::single_line(name),
            look.role(Role::Text).add_modifier(Modifier::BOLD),
        ),
        look,
    )
}
pub(in crate::board) fn waiting(look: Look) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled("│", look.role(Role::Dim)),
        Span::raw(" "),
        Span::styled("waiting for your reply", look.role(Role::Dim)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_share_the_gutter_without_an_arrow_or_extra_indent() {
        let look = Look::default();
        let reply = replier("you", look);
        assert_eq!(reply.to_string(), " │ you");
        assert!(!reply.to_string().contains('→'));
        assert_eq!(
            reply.spans[3].style,
            look.role(Role::Text).add_modifier(Modifier::BOLD)
        );
        let title = header("member", "you", "6h ago", false, 59, look);
        assert!(title.to_string().ends_with("6h ago "));
        let narrow = header(
            "an unreasonably long sender",
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
    fn metadata_and_compact_links_fit_without_changing_effect_targets() {
        for width in [0, 1, 2, 8, 20, 59] {
            assert!(
                header(
                    "very long sender",
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
        let url = "https://github.com/pj-tmt/tmt/pull/3176";
        let shown = link(url, "o", "open $link", 59, Look::default()).to_string();
        assert!(shown.starts_with(" pj-tmt/tmt#3176 · o open"));
        assert_eq!(url, "https://github.com/pj-tmt/tmt/pull/3176");
        assert_eq!(
            link_label("https://github.com/pj-tmt/tmt/pull/3176/files"),
            "https://github.com/pj-tmt/tmt/pull/3176/files"
        );
        assert!(
            !link_label("https://github.com/unsafe\nowner/repo/pull/1")
                .chars()
                .any(char::is_control)
        );
    }
}
