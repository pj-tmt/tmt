//! `{field}` templates: literal text and placeholders that together form one
//! value. A filled value is never re-split, re-quoted or shell-parsed.

use serde_json::Value;
use std::borrow::Cow;

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
    if let Some(path) = name
        .strip_prefix("row:")
        .or_else(|| name.strip_prefix("ops:"))
    {
        return path.starts_with('/')
            && !path.chars().any(char::is_control)
            && path
                .split('~')
                .skip(1)
                .all(|part| part.starts_with(['0', '1']));
    }
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
        self.argument_parameters(Parameters {
            row,
            ops: None,
            board_pane,
        })
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
        self.fill_parameters(Parameters {
            row,
            ops: None,
            board_pane,
        })
    }

    pub fn fill_parameters(&self, parameters: Parameters<'_>) -> Result<String, String> {
        self.fill_with(|field| parameters.resolve(field).map(Cow::into_owned))
    }

    /// Host templates declare whole-word slots; every other word is opaque,
    /// including shell braces and host format expressions.
    pub fn whole_word(text: &str, slots: &[&str]) -> Self {
        match slots.iter().find(|slot| text == format!("{{{slot}}}")) {
            Some(slot) => Self(vec![Part::Field((*slot).into())]),
            None => Self(vec![Part::Text(text.into())]),
        }
    }

    pub fn fill_with(
        &self,
        mut resolve: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<String, String> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(field) => out.push_str(&resolve(field)?),
            }
        }
        Ok(out)
    }

    pub fn argument_parameters(&self, parameters: Parameters<'_>) -> Result<String, String> {
        let filled = self.fill_parameters(parameters)?;
        match self.0.first() {
            Some(Part::Field(field)) if filled.starts_with('-') => Err(format!(
                "{field} starts with '-' and would be read as an option; refused"
            )),
            _ => Ok(filled),
        }
    }
}

/// Held observations only. Caller pane evidence never comes from either document.
#[derive(Clone, Copy)]
pub struct Parameters<'a> {
    pub row: &'a Value,
    pub ops: Option<&'a Value>,
    pub board_pane: Option<&'a str>,
}

impl Parameters<'_> {
    pub fn resolve(&self, field: &str) -> Result<Cow<'_, str>, String> {
        let missing = || format!("{field} is unavailable for this row");
        if field == "board_pane" {
            return self
                .board_pane
                .filter(|value| !value.is_empty())
                .map(Cow::Borrowed)
                .ok_or_else(|| "board_pane is empty for this row".into());
        }
        let pointed = if let Some(path) = field.strip_prefix("row:") {
            Some((self.row, path))
        } else if let Some(path) = field.strip_prefix("ops:") {
            Some((self.ops.ok_or_else(missing)?, path))
        } else {
            None
        };
        if let Some((document, path)) = pointed {
            if !parameter_path(path) || failed_path(document, path) {
                return Err(missing());
            }
            return match document.pointer(path) {
                Some(Value::String(value)) if !value.is_empty() => Ok(Cow::Borrowed(value)),
                Some(value @ (Value::Number(_) | Value::Bool(_))) => {
                    Ok(Cow::Owned(value.to_string()))
                }
                _ => Err(missing()),
            };
        }
        if !matches!(
            field,
            "name"
                | "id"
                | "member"
                | "lifetime"
                | "presence"
                | "pane"
                | "target"
                | "cwd"
                | "host"
                | "session"
                | "window"
                | "activity"
        ) && self.row["failed"]
            .as_array()
            .is_some_and(|failed| failed.iter().any(|value| value.as_str() == Some(field)))
        {
            return Err(missing());
        }
        field_value(self.row, field)
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed)
            .ok_or_else(|| format!("{field} is empty for this row"))
    }

    /// The same resolution used by execution: unavailable values are visible, not guessed.
    pub fn inventory(&self) -> Vec<String> {
        fn visit(
            parameters: Parameters<'_>,
            namespace: &str,
            value: &Value,
            path: &str,
            out: &mut Vec<String>,
        ) {
            match value {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                        if parameter_path(&path) {
                            visit(parameters, namespace, value, &path, out);
                        }
                    }
                }
                Value::Array(values) => {
                    for (index, value) in values.iter().enumerate() {
                        visit(
                            parameters,
                            namespace,
                            value,
                            &format!("{path}/{index}"),
                            out,
                        );
                    }
                }
                _ => {
                    let field = format!("{namespace}:{path}");
                    let kind = match value {
                        Value::String(_) => "string",
                        Value::Number(_) => "number",
                        Value::Bool(_) => "boolean",
                        _ => "absent",
                    };
                    let value = parameters
                        .resolve(&field)
                        .map(|resolved| {
                            if value.is_string() {
                                format!("{resolved:?}")
                            } else {
                                resolved.into_owned()
                            }
                        })
                        .unwrap_or_else(|_| "unavailable".into());
                    let source = match namespace {
                        "row" => "public Ops row",
                        _ if matches!(
                            path.split('/').nth(1),
                            Some("runningDriver" | "driverSupport" | "resume" | "location")
                        ) =>
                        {
                            "Core ls observation"
                        }
                        _ => "Ops selected scope",
                    };
                    out.push(format!("{{{field}}} · {kind} · {source} · {value}"));
                }
            }
        }
        let caller = self
            .board_pane
            .map(|pane| format!("{pane:?}"))
            .unwrap_or_else(|| "unavailable (resolved at dispatch)".into());
        let mut out = vec![format!(
            "{{board_pane}} · string · public whoami sample · {caller}"
        )];
        visit(*self, "row", self.row, "", &mut out);
        if let Some(ops) = self.ops {
            visit(*self, "ops", ops, "", &mut out);
        }
        out
    }
}

