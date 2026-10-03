//! A driver's side: answer one `__tmt-driver <protocol> <op>` invocation.
//! A host driver implements [`Handler`] and calls [`serve`] from `main`; a
//! runtime driver implements [`RuntimeHandler`] and calls [`serve_runtime`].
//! argv, the bounded request, the envelope and the output bound are handled
//! here, so every driver gets them right.

use crate::{MAX_REQUEST_BYTES, Op, PROTOCOL, SUBCOMMAND, wire::*};
use serde::{Serialize, de::DeserializeOwned};
use std::io::{Read, Write};

/// A host driver. Every operation but `capabilities` defaults to
/// `unsupported`; list the ones you implement in `capabilities().ops`, and
/// [`serve`] answers `unsupported` for any other.
#[allow(unused_variables)]
pub trait Handler {
    fn capabilities(&mut self) -> Capabilities;

    fn caller(&mut self, request: Request<CallerRequest>) -> Result<CallerResponse, DriverError> {
        Err(unsupported(Op::Caller))
    }
    fn server(&mut self, request: Request<ServerRequest>) -> Result<ServerResponse, DriverError> {
        Err(unsupported(Op::Server))
    }
    fn resolve_target(
        &mut self,
        request: Request<ResolveTargetRequest>,
    ) -> Result<ResolveTargetResponse, DriverError> {
        Err(unsupported(Op::ResolveTarget))
    }
    fn snapshot(
        &mut self,
        request: Request<SnapshotRequest>,
    ) -> Result<SnapshotResponse, DriverError> {
        Err(unsupported(Op::Snapshot))
    }
    fn probe(&mut self, request: Request<ProbeRequest>) -> Result<ProbeResponse, DriverError> {
        Err(unsupported(Op::Probe))
    }
    fn publish(&mut self, request: Request<PublishRequest>) -> Result<(), DriverError> {
        Err(unsupported(Op::Publish))
    }
    fn clear(&mut self, request: Request<ClearRequest>) -> Result<ClearResponse, DriverError> {
        Err(unsupported(Op::Clear))
    }
    fn capture(
        &mut self,
        request: Request<CaptureRequest>,
    ) -> Result<CaptureResponse, DriverError> {
        Err(unsupported(Op::Capture))
    }
    fn input(&mut self, request: Request<InputRequest>) -> Result<(), DriverError> {
        Err(unsupported(Op::Input))
    }
    fn prompt(&mut self, request: Request<PromptRequest>) -> Result<(), DriverError> {
        Err(unsupported(Op::Prompt))
    }
    fn focus(&mut self, request: Request<FocusRequest>) -> Result<(), DriverError> {
        Err(unsupported(Op::Focus))
    }
}

/// A runtime driver. `locations` is required; `resume` and `usage` default
/// to `unsupported`. List the ones you implement in `capabilities().ops`,
/// and [`serve_runtime`] answers `unsupported` for any other, including
/// every host operation.
#[allow(unused_variables)]
pub trait RuntimeHandler {
    fn capabilities(&mut self) -> RuntimeCapabilities;

    fn locations(
        &mut self,
        request: Request<LocationsRequest>,
    ) -> Result<LocationsResponse, DriverError>;

    fn resume(&mut self, request: Request<ResumeRequest>) -> Result<ResumeResponse, DriverError> {
        Err(unsupported(Op::Resume))
    }
    fn usage(&mut self, request: Request<UsageRequest>) -> Result<UsageResponse, DriverError> {
        Err(unsupported(Op::Usage))
    }
}

/// One kind of driver behind the shared invocation handling.
trait Dispatch {
    /// The `capabilities` answer and the operations it declares.
    fn declare(&mut self) -> (Vec<u8>, Vec<String>);
    /// Answers a declared operation other than `capabilities`.
    fn answer(&mut self, op: Op, request: &[u8]) -> Vec<u8>;
}

struct Host<'h, H>(&'h mut H);

