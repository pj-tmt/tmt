//! Offline authoring admission; the board never loads this file.
use crate::{
    core::SquadError,
    rows,
    source::{ColumnSource, Format},
};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs::File, io::Read};
use tmt_tui::binding::{self, Schema, Schemas, Scopes, Sources};

struct Registry;
impl Sources for Registry {
    type Source = (ColumnSource, Format);
    fn compile(
        &self,
        from: &str,
        format: &str,
        scopes: &Schemas<'_>,
    ) -> Result<Self::Source, String> {
        if !matches!(scopes.get("row"), Some(Schema::Object(f)) if matches!(f.get("id"), Some(Schema::StableId)))
        {
            return Err("Squad sources require lexical row.id: StableId".into());
        }
        let source = ColumnSource::parse(from, rows::field_name, |name| {
            rows::field_name(name) && !rows::OWN_FIELDS.contains(&name)
        })
        .ok_or_else(|| format!("unknown Squad source {from:?}"))?;
        let format =
            Format::parse(format).ok_or_else(|| format!("unknown Squad format {format:?}"))?;
        Ok((source, format))
    }
    fn resolve(&self, _: &Self::Source, _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("offline validation never acquires or materializes data".into())
    }
}
fn schema(element: &tmt_tui::MarkupElement) -> Schema {
    let mut fields = BTreeMap::new();
    let mut pending = vec![element];
    while let Some(element) = pending.pop() {
        pending.extend(&element.children);
        for (key, path) in &element.attributes {
            let parts: Vec<_> = path.split('.').collect();
            if let [_, "fields" | "colors", field] = parts.as_slice()
                && matches!(key.as_str(), "bind" | "token-bind")
                && rows::field_name(field)
            {
                fields.insert((*field).into(), Schema::Scalar);
            }
        }
    }
    let mut row = BTreeMap::from([
        ("id".into(), Schema::StableId),
        ("fields".into(), Schema::Object(fields.clone())),
        ("colors".into(), Schema::Object(fields)),
    ]);
    for name in ["name", "presence", "pending"] {
        row.insert(name.into(), Schema::Scalar);
    }
    let squad = Schema::Object(BTreeMap::from([("name".into(), Schema::Scalar)]));
    Schema::Object(BTreeMap::from([
        (
            "rows".into(),
            Schema::Collection(Box::new(Schema::Object(row))),
        ),
        ("squad".into(), squad),
    ]))
}
pub(crate) fn validate(file: &str, xml: &str) -> Result<(), SquadError> {
    tmt_tui::parse(file, xml)
        .and_then(|element| binding::compile(file, &element, &schema(&element), &Registry))
        .map(|_| ())
        .map_err(|error| SquadError::new("LAYOUT_INVALID", error.to_string()))
}
pub fn grammar() -> Command {
    tmt_cli_style::command(crate::specs::LAYOUT)
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(crate::specs::LAYOUT_VALIDATE).arg(
                Arg::new("file")
                    .required(true)
                    .help("XML file to check offline"),
            ),
        )
}
pub fn run(matches: &ArgMatches) -> Result<Value, SquadError> {
    let (_, flags) = matches.subcommand().expect("required layout subcommand");
    let path = flags.get_one::<String>("file").expect("required file");
    let read = || -> std::io::Result<String> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(tmt_tui::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        String::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    };
    let xml = read().map_err(|error| SquadError::new("LAYOUT_IO", format!("{path}: {error}")))?;
    validate(path, &xml)?;
    Ok(json!({"valid": true, "file": path, "version": 1, "schema": "squad-projected-v1"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_schema_checks_every_repeat_and_source_without_data() {
        let wrap = |body: &str| {
            format!(
                "<tmt-view version='1'><tmt-repeat each='$.rows' as='row'>{body}</tmt-repeat></tmt-view>"
            )
        };
        for field in rows::OWN_FIELDS
            .iter()
            .chain(["task", "model", "custom-field"].iter())
        {
            let body = format!(
                "<tmt-cell id-bind='row.id' token-bind='row.colors.state' bind='row.fields.{field}'/>"
            );
            validate("fields.xml", &wrap(&body)).unwrap();
        }
        for body in [
            "<tmt-cell bind='row.fields.Bad'/>",
            "<tmt-cell from='fields.Bad'/>",
            "<tmt-cell from='fields.member'/>",
            "<tmt-cell from='member' format='bad'/>",
            "<tmt-cell class='w-rem'/>",
        ] {
            let error = validate("example.xml", &wrap(body)).unwrap_err();
            assert_eq!(error.code, "LAYOUT_INVALID");
            assert!(error.message.starts_with("example.xml:1:"));
        }
        assert!(validate("large.xml", &" ".repeat(tmt_tui::MAX_BYTES + 1)).is_err());
    }

    /// The shipped skill's examples are what the command accepts, and a switch is
    /// checked as a whole offline: a missing default or a number is refused.
    #[test]
    fn the_skills_examples_validate_and_a_malformed_switch_does_not() {
        let skill = include_str!("../../../skills/tmt-ops/SKILL.md");
        let section = skill
            .split("## Offline markup authoring")
            .nth(1)
            .expect("the authoring section");
        let blocks = section
            .split("```xml\n")
            .skip(1)
            .map(|block| block.split("```").next().unwrap())
            .collect::<Vec<_>>();
        assert!(blocks.iter().any(|block| block.contains("<tmt-switch>")));
        for block in blocks {
            validate("skill.xml", block).unwrap();
        }
        let wrap = |body: &str| {
            format!(
                "<tmt-view version='1'><tmt-repeat each='$.rows' as='row'><tmt-row id-bind='row.id'>{body}</tmt-row></tmt-repeat></tmt-view>"
            )
        };
        for body in [
            "<tmt-switch><tmt-case min='md'><tmt-cell/></tmt-case></tmt-switch>",
            "<tmt-switch><tmt-case min='100'><tmt-cell/></tmt-case><tmt-default/></tmt-switch>",
            "<tmt-switch><tmt-case min='md'><tmt-cell/></tmt-case><tmt-case min='lg'><tmt-cell/></tmt-case><tmt-default/></tmt-switch>",
            "<tmt-cell hide-below='wide'/>",
            "<tmt-switch><tmt-case min='md'><tmt-cell bind='row.fields.Bad'/></tmt-case><tmt-default/></tmt-switch>",
        ] {
            let error = validate("switch.xml", &wrap(body)).unwrap_err();
            assert_eq!(error.code, "LAYOUT_INVALID", "{body}");
        }
    }
}
