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
    // The root locale is intentional: environment locale must not change keys.
    CaseMapper::new()
        .lowercase_to_string(&target_text(value), &LanguageIdentifier::UNKNOWN)
        .into_owned()
}

/// Text as a host reads it for target syntax: trimmed and composed like a
/// key, with its case kept. A host's literals are lowercase and its numbers
/// may be uppercase (Herdr's `w1:p2E`), so a lowercased key would hide them.
pub fn target_text(value: &str) -> String {
    let trimmed = value.trim_matches(ecmascript_space);
    ComposingNormalizer::new_nfkc()
        .normalize(trimmed)
        .into_owned()
}

/// Text that any host reads as one of its pane targets; such text is never a
/// name. Each host owns its syntax ([`HostKind::is_target`]).
pub fn is_pane_target(value: &str) -> bool {
    let text = target_text(value);
    HostKind::all().any(|host| host.is_target(&text))
}

/// Whether a name's text or its key reads as a pane target. A name is found
/// by its key, so `W1:P2` would answer for the pane `w1:p2`.
pub fn names_pane_target(value: &str) -> bool {
    is_pane_target(value) || is_pane_target(&normalize_name(value))
}

/// A name a new identity (or a rename) may take: never one whose text or key
/// a host reads as a pane target.
pub fn validate_name(value: &str) -> Result<ValidatedName, NameError> {
    let name = validate_existing_name(value)?;
    if names_pane_target(value) {
        return Err(NameError::PaneTarget);
    }
    Ok(name)
}

/// A name an existing identity may already hold. A host whose driver is
/// approved later (Herdr's `wN:pM`, #479) makes some earlier names look like
/// targets; such names stay valid for the identities that hold them and are
/// refused only as new names.
pub fn validate_existing_name(value: &str) -> Result<ValidatedName, NameError> {
    let display_name = value.trim_matches(ecmascript_space);
    if display_name.is_empty() || display_name.chars().any(|c| c < '\u{20}' || c == '\u{7f}') {
        return Err(NameError::EmptyOrControl);
    }
    let canonical_name = normalize_name(value);
    if HostKind::all()
        .any(|host| host.targets_never_named_identities() && host.is_target(&canonical_name))
    {
        return Err(NameError::PaneTarget);
    }
    Ok(ValidatedName {
        display_name: display_name.into(),
        canonical_name,
    })
}

#[cfg(test)]
mod tests;
