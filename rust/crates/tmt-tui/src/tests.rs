use super::*;

fn view(body: &str) -> String {
    format!("<tmt-view version=\"1\">{body}</tmt-view>")
}
fn rejects(source: &str, message: &str) {
    let err = parse("board.xml", source).unwrap_err();
    assert!(err.message.contains(message), "{err}");
    assert!(err.to_string().starts_with("board.xml:"));
    assert!(err.location.line > 0 && err.location.column > 0);
}

#[test]
fn literal_board_and_xml_escaping_are_preserved() {
    let root = parse("board.xml", &view("\n<tmt-row id=\"header\"><tmt-text>A &amp; B &lt; C &#x754c;</tmt-text></tmt-row>\n<tmt-col><tmt-cell from=\"member\"/></tmt-col>")).unwrap();
    assert_eq!(root.kind, Kind::View);
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].location, Location { line: 2, column: 1 });
    assert_eq!(root.children[0].attributes["id"], "header");
    assert_eq!(root.children[0].children[0].text, "A & B < C 界");
    assert_eq!(root.children[1].children[0].attributes["from"], "member");
}

#[test]
fn version_and_structure_fail_before_any_rendering() {
    for source in [
        "<tmt-view/>",
        "<tmt-view version=\"2\"/>",
        "<tmt-row version=\"1\"/>",
    ] {
        rejects(source, "root must be");
    }
    for (body, message) in [
        ("<other/>", "unknown element"),
        ("<tmt-view version=\"1\"/>", "nested tmt-view"),
        ("<tmt-row width=\"2\"/>", "width=\"2\""),
        ("<tmt-row version=\"1\"/>", "version=\"1\""),
        (
            "<tmt-text><tmt-cell/></tmt-text>",
            "cannot contain elements",
        ),
        ("<tmt-col>unexpected</tmt-col>", "literal text"),
        (
            "<tmt-cell bind=\"row.name\">literal</tmt-cell>",
            "literal text",
        ),
        (
            "<tmt-cell from=\"member\">literal</tmt-cell>",
            "literal text",
        ),
        (
            "<tmt-cell bind=\"x\" from=\"member\"/>",
            "either bind or from",
        ),
        (
            "<tmt-row id=\"x\" id-bind=\"row.id\"/>",
            "either id or id-bind",
        ),
        (
            "<tmt-row row-id=\"x\" row-bind=\"row.id\"/>",
            "either row-id or row-bind",
        ),
        ("<tmt-repeat each=\"rows\"/>", "requires nonempty as"),
        (
            "<tmt-repeat each=\"\" as=\"row\"/>",
            "requires nonempty each",
        ),
        ("<tmt-row xmlns=\"urn:evil\"/>", "namespaces"),
        ("<tmt-row xmlns:x=\"urn:evil\"/>", "namespaces"),
    ] {
        rejects(&view(body), message);
    }
    for source in [
        "<",
        "<tmt-view version=\"1\">",
        "<tmt-view version=\"1\" version=\"1\"/>",
        "<tmt-view version=\"1\"/><tmt-view version=\"1\"/>",
    ] {
        assert!(parse("broken.xml", source).is_err());
    }
    let err = parse("board.xml", &view("\n  <tmt-cell wrong=\"value\"/>")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "board.xml:2:3: <tmt-cell>: attribute wrong=\"value\" is not allowed"
    );
}

#[test]
fn repeat_templates_are_checked_without_materialized_rows() {
    rejects(
        &view("<tmt-repeat each=\"rows\" as=\"row\"><tmt-bad/></tmt-repeat>"),
        "unknown element",
    );
    assert!(
        parse(
            "board.xml",
            &view(
                "<tmt-repeat each=\"rows\" as=\"row\"><tmt-cell bind=\"row.name\"/></tmt-repeat>"
            )
        )
        .is_ok()
    );
}

#[test]
fn byte_depth_and_node_limits_have_positive_boundary_controls() {
    let overhead = view("<tmt-text></tmt-text>").len();
    let source = view(&format!(
        "<tmt-text>{}</tmt-text>",
        "x".repeat(MAX_BYTES - overhead)
    ));
    assert_eq!(source.len(), MAX_BYTES);
    assert!(parse("limit.xml", &source).is_ok());
    rejects(&(source + " "), "256 KiB");
    let nested = |count| {
        view(&format!(
            "{}{}",
            "<tmt-col>".repeat(count),
            "</tmt-col>".repeat(count)
        ))
    };
    assert!(parse("limit.xml", &nested(MAX_DEPTH - 1)).is_ok());
    rejects(&nested(MAX_DEPTH), "nesting exceeds");
    // Document and root consume two nodes. Empty leaves exercise the same cap
    // without introducing inter-element whitespace text nodes.
    assert!(
        parse(
            "limit.xml",
            &view(&"<tmt-cell/>".repeat(MAX_NODES as usize - 2))
        )
        .is_ok()
    );
    rejects(
        &view(&"<tmt-cell/>".repeat(MAX_NODES as usize - 1)),
        "nodes limit",
    );
    // An invalid XML suffix cannot mask pre-parse byte/depth budget admission.
    rejects(
        &format!("{}<", "<tmt-col>".repeat(MAX_DEPTH + 1)),
        "nesting exceeds",
    );
    rejects(&"<".repeat(MAX_BYTES + 1), "256 KiB");
}

#[test]
fn depth_preflight_ignores_xml_literals_but_not_real_nested_tags() {
    let decoys = "<tmt-col>".repeat(MAX_DEPTH + 1);
    let source = view(&format!(
        "<!--{decoys}--><?probe {decoys}?><tmt-text><![CDATA[{decoys}]]></tmt-text><tmt-cell id=\"greater > sign\"/>"
    ));
    assert!(parse("literal.xml", &source).is_ok());
    rejects(
        &format!(
            "<tmt-view version=\"1\">{}<tmt-cell/>{}",
            "<tmt-col>".repeat(MAX_DEPTH - 1),
            "</tmt-col>".repeat(MAX_DEPTH - 1)
        ),
        "nesting exceeds",
    );
}

#[test]
fn dtd_and_entity_expansion_are_refused() {
    let declarations = "<!ENTITY a 'ha'><!ENTITY b '&a;&a;&a;&a;'><!ENTITY c '&b;&b;&b;&b;'>";
    rejects(
        &format!(
            "<!DOCTYPE tmt-view [{declarations}]>{}",
            view("<tmt-text>&c;</tmt-text>")
        ),
        "DOCTYPE",
    );
    rejects(
        &format!(
            "<!DOCTYPE tmt-view SYSTEM 'file:///not-opened'>{}",
            view("")
        ),
        "DOCTYPE",
    );
    assert!(parse("entity.xml", &view("<tmt-text>&unknown;</tmt-text>")).is_err());
    assert_eq!(
        parse(
            "entity.xml",
            &view("<tmt-text>&amp;&lt;&gt;&quot;&apos;</tmt-text>")
        )
        .unwrap()
        .children[0]
            .text,
        "&<>\"'"
    );
}
