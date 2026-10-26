//! Fixed-path, fixed-locale ps execution shared by runtime observations.

use super::{CommandError, CommandFailure, CommandOutput, CommandRequest, CommandRunner};
use std::{
    ffi::{OsStr, OsString},
    time::Instant,
};

pub fn query_ps<R: CommandRunner>(
    runner: &R,
    arguments: &[OsString],
    deadline: Instant,
    max_output_bytes: usize,
) -> Result<CommandOutput, CommandError> {
    let mut last = None;
    for program in ["/bin/ps", "/usr/bin/ps"] {
        let mut args: Vec<OsString> = ["LC_ALL=C", "TZ=UTC", program]
            .into_iter()
            .map(Into::into)
            .collect();
        args.extend_from_slice(arguments);
        // `env` execs `ps`, and `ps` never forks.
        match runner.execute_leaf(CommandRequest {
            program: OsStr::new("/usr/bin/env"),
            args: &args,
            input: &[],
            deadline,
            max_output_bytes,
        }) {
            Err(error)
                if !error.cleanup_failed()
                    && error.kind
                        == (CommandFailure::Exit {
                            code: Some(127),
                            signal: None,
                        }) =>
            {
                last = Some(error)
            }
            result => return result,
        }
    }
    Err(last.expect("both fixed candidates were missing"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct LeafOnly;

    impl CommandRunner for LeafOnly {
        fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            panic!("ps must start through the leaf path (#3834)")
        }

        fn execute_leaf(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.args[2], "/bin/ps");
            Ok(CommandOutput {
                stdout: b"1\n".to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn ps_starts_as_a_leaf_probe_in_the_callers_group() {
        let output = query_ps(
            &LeafOnly,
            &["-o".into(), "pid=".into()],
            Instant::now() + Duration::from_secs(1),
            16,
        )
        .unwrap();
        assert_eq!(output.stdout, b"1\n");
    }
}
