use crate::process::{CommandError, CommandFailure, CommandOutput, CommandRequest, CommandRunner};
use std::{cell::RefCell, collections::VecDeque, io};

#[derive(Debug)]
pub(crate) struct Invocation {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) input: Vec<u8>,
    pub(crate) deadline: std::time::Instant,
    pub(crate) max_output_bytes: usize,
}

#[derive(Default)]
pub(crate) struct ScriptedRunner {
    pub(crate) results: RefCell<VecDeque<Result<CommandOutput, CommandError>>>,
    pub(crate) calls: RefCell<Vec<Invocation>>,
}

impl ScriptedRunner {
    pub(crate) fn new(
        results: impl IntoIterator<Item = Result<&'static str, CommandError>>,
    ) -> Self {
        Self {
            results: RefCell::new(
                results
                    .into_iter()
                    .map(|result| {
                        result.map(|text| CommandOutput {
                            stdout: text.as_bytes().to_vec(),
                            stderr: Vec::new(),
                        })
                    })
                    .collect(),
            ),
            ..Self::default()
        }
    }

    pub(crate) fn push_output(&self, stdout: Vec<u8>, stderr: Vec<u8>) {
        self.results
            .borrow_mut()
            .push_back(Ok(CommandOutput { stdout, stderr }));
    }
}

impl CommandRunner for ScriptedRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert!(request.input.is_empty());
        self.calls.borrow_mut().push(Invocation {
            program: request.program.to_str().unwrap().into(),
            args: request
                .args
                .iter()
                .map(|value| value.to_str().unwrap().into())
                .collect(),
            input: request.input.to_vec(),
            deadline: request.deadline,
            max_output_bytes: request.max_output_bytes,
        });
        self.results
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra subprocess")
    }
}

pub(crate) fn failure(cleanup_failed: bool) -> CommandError {
    failure_with_kind(CommandFailure::Timeout, cleanup_failed)
}

pub(crate) fn failure_with_kind(kind: CommandFailure, cleanup_failed: bool) -> CommandError {
    let mut error = CommandError::new(kind);
    if cleanup_failed {
        error.cleanup_error = Some(io::Error::from_raw_os_error(
            nix::errno::Errno::EPERM as i32,
        ));
    }
    error
}
