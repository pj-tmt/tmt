use super::*;
use crate::parse;

fn styled(attrs: &str) -> CellStyle {
    parse(
        "styles.xml",
        &format!("<tmt-view version=\"1\"><tmt-cell {attrs}/></tmt-view>"),
    )
    .unwrap()
    .children
    .remove(0)
    .style
}

#[test]
fn literal_cell_styles_and_defaults() {
    let node = parse(
        "board.xml",
        r#"<tmt-view version="1">
      <tmt-row class="flex gap-x-2 gap-y-3 px-4 py-5 grow-2 shrink-0 w-full h-7" token="working">
        <tmt-col/><tmt-cell class="w-0 h-full truncate"/>
      </tmt-row>
    </tmt-view>"#,
    )
    .unwrap();
    assert_eq!(
        node.style,
        CellStyle {
            direction: Direction::Column,
            width: Extent::Auto,
            height: Extent::Auto,
            gap: [0, 0],
            padding: [0, 0],
            grow: 0,
            shrink: 1,
            text_flow: TextFlow::Clip,
            token: None,
        }
    );
    assert_eq!(
        node.children[0].style,
        CellStyle {
            direction: Direction::Row,
            width: Extent::Full,
            height: Extent::Cells(7),
            gap: [2, 3],
            padding: [4, 5],
            grow: 2,
            shrink: 0,
            text_flow: TextFlow::Clip,
            token: Some(Role::Working),
        }
    );
    assert_eq!(
        node.children[0].children[0].style.direction,
        Direction::Column
    );
    let cell = &node.children[0].children[1].style;
    assert_eq!(cell.width, Extent::Cells(0));
    assert_eq!(cell.height, Extent::Full);
    assert_eq!(cell.text_flow, TextFlow::Truncate);
}

#[test]
fn integer_boundaries_and_axis_shorthands() {
    for n in [0, 1, 4096] {
        let result = styled(&format!(
            r#"class="w-{n} h-{n} gap-{n} p-{n} grow-{n} shrink-{n}""#
        ));
        assert_eq!(
            (result.width, result.height),
            (Extent::Cells(n), Extent::Cells(n))
        );
        assert_eq!(result.gap, [n, n]);
        assert_eq!(result.padding, [n, n]);
        assert_eq!((result.grow, result.shrink), (n, n));
    }
    let result = styled(r#"class="flex-col grow shrink" wrap="true""#);
    assert_eq!(result.direction, Direction::Column);
    assert_eq!((result.grow, result.shrink), (1, 1));
    assert_eq!(result.text_flow, TextFlow::Wrap);
    assert_eq!(styled(r#"wrap="false""#).text_flow, TextFlow::Clip);
    let row = parse(
        "direction.xml",
        r#"<tmt-view version="1" class="flex-row"/>"#,
    )
    .unwrap();
    assert_eq!(row.style.direction, Direction::Row);
    assert_eq!(styled("class='  px-01&#9;py-2  '").padding, [1, 2]);
}

fn invalid(attrs: &str) -> String {
    // No data resolver is present: even a repeat that could be empty must fail.
    let source = format!(
        "<tmt-view version=\"1\">\n  <tmt-repeat each=\"$.empty\" as=\"row\">\n    <tmt-cell {attrs}/>\n  </tmt-repeat>\n</tmt-view>"
    );
    let err = parse("invalid.xml", &source).unwrap_err();
    assert_eq!(err.location, crate::Location { line: 3, column: 5 });
    assert!(err.to_string().starts_with("invalid.xml:3:5: <tmt-cell>:"));
    err.message
}

#[test]
fn overlaps_and_duplicates_fail_in_both_orders() {
    for (a, b, property) in [
        ("w-1", "w-full", "w"),
        ("h-full", "h-2", "h"),
        ("gap-1", "gap-x-2", "gap-x"),
        ("gap-1", "gap-y-2", "gap-y"),
        ("p-1", "px-2", "px"),
        ("p-1", "py-2", "py"),
        ("grow", "grow-1", "grow"),
        ("shrink", "shrink-0", "shrink"),
        ("flex-row", "flex-col", "direction"),
        ("flex", "flex", "display"),
        ("truncate", "truncate", "text-flow"),
        ("w-2", "w-2", "w"),
        ("px-1", "px-1", "px"),
        ("gap-y-0", "gap-y-0", "gap-y"),
    ] {
        for (first, second) in [(a, b), (b, a)] {
            let message = invalid(&format!(r#"class="{first} {second}""#));
            for expected in ["conflicts", first, second, property] {
                assert!(message.contains(expected), "{message}");
            }
        }
    }
    for value in ["true", "false"] {
        let message = invalid(&format!(r#"class="truncate" wrap="{value}""#));
        assert!(message.contains("wrap="), "{message}");
        assert!(message.contains("truncate"), "{message}");
    }
}

#[test]
fn unsupported_syntax_and_numbers_fail_without_coercion() {
    for class in [
        "hidden",
        "grid",
        "hover:w-1",
        "sm:flex",
        "w-1/2",
        "w-[3]",
        "w-auto",
        "w-1px",
        "w-1.0",
        "w--1",
        "w-+1",
        "w-4097",
        "h-65536",
        "p-999999999999999999999",
        "gap-x-4097",
        "gap-y-",
        "px-4097",
        "py-4097",
        "grow-4097",
        "shrink-4097",
    ] {
        let message = invalid(&format!(r#"class="{class}""#));
        assert!(message.contains(class), "{message}");
    }
    for value in ["1", "True", "", "yes"] {
        let message = invalid(&format!(r#"wrap="{value}""#));
        assert!(message.contains(&format!("wrap={value:?}")), "{message}");
        assert!(message.contains("must be true or false"));
    }
}

#[test]
fn tokens_reuse_the_style_owner_without_resolving_a_palette() {
    for role in Role::ALL {
        let style = styled(&format!(r#"token="{}""#, role.name()));
        assert_eq!(style.token, Some(role));
    }
    for value in ["", "Working", "working ", "red", "#ff0000", "unknown"] {
        let message = invalid(&format!(r#"token="{value}""#));
        assert!(
            message.contains(&format!("unknown token={value:?}")),
            "{message}"
        );
    }
    // Dynamic values belong to later binding validation, not literal Role parsing.
    assert_eq!(styled(r#"token-bind="row.role""#).token, None);
}
