use super::*;
use serde_json::json;

struct Registry;
impl Sources for Registry {
    type Source = (String, String);
    fn compile(&self, from: &str, format: &str, _: &Schemas<'_>) -> Result<Self::Source, String> {
        if from != "member" || format != "text" {
            return Err("unknown source/format".into());
        }
        Ok((from.into(), format.into()))
    }
    fn resolve(&self, _: &Self::Source, _: &Scopes<'_>) -> Result<Option<String>, String> {
        Ok(Some("already displayed".into()))
    }
}
fn object(fields: impl IntoIterator<Item = (&'static str, Schema)>) -> Schema {
    Schema::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn schema() -> Schema {
    object([
        (
            "rows",
            Schema::Collection(Box::new(object([
                ("id", Schema::StableId),
                ("shown", Schema::Scalar),
                ("token", Schema::Scalar),
                (
                    "children",
                    Schema::Collection(Box::new(object([("id", Schema::StableId)]))),
                ),
            ]))),
        ),
        ("value", Schema::Scalar),
    ])
}
fn parse(body: &str) -> MarkupElement {
    crate::parse(
        "test.xml",
        &format!("<tmt-view version=\"1\">{body}</tmt-view>"),
    )
    .unwrap()
}
fn render(body: &str, data: &Value) -> Result<Node, Error> {
    compile("test.xml", &parse(body), &schema(), &Registry)?
        .materialize("test.xml", data, &Registry)
}

#[test]
fn admission_checks_empty_repeats_and_reports_the_attribute_and_location() {
    for (attrs, message) in [
        ("bind=\"row.missing\"", "unknown path"),
        ("bind=\"row.id[0]\"", "without indexing"),
        ("from=\"missing\"", "unknown source"),
        ("from=\"member\" format=\"missing\"", "unknown source"),
        ("bind=\"row.shown\" format=\"count\"", "already display"),
        ("id-bind=\"row.shown\"", "StableId"),
        ("selected=\"yes\"", "true or false"),
        ("token=\"muted\" token-bind=\"row.token\"", "either token"),
    ] {
        let markup = parse(&format!(
            "\n<tmt-repeat each=\"$.rows\" as=\"row\">\n<tmt-cell {attrs}/></tmt-repeat>"
        ));
        let error = compile("test.xml", &markup, &schema(), &Registry)
            .err()
            .unwrap();
        assert!(error.message.contains(message), "{error}");
        assert_eq!((error.file.as_str(), error.location.line), ("test.xml", 3));
    }
    for body in [
        "<tmt-repeat each=\"$.value\" as=\"row\"/>",
        "<tmt-repeat each=\"$.rows\" as=\"$\"/>",
        "<tmt-row row-id=\"member\"/>",
    ] {
        assert!(render(body, &json!({"rows": []})).is_err());
    }
}

#[test]
fn scalar_null_missing_and_dynamic_tokens_have_distinct_behavior() {
    for (value, shown) in [
        (json!("487k"), Some("487k")),
        (json!(42), Some("42")),
        (json!(true), Some("true")),
        (Value::Null, None),
    ] {
        let node = render("<tmt-cell bind=\"$.value\"/>", &json!({"value": value})).unwrap();
        assert_eq!(node.children[0].text.as_deref(), shown);
    }
    assert!(
        render("<tmt-cell bind=\"$.value\"/>", &json!({}))
            .unwrap_err()
            .message
            .contains("missing path")
    );
    for value in [json!([]), json!({})] {
        assert!(render("<tmt-cell bind=\"$.value\"/>", &json!({"value": value})).is_err());
    }
    let body = "<tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell token-bind=\"row.token\" from=\"member\" selected=\"true\"/></tmt-repeat>";
    for (token, expected) in [(json!("muted"), Some(Role::Muted)), (Value::Null, None)] {
        let node = render(body, &json!({"rows": [{"token": token}]})).unwrap();
        assert_eq!(node.children[0].style.token, expected);
        assert_eq!(node.children[0].text.as_deref(), Some("already displayed"));
        assert!(node.children[0].selected);
    }
    assert!(
        render(body, &json!({"rows": [{"token": "bad"}]}))
            .unwrap_err()
            .message
            .contains("unknown token")
    );
    assert!(render("", &json!([])).is_err());
    assert!(
        render(
            "<tmt-repeat each=\"$.rows\" as=\"row\"/>",
            &json!({"rows": [42]})
        )
        .is_err()
    );
}

#[test]
fn scoped_stable_ids_survive_reordering_and_shadowed_repeats() {
    let body = "<tmt-repeat each=\"$.rows\" as=\"row\"><tmt-row id-bind=\"row.id\" row-bind=\"row.id\"><tmt-repeat each=\"row.children\" as=\"row\"><tmt-cell id-bind=\"row.id\"/></tmt-repeat></tmt-row></tmt-repeat>";
    let a = json!({"id": "member-a", "children": [{"id": "cell"}]});
    let b = json!({"id": "member-b", "children": [{"id": "cell"}]});
    let first = render(body, &json!({"rows": [a, b]})).unwrap();
    let moved = render(body, &json!({"rows": [b, a]})).unwrap();
    assert_eq!(first.children[0], moved.children[1]);
    assert_eq!(first.children[0].row_id.as_deref(), Some("member-a"));
    assert_eq!(
        first.children[0].children[0].id,
        Some(vec!["member-a".into(), "cell".into()])
    );
    assert!(
        render(body, &json!({"rows": [a, a]}))
            .unwrap_err()
            .message
            .contains("duplicate resolved")
    );
    for id in [
        Value::Null,
        json!("0"),
        json!(0),
        json!(" leading"),
        json!("bad\n"),
        json!("x".repeat(MAX_ID_BYTES + 1)),
    ] {
        assert!(render(body, &json!({"rows": [{"id": id, "children": []}]})).is_err());
    }
    render(
        body,
        &json!({"rows": [{"id": "x".repeat(MAX_ID_BYTES), "children": []}]}),
    )
    .unwrap();
}

#[test]
fn output_and_work_limits_accept_exact_bounds_then_reject_one_more() {
    for (body, limit, message) in [
        (
            "<tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell/></tmt-repeat>",
            MAX_EXPANDED_NODES - 1,
            "expanded nodes",
        ),
        (
            "<tmt-repeat each=\"$.rows\" as=\"row\"/>",
            MAX_REPEAT_WORK,
            "repeat work",
        ),
    ] {
        let template = compile("test.xml", &parse(body), &schema(), &Registry).unwrap();
        let data = json!({"rows": vec![json!({}); limit]});
        template.materialize("test.xml", &data, &Registry).unwrap();
        let data = json!({"rows": vec![json!({}); limit + 1]});
        assert!(
            template
                .materialize("test.xml", &data, &Registry)
                .unwrap_err()
                .message
                .contains(message)
        );
    }
    let text = "x".repeat(MAX_BOUND_BYTES);
    render("<tmt-cell bind=\"$.value\"/>", &json!({"value": text})).unwrap();
    assert!(
        render(
            "<tmt-cell bind=\"$.value\"/>",
            &json!({"value": format!("{text}x")})
        )
        .unwrap_err()
        .message
        .contains("8 MiB")
    );
    // Charging includes both scoped-ID copies, row IDs and text across all nodes.
    let data = json!({"value": "x".repeat(MAX_BOUND_BYTES - 3)});
    render(
        "<tmt-row id=\"x\" row-id=\"x\"><tmt-cell bind=\"$.value\"/></tmt-row>",
        &data,
    )
    .unwrap();
    assert!(render("<tmt-row id=\"x\" row-id=\"x\"><tmt-cell bind=\"$.value\"/><tmt-text>y</tmt-text></tmt-row>", &data).is_err());
}

fn switch_result(body: &str, data: &serde_json::Value) -> Result<Node, Error> {
    let element = parse(body);
    compile("test.xml", &element, &schema(), &Registry)?.materialize("test.xml", data, &Registry)
}

#[test]
fn branches_may_share_an_id_but_not_with_anything_outside_the_switch() {
    let data = json!({"value": "v", "rows": []});
    let branches = "<tmt-switch><tmt-case min=\"md\"><tmt-cell id=\"squad\"/></tmt-case><tmt-default><tmt-cell id=\"squad\"/></tmt-default></tmt-switch>";
    let scene = switch_result(branches, &data).unwrap();
    assert_eq!(scene.children[0].kind, Kind::Switch);
    assert_eq!(scene.children[0].cond, Cond::Switch(Of::Container));
    assert_eq!(scene.children[0].children[0].cond, Cond::Case(100));
    assert_eq!(scene.children[0].children[1].cond, Cond::None);
    for outside in [
        format!("<tmt-cell id=\"squad\"/>{branches}"),
        format!("{branches}<tmt-cell id=\"squad\"/>"),
    ] {
        let err = switch_result(&outside, &data).unwrap_err();
        assert!(err.message.contains("duplicate resolved id"), "{err}");
    }
    // Two branches of one switch still cannot repeat an ID inside one branch.
    let twice = "<tmt-switch><tmt-case min=\"md\"/><tmt-default><tmt-cell id=\"a\"/><tmt-cell id=\"a\"/></tmt-default></tmt-switch>";
    assert!(switch_result(twice, &data).is_err());
}

#[test]
fn every_branch_is_checked_and_materialized_not_only_the_one_that_shows() {
    // An unknown path in the narrowest branch fails at admission, at every width.
    let bad = "<tmt-switch><tmt-case min=\"lg\"><tmt-cell bind=\"$.value\"/></tmt-case><tmt-default><tmt-cell bind=\"$.missing\"/></tmt-default></tmt-switch>";
    let err = compile("test.xml", &parse(bad), &schema(), &Registry)
        .err()
        .unwrap();
    assert!(err.message.contains("unknown path"), "{err}");
    // Branch text, scope and repeats resolve like any other child.
    let good = "<tmt-switch of=\"terminal\"><tmt-case min=\"lg\"><tmt-cell bind=\"$.value\"/></tmt-case><tmt-default><tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell bind=\"row.shown\"/></tmt-repeat></tmt-default></tmt-switch>";
    let scene = switch_result(
        good,
        &json!({"value": "wide", "rows": [{"id": "r1", "shown": "one", "token": null, "children": []}]}),
    )
    .unwrap();
    let switch = &scene.children[0];
    assert_eq!(switch.cond, Cond::Switch(Of::Terminal));
    assert_eq!(switch.children[0].children[0].text.as_deref(), Some("wide"));
    assert_eq!(switch.children[1].children[0].text.as_deref(), Some("one"));
}

#[test]
fn branches_count_against_the_work_budget() {
    let rows: Vec<_> = (0..MAX_REPEAT_WORK / 2 + 1)
        .map(|i| json!({"id": format!("r{i}"), "shown": "x", "token": null, "children": []}))
        .collect();
    let data = json!({"value": "v", "rows": rows});
    let branch = "<tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell/></tmt-repeat>";
    let one = format!(
        "<tmt-switch><tmt-case min=\"md\"/><tmt-default>{branch}</tmt-default></tmt-switch>"
    );
    switch_result(&one, &data).unwrap();
    // Every branch is expanded, so two of them cost twice as much.
    let both = format!(
        "<tmt-switch><tmt-case min=\"md\">{branch}</tmt-case><tmt-default>{branch}</tmt-default></tmt-switch>"
    );
    let err = switch_result(&both, &data).unwrap_err();
    assert!(err.message.contains("20,000"), "{err}");
}
