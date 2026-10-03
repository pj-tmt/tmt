use super::*;

fn inspect(shell: Shell, text: &str) -> Plan {
    Plan::from_bytes(shell, "unused".into(), text.as_bytes().to_vec()).unwrap()
}

#[test]
fn configured_state_recognizes_only_current_shell_lines() {
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        assert!(inspect(shell, shell.line()).configured);
        assert!(!inspect(shell, &format!("# {}", shell.line())).configured);
        assert!(!inspect(shell, "").configured);
    }
    assert!(!inspect(Shell::Bash, Shell::Fish.line()).configured);
}

#[test]
fn duplicate_and_ambiguous_lines_have_distinct_evidence() {
    let plan = inspect(Shell::Bash, &format!("{0}\n{0}\n", Shell::Bash.line()));
    assert!(plan.configured);
    assert_eq!(plan.warnings.len(), 1);
    assert!(plan.warnings[0].contains("Multiple"));
    let plan = inspect(Shell::Bash, "eval tmt __completion-script bash");
    assert!(!plan.configured);
    assert!(plan.warnings[0].contains("inspect it manually"));
}

#[test]
fn framework_initialization_is_advisory_and_ordering_requires_literal_evidence() {
    for framework in [
        "source \"$ZSH/oh-my-zsh.sh\"",
        "source \"${ZDOTDIR:-$HOME}/.zprezto/init.zsh\"",
        "autoload -Uz compinit",
        "",
    ] {
        let plan = inspect(Shell::Zsh, &format!("{framework}\n{}", Shell::Zsh.line()));
        assert!(plan.configured);
        assert!(plan.warnings.iter().any(|s| s.contains("may initialize")));
        assert!(!plan.warnings.iter().any(|s| s.contains("precedes")));
    }
    let plan = inspect(Shell::Zsh, &format!("{}\ncompinit\n", Shell::Zsh.line()));
    assert!(plan.warnings.iter().any(|s| s.contains("precedes")));
    let plan = inspect(
        Shell::Zsh,
        &format!("autoload -Uz compinit; compinit\n{}\n", Shell::Zsh.line()),
    );
    assert!(plan.warnings.is_empty());
}

#[test]
fn invalid_text_is_an_inspection_error() {
    assert!(Plan::from_bytes(Shell::Bash, "unused".into(), vec![0xff]).is_err());
}
