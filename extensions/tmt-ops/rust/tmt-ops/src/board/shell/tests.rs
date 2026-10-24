use super::*;

#[test]
fn shell_digits_preserve_typing_and_switch_every_non_typing_owner() {
    let mut shell = OpsShell::default();
    let mut selected = None;
    for owner in [
        OpsInputOwner::Composer,
        OpsInputOwner::Search,
        OpsInputOwner::Completion,
    ] {
        for digit in ['1', '2'] {
            assert!(!shell.select_digit(digit, owner, &mut selected));
            assert_eq!(shell.tab, OpsTab::Agents);
        }
    }
    for owner in [
        OpsInputOwner::List,
        OpsInputOwner::Reader,
        OpsInputOwner::Popup,
    ] {
        for (digit, expected) in [('2', OpsTab::Digest), ('1', OpsTab::Agents)] {
            assert!(shell.select_digit(digit, owner, &mut selected));
            assert_eq!(shell.tab, expected);
        }
    }
    assert!(!shell.select_digit('9', OpsInputOwner::List, &mut selected));
    assert_eq!(shell.tab, OpsTab::Agents);
    assert!(shell.select_digit('2', OpsInputOwner::List, &mut selected));
    assert_eq!(shell.tab, OpsTab::Digest);
    assert!(shell.select_digit('1', OpsInputOwner::List, &mut selected));
}

#[test]
fn agents_label_never_includes_an_aggregate_badge() {
    assert_eq!(OpsTab::Agents.label(), "Agents");
    assert_eq!(OpsTab::Digest.label(), "Digest");
}