impl<H: Handler> Dispatch for Host<'_, H> {
    fn declare(&mut self) -> (Vec<u8>, Vec<String>) {
        let capabilities = self.0.capabilities();
        let ops = capabilities.ops.clone();
        (bounded(Op::Capabilities, Ok(capabilities)), ops)
    }

    fn answer(&mut self, op: Op, request: &[u8]) -> Vec<u8> {
        let handler = &mut *self.0;
        match op {
            Op::Caller => call(op, request, |r| handler.caller(r)),
            Op::Server => call(op, request, |r| handler.server(r)),
            Op::ResolveTarget => call(op, request, |r| handler.resolve_target(r)),
            Op::Snapshot => call(op, request, |r| handler.snapshot(r)),
            Op::Probe => call(op, request, |r| handler.probe(r)),
            Op::Publish => call(op, request, |r| handler.publish(r).map(|()| Empty {})),
            Op::Clear => call(op, request, |r| handler.clear(r)),
            Op::Capture => call(op, request, |r| handler.capture(r)),
            Op::Input => call(op, request, |r| handler.input(r).map(|()| Empty {})),
            Op::Prompt => call(op, request, |r| handler.prompt(r).map(|()| Empty {})),
            Op::Focus => call(op, request, |r| handler.focus(r).map(|()| Empty {})),
            Op::Capabilities | Op::Locations | Op::Resume | Op::Usage => error(unsupported(op)),
        }
    }
}

struct Runtime<'h, H>(&'h mut H);

impl<H: RuntimeHandler> Dispatch for Runtime<'_, H> {
    fn declare(&mut self) -> (Vec<u8>, Vec<String>) {
        let capabilities = self.0.capabilities();
        let ops = capabilities.ops.clone();
        (bounded(Op::Capabilities, Ok(capabilities)), ops)
    }

    fn answer(&mut self, op: Op, request: &[u8]) -> Vec<u8> {
        let handler = &mut *self.0;
        match op {
            Op::Locations => call(op, request, |r| handler.locations(r)),
            Op::Resume => call(op, request, |r| handler.resume(r)),
            Op::Usage => call(op, request, |r| handler.usage(r)),
            _ => error(unsupported(op)),
        }
    }
}

fn unsupported(op: Op) -> DriverError {
    DriverError::new(
        ErrorCode::Unsupported,
        format!("{} is not supported", op.as_str()),
    )
}

fn bad_request(message: impl Into<String>) -> DriverError {
    DriverError::new(ErrorCode::BadRequest, message)
}

/// Answers one invocation. `args` are the arguments after the program name.
/// Returns the exit status: 0 whenever an answer was printed, 2 when the
/// invocation itself was malformed (an error answer is still printed).
pub fn serve(
    args: &[String],
    input: impl Read,
    output: impl Write,
    handler: &mut impl Handler,
) -> i32 {
    run(args, input, output, &mut Host(handler))
}

/// [`serve`] for a runtime driver.
pub fn serve_runtime(
    args: &[String],
    input: impl Read,
    output: impl Write,
    handler: &mut impl RuntimeHandler,
) -> i32 {
    run(args, input, output, &mut Runtime(handler))
}

fn run(
    args: &[String],
    input: impl Read,
    mut output: impl Write,
    driver: &mut impl Dispatch,
) -> i32 {
    let (status, answer) = match args {
        [subcommand, protocol, op] if subcommand == SUBCOMMAND => {
            if protocol.parse::<u32>().ok() != Some(PROTOCOL) {
                (
                    2,
                    error(bad_request(format!("protocol {protocol} is not spoken"))),
                )
            } else {
                match Op::parse(op) {
                    Some(op) => (0, answer(op, input, driver)),
                    None => (
                        0,
                        error(DriverError::new(
                            ErrorCode::Unsupported,
                            format!("unknown operation {op}"),
                        )),
                    ),
                }
            }
        }
        _ => (
            2,
            error(bad_request(format!("usage: {SUBCOMMAND} <protocol> <op>"))),
        ),
    };
    if output
        .write_all(&answer)
        .and_then(|()| output.flush())
        .is_err()
    {
        return 1;
    }
    status
}

