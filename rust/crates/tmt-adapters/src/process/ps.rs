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
        match runner.execute(CommandRequest {
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
