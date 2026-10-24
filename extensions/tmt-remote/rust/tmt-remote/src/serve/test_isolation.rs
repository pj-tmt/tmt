use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

/// Open project locks only after exec in a one-case process: parallel sibling
/// invocation forks must not inherit this fixture's credential, budget or serving locks.
/// The existing invocation owner bounds, terminates and reaps a failed child.
pub(crate) fn isolated_case(case: &str) -> bool {
    const MARKER: &str = "TMT_REMOTE_ISOLATED_TEST_CASE";
    if std::env::var(MARKER).as_deref() == Ok(case) {
        return false;
    }
    let program = std::env::current_exe().unwrap();
    let args = ["--exact", case, "--test-threads=1", "--nocapture"].map(std::ffi::OsString::from);
    let environment = [(
        std::ffi::OsString::from(MARKER),
        std::ffi::OsString::from(case),
    )];
    let stop = AtomicBool::new(false);
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &program,
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(45),
            max_stream_bytes: 65_536,
            launch: tmt_invoke::LaunchOptions {
                environment: tmt_invoke::EnvironmentPolicy::InheritWith(&environment),
                ..Default::default()
            },
        },
        Some(&stop),
    )
    .expect("isolated Remote test case must complete with owned cleanup");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{case} failed:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("running 1 test") && stdout.contains("1 passed; 0 failed"),
        "must execute the selected case, not an empty selection: {stdout}"
    );
    true
}
