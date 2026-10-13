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
