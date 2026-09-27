//! Deterministic process-shape fixture, not a provider or a delivery adapter.
//! The Docker image copies this executable as `codex` so real ps observations
//! exercise the shared/independent runtime boundary without credentials or AI.

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().expect("fixture mode");
    assert!(mode == "app-server" || mode == "--no-daemon");
    let command = args.next().expect("selected CLI executable");
    let status = std::process::Command::new(command)
        .args(args)
        .env("CODEX_THREAD_ID", "11111111-1111-4111-8111-111111111111")
        .status()
        .expect("start selected CLI");
    std::process::exit(status.code().unwrap_or(1));
}
