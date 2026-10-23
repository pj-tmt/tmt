use super::*;
use serde_json::json;

fn render(account: &Account<'_>) -> String {
    let mut bytes = Vec::new();
    write(&mut bytes, Terminal::PLAIN, account).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn before(versions: &[(&str, &str)]) -> BTreeMap<String, String> {
    versions
        .iter()
        .map(|(product, version)| ((*product).into(), (*version).into()))
        .collect()
}

fn account<'a>(
    products: &'a [Value],
    skills: &'a Value,
    before: &'a BTreeMap<String, String>,
) -> Account<'a> {
    Account {
        products,
        has_report: true,
        skipped_pinned: false,
        before: Some(before),
        skills,
        repeat: None,
        followups: Followups {
            required: &[],
            optional: &[],
            conflict_hints: Vec::new(),
            reload_hint: None,
        },
    }
}

fn changed(product: &str, version: &str) -> Value {
    json!({"product": product, "status": "changed", "version": version})
}

fn current(product: &str, version: &str) -> Value {
    json!({"product": product, "status": "unchanged", "version": version})
}

fn skills(changed: bool) -> Value {
    json!({"refreshed": [{"target": "/a/tmt", "changed": changed}], "skipped": [], "conflicts": []})
}

fn versions() -> BTreeMap<String, String> {
    before(&[
        ("cli", "5.0.0-alpha.40"),
        ("remote", "0.1.0-alpha.8"),
        ("ops", "0.1.0-alpha.88"),
        ("digest", "0.1.0-alpha.3"),
    ])
}

#[test]
fn three_changed_components_make_one_table_one_summary_and_one_next_list() {
    let products = [
        changed("cli", "5.0.0-alpha.41"),
        changed("remote", "0.1.0-alpha.9"),
        changed("ops", "0.1.0-alpha.89"),
        current("digest", "0.1.0-alpha.3"),
    ];
    let (versions, skills) = (versions(), skills(true));
    let mut shown = account(&products, &skills, &versions);
    let required = ["Pane option cutover unavailable: boom".to_owned()];
    shown.followups = Followups {
        required: &required,
        optional: &["The previous installation at /x is kept."],
        conflict_hints: vec!["User-owned skill preserved: mv a b".into()],
        reload_hint: Some("Reload or restart your agent"),
    };
    assert_eq!(
        render(&shown),
        "  tmt     5.0.0-alpha.40 → 5.0.0-alpha.41  ✓ updated\n\
         \x20 remote  0.1.0-alpha.8 → 0.1.0-alpha.9    ✓ updated\n\
         \x20 ops     0.1.0-alpha.88 → 0.1.0-alpha.89  ✓ updated\n\
         Up to date: digest 0.1.0-alpha.3\n\
         ✓ Updated 3 · skills refreshed\n\
         Next:\n\
         \x20 Pane option cutover unavailable: boom\n\
         \x20 User-owned skill preserved: mv a b\n\
         \x20 The previous installation at /x is kept.\n\
         \x20 Reload or restart your agent\n"
    );
}

#[test]
fn the_reload_hint_follows_only_a_skill_that_changed() {
    let products = [current("cli", "5.0.0-alpha.41")];
    let versions = versions();
    for (skills, hinted) in [(skills(false), false), (skills(true), true)] {
        let mut shown = account(&products, &skills, &versions);
        shown.followups.reload_hint = Some("Reload or restart your agent");
        let text = render(&shown);
        assert_eq!(
            text.contains("Reload or restart your agent"),
            hinted,
            "{text}"
        );
    }
}

