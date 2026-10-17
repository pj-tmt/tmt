//! Shared row-message framing. Callers own admission,
//! source identity and byte ranges; this projection neither reads nor clips them.

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
    fn unselected_or_non_row_messages_keep_their_existing_text() {
        assert_eq!(
            body(Some(("product", "worker")), &[], "  comment  "),
            "[product · worker]   comment  "
        );
        assert_eq!(body(None, &[], "  comment\nnext  "), "  comment\nnext  ");
        assert_eq!(body(None, &["source"], "comment"), "> source\n\ncomment");
    }
}
