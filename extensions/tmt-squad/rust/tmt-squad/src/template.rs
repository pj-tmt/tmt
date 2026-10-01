//! `{field}` templates: literal text and placeholders that together form one
//! value. A filled value is never re-split, re-quoted or shell-parsed.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(String),
}

/// Literal text and `{field}` placeholders that together form one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template(Vec<Part>);

pub const DEFAULT_COPY: &str = "{name}: {task} ({state})";

fn field_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

impl Template {
    /// The text of a template without placeholders.
    pub fn literal(&self) -> Option<&str> {
        match self.0.as_slice() {
            [Part::Text(text)] => Some(text),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            if rest[..start].contains('}') {
                return Err("unmatched '}'".into());
            }
            if start > 0 {
                parts.push(Part::Text(rest[..start].to_owned()));
            }
            let end = rest[start..].find('}').ok_or("unclosed '{'")? + start;
            let name = &rest[start + 1..end];
            if !field_name(name) {
                return Err(format!("'{{{name}}}' is not a field name"));
            }
            parts.push(Part::Field(name.to_owned()));
            rest = &rest[end + 1..];
        }
        if rest.contains('}') {
            return Err("unmatched '}'".into());
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        Ok(Self(parts))
    }

    /// Fills one program argument. A value placed at the start that begins
    /// with `-` is refused: row fields are written by agents, and such a
    /// value would reach the program as an option. A literal `-q` is fine.
    pub fn fill_argument(&self, row: &Value) -> Result<String, String> {
        let filled = self.fill(row)?;
        match self.0.first() {
            Some(Part::Field(field)) if filled.starts_with('-') => Err(format!(
                "{field} starts with '-' and would be read as an option; refused"
            )),
            _ => Ok(filled),
        }
    }

    /// Fills every field from the row. A missing or empty value refuses the
    /// action rather than producing an empty or shifted argument.
    pub fn fill(&self, row: &Value) -> Result<String, String> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(field) => {
                    let value = field_value(row, field)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| format!("{field} is empty for this row"))?;
                    out.push_str(value);
                }
            }
        }
        Ok(out)
    }
}

/// A row's value for a field: identity basics, location, activity, or any
/// squad field.
pub fn field_value<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    match field {
        "name" | "id" | "lifetime" | "presence" | "state" | "pending" | "note" => {
            row[field].as_str()
        }
        "member" => row["name"].as_str(),
        "pane" => row["pane"]["id"].as_str(),
        "target" => row["pane"]["target"].as_str(),
        "cwd" => row["pane"]["cwd"].as_str(),
        "activity" => row["activity"]["activity"].as_str(),
        other => row["fields"][other].as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_fill_whole_or_embedded_and_missing_values_refuse() {
        let row = json!({
            "name": "auth-fix", "state": "blocked", "pending": null,
            "pane": {"id": "%5", "cwd": "/w/app 3"},
            "activity": {"activity": "editing"},
            "fields": {"task": "rotate; $(rm -rf ~)", "pr_link": "https://example.com/pull/412"}
        });
        let fill = |text: &str| Template::parse(text).unwrap().fill(&row);
        assert_eq!(
            fill(DEFAULT_COPY).unwrap(),
            "auth-fix: rotate; $(rm -rf ~) (blocked)"
        );
        assert_eq!(
            fill("{member}@{pane} in {cwd}: {activity}").unwrap(),
            "auth-fix@%5 in /w/app 3: editing"
        );
        assert_eq!(
            fill("[{name}]({pr_link})").unwrap(),
            "[auth-fix](https://example.com/pull/412)"
        );
        assert_eq!(
            fill("{pending}").unwrap_err(),
            "pending is empty for this row"
        );
        assert_eq!(
            fill("{worktree}").unwrap_err(),
            "worktree is empty for this row"
        );
        assert_eq!(Template::parse("code").unwrap().literal(), Some("code"));
        assert_eq!(Template::parse("{name}").unwrap().literal(), None);
        for bad in ["{Bad}", "{two words}", "{unclosed", "close}", "{}", "a}{b"] {
            assert!(Template::parse(bad).is_err(), "{bad}");
        }
    }

    /// An agent-written value never becomes an option; literal options and
    /// a `-` inside a value stay as they are.
    #[test]
    fn an_argument_refuses_a_leading_dash_only_from_a_value() {
        let row = json!({
            "name": "rin",
            "fields": {"pr_link": "--repo=evil/x", "task": "a -b", "branch": "-x"}
        });
        let argument = |text: &str| Template::parse(text).unwrap().fill_argument(&row);
        assert_eq!(
            argument("{pr_link}").unwrap_err(),
            "pr_link starts with '-' and would be read as an option; refused"
        );
        assert!(argument("{branch}").is_err());
        assert_eq!(argument("--json").unwrap(), "--json");
        assert_eq!(argument("--head={branch}").unwrap(), "--head=-x");
        assert_eq!(argument("{task}").unwrap(), "a -b");
        assert_eq!(argument("{name}").unwrap(), "rin");
        // Copy text is not an argument: it keeps any value.
        assert_eq!(
            Template::parse("{pr_link}").unwrap().fill(&row).unwrap(),
            "--repo=evil/x"
        );
    }
}