fn error(error: DriverError) -> Vec<u8> {
    serde_json::to_vec(&Response::<Empty>::Error(error)).expect("an error serializes")
}

fn answer(op: Op, input: impl Read, driver: &mut impl Dispatch) -> Vec<u8> {
    let (capabilities, declared) = driver.declare();
    if op == Op::Capabilities {
        return capabilities;
    }
    if !declared.iter().any(|name| name == op.as_str()) {
        return error(unsupported(op));
    }
    let mut request = Vec::new();
    if input
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut request)
        .is_err()
    {
        return error(bad_request("the request could not be read"));
    }
    if request.len() > MAX_REQUEST_BYTES {
        return error(bad_request("the request is over 1 MiB"));
    }
    driver.answer(op, &request)
}

fn call<T: DeserializeOwned, U: Serialize>(
    op: Op,
    request: &[u8],
    handle: impl FnOnce(Request<T>) -> Result<U, DriverError>,
) -> Vec<u8> {
    match serde_json::from_slice(request) {
        Ok(request) => bounded(op, handle(request)),
        Err(reason) => error(bad_request(format!("the request is invalid: {reason}"))),
    }
}

/// An answer over the operation's bound would be refused by core; say so
/// instead of printing it.
fn bounded<U: Serialize>(op: Op, result: Result<U, DriverError>) -> Vec<u8> {
    let response = match result {
        Ok(value) => Response::Ok(value),
        Err(error) => Response::Error(error),
    };
    let bytes = serde_json::to_vec(&response).expect("an answer serializes");
    if bytes.len() > op.bounds().max_output_bytes {
        return error(DriverError::new(
            ErrorCode::Failed,
            format!("the {} answer is over its bound", op.as_str()),
        ));
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::tests::herdr_like;
    use serde_json::{Value, json};

    struct Fake {
        panes: usize,
    }

    impl Handler for Fake {
        fn capabilities(&mut self) -> Capabilities {
            let mut capabilities = herdr_like();
            capabilities.ops = vec!["snapshot".into(), "clear".into()];
            capabilities
        }
        fn snapshot(
            &mut self,
            _: Request<SnapshotRequest>,
        ) -> Result<SnapshotResponse, DriverError> {
            let pane = |n: usize| Pane {
                id: format!("term_{n}"),
                target: None,
                cwd: Some("/".repeat(4000)),
                command: "sh".into(),
                pane_pid: 1,
                suggested_name: None,
                marker: None,
            };
            Ok(SnapshotResponse {
                panes: (0..self.panes).map(pane).collect(),
            })
        }
        fn clear(&mut self, request: Request<ClearRequest>) -> Result<ClearResponse, DriverError> {
            Ok(ClearResponse {
                cleared: request.body.pane_id == "term_1",
            })
        }
    }

    fn run(args: &[&str], request: &[u8], panes: usize) -> (i32, Value) {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let mut output = Vec::new();
        let status = serve(&args, request, &mut output, &mut Fake { panes });
        (status, serde_json::from_slice(&output).unwrap())
    }

    const CLEAR: &[u8] =
        br#"{"deadlineMs": 300, "socket": "/s", "paneId": "term_1", "bindingId": "b"}"#;

    #[test]
    fn a_declared_operation_is_answered() {
        assert_eq!(
            run(&["__tmt-driver", "1", "clear"], CLEAR, 0),
            (0, json!({"ok": {"cleared": true}}))
        );
        let (status, capabilities) = run(&["__tmt-driver", "1", "capabilities"], b"", 0);
        assert_eq!((status, &capabilities["ok"]["name"]), (0, &json!("herdr")));
    }

    #[test]
    fn anything_else_gets_an_error_answer() {
        let code = |args: &[&str], request: &[u8]| {
            let (status, answer) = run(args, request, 0);
            (status, answer["error"]["code"].as_str().unwrap().to_owned())
        };
        assert_eq!(
            code(&["__tmt-driver", "1", "input"], b"{}"),
            (0, "unsupported".into())
        );
        assert_eq!(
            code(&["__tmt-driver", "1", "status"], b"{}"),
            (0, "unsupported".into())
        );
        assert_eq!(
            code(&["__tmt-driver", "2", "clear"], CLEAR),
            (2, "bad_request".into())
        );
        assert_eq!(
            code(&["__tmt-hooks", "1", "clear"], CLEAR),
            (2, "bad_request".into())
        );
        assert_eq!(
            code(&["__tmt-driver", "1", "clear"], b"{"),
            (0, "bad_request".into())
        );
        assert_eq!(
            code(&["__tmt-driver", "1", "clear"], br#"{"deadlineMs": 1}"#),
            (0, "bad_request".into()),
            "a missing member"
        );
        let huge = vec![b' '; MAX_REQUEST_BYTES + 1];
        assert_eq!(
            code(&["__tmt-driver", "1", "clear"], &huge),
            (0, "bad_request".into())
        );
    }

    struct Agent;

    impl RuntimeHandler for Agent {
        fn capabilities(&mut self) -> RuntimeCapabilities {
            let mut capabilities = crate::runtime::tests::claude_like();
            capabilities.ops = vec!["locations".into(), "resume".into()];
            capabilities
        }
        fn locations(
            &mut self,
            request: Request<LocationsRequest>,
        ) -> Result<LocationsResponse, DriverError> {
            Ok(LocationsResponse {
                config_dirs: vec![format!("{}/.kimi", request.body.home)],
                skills: format!("{}/.kimi/skills", request.body.home),
                hook_settings: None,
                transcript_root: None,
            })
        }
        fn resume(
            &mut self,
            request: Request<ResumeRequest>,
        ) -> Result<ResumeResponse, DriverError> {
            Ok(ResumeResponse {
                argv: vec!["kimi".into(), "--resume".into(), request.body.session],
            })
        }
    }

    fn run_runtime(args: &[&str], request: &[u8]) -> (i32, Value) {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let mut output = Vec::new();
        let status = serve_runtime(&args, request, &mut output, &mut Agent);
        (status, serde_json::from_slice(&output).unwrap())
    }

    #[test]
    fn a_runtime_driver_answers_its_declared_ops_and_nothing_else() {
        assert_eq!(
            run_runtime(
                &["__tmt-driver", "1", "resume"],
                br#"{"deadlineMs": 1000, "session": "s-1", "model": null}"#
            ),
            (0, json!({"ok": {"argv": ["kimi", "--resume", "s-1"]}}))
        );
        let (_, capabilities) = run_runtime(&["__tmt-driver", "1", "capabilities"], b"");
        assert_eq!(capabilities["ok"]["kind"], "runtime");
        for op in ["usage", "snapshot", "input"] {
            let (status, answer) = run_runtime(&["__tmt-driver", "1", op], b"{}");
            assert_eq!(
                (status, answer["error"]["code"].as_str()),
                (0, Some("unsupported")),
                "{op}"
            );
        }
        let (status, answer) = run_runtime(&["__tmt-driver", "1", "locations"], b"{");
        assert_eq!(
            (status, answer["error"]["code"].as_str()),
            (0, Some("bad_request"))
        );
        let (status, _) = run_runtime(&["__tmt-driver", "2", "locations"], b"{}");
        assert_eq!(status, 2);
    }

    #[test]
    fn a_host_driver_never_answers_a_runtime_op() {
        let (_, answer) = run(
            &["__tmt-driver", "1", "resume"],
            br#"{"deadlineMs": 1, "session": "s", "model": null}"#,
            0,
        );
        assert_eq!(answer["error"]["code"], "unsupported");
    }

    #[test]
    fn an_answer_over_its_bound_becomes_an_error() {
        let request = br#"{"deadlineMs": 2000, "socket": "/s", "panes": null}"#;
        let (_, fits) = run(&["__tmt-driver", "1", "snapshot"], request, 10);
        assert_eq!(fits["ok"]["panes"].as_array().unwrap().len(), 10);
        let (_, over) = run(&["__tmt-driver", "1", "snapshot"], request, 300);
        assert_eq!(over["error"]["code"], "failed");
    }
}
