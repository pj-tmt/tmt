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

const KEY_LINE: &str = "<tmt-switch><tmt-case min=\"lg\"><tmt-text>full</tmt-text></tmt-case><tmt-case min=\"md\"><tmt-text>short</tmt-text></tmt-case><tmt-default><tmt-text>min</tmt-text></tmt-default></tmt-switch>";

#[test]
fn switch_admits_ordered_cases_with_a_default() {
    let root = parse("switch.xml", &view(KEY_LINE)).unwrap();
    let switch = &root.children[0];
    assert_eq!(switch.kind, Kind::Switch);
    let kinds: Vec<_> = switch.children.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [Kind::Case, Kind::Case, Kind::Default]);
    assert_eq!(switch.children[1].attributes["min"], "md");
    // `of` is optional and takes exactly container or terminal.
    for of in ["container", "terminal"] {
        let source = KEY_LINE.replace("<tmt-switch>", &format!("<tmt-switch of=\"{of}\">"));
        parse("switch.xml", &view(&source)).unwrap();
    }
}

#[test]
fn switch_rejects_what_would_leave_a_gap_overlap_or_raw_number() {
    let case = |min: &str| format!("<tmt-case min=\"{min}\"><tmt-text>x</tmt-text></tmt-case>");
    let default = "<tmt-default/>";
    let switch = |body: &str| view(&format!("<tmt-switch>{body}</tmt-switch>"));
    for (source, message) in [
        // Only names: a raw number or a differently spelled name is refused.
        (
            switch(&format!("{}{default}", case("100"))),
            "breakpoint name (sm, md, lg)",
        ),
        (
            switch(&format!("{}{default}", case("MD"))),
            "breakpoint name",
        ),
        (switch(&format!("{}{default}", case(""))), "breakpoint name"),
        (
            switch(&format!("<tmt-case max=\"md\"/>{default}")),
            "max=\"md\"",
        ),
        (switch(&format!("<tmt-case/>{default}")), "requires min"),
        // Order and uniqueness.
        (
            switch(&format!("{}{}{default}", case("md"), case("lg"))),
            "must come before \"md\"",
        ),
        (
            switch(&format!("{}{}{default}", case("md"), case("md"))),
            "never repeat",
        ),
        // The default is required, single and last.
        (switch(&case("md")), "requires a final <tmt-default>"),
        (switch(&format!("{default}{}", case("md"))), "single last"),
        (
            switch(&format!("{}{default}{default}", case("md"))),
            "single last",
        ),
        (switch(default), "at least one <tmt-case"),
        (switch(""), "at least one <tmt-case"),
        // Branches are not boxes and a switch holds nothing else.
        (
            switch(&format!("{}<tmt-row/>{default}", case("md"))),
            "holds only",
        ),
        (view(&case("md")), "belong directly inside <tmt-switch>"),
        (
            view(&format!("<tmt-row>{default}</tmt-row>")),
            "belong directly inside",
        ),
        (
            view(&format!(
                "<tmt-switch><tmt-case min=\"md\">{}</tmt-case>{default}</tmt-switch>",
                case("sm")
            )),
            "belong directly inside",
        ),
        (
            view("<tmt-switch of=\"screen\"><tmt-default/></tmt-switch>"),
            "container or terminal",
        ),
        (
            view("<tmt-switch class=\"flex\"><tmt-default/></tmt-switch>"),
            "class=",
        ),
        (
            view("<tmt-switch><tmt-case min=\"md\" id=\"x\"/><tmt-default/></tmt-switch>"),
            "id=",
        ),
        (
            view("<tmt-switch><tmt-case min=\"md\"/><tmt-default token=\"dim\"/></tmt-switch>"),
            "token=",
        ),
    ] {
        rejects(&source, message);
    }
    // Located at the offending branch, not at the switch.
    let err = parse(
        "switch.xml",
        &view(&format!(
            "<tmt-switch>\n{}\n{}\n{default}</tmt-switch>",
            case("md"),
            case("lg")
        )),
    )
    .unwrap_err();
    assert_eq!(err.location.line, 3, "{err}");
}

#[test]
fn hide_below_is_a_switch_with_the_element_and_an_empty_default() {
    let root = parse(
        "hide.xml",
        &view("<tmt-cell id=\"squad\" hide-below=\"md\">squad</tmt-cell>"),
    )
    .unwrap();
    let switch = &root.children[0];
    assert_eq!(switch.kind, Kind::Switch);
    assert!(switch.attributes.is_empty());
    let [case, default] = &switch.children[..] else {
        panic!("one case and a default");
    };
    assert_eq!(
        (case.kind, case.attributes["min"].as_str()),
        (Kind::Case, "md")
    );
    assert_eq!(default.kind, Kind::Default);
    assert!(default.children.is_empty());
    let cell = &case.children[0];
    assert_eq!((cell.kind, cell.text.as_str()), (Kind::Cell, "squad"));
    assert!(!cell.attributes.contains_key("hide-below"));
    for tag in ["tmt-row", "tmt-col", "tmt-text"] {
        parse("hide.xml", &view(&format!("<{tag} hide-below=\"sm\"/>"))).unwrap();
    }
    for (body, message) in [
        ("<tmt-cell hide-below=\"100\"/>", "breakpoint name"),
        ("<tmt-cell hide-below=\"\"/>", "breakpoint name"),
        (
            "<tmt-repeat each=\"$.x\" as=\"x\" hide-below=\"md\"/>",
            "hide-below",
        ),
        (
            "<tmt-switch hide-below=\"md\"><tmt-default/></tmt-switch>",
            "hide-below",
        ),
    ] {
        rejects(&view(body), message);
    }
}