/// These root containers carry message text/receipts, not row metadata. A custom
/// `fields.message` is ordinary public metadata and is intentionally unaffected.
fn parameter_path(path: &str) -> bool {
    let root = path
        .split('/')
        .nth(1)
        .unwrap_or("")
        .replace("~1", "/")
        .replace("~0", "~");
    if matches!(root.as_str(), "annotation" | "waitingOnYou" | "receipt") {
        return false;
    }
    true
}

fn failed_path(row: &Value, path: &str) -> bool {
    let mut segments = path
        .split('/')
        .skip(1)
        .map(|part| part.replace("~1", "/").replace("~0", "~"));
    if segments.next().as_deref() != Some("fields") {
        return false;
    }
    segments.next().is_some_and(|key| {
        row["failed"]
            .as_array()
            .is_some_and(|failed| failed.iter().any(|value| value.as_str() == Some(&key)))
    })
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
    fn absent_namespace_cannot_fall_back_to_authored_fields_and_failed_custom_names_do_not_mask_identity()
     {
        let row = json!({"name":"worker", "fields":{"name":"failed custom name", "ops:/runningDriver":"spoof"}, "failed":["name"]});
        assert!(
            Template::parse("{ops:/runningDriver}")
                .unwrap()
                .fill(&row)
                .unwrap_err()
                .contains("ops:/runningDriver")
        );
        assert_eq!(
            Template::parse("{name}").unwrap().fill(&row).unwrap(),
            "worker"
        );
        assert!(
            Template::parse("{row:/fields/name}")
                .unwrap()
                .fill(&row)
                .is_err()
        );
    }

    #[test]
    fn held_context_preserves_core_names_and_does_not_shadow_custom_fields() {
        let row = json!({"fields":{"runtime":"custom", "board_pane":"%99", "failed_value":"—"}, "failed":["failed_value"], "ops":{"runningDriver":"spoof"}});
        let ops = json!({"runningDriver":"codex", "resume":{"model":"startup", "observedModel":{"model":"observed", "observedAtMs":123}}, "timers":{"active":0,"total":2}});
        let parameters = Parameters {
            row: &row,
            ops: Some(&ops),
            board_pane: Some("%7"),
        };
        for (source, value) in [
            ("{runtime}", "custom"),
            ("{ops:/runningDriver}", "codex"),
            ("{ops:/resume/observedModel/model}", "observed"),
            ("{ops:/timers/active}", "0"),
            ("{board_pane}", "%7"),
        ] {
            assert_eq!(
                Template::parse(source)
                    .unwrap()
                    .fill_parameters(parameters)
                    .unwrap(),
                value
            );
        }
        for source in [
            "{row:/fields/failed_value}",
            "{failed_value}",
            "{ops:/resume/missing}",
        ] {
            assert!(
                Template::parse(source)
                    .unwrap()
                    .fill_parameters(parameters)
                    .is_err()
            );
        }
        assert!(
            Template::parse("{ops:/runningDriver}")
                .unwrap()
                .fill(&row)
                .is_err(),
            "row metadata cannot supply held Ops context"
        );
        let inventory = parameters.inventory().join("\n");
        assert!(
            inventory.contains("ops:/runningDriver} · string · Core ls observation · \"codex\"")
        );
        assert!(
            inventory.contains("row:/fields/failed_value} · string · public Ops row · unavailable")
        );
    }

    #[test]
    fn public_row_scalar_paths_keep_types_escaping_and_literal_argument_boundaries() {
        let row = json!({"name":"worker", "pane":{"id":"%3"}, "fields":{"a/b~c":"literal $(id); words"}, "count":0, "enabled":false});
        for (source, expected) in [
            ("{row:/pane/id}", "%3"),
            ("{row:/fields/a~1b~0c}", "literal $(id); words"),
            ("{row:/count}", "0"),
            ("{row:/enabled}", "false"),
        ] {
            let template = Template::parse(source).unwrap();
            assert_eq!(template.fill_argument(&row).unwrap(), expected);
            assert_eq!(Template::parse(&template.source()).unwrap(), template);
        }
    }

    #[test]
    fn unavailable_nonscalar_and_message_sources_refuse_the_named_path() {
        let row = json!({"missing":null, "object":{"value":1}, "array":[1],
            "annotation":{"text":"private request"}, "waitingOnYou":[{"preview":"private question"}],
            "receipt":{"id":"private receipt"}, "fields":{"message":"public metadata"}});
        for path in [
            "/absent",
            "/missing",
            "/object",
            "/array",
            "/annotation/text",
            "/waitingOnYou/0/preview",
            "/receipt/id",
        ] {
            let field = format!("row:{path}");
            let error = Template::parse(&format!("{{{field}}}"))
                .unwrap()
                .fill(&row)
                .unwrap_err();
            assert!(error.contains(&field), "{error}");
        }
        assert_eq!(
            Template::parse("{row:/array/0}")
                .unwrap()
                .fill(&row)
                .unwrap(),
            "1"
        );
        assert_eq!(
            Template::parse("{row:/fields/message}")
                .unwrap()
                .fill(&row)
                .unwrap(),
            "public metadata"
        );
    }

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
