use super::*;

#[test]
fn canonical_keys_preserve_javascript_normalization_and_contextual_lowercase() {
    // Non-English text is required Unicode compatibility fixture data.
    for (input, expected) in [
        (" Ａlice ", "alice"),
        ("\u{feff}Alice\u{feff}", "alice"),
        ("\u{85}Alice\u{85}", "\u{85}alice\u{85}"),
        ("ΟΣ", "ος"),
        ("ΟΣΑ", "οσα"),
        ("AΣ\u{301}", "aς\u{301}"),
        ("AΣ\u{301}A", "aσ\u{301}a"),
        ("Σ", "σ"),
        ("İIı", "i\u{307}iı"),
        ("Straße", "straße"),
        ("ẞ", "ß"),
        // Unicode 17 additions catch accidental use of the MSRV compiler's
        // older casing tables instead of the pinned normalization dependency.
        ("\u{16ea0}\u{16ea1}", "\u{16ebb}\u{16ebc}"),
        ("E\u{301}", "é"),
        ("KÅﬃ", "kåffi"),
        ("\u{a0}\u{2007}ALICE\u{202f}", "alice"),
        ("\u{200b}ALICE\u{200b}", "\u{200b}alice\u{200b}"),
    ] {
        assert_eq!(normalize_name(input), expected, "input {input:?}");
    }
    assert_ne!(
        normalize_name("Straße"),
        normalize_name("STRASSE"),
        "case folding would merge distinct names"
    );
}

#[test]
fn display_names_are_trimmed_but_not_compatibility_normalized() {
    let name = validate_name("\u{feff} Ａlice \u{2028}").unwrap();
    assert_eq!(name.display_name(), "Ａlice");
    assert_eq!(name.canonical_name(), "alice");
    assert!(
        validate_name("\u{85}").is_ok(),
        "NEL is neither ECMAScript whitespace nor a rejected C0 control"
    );
}

#[test]
fn all_ecmascript_spaces_trim_at_edges_but_not_inside_names() {
    let spaces = [
        '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', ' ', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
        '\u{3000}', '\u{feff}',
    ];
    for space in spaces {
        assert_eq!(normalize_name(&format!("{space}ALICE{space}")), "alice");
        assert_eq!(
            validate_name(&space.to_string()),
            Err(NameError::EmptyOrControl)
        );
        assert!(!is_pane_target(&format!("a{space}b:1.2")));
    }
}

#[test]
fn controls_are_checked_after_trim_not_silently_removed_from_the_middle() {
    for code in (0..0x20).chain([0x7f]) {
        let character = char::from_u32(code).unwrap();
        assert_eq!(
            validate_name(&format!("a{character}b")),
            Err(NameError::EmptyOrControl)
        );
    }
    assert_eq!(validate_name("\n Alice\t").unwrap().display_name(), "Alice");
    assert_eq!(validate_name(""), Err(NameError::EmptyOrControl));
    assert_eq!(validate_name("\0Alice"), Err(NameError::EmptyOrControl));
}

#[test]
fn pane_classification_and_name_rejection_share_exact_target_forms() {
    for target in [
        "%14",
        "10.3",
        "session:2.1",
        "％１４",
        "１０.３",
        "session：２.１",
        " %0014 ",
        "a\u{85}b:2.1",
    ] {
        assert!(is_pane_target(target), "{target:?}");
        assert_eq!(validate_name(target), Err(NameError::PaneTarget));
    }
    for name in [
        "backend",
        "%",
        "%1a",
        "%١",
        "١.٢",
        "-1.2",
        "1.-2",
        "1.2.3",
        ":1.2",
        "a:b:1.2",
        "a b:1.2",
        "a\u{feff}b:1.2",
        "1.",
        ".2",
        "repo/branch",
    ] {
        assert!(!is_pane_target(name), "{name:?}");
        assert!(validate_name(name).is_ok(), "{name:?}");
    }
}

#[test]
fn herdr_shaped_names_are_names_while_no_driver_claims_them() {
    // Herdr's targets belong to its driver (#1082): with none approved in
    // this process they are ordinary names, new or existing.
    for name in ["w1:p2", "W1:P2", "ｗ１:ｐ２"] {
        assert!(!is_pane_target(name), "{name:?}");
        assert!(validate_name(name).is_ok(), "{name:?}");
        assert!(validate_existing_name(name).is_ok(), "{name:?}");
    }
    // tmux targets were never names, so no existing identity holds one.
    for target in ["%14", "10.3", "session:2.1"] {
        assert_eq!(validate_existing_name(target), Err(NameError::PaneTarget));
    }
    assert_eq!(validate_existing_name(" "), Err(NameError::EmptyOrControl));
}
