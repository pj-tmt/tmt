//! Shared row-message framing. Callers own admission,
//! source identity and byte ranges. Exact row framing preserves every source
//! byte; compact chat insertion has an explicit character bound and no authority.

/// The retained row-annotation prefix, including its comment separator.
pub fn tag(squad: &str, row: &str) -> String {
    format!("[{squad} · {row}] ")
}

/// Quote every source line without escaping text or flattening its newlines.
pub fn blockquote(passage: &str) -> String {
    let mut quote = String::new();
    for (index, line) in passage.split('\n').enumerate() {
        if index > 0 {
            quote.push('\n');
        }
        quote.push('>');
        if !line.is_empty() {
            quote.push(' ');
            quote.push_str(line);
        }
    }
    quote
}

/// The compact selection gesture: one quoted line, at most 200 source
/// characters including a final ellipsis. Ordinary row notes keep `blockquote`.
pub fn inline_quote(passage: &str) -> String {
    let flattened = passage.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let mut chars = flattened.chars();
    let mut text: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        text.pop();
        text.push('…');
    }
    format!("> \"{text}\"")
}

/// Compact chat chips precede the existing message body. Exact row-note
/// framing remains independent and is already projected by its own owner.
pub fn chat_body(passages: &[&str], message: &str) -> String {
    if passages.is_empty() {
        return message.to_owned();
    }
    let mut text = passages
        .iter()
        .map(|text| inline_quote(text))
        .collect::<Vec<_>>()
        .join("\n");
    if !message.is_empty() {
        text.push_str("\n\n");
        text.push_str(message);
    }
    text
}

/// Freeze one delivered body. An unselected row keeps the historical inline
/// tag; selected passages precede the exact comment as Markdown blockquotes.
pub fn body(context: Option<(&str, &str)>, passages: &[&str], comment: &str) -> String {
    let mut body = context.map_or_else(String::new, |(squad, row)| tag(squad, row));
    if passages.is_empty() {
        body.push_str(comment);
        return body;
    }
    if context.is_some() {
        body.pop(); // Replace the tag's one comment separator with a new line.
        body.push('\n');
    }
    for (index, passage) in passages.iter().enumerate() {
        if index > 0 {
            body.push_str("\n\n");
        }
        body.push_str(&blockquote(passage));
    }
    if !comment.is_empty() {
        body.push_str("\n\n");
        body.push_str(comment);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_passages_keep_literal_lines_quotes_and_unicode() {
        assert_eq!(
            body(
                Some(("product", "worker")),
                &["first \"line\"\n\n漢字 e\u{301}\n"],
                "  my comment\nnext  "
            ),
            "[product · worker]\n> first \"line\"\n>\n> 漢字 e\u{301}\n>\n\n  my comment\nnext  "
        );
        assert_eq!(
            body(Some(("Common pool", "worker")), &["one", "two"], ""),
            "[Common pool · worker]\n> one\n\n> two"
        );
        assert_eq!(blockquote("a\n\n"), "> a\n>\n>");
    }

    #[test]
    fn compact_chat_quotes_flatten_lines_and_bound_unicode_without_an_author() {
        assert_eq!(
            inline_quote("first \"line\"\r\n漢字\nlast\r"),
            "> \"first \"line\" 漢字 last \""
        );
        assert_eq!(inline_quote(""), "> \"\"");
        assert_eq!(
            inline_quote(&"漢".repeat(200)),
            format!("> \"{}\"", "漢".repeat(200))
        );
        assert_eq!(
            inline_quote(&"漢".repeat(201)),
            format!("> \"{}…\"", "漢".repeat(199))
        );
        assert_eq!(blockquote("first\nsecond"), "> first\n> second");
    }

    #[test]
    fn unselected_or_non_row_messages_keep_their_existing_text() {
        assert_eq!(
            body(Some(("product", "worker")), &[], "  comment  "),
            "[product · worker]   comment  "
        );
        assert_eq!(body(None, &[], "  comment\nnext  "), "  comment\nnext  ");
        assert_eq!(body(None, &["source"], "comment"), "> source\n\ncomment");
    }
}
