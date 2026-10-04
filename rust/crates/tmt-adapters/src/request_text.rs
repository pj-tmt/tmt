//! Pure normalization shared by request previews and reply notice presentation.

/// Normalize line endings (including Unicode separators) to LF and other
/// controls to spaces. Callers choose a single line or frame multiline text as data.
pub(crate) fn normalized(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        loop {
            return Some(match chars.next()? {
                '\r' if chars.peek() == Some(&'\n') => continue,
                '\r' | '\n' | '\u{2028}' | '\u{2029}' => '\n',
                c if c.is_control() => ' ',
                c => c,
            });
        }
    })
}
