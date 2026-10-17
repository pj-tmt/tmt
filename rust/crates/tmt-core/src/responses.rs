//! Bounded, untrusted provider-final text. This is pane output, never a reply.

pub const MAX_TEXT_BYTES: usize = 8192;
pub const MAX_RECENT: u64 = 10;

/// Kept out of debug output as well as opaque driver state.
#[derive(Clone, PartialEq, Eq)]
pub struct FinalText {
    text: String,
    truncated: bool,
}
impl std::fmt::Debug for FinalText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinalText")
            .field("bytes", &self.text.len())
            .field("truncated", &self.truncated)
            .finish_non_exhaustive()
    }
}
impl FinalText {
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let mut end = text.len().min(MAX_TEXT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Some(Self {
            text: text[..end].to_owned(),
            truncated: end < text.len(),
        })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_cap_preserves_scalars_and_line_endings_without_a_marker() {
        let text = format!("{}🦀tail", "x".repeat(MAX_TEXT_BYTES - 2));
        let bounded = FinalText::new(&text).unwrap();
        assert_eq!(bounded.text(), "x".repeat(MAX_TEXT_BYTES - 2));
        assert!(bounded.truncated());
        let exact = format!("{}🦀", "x".repeat(MAX_TEXT_BYTES - 4));
        assert_eq!(FinalText::new(&exact).unwrap().text(), exact);
        assert!(!FinalText::new(&exact).unwrap().truncated());
        assert_eq!(FinalText::new("a\r\nb\n").unwrap().text(), "a\r\nb\n");
        assert!(FinalText::new("").is_none());
        assert!(
            !format!("{:?}", FinalText::new("private final text").unwrap())
                .contains("private final text")
        );
    }
}
