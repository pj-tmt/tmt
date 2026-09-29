//! Identity keys follow ECMAScript trim -> NFKC -> default lowercase.
//! Fixed ICU data avoids compiler-dependent casing of persistent names.

use icu_casemap::CaseMapper;
use icu_locale_core::LanguageIdentifier;
use icu_normalizer::ComposingNormalizer;
use std::{error::Error, fmt};

use crate::host::HostKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedName {
    display_name: String,
    canonical_name: String,
}

impl ValidatedName {
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    EmptyOrControl,
    PaneTarget,
}

impl fmt::Display for NameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyOrControl => {
                "Identity name must not be empty or contain control characters."
            }
            Self::PaneTarget => "Identity name must not look like a pane target.",
        })
    }
}

impl Error for NameError {}

// ECMAScript WhiteSpace plus LineTerminator, also used by its regular-expression
// \s. Rust's is_whitespace differs: it includes NEL and excludes BOM.
pub(crate) fn ecmascript_space(character: char) -> bool {
    matches!(character,
        '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' |
        '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub fn normalize_name(value: &str) -> String {
    let trimmed = value.trim_matches(ecmascript_space);
    let normalized = ComposingNormalizer::new_nfkc().normalize(trimmed);
    // The root locale is intentional: environment locale must not change keys.
    CaseMapper::new()
        .lowercase_to_string(&normalized, &LanguageIdentifier::UNKNOWN)
        .into_owned()
}

/// Text that any host reads as one of its pane targets; such text is never a
/// name. Each host owns its syntax ([`HostKind::is_target`]).
pub fn is_pane_target(value: &str) -> bool {
    let canonical = normalize_name(value);
    HostKind::ALL
        .into_iter()
        .any(|host| host.is_target(&canonical))
}

pub fn validate_name(value: &str) -> Result<ValidatedName, NameError> {
    let display_name = value.trim_matches(ecmascript_space);
    if display_name.is_empty() || display_name.chars().any(|c| c < '\u{20}' || c == '\u{7f}') {
        return Err(NameError::EmptyOrControl);
    }
    let canonical_name = normalize_name(value);
    if is_pane_target(&canonical_name) {
        return Err(NameError::PaneTarget);
    }
    Ok(ValidatedName {
        display_name: display_name.into(),
        canonical_name,
    })
}

#[cfg(test)]
mod tests;