#[test]
fn nothing_to_do_is_one_muted_line_and_a_summary() {
    let products = [
        current("cli", "5.0.0-alpha.41"),
        current("remote", "0.1.0-alpha.9"),
    ];
    let (versions, skills) = (versions(), skills(false));
    assert_eq!(
        render(&account(&products, &skills, &versions)),
        "Up to date: tmt 5.0.0-alpha.41, remote 0.1.0-alpha.9\n✓ Up to date · skills current\n"
    );
    let one = [
        changed("cli", "5.0.0-alpha.41"),
        current("remote", "0.1.0-alpha.9"),
    ];
    let skills = self::skills(true);
    assert_eq!(
        render(&account(&one, &skills, &versions)),
        "  tmt  5.0.0-alpha.40 → 5.0.0-alpha.41  ✓ updated\n\
         Up to date: remote 0.1.0-alpha.9\n\
         ✓ Updated 1 · skills refreshed\n"
    );
}

#[test]
fn a_failed_component_keeps_its_error_and_the_others_their_result() {
    let products = [
        changed("cli", "5.0.0-alpha.41"),
        json!({"product":"ops","status":"failed","error":{"code":"EXTENSION_UPGRADE_FAILED","message":"refused","suggestion":"tmt upgrade"}}),
        changed("remote", "0.1.0-alpha.9"),
    ];
    let (versions, skills) = (versions(), skills(true));
    let mut shown = account(&products, &skills, &versions);
    shown.repeat = Some("tmt upgrade --channel alpha --yes");
    assert_eq!(
        render(&shown),
        "  tmt     5.0.0-alpha.40 → 5.0.0-alpha.41  ✓ updated\n\
         \x20 ops     0.1.0-alpha.88                   ✗ failed\n\
         \x20 remote  0.1.0-alpha.8 → 0.1.0-alpha.9    ✓ updated\n\
         error: ops: EXTENSION_UPGRADE_FAILED: refused\n\
         ✗ 1 failed · updated 2 · skills refreshed\n\
         Next:\n\
         \x20 tmt upgrade --channel alpha --yes\n"
    );
    // Without a known invocation the row's own retry hint stands in.
    assert!(render(&account(&products, &skills, &versions)).ends_with("Next:\n  tmt upgrade\n"));
}

#[test]
fn an_update_waiting_for_approval_names_the_command_that_applies_it() {
    let products = [
        current("cli", "5.0.0-alpha.41"),
        json!({"product":"ops","status":"consentRequired","version":"0.1.0-alpha.88","availableVersion":"0.1.0-alpha.89","hint":"tmt upgrade --yes"}),
    ];
    let (versions, skills) = (versions(), skills(false));
    assert_eq!(
        render(&account(&products, &skills, &versions)),
        "  ops  0.1.0-alpha.88 → 0.1.0-alpha.89  ! needs approval\n\
         Up to date: tmt 5.0.0-alpha.41\n\
         ! Nothing changed · 1 waiting for approval · skills current\n\
         Next:\n\
         \x20 tmt upgrade --yes\n"
    );
}

#[test]
fn pinned_components_are_listed_and_the_way_out_is_optional() {
    let products = [
        json!({"product":"cli","status":"skippedPinned","version":"5.0.0-alpha.40"}),
        json!({"product":"ops","status":"skippedPinned","version":"0.1.0-alpha.88","hint":"tmt extension upgrade ops --unpin","message":"ops 0.1.0-alpha.88 is pinned; nothing changed. Clear the pin with: tmt extension upgrade ops --unpin"}),
    ];
    let versions = versions();
    let mut shown = account(&products, &Value::Null, &versions);
    shown.skipped_pinned = true;
    assert_eq!(
        render(&shown),
        "  tmt  5.0.0-alpha.40  ! pinned\n\
         \x20 ops  0.1.0-alpha.88  ! pinned\n\
         ! Nothing changed · 2 pinned\n\
         Next:\n\
         \x20 ops 0.1.0-alpha.88 is pinned; nothing changed. Clear the pin with: tmt extension upgrade ops --unpin\n"
    );
}

