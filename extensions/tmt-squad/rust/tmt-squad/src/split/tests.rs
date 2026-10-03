use super::*;

fn parse(text: &str) -> Result<Split, SquadError> {
    let document: toml_edit::DocumentMut = format!("layout = {text}\n").parse().unwrap();
    read(&document["layout"], "board.layout")
}

fn message(text: &str) -> String {
    let error = parse(text).expect_err(text);
    assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{text}");
    error.message
}

#[test]
fn unsized_children_share_the_split_and_grow_takes_what_percentages_leave() {
    let even = parse(r#"{ direction = "top-bottom", panes = ["rows", "notes"] }"#).unwrap();
    assert_eq!(
        even,
        Split::Group {
            direction: Direction::TopBottom,
            children: vec![
                (Size::Grow(1), Split::Pane(Pane::Rows)),
                (Size::Grow(1), Split::Pane(Pane::Notes)),
            ],
        }
    );
    let mixed = parse(
        r#"{ direction = "left-right", sizes = [30, { grow = 2 }, { grow = 1 }], panes = ["notes", "rows", "detail"] }"#,
    )
    .unwrap();
    let Split::Group { children, .. } = &mixed else {
        panic!("a group");
    };
    assert_eq!(
        children.iter().map(|(size, _)| *size).collect::<Vec<_>>(),
        [Size::Percent(30), Size::Grow(2), Size::Grow(1)]
    );
    assert_eq!(
        mixed.panes(),
        [Pane::Notes, Pane::Rows, Pane::Detail],
        "reading order"
    );
}

#[test]
fn nesting_reads_in_order_up_to_three_levels() {
    let three = parse(
        r#"{ direction = "left-right", panes = ["rows", { direction = "top-bottom", panes = ["detail", { direction = "left-right", panes = ["notes", "replies"] }] }] }"#,
    )
    .unwrap();
    assert_eq!(
        three.panes(),
        [Pane::Rows, Pane::Detail, Pane::Notes, Pane::Replies]
    );
    let four = message(
        r#"{ direction = "left-right", panes = ["rows", { direction = "top-bottom", panes = ["detail", { direction = "left-right", panes = ["notes", { direction = "top-bottom", panes = ["replies"] }] }] }] }"#,
    );
    assert!(four.contains("deeper than 3"), "{four}");
}

#[test]
fn mistakes_are_refused_with_their_place() {
    for (text, place) in [
        (
            r#"{ direction = "left-right", panes = ["notes"] }"#,
            "must include rows",
        ),
        (
            r#"{ direction = "left-right", panes = ["rows", "rows"] }"#,
            "rows twice",
        ),
        (
            r#"{ direction = "left-right", panes = ["rows", { direction = "top-bottom", panes = ["rows"] }] }"#,
            "rows twice",
        ),
        (
            r#"{ direction = "diagonal", panes = ["rows"] }"#,
            "board.layout.direction",
        ),
        (
            r#"{ direction = "left-right", panes = [] }"#,
            "board.layout.panes",
        ),
        (
            r#"{ direction = "left-right", panes = ["rows", "chat"] }"#,
            "board.layout.panes[1]",
        ),
        (
            r#"{ direction = "left-right", sizes = [50], panes = ["rows", "notes"] }"#,
            "one size per pane",
        ),
        (
            r#"{ direction = "left-right", sizes = [60, 30], panes = ["rows", "notes"] }"#,
            "total 100",
        ),
        (
            r#"{ direction = "left-right", sizes = [80, 40], panes = ["rows", "notes"] }"#,
            "total 100",
        ),
        (
            r#"{ direction = "left-right", sizes = [5, 95], panes = ["rows", "notes"] }"#,
            "board.layout.sizes[0]",
        ),
        (
            r#"{ direction = "left-right", sizes = [{ grow = 0 }, 50], panes = ["rows", "notes"] }"#,
            "grow",
        ),
        (
            r#"{ direction = "left-right", sizes = [{ share = 1 }, 50], panes = ["rows", "notes"] }"#,
            "not a size setting",
        ),
        (
            r#"{ direction = "left-right", panes = ["rows"], mode = "tabs" }"#,
            "not a layout setting",
        ),
    ] {
        let message = message(text);
        assert!(message.contains(place), "{text}: {message}");
    }
}
