use super::*;

#[test]
fn shell_digits_preserve_typing_and_switch_every_non_typing_owner() {
    let mut shell = OpsShell::default();
    for owner in [
        OpsInputOwner::Composer,
        OpsInputOwner::Search,
        OpsInputOwner::Completion,
    ] {
        for digit in ['1', '2', '3'] {
            assert!(!shell.select_digit(digit, owner));
            assert_eq!(shell.tab, OpsTab::Overview);
        }
    }
    for owner in [
        OpsInputOwner::List,
        OpsInputOwner::Reader,
        OpsInputOwner::Popup,
    ] {
        for (digit, expected) in [
            ('2', OpsTab::Agents),
            ('3', OpsTab::Digest),
            ('1', OpsTab::Overview),
        ] {
            assert!(shell.select_digit(digit, owner));
            assert_eq!(shell.tab, expected);
        }
    }
    assert!(!shell.select_digit('9', OpsInputOwner::List));
    assert_eq!(shell.tab, OpsTab::Overview);
    assert!(shell.select_digit('3', OpsInputOwner::List));
    assert_eq!(shell.tab, OpsTab::Digest);
    assert!(shell.select_digit('1', OpsInputOwner::List));
}

#[test]
fn agents_label_never_includes_an_aggregate_badge() {
    assert_eq!(OpsTab::Overview.label(), "Overview");
    assert_eq!(OpsTab::Agents.label(), "Agents");
    assert_eq!(OpsTab::Digest.label(), "Digest");
}
