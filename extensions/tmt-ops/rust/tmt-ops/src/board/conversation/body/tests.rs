use super::*;
fn mapped() -> Mapped {
    Mapped {
        lines: (0..35)
            .map(|index| Line::raw(format!("row {index}")))
            .collect(),
        sources: (0..35).collect(),
        mentions: vec![vec![]; 35],
    }
}
fn bodies() -> Vec<Body> {
    vec![
        Body {
            key: Key::Reply("older".into()),
            sources: 0..18,
            newest_reply: false,
        },
        Body {
            key: Key::Reply("newest".into()),
            sources: 18..35,
            newest_reply: true,
        },
    ]
}
#[test]
fn each_body_folds_independently_and_newest_reply_starts_expanded() {
    let mut folds = Folds::default();
    let projected = folds.project(mapped(), None, &bodies(), 80, Look::default());
    assert_eq!(projected.mapped.lines.len(), 12 + 17 + 1);
    assert_eq!(
        projected.mapped.lines[11].to_string().trim_end(),
        "… 7 more lines · ⏎ expand"
    );
    assert_eq!(
        projected.original[11], None,
        "the caption is never an exact response source"
    );
    assert!(projected.mapped.mentions[11].is_empty());
    assert_eq!(projected.mapped.lines[12].to_string(), "row 18");
    assert_eq!(
        projected
            .mapped
            .lines
            .last()
            .unwrap()
            .to_string()
            .trim_end(),
        "⏎ collapse"
    );
    assert!(folds.toggle(&Key::Reply("older".into())));
    let both = folds.project(mapped(), None, &bodies(), 80, Look::default());
    assert_eq!(both.mapped.lines.len(), 37);
    assert!(folds.toggle(&Key::Reply("newest".into())));
    let manual = folds.project(mapped(), None, &bodies(), 80, Look::default());
    assert_eq!(
        manual.mapped.lines.len(),
        19 + 12,
        "a refresh never re-expands an explicitly collapsed newest reply"
    );
}
#[test]
fn stale_body_state_is_pruned_and_source_mapping_survives_folding() {
    let mut folds = Folds {
        focused: Some(Key::Reply("gone".into())),
        ..Default::default()
    };
    let result = folds.project(
        mapped(),
        Some((0..35).map(Some).collect()),
        &bodies(),
        30,
        Look::default(),
    );
    assert_eq!(folds.focused, None);
    assert_eq!(result.original[12], Some(18));
    assert_eq!(
        result.mapped.sources,
        (0..result.mapped.lines.len()).collect::<Vec<_>>()
    );
    assert!(!folds.toggle(&Key::Reply("gone".into())));
    let empty = folds.project(
        Mapped {
            lines: vec![],
            sources: vec![],
            mentions: vec![],
        },
        None,
        &[],
        1,
        Look::default(),
    );
    assert!(empty.controls.is_empty() && folds.expanded.is_empty());
}

#[test]
fn a_body_that_no_longer_needs_a_fold_control_releases_control_focus() {
    let mut folds = Folds::default();
    folds.project(mapped(), None, &bodies(), 80, Look::default());
    folds.focused = Some(Key::Reply("older".into()));
    let short = Mapped {
        lines: vec![Line::raw("now short")],
        sources: vec![0],
        mentions: vec![vec![]],
    };
    let body = Body {
        key: Key::Reply("older".into()),
        sources: 0..1,
        newest_reply: false,
    };
    let projected = folds.project(short, None, &[body], 80, Look::default());
    assert!(projected.controls.is_empty());
    assert_eq!(
        folds.focused, None,
        "Enter must not be captured by an invisible control"
    );
}
