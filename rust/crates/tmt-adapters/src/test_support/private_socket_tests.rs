use super::*;
use std::os::unix::fs::symlink;

fn host(uid: u32, tmux_tmpdir: Option<&Path>, tmux: Option<&str>) -> HostTmux {
    HostTmux {
        uid,
        tmux_tmpdir: tmux_tmpdir.map(Path::to_path_buf),
        tmux: tmux.map(str::to_owned),
    }
}

fn refusal(socket: &Path, root: &Path, host: &HostTmux) -> String {
    private_socket_refusal(socket, root, host).expect("the socket is refused")
}

#[test]
fn a_socket_inside_the_test_root_is_private() {
    let directory = TestDirectory::new();
    let socket = directory.path.join("probe.sock");
    assert_eq!(
        private_socket_refusal(&socket, &directory.path, &host(501, None, None)),
        None
    );
    // The host's own socket directories never hold a test root's sockets.
    let elsewhere = host(501, None, Some("/private/tmp/tmux-501/default,1,0"));
    assert_eq!(
        private_socket_refusal(&socket, &directory.path, &elsewhere),
        None
    );
}

#[test]
fn a_socket_outside_the_root_or_climbing_out_of_it_is_refused() {
    let directory = TestDirectory::new();
    let none = host(501, None, None);
    let outside = directory.path.parent().unwrap().join("outside.sock");
    assert!(refusal(&outside, &directory.path, &none).contains("outside the test root"));
    assert!(refusal(&directory.path, &directory.path, &none).contains("outside the test root"));
    let climbing = directory.path.join("gone/../escape.sock");
    assert!(refusal(&climbing, &directory.path, &none).contains(".. component"));
    assert!(!directory.path.join("gone").exists());
    assert!(refusal(Path::new("relative.sock"), &directory.path, &none).contains("not absolute"));
    let missing = directory.path.join("missing");
    assert!(
        refusal(&missing.join("x.sock"), &missing, &none).contains("does not exist"),
        "a root that does not exist"
    );
}

#[test]
fn a_root_inside_a_host_socket_directory_is_refused() {
    let directory = TestDirectory::new();
    let tmpdir = directory.path.join("host");
    let default_dir = tmpdir.join("tmux-501");
    fs::create_dir_all(&default_dir).unwrap();
    // $TMUX_TMPDIR/tmux-<uid> is where the host's default server lives.
    let by_tmpdir = host(501, Some(&tmpdir), None);
    let socket = default_dir.join("probe.sock");
    assert!(refusal(&socket, &default_dir, &by_tmpdir).contains("host tmux socket directory"));
    // The directory of the socket $TMUX names, and that socket itself.
    let named = format!("{}/default,5,1", tmpdir.display());
    let by_tmux = host(501, None, Some(&named));
    let inside = tmpdir.join("probe.sock");
    assert!(refusal(&inside, &tmpdir, &by_tmux).contains("host tmux socket directory"));
    // The socket $TMUX names lies in the directory just refused, so it is refused with it.
    let owned = directory.path.join("probe.sock");
    let same = host(501, None, Some(&format!("{},5,1", owned.display())));
    assert!(refusal(&owned, &directory.path, &same).contains("host tmux socket directory"));
}

#[test]
fn the_default_socket_directories_are_always_host_directories() {
    let directories = host(501, None, None).socket_directories();
    for expected in ["/tmp/tmux-501", "/private/tmp/tmux-501"] {
        let resolved = resolve_loosely(Path::new(expected));
        assert!(
            directories.contains(&resolved),
            "{expected}: {directories:?}"
        );
    }
}

#[test]
fn a_symlinked_socket_or_a_path_too_long_to_bind_is_refused() {
    let directory = TestDirectory::new();
    let none = host(501, None, None);
    let linked = directory.path.join("probe.sock");
    symlink("/private/tmp/tmux-501/default", &linked).unwrap();
    assert!(refusal(&linked, &directory.path, &none).contains("is a symlink"));
    let long = directory.path.join("x".repeat(120));
    assert!(refusal(&long, &directory.path, &none).contains("too long"));
}

#[test]
#[should_panic(expected = "private tmux refused")]
fn the_asserting_form_panics_before_any_tmux_command() {
    let directory = TestDirectory::new();
    assert_private_socket(Path::new("relative.sock"), &directory.path);
}
