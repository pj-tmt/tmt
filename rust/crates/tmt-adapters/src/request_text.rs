//! Display-control policy shared by label validation and request presentation.

/// Normalize line endings (including Unicode separators) to LF and other
/// display controls to spaces. Callers choose a single line or frame multiline text as data.
pub(crate) fn normalized(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        loop {
            return Some(match chars.next()? {
                '\r' if chars.peek() == Some(&'\n') => continue,
                '\r' | '\n' | '\u{2028}' | '\u{2029}' => '\n',
                c if display_control(c) => ' ',
                c => c,
            });
        }
    })
}

/// Controls that can alter terminal framing or text direction. Script joiners
/// and variation selectors remain ordinary data.
pub(crate) fn display_control(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}
