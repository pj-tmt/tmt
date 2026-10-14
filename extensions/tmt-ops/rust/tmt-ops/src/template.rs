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
    /// Display the original argument without filling or executing it.
    pub fn source(&self) -> String {
        self.0
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.replace('{', "{{").replace('}', "}}"),
                Part::Field(field) => format!("{{{field}}}"),
            })
            .collect()
    }

    /// The text of a template without placeholders.
    pub fn literal(&self) -> Option<&str> {
        match self.0.as_slice() {
            [Part::Text(text)] => Some(text),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(character) = chars.next() {
            match character {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    if !literal.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut literal)));
                    }
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => name.push(c),
                            None => return Err("unclosed '{'".into()),
                        }
                    }
                    if !field_name(&name) {
                        return Err(format!("'{{{name}}}' is not a field name"));
                    }
                    parts.push(Part::Field(name));
                }
                '}' => return Err("unmatched '}'".into()),
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        Ok(Self(parts))
    }

    /// Fills one program argument. A value placed at the start that begins
    /// with `-` is refused: row fields are written by agents, and such a
    /// value would reach the program as an option. A literal `-q` is fine.
    pub fn fill_argument(&self, row: &Value) -> Result<String, String> {
        self.fill_argument_with_board_pane(row, None)
    }

    pub fn uses_field(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|part| matches!(part, Part::Field(field) if field == name))
    }

    pub fn fill_argument_with_board_pane(
        &self,
        row: &Value,
        board_pane: Option<&str>,
    ) -> Result<String, String> {
        let filled = self.fill_with_board_pane(row, board_pane)?;
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
        self.fill_with_board_pane(row, None)
    }

    pub fn fill_with_board_pane(
        &self,
        row: &Value,
        board_pane: Option<&str>,
    ) -> Result<String, String> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(field) => {
                    let value = (if field == "board_pane" {
                        board_pane
                    } else {
                        field_value(row, field)
                    })
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
        "board_pane" => None, // Reserved invocation context is never agent-authored metadata.
        "name" | "id" | "lifetime" | "presence" | "state" | "pending" | "note" => {
            row[field].as_str()
        }
        "member" => row["name"].as_str(),
        "pane" => row["pane"]["id"].as_str(),
        "target" => row["pane"]["target"].as_str(),
        "cwd" => row["pane"]["cwd"].as_str(),
        // Core's published `location`; never parsed out of `target`.
        "host" | "session" | "window" => row["pane"]["location"][field].as_str(),
        "activity" => row["activity"]["activity"].as_str(),
        other => row["fields"][other].as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literal_braces_round_trip_without_becoming_placeholders() {
        let row = json!({"pane": {"id": "%5"}});
        for (source, filled) in [
            ("{{left-of}}", "{left-of}"),
            ("{{{pane}}}", "{%5}"),
            ("x}}y{{z", "x}y{z"),
        ] {
            let template = Template::parse(source).unwrap();
            assert_eq!(template.fill(&row).unwrap(), filled);
            assert_eq!(Template::parse(&template.source()).unwrap(), template);
        }
    }

    #[test]
    fn board_pane_requires_supplied_context_and_never_reads_agent_metadata() {
        let row = json!({"board_pane": "%99", "fields": {"board_pane": "%88"}, "name": "a; $(touch /never)"});
        let template = Template::parse("{board_pane}").unwrap();
        assert!(template.uses_field("board_pane"));
        assert_eq!(
            template.fill(&row).unwrap_err(),
            "board_pane is empty for this row"
        );
        assert_eq!(
            template.fill_with_board_pane(&row, Some("%7")).unwrap(),
            "%7"
        );
        assert!(template.fill_with_board_pane(&row, Some("")).is_err());
        assert!(
            template
                .fill_argument_with_board_pane(&row, Some("--bad"))
                .is_err()
        );
        assert_eq!(
            Template::parse("--board={board_pane}")
                .unwrap()
                .fill_argument_with_board_pane(&row, Some("%7"))
                .unwrap(),
            "--board=%7"
        );
    }

    #[test]
    fn host_session_and_window_fill_only_from_the_published_location() {
        let row = json!({
            "name": "a",
            "pane": {
                "id": "%5", "target": "ignored:9.9",
                "location": {"host": "tmux", "session": "my session", "window": "2"}
            },
            "fields": {"session": "a squad field", "window": "also a squad field"}
        });
        let fill = |text: &str| Template::parse(text).unwrap().fill(&row);
        assert_eq!(fill("{host}"), Ok("tmux".into()));
        assert_eq!(fill("{session}:{window}"), Ok("my session:2".into()));
        // A host that reports no coordinates, and a row with no location, refuse.
        for pane in [
            json!({"id": "%5", "target": "work:2.1", "location": {"host": "herdr"}}),
            json!({"id": "%5", "target": "work:2.1"}),
        ] {
            let row = json!({"name": "a", "pane": pane, "fields": {"session": "ignored"}});
            let fill = |text: &str| Template::parse(text).unwrap().fill(&row);
            assert_eq!(
                fill("{session}"),
                Err("session is empty for this row".into())
            );
            assert_eq!(fill("{window}"), Err("window is empty for this row".into()));
        }
    }

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
