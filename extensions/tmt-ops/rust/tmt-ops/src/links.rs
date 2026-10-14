//! Authored handler admission and inert response-link styling. This module
//! never constructs an effect from an agent-provided destination.
use crate::action::Action;
use std::collections::BTreeMap;

pub type Handlers = BTreeMap<String, Action>;

pub fn scheme(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase()
                || (i > 0 && (b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')))
        })
}

/// Whether a built-in destination is recognized for text styling only.
/// Custom handlers remain authored Config data, not response activation routes.
pub fn styled_target(target: &str) -> bool {
    if target.is_empty() || target.len() > 4096 || target.chars().any(char::is_control) {
        return false;
    }
    if crate::effects::web_link(target).is_ok() {
        return true;
    }
    if target.starts_with('/') {
        return decode(target).is_some();
    }
    if let Some(path) = target.strip_prefix("file://") {
        return path.starts_with('/') && decode(path).is_some();
    }
    let Some(path) = target.strip_prefix("tmt:") else {
        return false;
    };
    let (path, text) = path.split_once("?text=").unwrap_or((path, ""));
    let (verb, member) = path
        .split_once('/')
        .map_or((path, None), |(v, m)| (v, Some(m)));
    if !matches!(
        verb,
        "jump" | "back" | "talk" | "answer" | "open" | "copy" | "annotate"
    ) || (verb == "back" && (member.is_some() || !text.is_empty()))
        || (verb != "back" && member.is_none())
        || member.is_some_and(|member| {
            member.is_empty()
                || member.contains(['/', '?', '#', '%'])
                || member.chars().any(char::is_whitespace)
        })
        || (!matches!(verb, "talk" | "answer" | "annotate") && !text.is_empty())
    {
        return false;
    }
    decode(text).is_some()
}

fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut rest = text.as_bytes().iter().copied();
    while let Some(b) = rest.next() {
        bytes.push(if b == b'%' {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            hex(rest.next()?)? * 16 + hex(rest.next()?)?
        } else {
            b
        });
    }
    let text = String::from_utf8(bytes).ok()?;
    (text.chars().count() <= 4000 && !text.chars().any(char::is_control)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_destinations_have_only_inert_style_recognition() {
        for target in [
            "javascript:alert(1)",
            "gh:42",
            "tmt:run/id",
            "tmt:jump/other/path",
            "tmt:talk/-x?text=%0A",
            "tmt:back/name",
            "#412",
            "file://host/etc/passwd",
            "./relative",
            "../relative",
            "file:///%GG",
            "tmt:talk/id?text=%FF",
            "tmt:talk/id?text=%",
            "https://example.test/\u{1b}",
        ] {
            assert!(!styled_target(target), "{target}");
        }
        for target in [
            "https://github.com/o/r/pull/42",
            "https://github.com.evil/o/r/pull/42",
            "/my%20file",
            "file:///my%20file",
            "tmt:back",
            "tmt:answer/auth-fix?text=hello%20%24%28id%29",
        ] {
            assert!(styled_target(target), "{target}");
        }
        assert!(!styled_target(&format!(
            "https://example.test/{}",
            "x".repeat(4096)
        )));
        assert!(!styled_target(&format!(
            "tmt:talk/id?text={}",
            "x".repeat(4001)
        )));
    }
}
