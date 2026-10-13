use super::*;

#[test]
fn shell_digits_do_not_consume_controls_or_reader_input() {
    let mut shell = OpsShell::default();
    for owner in [
        OpsInputOwner::Composer,
        OpsInputOwner::Search,
        OpsInputOwner::Completion,
        OpsInputOwner::Reader,
        OpsInputOwner::Popup,
    ] {
        for digit in ['1', '2', '3'] {
            assert!(!shell.select_digit(digit, owner));
            assert_eq!(shell.tab, OpsTab::Overview);
        }
    }
    assert!(shell.select_digit('2', OpsInputOwner::List));
    assert_eq!(shell.tab, OpsTab::Agents);
    assert!(!shell.select_digit('9', OpsInputOwner::List));
    assert_eq!(shell.tab, OpsTab::Agents);
    assert!(shell.select_digit('3', OpsInputOwner::List));
    assert_eq!(shell.tab, OpsTab::Digest);
    assert!(shell.select_digit('1', OpsInputOwner::List));
}

#[test]
fn agents_label_never_includes_an_aggregate_badge() {
    assert_eq!(OpsTab::Overview.label(), "1 Overview");
    assert_eq!(OpsTab::Agents.label(), "2 Agents");
    assert_eq!(OpsTab::Digest.label(), "3 Digest");
}
