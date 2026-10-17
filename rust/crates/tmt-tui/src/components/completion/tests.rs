use super::*;
fn candidates() -> Vec<Candidate> {
    [("agent:1", "agent", "core"), ("squad:2", "squad", "core")]
        .into_iter()
        .map(|(id, kind, label)| Candidate {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            insertion: format!("@{label}"),
        })
        .collect()
}
#[test]
fn completion_replaces_only_the_query_and_preserves_candidate_identity() {
    let mut editor = Editor::from("hello @co");
    let mut completion = Completion::default();
    completion.update(&editor, &candidates());
    assert!(completion.open());
    assert_eq!(completion.matches.len(), 2);
    completion.move_by(1);
    completion.update(&editor, &candidates());
    assert_eq!(completion.matches[completion.selected].id, "squad:2");
    assert!(completion.accept(&mut editor, 100));
    assert_eq!(editor.as_str(), "hello @core ");
    assert!(!completion.open());
}
#[test]
fn dismiss_no_matches_and_overflow_never_change_source() {
    let mut editor = Editor::from("@unknown");
    let mut completion = Completion::default();
    completion.update(&editor, &candidates());
    assert!(completion.open() && completion.matches.is_empty());
    assert!(!completion.accept(&mut editor, 100));
    completion.dismiss(&editor);
    completion.update(&editor, &candidates());
    assert!(!completion.open());
    editor = "@c".into();
    completion.update(&editor, &candidates());
    assert!(!completion.accept(&mut editor, 2));
    assert_eq!(editor.as_str(), "@c");
    assert!(completion.open());
}
#[test]
fn emails_and_folded_source_do_not_open_completion() {
    for text in ["name@example", "word@@core", "plain"] {
        let mut completion = Completion::default();
        completion.update(&text.into(), &candidates());
        assert!(!completion.open());
    }
    let mut editor = Editor::default();
    editor.insert_folded("@core", "[source]".into(), 100);
    let mut completion = Completion::default();
    completion.update(&editor, &candidates());
    assert!(!completion.open());
}

fn named_candidates(labels: &[&str]) -> Vec<Candidate> {
    labels
        .iter()
        .map(|label| Candidate {
            id: (*label).into(),
            kind: "agent".into(),
            label: (*label).into(),
            insertion: format!("@{label}"),
        })
        .collect()
}

#[test]
fn fuzzy_completion_ignores_separators_and_keeps_unicode_names() {
    for (query, label) in [
        ("ux1", "ux-1"),
        ("rvmp", "ux-revamp"),
        ("coreops", "core-ops"),
        ("nl", "night-lead"),
        ("tul", "tmt-ux-lead"),
        ("UX1", "ux_1"),
        ("é班", "Équipe.班"),
    ] {
        let editor = Editor::from(format!("hello @{query}"));
        let mut completion = Completion::default();
        completion.update(&editor, &named_candidates(&[label]));
        assert_eq!(completion.matches.len(), 1, "{query} should match {label}");
    }
}

#[test]
fn fuzzy_completion_ranks_matches_and_keeps_inventory_order_on_ties() {
    let mut completion = Completion::default();
    completion.update(
        &Editor::from("@abc"),
        &named_candidates(&[
            "azbzc",
            "xxabcxx",
            "alpha beta charlie",
            "a-b-c-tail",
            "abc",
            "axbyc",
            "unrelated",
        ]),
    );
    assert_eq!(
        completion
            .matches
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        [
            "abc",
            "a-b-c-tail",
            "alpha beta charlie",
            "xxabcxx",
            "azbzc",
            "axbyc"
        ]
    );
}

#[test]
fn fuzzy_highlights_are_label_source_ranges_and_clear_with_the_picker() {
    for (query, label, marked) in [
        ("ux1", "u-x_1", "ux1"),
        ("tul", "tmt-ux-lead", "tul"),
        ("é班", "Équipe.班", "É班"),
        ("i", "İpek", "İ"),
    ] {
        let mut editor = Editor::from(format!("@{query}"));
        let mut completion = Completion::default();
        completion.update(&editor, &named_candidates(&[label]));
        let highlighted: String = completion
            .matched_ranges(0)
            .iter()
            .map(|range| &label[range.clone()])
            .collect();
        assert_eq!(highlighted, marked);
        assert!(completion.accept(&mut editor, 100));
        assert_eq!(editor.as_str(), format!("@{label} "));
        assert!(completion.matched_ranges(0).is_empty());
    }
    let editor = Editor::from("@ux1");
    let mut completion = Completion::default();
    completion.update(&editor, &named_candidates(&["ux-1"]));
    completion.dismiss(&editor);
    assert!(completion.matched_ranges(0).is_empty());
    completion.update(&Editor::from("plain"), &named_candidates(&["ux-1"]));
    assert!(completion.matched_ranges(0).is_empty());
}

#[test]
fn empty_query_preserves_inventory_order_without_highlights() {
    let mut completion = Completion::default();
    completion.update(&Editor::from("@"), &named_candidates(&["z", "", "a"]));
    assert_eq!(
        completion
            .matches
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        ["z", "", "a"]
    );
    assert!((0..3).all(|at| completion.matched_ranges(at).is_empty()));
}
