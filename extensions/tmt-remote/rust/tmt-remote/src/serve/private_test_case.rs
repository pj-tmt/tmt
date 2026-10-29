use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

/// A one-case exec with disposable HOME/TMT_HOME and owned process cleanup.
/// This adds state isolation to isolated_case's exec-only credential-lock boundary.
pub(crate) fn private_case(case: &str, check: impl FnOnce(std::path::PathBuf)) {
    use std::{ffi::OsString, path::PathBuf};
    const MARKER: &str = "TMT_REMOTE_PRIVATE_CASE";
    const ROOT: &str = "TMT_REMOTE_PRIVATE_HOME";
    if std::env::var(MARKER).as_deref() == Ok(case) {
        let root = PathBuf::from(std::env::var_os(ROOT).expect("private root"))
            .canonicalize()
            .unwrap();
        assert_eq!(
            PathBuf::from(std::env::var_os("TMT_HOME").expect("private TMT_HOME"))
                .canonicalize()
                .unwrap(),
            root
        );
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("tmt-remote-private-")
        );
        check(root.clone());
        std::fs::write(root.join("proof-complete"), case).unwrap();
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "tmt-remote-private-{}",
        crate::store::uuid_v4().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let program = std::env::current_exe().unwrap();
    let args = ["--exact", case, "--test-threads=1", "--nocapture"].map(OsString::from);
    let mut environment = vec![
        (OsString::from(MARKER), OsString::from(case)),
        (OsString::from(ROOT), root.as_os_str().to_owned()),
        (OsString::from("TMT_AGENT"), OsString::from("1")),
        (OsString::from("BROWSER"), OsString::from("true")),
    ];
    for name in ["HOME", "TMT_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        environment.push((name.into(), root.as_os_str().to_owned()));
    }
    let stop = AtomicBool::new(false);
    let result = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &program,
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(120),
            max_stream_bytes: 65_536,
            launch: tmt_invoke::LaunchOptions {
                environment: tmt_invoke::EnvironmentPolicy::InheritWith(&environment),
                ..Default::default()
            },
        },
        Some(&stop),
    );
    let completed = std::fs::read_to_string(root.join("proof-complete")).unwrap_or_default();
    std::fs::remove_dir_all(&root).unwrap();
    let output = result.expect("private case must complete with owned cleanup");
    assert!(
        output.status.success(),
        "{case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        completed, case,
        "the exact selector must execute its assertions, not zero tests"
    );
}
