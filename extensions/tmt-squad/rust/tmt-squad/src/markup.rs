//! Test-scoped adoption seam until the row-track compiler consumes it (#774).
use crate::{
    rows,
    source::{ColumnSource, Format},
    squad::Member,
};
use std::collections::BTreeMap;
use tmt_tui::binding::{Schema, Schemas, Scopes, Sources};

struct Adapter<'a> {
    members: BTreeMap<&'a str, &'a Member>,
    provided: &'a [String],
    now_ms: u64,
}
impl Sources for Adapter<'_> {
    type Source = (ColumnSource, Format);
    fn compile(
        &self,
        from: &str,
        format: &str,
        scopes: &Schemas<'_>,
    ) -> Result<Self::Source, String> {
        if !matches!(scopes.get("row"), Some(Schema::Object(fields)) if matches!(fields.get("id"), Some(Schema::StableId)))
        {
            return Err("Squad sources require lexical row.id: StableId".into());
        }
        let source = ColumnSource::parse(from, rows::field_name, |name| {
            self.provided.iter().any(|v| v == name)
        })
        .ok_or_else(|| format!("unknown Squad source {from:?}"))?;
        let format =
            Format::parse(format).ok_or_else(|| format!("unknown Squad format {format:?}"))?;
        Ok((source, format))
    }
    fn resolve(
        &self,
        (source, format): &Self::Source,
        scopes: &Scopes<'_>,
    ) -> Result<Option<String>, String> {
        let member = scopes
            .get("row")
            .and_then(|row| row["id"].as_str())
            .and_then(|id| self.members.get(id))
            .ok_or("Squad source requires an acquired member matching row.id")?;
        Ok(source.value(member, *format, self.now_ms))
    }
}

#[test]
fn binding_reuses_acquired_sources_and_never_reformats_projected_fields() {
    use serde_json::{Value, json};
    use tmt_tui::{binding, parse};
    let member = Member {
        lead_marker: None,
        id: "member-a".into(),
        name: "rin".into(),
        lifetime: "saved".into(),
        presence: "active".into(),
        pane: Value::Null,
        activity: Value::Null,
        fields: BTreeMap::from([("pr_state".into(), "?".into())]),
        meta: BTreeMap::from([("seen".into(), "60000".into())]),
        numbers: BTreeMap::new(),
        colors: BTreeMap::new(),
        failed: ["pr_state".into()].into(),
        seen: json!({"resume": {"usage": {"tokens": 487000}}}),
    };
    let provided = vec!["pr_state".into()];
    let adapter = Adapter {
        members: BTreeMap::from([(member.id.as_str(), &member)]),
        provided: &provided,
        now_ms: 120000,
    };
    let row = Schema::Object(BTreeMap::from([
        ("id".into(), Schema::StableId),
        ("shown".into(), Schema::Scalar),
    ]));
    let schema = Schema::Object(BTreeMap::from([(
        "rows".into(),
        Schema::Collection(Box::new(row)),
    )]));
    for (from, format, expected) in [
        ("session.usage.tokens", "tokens", Some("487k")),
        ("session.usage.tokens", "count", Some("487,000")),
        ("meta.seen", "age", Some("1m")),
        ("member", "text", Some("rin")),
        ("fields.pr_state", "count", Some("?")),
        ("cwd", "text", None),
    ] {
        let xml = format!(
            "<tmt-view version=\"1\"><tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell from=\"{from}\" format=\"{format}\"/><tmt-cell bind=\"row.shown\"/></tmt-repeat></tmt-view>"
        );
        let template = binding::compile(
            "squad.xml",
            &parse("squad.xml", &xml).unwrap(),
            &schema,
            &adapter,
        )
        .unwrap();
        let node = template
            .materialize(
                "squad.xml",
                &json!({"rows": [{"id": member.id, "shown": "487k"}]}),
                &adapter,
            )
            .unwrap();
        let configured: toml_edit::DocumentMut = format!(
            "[p]\nrows.columns=[{{name='shown',from='{from}',format='{format}'}}]\nfields.pr_state={{}}\n"
        ).parse().unwrap();
        let rows = rows::read(configured["p"].as_table_like(), "p").unwrap();
        let column = &rows.columns[0];
        assert_eq!(
            node.children[0].text,
            column
                .source()
                .unwrap()
                .value(&member, column.format, adapter.now_ms)
        );
        assert_eq!(node.children[0].text.as_deref(), expected);
        assert_eq!(node.children[1].text.as_deref(), Some("487k"));
        assert!(
            template
                .materialize(
                    "squad.xml",
                    &json!({"rows": [{"id": "missing", "shown": "487k"}]}),
                    &adapter
                )
                .is_err()
        );
    }
    for attrs in [
        "from=\"unknown\"",
        "from=\"member\" format=\"unknown\"",
        "from=\"fields.missing\"",
        "bind=\"row.shown\" format=\"tokens\"",
    ] {
        let xml = format!(
            "<tmt-view version=\"1\"><tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell {attrs}/></tmt-repeat></tmt-view>"
        );
        assert!(
            binding::compile(
                "squad.xml",
                &parse("squad.xml", &xml).unwrap(),
                &schema,
                &adapter
            )
            .is_err()
        );
    }
}
