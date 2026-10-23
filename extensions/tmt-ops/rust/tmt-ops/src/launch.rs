//! Published host launch argv. Only declared whole-word slots are substituted.
use crate::template::Template;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    pub state: String,
    pub argv: Option<Vec<String>>,
}
impl Host {
    pub fn eligible(&self) -> bool {
        matches!(self.state.as_str(), "builtin" | "ok") && self.argv.is_some()
    }
}

pub fn hosts(document: &Value) -> Result<Vec<Host>, String> {
    if document["version"] != 1 {
        return Err("Unsupported host template version.".into());
    }
    let mut result = Vec::new();
    for host in document["hosts"]
        .as_array()
        .ok_or("Host templates unavailable.")?
    {
        let name = host["host"]
            .as_str()
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
            .ok_or("Invalid host name.")?;
        if result.iter().any(|h: &Host| h.name == name) {
            return Err("Duplicate host template.".into());
        }
        let state = host["state"].as_str().ok_or("Missing host state.")?;
        let launch = host.get("launch").ok_or("Missing launch observation.")?;
        let argv = if launch.is_null() {
            None
        } else {
            if !matches!(state, "builtin" | "ok") {
                return Err("Unavailable host published a launch template.".into());
            }
            let slots = host["launch"]["placeholders"]
                .as_array()
                .ok_or("Missing launch placeholders.")?;
            if slots.len() != 2
                || !slots.contains(&Value::from("session"))
                || !slots.contains(&Value::from("command"))
            {
                return Err("Unknown launch placeholders.".into());
            }
            let argv = words(&host["launch"]["argv"])?;
            validate(&argv)?;
            Some(argv)
        };
        result.push(Host {
            name: name.into(),
            state: state.into(),
            argv,
        });
    }
    Ok(result)
}
pub fn words(value: &Value) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or("Launch argv must be a JSON array.")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Launch argv words must be strings.".into())
        })
        .collect()
}
pub fn validate(argv: &[String]) -> Result<(), String> {
    if !(2..=16).contains(&argv.len())
        || argv.iter().map(String::len).sum::<usize>() > 4096
        || argv
            .iter()
            .any(|word| word.is_empty() || word.len() > 1024 || word.chars().any(char::is_control))
    {
        return Err("Launch argv requires 2–16 nonempty words, at most 1024 bytes each and 4096 total, without control characters.".into());
    }
    if argv[0].contains(['{', '}']) || argv[0].starts_with('-') {
        return Err("Launch program must be literal.".into());
    }
    if argv
        .iter()
        .filter(|word| word.as_str() == "{command}")
        .count()
        != 1
        || !argv.iter().any(|word| word == "{session}")
    {
        return Err("Launch argv needs exactly one whole {command} word and at least one whole {session} word.".into());
    }
    for word in argv {
        if (word.contains("{command}") && word != "{command}")
            || (word.contains("{session}") && word != "{session}")
        {
            return Err("Launch slots must occupy whole argv words.".into());
        }
        // Host scripts may contain other braces (including #{pane_id}). Only
        // a whole unknown placeholder is ambiguous and must be refused.
        if word.starts_with('{')
            && word.ends_with('}')
            && !matches!(word.as_str(), "{session}" | "{command}")
            && word[1..word.len() - 1]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("Unknown whole-word launch placeholder.".into());
        }
    }
    Ok(())
}
pub fn fill(argv: &[String], session: &str, command: &str) -> Result<Vec<String>, String> {
    validate(argv)?;
    if session.is_empty() || session.contains(['.', ':']) || session.chars().any(char::is_control) {
        return Err("Session must be nonempty, without '.', ':' or control characters.".into());
    }
    argv.iter()
        .map(|word| {
            Template::whole_word(word, &["session", "command"]).fill_with(|slot| match slot {
                "session" => Ok(session.into()),
                "command" if !command.is_empty() => Ok(command.into()),
                _ => Err("Missing launch value.".into()),
            })
        })
        .collect()
}

/// POSIX shell display, also used for the one command string the host consumes.
pub fn shell_word(word: &str) -> String {
    if !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
    {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}
pub fn line(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shell_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
