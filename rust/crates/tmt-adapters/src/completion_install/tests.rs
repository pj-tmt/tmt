use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tmt-completion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn append_preserves_bytes_and_is_idempotent_for_each_shell() {
    let root = Directory::new();
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        let path = root.0.join(shell.name());
        let original = if shell == Shell::Zsh {
            "# keep this\nautoload -Uz compinit\ncompinit"
        } else {
            "# keep this"
        };
        fs::write(&path, original).unwrap();
        assert!(
            Plan::inspect(shell, path.clone())
                .unwrap()
                .append()
                .unwrap()
        );
        let expected = format!("{original}\n{}\n", shell.line());
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert!(
            !Plan::inspect(shell, path.clone())
                .unwrap()
                .append()
                .unwrap()
        );
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
    assert_eq!(
        fs::read_dir(&root.0).unwrap().count(),
        3,
        "no backup or lock sidecars"
    );
}

#[test]
fn old_duplicate_and_commented_lines_have_distinct_evidence() {
    let text = "# source <(tmt completion bash)\nsource <(tmux-team completion bash)\nsource <(tmt __completion-script bash)\n";
    let plan = Plan::from_bytes(Shell::Bash, "ignored".into(), text.into()).unwrap();
    assert!(plan.configured);
    assert_eq!(plan.warnings.len(), 2);
    assert!(!plan.append().unwrap());
    let plan = Plan::from_bytes(
        Shell::Bash,
        "ignored".into(),
        b"# source <(tmt completion bash)".to_vec(),
    )
    .unwrap();
    assert!(!plan.configured);
    assert!(plan.warnings.is_empty());
}

#[test]
fn zsh_requires_recognized_initialization_and_never_edits_existing_lines() {
    let root = Directory::new();
    let path = root.0.join(".zshrc");
    fs::write(&path, "autoload -Uz compinit\n").unwrap();
    assert!(
        Plan::inspect(Shell::Zsh, path.clone())
            .unwrap()
            .append()
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "autoload -Uz compinit\n"
    );
    fs::write(&path, "source <(tmt completion zsh)\ncompinit\n").unwrap();
    let plan = Plan::inspect(Shell::Zsh, path).unwrap();
    assert!(plan.configured);
    assert!(plan.warnings.iter().any(|s| s.contains("precedes")));
}

#[test]
fn changed_unreadable_and_ambiguous_files_are_preserved() {
    let root = Directory::new();
    let path = root.0.join(".bashrc");
    let plan = Plan::inspect(Shell::Bash, path.clone()).unwrap();
    fs::write(&path, "new user content\n").unwrap();
    assert!(plan.append().is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "new user content\n");
    fs::write(&path, "eval tmt completion bash\n").unwrap();
    let plan = Plan::inspect(Shell::Bash, path.clone()).unwrap();
    assert!(!plan.configured);
    assert!(plan.append().is_err());
    fs::write(&path, [0xff]).unwrap();
    assert!(Plan::inspect(Shell::Bash, path).is_err());
    assert!(Plan::inspect(Shell::Bash, root.0.clone()).is_err());
}
