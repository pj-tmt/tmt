//! Notebook destinations. Only user configuration grants program authority.
use crate::action::{Action, Verb};
use serde_json::json;
use std::collections::BTreeMap;

pub type Handlers = BTreeMap<String, Action>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Web,
    Github,
    File(String),
    Tmt {
        verb: Verb,
        member: Option<String>,
        text: String,
    },
    Custom {
        scheme: String,
        path: String,
    },
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Github => "github",
            Self::File(_) => "file",
            Self::Tmt {
                verb: Verb::Reply, ..
            } => "answer",
            Self::Tmt { verb, .. } => verb.name(),
            Self::Custom { .. } => "configured",
        }
    }
}

pub fn scheme(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase()
                || (i > 0 && (b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')))
        })
}

pub fn classify(target: &str, handlers: &Handlers) -> Option<Kind> {
    if target.is_empty() || target.len() > 4096 || target.chars().any(char::is_control) {
        return None;
    }
    if crate::effects::web_link(target).is_ok() {
        let path = target
            .strip_prefix("https://github.com/")
            .or_else(|| target.strip_prefix("http://github.com/"));
        let github = path.is_some_and(|path| {
            let parts: Vec<_> = path.split('/').collect();
            parts.len() >= 4
                && !parts[0].is_empty()
                && !parts[1].is_empty()
                && matches!(parts[2], "issues" | "pull")
                && parts[3]
                    .split(['?', '#'])
                    .next()
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        });
        return Some(if github { Kind::Github } else { Kind::Web });
    }
    if target.starts_with('/') {
        return Some(Kind::File(decode(target)?));
    }
    if let Some(path) = target.strip_prefix("file://") {
        return path
            .starts_with('/')
            .then(|| decode(path))
            .flatten()
            .map(Kind::File);
    }
    let (name, path) = target.split_once(':')?;
    if name == "tmt" {
        let (path, text) = path.split_once("?text=").unwrap_or((path, ""));
        let (verb, member) = path
            .split_once('/')
            .map_or((path, None), |(v, m)| (v, Some(m)));
        let verb = match verb {
            "jump" => Verb::Jump,
            "back" => Verb::Back,
            "talk" => Verb::Talk,
            "answer" => Verb::Reply,
            "open" => Verb::Open,
            "copy" => Verb::Copy,
            "annotate" => Verb::Annotate,
            _ => return None,
        };
        if (verb == Verb::Back && (member.is_some() || !text.is_empty()))
            || (verb != Verb::Back && member.is_none())
            || member.is_some_and(|m| {
                m.is_empty()
                    || m.contains(['/', '?', '#', '%'])
                    || m.chars().any(char::is_whitespace)
            })
            || (!matches!(verb, Verb::Talk | Verb::Reply | Verb::Annotate) && !text.is_empty())
        {
            return None;
        }
        // Query data is one message value, never parsed as command arguments.
        let text = decode(text)?;
        return Some(Kind::Tmt {
            verb,
            member: member.map(str::to_owned),
            text,
        });
    }
    handlers.contains_key(name).then(|| Kind::Custom {
        scheme: name.to_owned(),
        path: path.to_owned(),
    })
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

pub fn argv(handlers: &Handlers, scheme: &str, path: &str) -> Result<Vec<String>, String> {
    handlers
        .get(scheme)
        .ok_or("The link handler is no longer configured.")?
        .argv(&json!({"fields":{"path":path}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schemes_and_fixed_verbs_have_no_program_authority_from_notes() {
        let mut handlers = Handlers::new();
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
        ] {
            assert!(classify(target, &handlers).is_none(), "{target}");
        }
        assert_eq!(
            classify("https://github.com/o/r/pull/42", &handlers),
            Some(Kind::Github)
        );
        assert_eq!(
            classify("https://github.com.evil/o/r/pull/42", &handlers),
            Some(Kind::Web)
        );
        assert_eq!(
            classify("/my%20file", &handlers),
            Some(Kind::File("/my file".into()))
        );
        assert_eq!(
            classify("tmt:answer/auth-fix?text=hello%20%24%28id%29", &handlers),
            Some(Kind::Tmt {
                verb: Verb::Reply,
                member: Some("auth-fix".into()),
                text: "hello $(id)".into()
            })
        );
        handlers.insert(
            "gh".into(),
            Action::parse("run gh issue view {path}").unwrap(),
        );
        assert!(matches!(
            classify("gh:42", &handlers),
            Some(Kind::Custom { .. })
        ));
        assert_eq!(
            argv(&handlers, "gh", "a b;$(id)").unwrap(),
            ["gh", "issue", "view", "a b;$(id)"]
        );
        assert!(argv(&handlers, "gh", "--repo=evil").is_err());
        handlers.clear();
        assert!(argv(&handlers, "gh", "42").is_err());
    }
}
