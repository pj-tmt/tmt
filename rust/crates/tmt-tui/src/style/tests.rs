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
            display: Display::Flex,
            basis: Extent::Auto,
            min_width: None,
            max_width: None,
            columns: Default::default(),
            col_span: 1,
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
            display: Display::Flex,
            basis: Extent::Auto,
            min_width: None,
            max_width: None,
            columns: Default::default(),
            col_span: 1,
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
        "hover:w-1",
        "sm:flex",
        "w-1/2",
        "w-30%",
        "text-ellipsis-middle",
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

#[test]
fn grid_and_integer_utilities_admit_only_the_canonical_vocabulary() {
    let s =
        styled("class='grid grid-cols-[4_30%_2fr_minmax(2,3fr)] gap-2 col-span-2 truncate-middle'");
    assert_eq!(s.display, Display::Grid);
    assert_eq!(s.col_span, 2);
    assert_eq!(s.gap, [2, 2]);
    assert_eq!(s.text_flow, TextFlow::Middle);
    assert_eq!(
        s.columns[0],
        GridTrack {
            min: Breadth::Cells(4),
            max: Breadth::Cells(4)
        }
    );
    assert_eq!(s.columns[1].max, Breadth::Percent(30));
    assert_eq!(s.columns[2].max, Breadth::Fraction(2));
    assert_eq!(
        s.columns[3],
        GridTrack {
            min: Breadth::Cells(2),
            max: Breadth::Fraction(3)
        }
    );
    let s = styled("class='basis-[30%] w-[100%] h-[50%] min-w-2 max-w-20 grow-3 line-clamp-2'");
    assert_eq!(
        (s.basis, s.width, s.height),
        (
            Extent::Percent(30),
            Extent::Percent(100),
            Extent::Percent(50)
        )
    );
    assert_eq!(
        (s.min_width, s.max_width, s.grow, s.text_flow),
        (Some(2), Some(20), 3, TextFlow::Clamp(2))
    );
    for class in [
        "w-[0%]",
        "basis-[100%]",
        "grid grid-cols-[0_0%_0fr_minmax(0,1)]",
    ] {
        styled(&format!("class='{class}'"));
    }
}
#[test]
fn malformed_grid_tracks_sizes_and_conflicts_have_located_errors() {
    for class in [
        "grid-cols-[2]",
        "grid flex-row",
        "grid grid-cols-[]",
        "grid grid-cols-[2_]",
        "grid grid-cols-[1.2fr]",
        "grid grid-cols-[101%]",
        "grid grid-cols-[minmax(1fr,2fr)]",
        "grid grid-cols-[minmax(2,3,4)]",
        "grid grid-cols-[minmax(1,minmax(2,3))]",
        "col-span-0",
        "line-clamp-0",
        "w-[101%]",
        "min-w-[20%]",
        "w-1 w-1",
        "w-[20%] w-full",
        "truncate-middle truncate",
        "line-clamp-2 truncate",
        "grid flex",
        "grid grid-cols-[1] grid-cols-[2]",
        "text-ellipsis-middle",
    ] {
        let message = invalid(&format!("class='{class}'"));
        assert!(
            class.split_whitespace().any(|word| message.contains(word)),
            "{message}"
        );
    }
    let message = invalid("class='w-30%'");
    assert!(message.contains("w-30%") && message.contains("w-[n%]"));
    assert!(invalid("class='line-clamp-2' wrap='true'").contains("conflicts"));
}
#[test]
fn repeated_static_track_lists_share_storage() {
    let original = styled("class='grid grid-cols-[1_2_3]'");
    let copied = original.clone();
    assert!(std::sync::Arc::ptr_eq(&original.columns, &copied.columns));
}

#[test]
fn bracket_integers_fail_with_the_canonical_bare_hint() {
    for key in [
        "w",
        "h",
        "grow",
        "gap",
        "gap-x",
        "gap-y",
        "basis",
        "min-w",
        "max-w",
        "col-span",
        "line-clamp",
        "p",
        "shrink",
    ] {
        let class = format!("{key}-[3]");
        let error = invalid(&format!("class='{class}'"));
        assert!(
            error.contains(&class) && error.contains(&format!("hint: use {key}-N")),
            "{error}"
        );
    }
}

#[test]
fn auto_grid_tracks_and_each_conflict_have_specific_located_errors() {
    let style = styled("class='grid grid-cols-[auto_minmax(auto,12)]'");
    assert_eq!(
        style.columns[0],
        GridTrack {
            min: Breadth::Auto,
            max: Breadth::Auto
        }
    );
    assert_eq!(
        style.columns[1],
        GridTrack {
            min: Breadth::Auto,
            max: Breadth::Cells(12)
        }
    );
    let maximum = invalid("class='grid grid-cols-[minmax(2,auto)]'");
    assert!(maximum.contains("minmax(auto,N)"), "{maximum}");
    let needs_grid = invalid("class='grid-cols-[2]'");
    assert!(
        needs_grid.contains("grid-cols-[2]") && needs_grid.contains("requires grid"),
        "{needs_grid}"
    );
    assert!(!needs_grid.contains("conflicts"));
    let direction = invalid("class='grid flex-row'");
    assert!(
        direction.contains("flex-row") && direction.contains("conflicts with grid"),
        "{direction}"
    );
    assert!(!direction.contains("requires grid"));
}
