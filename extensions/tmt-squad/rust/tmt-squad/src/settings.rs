//! Settings shared by the CLI and board; Config owns resolution, validation and writes.
use crate::{config::Config, core::SquadError, effects, tab_view};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{Terminal, detail};

pub struct BoardSetting {
    pub key: String,
    pub value: Value,
    pub source: String,
    pub editable: bool,
}
pub struct BoardSettings {
    pub path: String,
    pub context: Option<String>,
    pub host: &'static str,
    pub entries: Vec<BoardSetting>,
    pub notices: Vec<String>,
}
impl BoardSettings {
    pub fn value(&self) -> Value {
        json!({"path": self.path, "context": self.context, "host": self.host, "notices": self.notices, "entries": self.entries.iter().map(|entry| json!({"key":entry.key, "value":entry.value, "source":entry.source, "editable":entry.editable})).collect::<Vec<_>>()})
    }
    pub fn push(&mut self, key: impl Into<String>, value: Value, source: impl Into<String>) {
        self.entries.push(BoardSetting {
            key: key.into(),
            value,
            source: source.into(),
            editable: false,
        });
    }
}

pub fn grammar() -> Command {
    tmt_cli_style::command(crate::specs::CONFIG)
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(crate::specs::CONFIG_SET)
                .arg(Arg::new("key").required(true).value_name("KEY"))
                .arg(Arg::new("value").required(true).value_name("VALUE"))
                .arg(Arg::new("squad").long("squad").value_name("NAME")),
        )
        .subcommand(
            tmt_cli_style::command(crate::specs::CONFIG_SHOW)
                .arg(
                    Arg::new("squad")
                        .long("squad")
                        .value_name("NAME")
                        .conflicts_with("tab"),
                )
                .arg(Arg::new("tab").long("tab").value_name("NAME")),
        )
}

pub fn run(config: &mut Config, matches: &ArgMatches) -> Result<Value, SquadError> {
    let (command, flags) = matches.subcommand().expect("required config subcommand");
    let squad = flags.get_one::<String>("squad");
    if let Some(name) = squad
        && !crate::squad::valid_name(name)
    {
        return Err(crate::squad::name_invalid(name));
    }
    let changed = if command == "set" {
        Some(config.set_setting(
            squad.map(String::as_str),
            flags.get_one::<String>("key").unwrap(),
            flags.get_one::<String>("value").unwrap(),
        )?)
    } else {
        None
    };
    let tab = (command == "show")
        .then(|| {
            flags
                .get_one::<String>("tab")
                .map(|name| tab_view::key(config, name))
                .transpose()
        })
        .transpose()?
        .flatten();
    let context = tab.as_deref().or(squad.map(String::as_str));
    let mut result = config
        .settings(context, effects::tmux_socket().is_some(), None)?
        .value();
    if let Some(changed) = changed {
        result["changed"] = json!(changed);
        if matches!(
            flags.get_one::<String>("key").unwrap().as_str(),
            "board.direction" | "board.sizes" | "board.panes"
        ) {
            result["notices"].as_array_mut().unwrap().push(json!(format!("This edit pins workflow layout {} and the full flat split (direction, panes and sizes) in squad.toml; future preset changes will not replace them.", config.layout(squad.unwrap())?.as_str())));
        }
    }
    Ok(result)
}

pub fn display(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    let mut fields = vec![
        ("file".to_owned(), display(&document["path"])),
        (
            "context".into(),
            document["context"]
                .as_str()
                .unwrap_or("board defaults")
                .into(),
        ),
        ("host".into(), display(&document["host"])),
    ];
    for entry in document["entries"].as_array().into_iter().flatten() {
        fields.push((
            display(&entry["key"]),
            format!(
                "{}  [from {}; {}]",
                display(&entry["value"]),
                display(&entry["source"]),
                if entry["editable"] == true {
                    "editable"
                } else {
                    "read-only"
                }
            ),
        ));
    }
    for notice in document["notices"].as_array().into_iter().flatten() {
        fields.push(("notice".into(), display(notice)));
    }
    let refs: Vec<_> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    let mut out = Vec::new();
    let _ = detail::write(&mut out, terminal, "SETTINGS", &refs);
    String::from_utf8(out).unwrap_or_default()
}