#[test]
fn one_restart_notice_and_every_warning_survive_two_components_saying_the_same() {
    let products = [
        changed("cli", "5.0.0-alpha.41"),
        json!({"product":"remote","status":"changed","version":"0.1.0-alpha.9","details":{"restartHint":"one restart notice","skills":{"warning":{"message":"permission denied at /agent/tmt-remote"}}}}),
        json!({"product":"ops","status":"changed","version":"0.1.0-alpha.89","details":{"restartHint":"one restart notice"}}),
    ];
    let (versions, skills) = (versions(), skills(true));
    let text = render(&account(&products, &skills, &versions));
    assert_eq!(text.matches("one restart notice").count(), 1, "{text}");
    assert!(
        text.contains("warning: permission denied at /agent/tmt-remote\n"),
        "{text}"
    );
    assert!(text.ends_with("Next:\n  one restart notice\n"), "{text}");
}

#[test]
fn the_summary_counts_only_missing_core_skills_and_names_conflicts() {
    let products = [current("cli", "5.0.0-alpha.41")];
    let versions = versions();
    let office = json!({
        "refreshed": vec![json!({"target": "/a/tmt", "changed": false}); 8],
        "skipped": [
            "/a/tmt-office", "/a/tmt-avatar-create", "/a/tmt-prop-create",
            "/b/tmt-office", "/b/tmt-avatar-create", "/b/tmt-prop-create"
        ],
        "conflicts": [],
    });
    assert_eq!(
        render(&account(&products, &office, &versions)),
        "Up to date: tmt 5.0.0-alpha.41\n✓ Up to date · skills current\n"
    );
    let mut missing = office.clone();
    missing["skipped"]
        .as_array_mut()
        .unwrap()
        .push(json!("/c/tmt"));
    missing["conflicts"] = json!(["/d/tmt-inbox"]);
    assert_eq!(
        render(&account(&products, &missing, &versions)),
        "Up to date: tmt 5.0.0-alpha.41\n✓ Up to date · skills current · 1 skill missing · 1 skill conflict preserved\n"
    );
}

#[test]
fn a_failure_before_any_installation_prints_no_table_or_summary() {
    let products = [json!({"product":"cli","status":"failed","error":{"code":"X","message":"y"}})];
    let (versions, skills) = (versions(), Value::Null);
    let mut shown = account(&products, &skills, &versions);
    shown.has_report = false;
    assert_eq!(render(&shown), "");
}

#[test]
fn colour_marks_the_state_and_mutes_the_optional_while_plain_text_is_unchanged() {
    let products = [
        changed("cli", "5.0.0-alpha.41"),
        json!({"product":"ops","status":"failed","error":{"code":"E","message":"m"}}),
    ];
    let (versions, skills) = (versions(), skills(true));
    let mut shown = account(&products, &skills, &versions);
    shown.followups.optional = &["only helps"];
    let plain = render(&shown);
    let mut bytes = Vec::new();
    let colour = Terminal {
        color: true,
        width: None,
        theme: None,
    };
    write(&mut bytes, colour, &shown).unwrap();
    let coloured = String::from_utf8(bytes).unwrap();
    assert!(coloured.contains('\u{1b}'), "{coloured:?}");
    let stripped = strip_ansi(&coloured);
    assert_eq!(stripped, plain);
    assert!(!plain.contains('\u{1b}'));
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[test]
fn the_summary_mark_and_the_table_mark_are_one_colour() {
    let products = [changed("cli", "5.0.0-alpha.41")];
    let (versions, skills) = (versions(), skills(true));
    let mut bytes = Vec::new();
    let colour = Terminal {
        color: true,
        width: None,
        theme: None,
    };
    write(&mut bytes, colour, &account(&products, &skills, &versions)).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let escape_before = |at: usize| text[..at].rsplit('\u{1b}').next().unwrap().to_owned();
    let marks: Vec<_> = text
        .match_indices('✓')
        .map(|(at, _)| escape_before(at))
        .collect();
    assert_eq!(marks.len(), 2, "{text:?}");
    assert_eq!(marks[0], marks[1], "{text:?}");
}
