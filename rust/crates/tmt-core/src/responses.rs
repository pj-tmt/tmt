//! Bounded, untrusted provider-final text. This is pane output, never a reply.

/// An answer is kept to this many UTF-8 bytes; a longer one keeps its first bytes, says it was
/// cut and remembers how long it was (#4914).
pub const MAX_TEXT_BYTES: usize = 16_384;
/// An agent's answers stay readable this long after the turn that gave them.
pub const RETENTION_MS: u64 = 7 * 86_400_000;
/// At most this many answers are kept per agent, oldest pruned first.
pub const MAX_KEPT: u64 = 1_000;
/// One read returns at most this many answers; with no `limit` it returns [`DEFAULT_PAGE`].
pub const MAX_PAGE: u64 = 50;
pub const DEFAULT_PAGE: u64 = 10;

/// The name of a retained final, made from its row key in the store (#4993). It is opaque: a
/// reader compares two for equality and infers nothing else from one. The key is never reused,
/// so the name of a pruned final resolves to nothing, not to another final.
pub fn response_id(sequence: u64) -> String {
    format!("rsp_{sequence}")
}

/// `text` cut to at most `max` bytes on a Unicode scalar boundary.
pub fn cut_scalars(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Kept out of debug output as well as opaque driver state.
#[derive(Clone, PartialEq, Eq)]
pub struct FinalText {
    text: String,
    truncated: bool,
    original_bytes: usize,
    digest: Option<String>,
}
impl std::fmt::Debug for FinalText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinalText")
            .field("bytes", &self.text.len())
            .field("truncated", &self.truncated)
            .field("original_bytes", &self.original_bytes)
            .finish_non_exhaustive()
    }
}
impl FinalText {
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let kept = cut_scalars(text, MAX_TEXT_BYTES);
        Some(Self {
            text: kept.to_owned(),
            truncated: kept.len() < text.len(),
            original_bytes: text.len(),
            digest: Some(crate::content_digest::sha256(text.as_bytes())),
        })
    }
    /// Admit already bounded durable text without losing its truncation provenance.
    pub fn from_kept(text: &str, original_bytes: usize, digest: Option<&str>) -> Option<Self> {
        if text.is_empty()
            || original_bytes as u128 > crate::limits::MAX_JS_SAFE_INTEGER as u128
            || text.len() > MAX_TEXT_BYTES
            || original_bytes < text.len()
            || (original_bytes > text.len() && text.len() < MAX_TEXT_BYTES - 3)
        {
            return None;
        }
        if digest.is_some_and(|value| {
            !crate::content_digest::is_sha256(value)
                || (original_bytes == text.len()
                    && value != crate::content_digest::sha256(text.as_bytes()))
        }) {
            return None;
        }
        Some(Self {
            text: text.into(),
            truncated: original_bytes > text.len(),
            original_bytes,
            digest: digest.map(str::to_owned).or_else(|| {
                (original_bytes == text.len())
                    .then(|| crate::content_digest::sha256(text.as_bytes()))
            }),
        })
    }
    /// Private identity of the full provider text, computed before truncation.
    /// Legacy truncated captures cannot prove equality and retain no digest.
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    /// The length the provider gave, in UTF-8 bytes, before any cut.
    pub fn original_bytes(&self) -> usize {
        self.original_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_final_identity_survives_truncation_and_legacy_uncertainty_is_explicit() {
        let prefix = "x".repeat(MAX_TEXT_BYTES);
        let a = FinalText::new(&format!("{prefix}A")).unwrap();
        let b = FinalText::new(&format!("{prefix}B")).unwrap();
        assert_eq!(a.text(), b.text());
        assert_eq!(a.original_bytes(), b.original_bytes());
        assert_ne!(a.digest(), b.digest());
        let restored = FinalText::from_kept(a.text(), a.original_bytes(), a.digest()).unwrap();
        assert_eq!(restored, a);
        assert!(
            FinalText::from_kept(a.text(), a.original_bytes(), None)
                .unwrap()
                .digest()
                .is_none()
        );
        let short = FinalText::new("complete").unwrap();
        assert_eq!(FinalText::from_kept("complete", 8, None).unwrap(), short);
        assert!(FinalText::from_kept("complete", 8, a.digest()).is_none());
        assert!(FinalText::from_kept(a.text(), a.original_bytes(), Some("invalid")).is_none());
    }

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
        assert_eq!(bounded.original_bytes(), text.len());
        assert_eq!(
            FinalText::new(&exact).unwrap().original_bytes(),
            exact.len()
        );
        assert!(
            !format!("{:?}", FinalText::new("private final text").unwrap())
                .contains("private final text")
        );
    }
}
