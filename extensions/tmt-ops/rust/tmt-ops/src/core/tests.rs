//! Executable selection regressions: reject self before a Core can spawn it.

use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        // The hardlink scenario must share the executable's filesystem, even
        // when the system temporary directory is a separate Linux tmpfs.
        let current = std::env::current_exe().unwrap();
        let path = current.parent().unwrap().join(format!(
            "squad-core-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn assert_self_refused(result: Result<Core, SquadError>) {
    let error = match result {
        Ok(_) => panic!("self-selection must fail before any Core invocation"),
        Err(error) => error,
    };
    assert_eq!(error.code, "SQUAD_CORE_UNAVAILABLE");
    assert!(error.message.contains("TMT_EXECUTABLE"));
    assert!(error.message.contains("selects Squad itself"));
}

#[test]
fn supplied_self_is_refused_before_spawn() {
    assert_self_refused(Core::discover_with(
        Some(std::env::current_exe().unwrap()),
        None,
    ));
}

#[test]
fn supplied_symlink_and_hardlink_to_self_are_refused_before_spawn() {
    let fixture = Fixture::new();
    let current = std::env::current_exe().unwrap();
    let symbolic = fixture.0.join("symbolic-tmt");
    let hard = fixture.0.join("hard-tmt");
    symlink(&current, &symbolic).unwrap();
    fs::hard_link(&current, &hard).unwrap();
    for selected in [symbolic, hard] {
        assert_self_refused(Core::discover_with(Some(selected), None));
    }
}

#[test]
fn path_self_alias_is_refused_instead_of_trying_another_executable() {
    let fixture = Fixture::new();
    let first = fixture.0.join("first");
    let second = fixture.0.join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    symlink(std::env::current_exe().unwrap(), first.join("tmt")).unwrap();
    symlink("/bin/sh", second.join("tmt")).unwrap();
    assert_self_refused(Core::discover_with(
        None,
        Some(std::env::join_paths([first, second]).unwrap()),
    ));
}

#[test]
fn normal_supplied_and_path_core_still_return_configuration() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("tmt");
    crate::test_support::write_ready_executable(
        &selected,
        "#!/bin/sh\nif [ \"$*\" != 'config show --json' ]; then exit 2; fi\nprintf '{\"config\":\"fixture\",\"lookup\":\"%s\"}' \"${TMT_OPS_CONFIG_LOOKUP-unset}\"\n",
    );
    for (supplied, search) in [
        (Some(selected.clone()), None),
        (None, Some(fixture.0.as_os_str().to_owned())),
    ] {
        let core = Core::discover_with(supplied, search).unwrap();
        assert_eq!(core.executable(), selected);
        assert_eq!(
            core.json(&["config", "show"]).unwrap(),
            json!({"config": "fixture", "lookup": "unset"})
        );
        assert_eq!(
            core.config_show().unwrap(),
            json!({"config": "fixture", "lookup": "1"})
        );
        assert_eq!(
            core.json(&["config", "show"]).unwrap(),
            json!({"config": "fixture", "lookup": "unset"})
        );
    }
}

#[test]
fn local_cancellation_is_distinct_from_an_identical_public_error() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("tmt");
    crate::test_support::write_ready_executable(
        &selected,
        "#!/bin/sh\nprintf '%s' '{\"error\":{\"code\":\"SQUAD_CORE_UNAVAILABLE\",\"message\":\"The board load was superseded.\"}}'\nexit 1\n",
    );
    let core = Core::at(selected);
    let real_error = core.json(&["config", "show"]).unwrap_err();
    assert!(!real_error.was_cancelled());
    let token = runner::Cancellation::default();
    token.cancel();
    let cancelled = core
        .cancellable(token)
        .json(&["config", "show"])
        .unwrap_err();
    assert!(cancelled.was_cancelled());
    assert_eq!(cancelled.to_json(), real_error.to_json());
    assert_eq!(cancelled.human(), real_error.human());
    assert!(!SquadError::new(&cancelled.code, &cancelled.message).was_cancelled());
    assert!(!SquadError::hinted("SQUAD_CORE_UNAVAILABLE", "failed", " ", "retry").was_cancelled());
}

#[test]
fn the_version_is_the_bare_or_prefixed_line_and_anything_else_is_unknown() {
    for (stdout, expected) in [
        ("5.0.0-alpha.378\n", Some("5.0.0-alpha.378")),
        ("tmt 5.0.0-alpha.378\n", Some("5.0.0-alpha.378")),
        ("  5.0.0+build.7  ", Some("5.0.0+build.7")),
        ("", None),
        ("tmt \n", None),
        ("5.0.0\n6.0.0\n", None),
        ("tmt 5.0.0 (dev)\n", None),
        ("error: unknown flag\n", None),
    ] {
        assert_eq!(
            version_of(stdout.as_bytes()),
            expected.map(str::to_owned),
            "{stdout:?}"
        );
    }
    assert_eq!(version_of(&[0xff, b'1']), None, "not text");
    assert_eq!(
        version_of("1".repeat(65).as_bytes()),
        None,
        "an unbounded line"
    );
}

#[test]
fn a_core_reads_the_cli_version_once_from_dash_dash_version_and_a_failure_is_unknown() {
    let dir = Fixture::new();
    let log = dir.0.join("argv");
    let fake = dir.0.join("tmt");
    crate::test_support::write_ready_executable(
        &fake,
        &format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif [ \"$1\" = --version ] && [ -z \"$FAIL\" ]; then echo 'tmt 5.0.0-alpha.9'; else echo nope; exit 2; fi\n",
            log = log.display()
        ),
    );
    let core = Core::at(fake.clone());
    assert_eq!(core.cli_version().as_deref(), Some("5.0.0-alpha.9"));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "--version\n",
        "one plain argv"
    );
    let failing = dir.0.join("tmt-fail");
    crate::test_support::write_ready_executable(&failing, "#!/bin/sh\necho 5.0.0\nexit 3\n");
    assert_eq!(
        Core::at(failing).cli_version(),
        None,
        "a nonzero exit is unknown"
    );
}
