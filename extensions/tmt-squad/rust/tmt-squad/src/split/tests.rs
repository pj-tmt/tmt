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
    assert_eq!(Size::Grow(2).constraint(), Constraint::Fill(2));
    assert_eq!(Size::Percent(30).constraint(), Constraint::Percentage(30));
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

#[test]
fn empty_fold_set_keeps_the_original_solver_rectangles() {
    fn original(split: &Split, area: Rect) -> Vec<(Pane, Rect)> {
        match split {
            Split::Pane(pane) => vec![(*pane, area)],
            Split::Group {
                direction,
                children,
            } => {
                let constraints = children.iter().map(|(size, _)| size.constraint());
                let areas = match direction {
                    Direction::LeftRight => Layout::horizontal(constraints).split(area),
                    Direction::TopBottom => Layout::vertical(constraints).split(area),
                };
                children
                    .iter()
                    .zip(areas.iter())
                    .flat_map(|((_, child), rect)| original(child, *rect))
                    .collect()
            }
        }
    }
    let tree = parse(r#"{ direction = "left-right", sizes = [30, { grow = 1 }], panes = ["rows", { direction = "top-bottom", sizes = [30, { grow = 2 }, { grow = 1 }], panes = ["detail", "notes", "replies"] }] }"#).unwrap();
    for (w, h) in [(100, 40), (31, 9), (3, 2), (0, 0)] {
        let area = Rect::new(2, 3, w, h);
        assert_eq!(tree.solve(area, &BTreeSet::new()), original(&tree, area));
    }
}

#[test]
fn folded_siblings_redistribute_percentages_and_grow_shares_in_both_directions() {
    for direction in [Direction::LeftRight, Direction::TopBottom] {
        let tree = Split::simple(
            direction,
            &[Pane::Rows, Pane::Detail, Pane::Notes],
            &[40, 20, 40],
        );
        let folded = BTreeSet::from([Pane::Detail]);
        let areas = tree.solve(Rect::new(0, 0, 100, 101), &folded);
        let extent = |r: Rect| {
            if direction == Direction::LeftRight {
                r.width
            } else {
                r.height
            }
        };
        assert!(extent(areas[0].1).abs_diff(extent(areas[2].1)) <= 1);
        if direction == Direction::LeftRight {
            assert_eq!(areas[1].1.x, areas[0].1.right() + 1);
        }
        assert_eq!(
            extent(areas[1].1),
            if direction == Direction::LeftRight {
                8
            } else {
                1
            }
        );
        assert_eq!(areas[1].1.height, 1);
    }
    let mixed=parse(r#"{ direction = "top-bottom", sizes = [30, { grow = 2 }, { grow = 1 }], panes = ["detail", "rows", "notes"] }"#).unwrap();
    let areas = mixed.solve(Rect::new(0, 0, 80, 31), &BTreeSet::from([Pane::Detail]));
    assert_eq!(
        areas.iter().map(|(_, r)| r.height).collect::<Vec<_>>(),
        [1, 20, 10]
    );
    let areas = mixed.solve(Rect::new(0, 0, 80, 32), &BTreeSet::from([Pane::Notes]));
    // Effective original shares: detail 30%, rows 70% * 2/3.
    assert_eq!(
        areas.iter().map(|(_, r)| r.height).collect::<Vec<_>>(),
        [12, 19, 1]
    );
}

#[test]
fn folded_subtree_shrinks_and_tiny_rectangles_never_overlap_or_escape() {
    let tree=parse(r#"{ direction = "left-right", panes = ["rows", { direction = "top-bottom", panes = ["detail", "notes", "replies"] }] }"#).unwrap();
    let right = BTreeSet::from([Pane::Detail, Pane::Notes, Pane::Replies]);
    let areas = tree.solve(Rect::new(0, 0, 100, 20), &right);
    assert_eq!(areas[0].1.width, 90);
    assert_eq!(areas[1].1.x, 91);
    assert_eq!(
        areas.iter().skip(1).map(|(_, r)| r.y).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    for collapsed in [
        right,
        BTreeSet::from([Pane::Rows, Pane::Detail, Pane::Notes, Pane::Replies]),
    ] {
        for w in 0..20 {
            for h in 0..6 {
                let area = Rect::new(2, 3, w, h);
                let solved = tree.solve(area, &collapsed);
                for (i, (_, rect)) in solved.iter().enumerate() {
                    assert!(rect.right() <= area.right() && rect.bottom() <= area.bottom());
                    for (_, other) in &solved[..i] {
                        assert!(rect.intersection(*other).is_empty());
                    }
                }
            }
        }
    }
}
