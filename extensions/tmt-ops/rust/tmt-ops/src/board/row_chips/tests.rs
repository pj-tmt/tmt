use super::*;
use crate::labels::{Label, Supplied};

const ID: &str = "10000000-0000-4000-8000-000000000001";

fn supplied(texts: &[(&str, Role)]) -> Supplied {
    Supplied::from_rows(vec![(
        "digest".into(),
        [(
            ID.to_owned(),
            texts
                .iter()
                .map(|(text, role)| Label {
                    text: (*text).into(),
                    role: *role,
                })
                .collect(),
        )]
        .into(),
    )])
}

fn row(held: u64) -> Value {
    json!({"id": ID, "digest": {"active": true, "digestUntilMs": 60_000, "heldCount": held}})
}

fn texts(chips: &[Chip]) -> Vec<&str> {
    chips.iter().map(|chip| chip.text.as_str()).collect()
}

#[test]
fn policy_chip_keeps_its_word_whole_and_its_suffix_complete() {
    let none = Supplied::default();
    let row = row(2);
    assert_eq!(fitted(&none, &row, 0, 5), (vec![], false));
    let (word, all) = fitted(&none, &row, 0, 6);
    assert_eq!((texts(&word), all), (vec!["digest"], false));
    let (full, all) = fitted(&none, &row, 0, 40);
    assert_eq!((texts(&full), all), (vec!["digest", " 1m · 2 held"], true));
    assert_eq!(full[0].role, Role::Text);
    assert_eq!(full[1].role, Role::Muted);
    // No policy, no chips; a row without an id has none either.
    assert!(of(&none, &json!({"id": ID}), 0).is_empty());
    assert!(of(&none, &json!({}), 0).is_empty());
}

#[test]
fn supplied_labels_replace_the_policy_chip_and_keep_their_roles() {
    let labels = supplied(&[
        ("Every 5m", Role::Text),
        ("4 held", Role::Muted),
        ("Due now", Role::Waiting),
    ]);
    let chips = of(&labels, &row(2), 0);
    assert_eq!(
        texts(&chips),
        ["Every 5m", " · 4 held", " · Due now"],
        "the policy chip must not stack with supplied labels"
    );
    assert_eq!(
        chips.iter().map(|chip| chip.role).collect::<Vec<_>>(),
        [Role::Text, Role::Muted, Role::Waiting]
    );
    // Another member keeps the policy chip: sources answer per identity.
    let other =
        json!({"id": "other", "digest": {"active": true, "digestUntilMs": 60_000, "heldCount": 0}});
    assert_eq!(texts(&of(&labels, &other, 0)), ["digest", " 1m"]);
}

#[test]
fn hidden_format_characters_in_label_text_never_reach_the_screen() {
    let labels = supplied(&[("Au\u{202e}to\u{200b}", Role::Text)]);
    assert_eq!(texts(&of(&labels, &row(0), 0)), ["Auto"]);
}

#[test]
fn narrow_budgets_keep_the_leading_labels_that_fit_whole() {
    let labels = supplied(&[
        ("Every 5m", Role::Text),
        ("4 held", Role::Muted),
        ("Due now", Role::Waiting),
    ]);
    let row = row(0);
    for (budget, shown, all) in [
        (7, 0, false),
        (8, 1, false),
        (16, 1, false),
        (17, 2, false),
        (27, 3, true),
        (80, 3, true),
    ] {
        let (chips, fits) = fitted(&labels, &row, 0, budget);
        assert_eq!((chips.len(), fits), (shown, all), "budget {budget}");
    }
}

#[test]
fn pieces_lead_with_one_gap_and_padded_closes_with_one() {
    let labels = supplied(&[("Auto", Role::Text), ("3 held", Role::Muted)]);
    let pieces = pieces(&labels, &row(0), 0, 40);
    assert_eq!(
        pieces,
        json!([
            {"id": "chip-0", "text": " Auto", "role": "text"},
            {"id": "chip-1", "text": " · 3 held", "role": "muted"}
        ])
    );
    assert_eq!(pieces_width(&pieces), 5 + 9);
    let padded = padded(pieces);
    assert_eq!(
        padded[2],
        json!({"id": "chip-2", "text": " ", "role": "text"})
    );
    assert_eq!(pieces_width(&padded), 5 + 9 + 1);
    // Nothing to show stays nothing, not a lone gap.
    assert_eq!(padded_empty(), json!([]));
}

fn padded_empty() -> Value {
    padded(pieces(&Supplied::default(), &json!({"id": ID}), 0, 40))
}

#[test]
fn detail_names_the_source_and_lists_every_label() {
    let labels = supplied(&[
        ("Auto", Role::Text),
        ("3 held", Role::Muted),
        ("When idle", Role::Muted),
    ]);
    assert_eq!(
        detail(&labels, &row(0), 0),
        [("digest".to_owned(), "Auto · 3 held · When idle".to_owned())]
    );
    assert_eq!(
        detail(&Supplied::default(), &row(2), 0),
        [("digest".to_owned(), "1m left · 2 held".to_owned())]
    );
    assert!(detail(&Supplied::default(), &json!({"id": ID}), 0).is_empty());
}

#[test]
fn only_the_policy_chip_depends_on_the_clock() {
    let labels = supplied(&[("Auto", Role::Text)]);
    assert_eq!(clock(&labels, &row(1), 0), None);
    assert_eq!(
        clock(&Supplied::default(), &row(1), 0).as_deref(),
        Some("digest 1m · 1 held")
    );
}
