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

/// The leading chips that fit `budget` columns, and whether that is all of them.
fn fitted(supplied: &Supplied, row: &Value, now: u64, budget: usize) -> (Vec<Chip>, bool) {
    let mut chips = of(supplied, row, now);
    let shown = prefix(&chips, budget);
    let all = shown == chips.len();
    chips.truncate(shown);
    (chips, all)
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
fn supplied_labels_follow_the_source_name_and_replace_the_policy_chip() {
    let labels = supplied(&[
        ("Every 5m", Role::Text),
        ("4 held", Role::Muted),
        ("Due now", Role::Waiting),
    ]);
    let chips = of(&labels, &row(2), 0);
    assert_eq!(
        texts(&chips),
        ["digest ", "Every 5m", " · 4 held", " · Due now"],
        "the policy chip must not stack with supplied labels"
    );
    assert_eq!(
        chips.iter().map(|chip| chip.role).collect::<Vec<_>>(),
        [Role::Muted, Role::Text, Role::Muted, Role::Waiting]
    );
    assert_eq!(
        chips.iter().map(|chip| chip.head).collect::<Vec<_>>(),
        [true, false, false, false]
    );
    // Another member keeps the policy chip: sources answer per identity.
    let other =
        json!({"id": "other", "digest": {"active": true, "digestUntilMs": 60_000, "heldCount": 0}});
    assert_eq!(texts(&of(&labels, &other, 0)), ["digest", " 1m"]);
}

#[test]
fn each_source_leads_its_own_labels() {
    let rows = |text: &str| {
        [(
            ID.to_owned(),
            vec![Label {
                text: text.into(),
                role: Role::Text,
            }],
        )]
        .into()
    };
    let labels = Supplied::from_rows(vec![
        ("digest".into(), rows("Auto")),
        ("other".into(), rows("Soon")),
    ]);
    let chips = of(&labels, &row(0), 0);
    assert_eq!(texts(&chips), ["digest ", "Auto", " · other ", "Soon"]);
    // A second source whose first label does not fit is dropped whole.
    assert_eq!(prefix(&chips, 10), 0);
    assert_eq!(prefix(&chips, 11), 2);
    assert_eq!(prefix(&chips, 23), 2);
    assert_eq!(prefix(&chips, 24), 4);
}

#[test]
fn hidden_format_characters_in_label_text_never_reach_the_screen() {
    let labels = supplied(&[("Au\u{202e}to\u{200b}", Role::Text)]);
    assert_eq!(texts(&of(&labels, &row(0), 0)), ["digest ", "Auto"]);
}

#[test]
fn narrow_budgets_keep_the_source_name_with_the_labels_that_fit_whole() {
    let labels = supplied(&[
        ("Every 5m", Role::Text),
        ("4 held", Role::Muted),
        ("Due now", Role::Waiting),
    ]);
    let row = row(0);
    // "digest " 7 + "Every 5m" 8, then " · 4 held" 9 and " · Due now" 10.
    for (budget, shown, all) in [
        (7, 0, false),
        (14, 0, false),
        (15, 2, false),
        (23, 2, false),
        (24, 3, false),
        (33, 3, false),
        (34, 4, true),
        (80, 4, true),
    ] {
        let (chips, fits) = fitted(&labels, &row, 0, budget);
        assert_eq!((chips.len(), fits), (shown, all), "budget {budget}");
    }
}

#[test]
fn shown_leads_with_one_gap_and_closes_with_one_when_asked() {
    let labels = supplied(&[("Auto", Role::Text), ("3 held", Role::Muted)]);
    let open = shown(&labels, &row(0), 0, 40, false);
    assert_eq!(
        open.pieces,
        json!([
            {"id": "chip-0", "text": " digest ", "role": "muted"},
            {"id": "chip-1", "text": "Auto", "role": "text"},
            {"id": "chip-2", "text": " · 3 held", "role": "muted"}
        ])
    );
    assert_eq!((open.width, open.all), (8 + 4 + 9, true));
    let closed = shown(&labels, &row(0), 0, 40, true);
    assert_eq!(
        closed.pieces[3],
        json!({"id": "chip-3", "text": " ", "role": "text"})
    );
    assert_eq!(closed.width, 8 + 4 + 9 + 1);
    // The room counts the gaps: trailing labels drop whole to make them fit.
    let tight = shown(&labels, &row(0), 0, 12 + 9 - 1, false);
    assert_eq!((tight.width, tight.all), (12, false));
    let tight = shown(&labels, &row(0), 0, 12 + 9, true);
    assert_eq!((tight.width, tight.all), (13, false));
    // Nothing to show stays nothing, not a lone gap.
    let none = shown(&Supplied::default(), &json!({"id": ID}), 0, 40, true);
    assert_eq!((none.pieces, none.width, none.all), (json!([]), 0, true));
    // When the name and its first label do not fit, no chip shows.
    let cramped = shown(&labels, &row(0), 0, 11, false);
    assert_eq!((cramped.pieces, cramped.all), (json!([]), false));
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
