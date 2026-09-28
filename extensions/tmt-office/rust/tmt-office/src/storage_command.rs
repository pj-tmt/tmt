//! Hidden development and diagnostics entry for Office storage migration.
//!
//! It requires an explicit absolute global directory and never uses ambient
//! configuration discovery, so development runs cannot reach a real
//! installation by accident. User-facing migration has its own consented owner.

use std::{
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};
use tmt_adapters::config::ConfigPaths;
use tmt_office_storage::{StorageLayout, migration};

pub const PREFIX: &str = "__tmt-office-storage";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Status,
    Prepare,
    Copy,
    Verify,
}

#[derive(Debug, PartialEq, Eq)]
struct StorageRequest {
    step: Step,
    global_dir: PathBuf,
}

/// Accepts exactly `__tmt-office-storage 1 <step> --global-dir <absolute>`.
fn parse(arguments: &[OsString]) -> Option<StorageRequest> {
    let [prefix, version, step, option, directory] = arguments else {
        return None;
    };
    if prefix != PREFIX || version != "1" || option != "--global-dir" {
        return None;
    }
    let step = match step.to_str()? {
        "status" => Step::Status,
        "prepare" => Step::Prepare,
        "copy" => Step::Copy,
        "verify" => Step::Verify,
        _ => return None,
    };
    let global_dir = PathBuf::from(directory);
    global_dir
        .is_absolute()
        .then_some(StorageRequest { step, global_dir })
}

pub fn run(arguments: &[OsString]) -> ExitCode {
    let Some(invocation) = parse(arguments) else {
        let _ = writeln!(
            io::stderr().lock(),
            "Usage: tmt-office {PREFIX} 1 <status|prepare|copy|verify> --global-dir <absolute directory>"
        );
        return ExitCode::from(2);
    };
    // Only the global directory matters here; the root stands in for the
    // working and home directories that select unrelated local settings.
    let root = Path::new("/");
    let paths = ConfigPaths::resolve(root, root, Some(&invocation.global_dir), None);
    let layout = StorageLayout::new(&paths);
    let result = match invocation.step {
        Step::Status => migration::status(&layout),
        Step::Prepare => migration::prepare(&layout),
        Step::Copy => migration::copy(&layout),
        Step::Verify => migration::verify(&layout),
    };
    let (document, code) = match result {
        Ok(status) => (status.json(), ExitCode::SUCCESS),
        Err(error) => (error.json(), ExitCode::FAILURE),
    };
    match writeln!(io::stdout().lock(), "{document}") {
        Ok(()) => code,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn requires_the_exact_versioned_grammar_and_an_absolute_root() {
        assert_eq!(
            parse(&arguments(&[
                PREFIX,
                "1",
                "verify",
                "--global-dir",
                "/tmp/root"
            ])),
            Some(StorageRequest {
                step: Step::Verify,
                global_dir: PathBuf::from("/tmp/root")
            })
        );
        for rejected in [
            &[PREFIX, "1", "verify", "--global-dir", "relative"][..],
            &[PREFIX, "2", "verify", "--global-dir", "/tmp/root"],
            &[PREFIX, "1", "switch", "--global-dir", "/tmp/root"],
            &[PREFIX, "1", "verify"],
            &[PREFIX, "1", "verify", "--global-dir", "/tmp/root", "extra"],
        ] {
            assert_eq!(parse(&arguments(rejected)), None, "{rejected:?}");
        }
    }
}
