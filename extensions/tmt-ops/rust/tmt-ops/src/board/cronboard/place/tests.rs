use super::*;

fn fake(dir: &std::path::Path, script: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let program = dir.join("tmux");
    crate::test_support::write_ready_executable(&program, script);
    program
}

fn dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tmt-squad-place-{name}-{}", std::process::id()))
}

#[test]
fn a_pane_resolves_once_on_the_invoker_socket_and_is_cached_per_pane_id() {
    let dir = dir("cache");
    let calls = dir.join("calls");
    let program = fake(
        &dir,
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\necho 'team:agents'\n",
            calls.display()
        ),
    );
    let mut places = Places::with_program(program, Some("/sock/a".into()));
    assert_eq!(places.resolve("%7").as_deref(), Some("team:agents"));
    assert_eq!(places.resolve("%7").as_deref(), Some("team:agents"));
    assert_eq!(places.resolve("%8").as_deref(), Some("team:agents"));
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "-S /sock/a display-message -p -t %7 #{session_name}:#{window_name}\n\
         -S /sock/a display-message -p -t %8 #{session_name}:#{window_name}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_failure_falls_back_to_the_pane_id_and_is_remembered() {
    let dir = dir("failure");
    let calls = dir.join("calls");
    let failing = fake(
        &dir,
        &format!("#!/bin/sh\necho x >> '{}'\nexit 1\n", calls.display()),
    );
    let mut places = Places::with_program(failing, Some("/sock/a".into()));
    assert_eq!(places.resolve("%7"), None);
    assert_eq!(places.resolve("%7"), None);
    assert_eq!(std::fs::read_to_string(&calls).unwrap(), "x\n", "one call");
    // A missing program, an empty answer and a non-pane id never resolve.
    let mut missing = Places::with_program(dir.join("absent"), Some("/sock/a".into()));
    assert_eq!(missing.resolve("%7"), None);
    let empty = fake(&dir, "#!/bin/sh\necho ''\n");
    assert_eq!(
        Places::with_program(empty, Some("/s".into())).resolve("%7"),
        None
    );
    let ok = fake(&dir, "#!/bin/sh\necho 'a:b'\n");
    let mut places = Places::with_program(ok, Some("/s".into()));
    for pane in ["-t", "7", "%", "%7; x", ""] {
        assert_eq!(places.resolve(pane), None, "{pane:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn outside_tmux_nothing_is_asked() {
    let dir = dir("outside");
    let calls = dir.join("calls");
    let program = fake(
        &dir,
        &format!("#!/bin/sh\necho x >> '{}'\necho 'a:b'\n", calls.display()),
    );
    let mut places = Places::with_program(program, None);
    assert_eq!(places.resolve("%7"), None);
    assert!(!calls.exists());
    let _ = std::fs::remove_dir_all(dir);
}
