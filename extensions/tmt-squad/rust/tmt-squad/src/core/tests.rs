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
        "#!/bin/sh\nif [ \"$*\" != 'config show --json' ]; then exit 2; fi\nprintf '%s' '{\"config\":\"fixture\"}'\n",
    );
    for (supplied, search) in [
        (Some(selected.clone()), None),
        (None, Some(fixture.0.as_os_str().to_owned())),
    ] {
        let core = Core::discover_with(supplied, search).unwrap();
        assert_eq!(core.executable(), selected);
        assert_eq!(
            core.json(&["config", "show"]).unwrap(),
            json!({"config": "fixture"})
        );
    }
}
