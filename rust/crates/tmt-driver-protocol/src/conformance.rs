//! Checks a driver against the contract, through whatever runs it: a
//! spawned executable, or [`serve`](crate::serve) or
//! [`serve_runtime`](crate::serve_runtime) in process.
//!
//! [`check`] is for a host driver. Every check uses panes and targets that
//! don't exist, so running it against a live server changes nothing there.
//! Checks that need a bound pane (publishing a marker, reading it back)
//! belong to the host's own fixture, which controls a pane; this harness
//! proves the protocol.
//!
//! [`check_runtime`] is for a runtime driver. It asks about a fixture home,
//! a fixture session and a transcript that doesn't exist, so it changes none
//! of the agent's files.

use crate::{
    Op, SUBCOMMAND,
    decode::{
        Answer, DecodeError, decode, decode_capabilities, decode_done, decode_runtime_capabilities,
    },
    grammar::Grammar,
    wire::*,
};
use serde::Serialize;
use std::{collections::BTreeMap, time::Duration};

/// What one invocation produced.
#[derive(Debug, Clone)]
pub struct DriverOutput {
    pub stdout: Vec<u8>,
    pub elapsed: Duration,
}

/// A live server the driver can reach, and nothing more is assumed of it.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub socket: String,
}

/// What a runtime driver is asked about. Nothing here needs to exist.
#[derive(Debug, Clone)]
pub struct RuntimeFixture {
    /// The home directory `locations` is asked about.
    pub home: String,
    /// A provider session ID `resume` is asked about.
    pub session: String,
}

/// One way the driver broke the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: String,
    pub detail: String,
}

/// Runs every check; no findings means the driver conforms. `invoke` gets
/// the arguments after the program name and the request bytes.
pub fn check(
    invoke: &mut dyn FnMut(&[&str], &[u8]) -> DriverOutput,
    fixture: &Fixture,
) -> Vec<Finding> {
    let mut run = Run {
        invoke,
        findings: Vec::new(),
    };
    let capabilities = run.call(Op::Capabilities, b"");
    let (capabilities, grammar) = match decode_capabilities(&capabilities.stdout) {
        Ok(decoded) => decoded,
        Err(error) => {
            run.found("capabilities", error.to_string());
            return run.findings;
        }
    };
    run.envelope_errors();
    // `capabilities` is always answered, listed or not.
    let (declared, undeclared): (Vec<Op>, Vec<Op>) = Op::ALL
        .into_iter()
        .filter(|op| *op != Op::Capabilities)
        .partition(|op| capabilities.ops.iter().any(|name| name == op.as_str()));
    for op in undeclared {
        run.expect_code(
            op,
            &request(Empty {}),
            ErrorCode::Unsupported,
            "an undeclared op",
        );
    }
    for &op in &declared {
        run.expect_code(op, b"{", ErrorCode::BadRequest, "a request that isn't JSON");
        Missing {
            run: &mut run,
            grammar: &grammar,
            fixture,
        }
        .check(op);
    }
    run.findings
}

/// Runs every runtime check; no findings means the driver conforms. `invoke`
/// is as for [`check`].
pub fn check_runtime(
    invoke: &mut dyn FnMut(&[&str], &[u8]) -> DriverOutput,
    fixture: &RuntimeFixture,
) -> Vec<Finding> {
    let mut run = Run {
        invoke,
        findings: Vec::new(),
    };
    let capabilities = run.call(Op::Capabilities, b"");
    let declaration = match decode_runtime_capabilities(&capabilities.stdout) {
        Ok((_, declaration)) => declaration,
        Err(error) => {
            run.found("capabilities", error.to_string());
            return run.findings;
        }
    };
    run.envelope_errors();
    for op in Op::ALL {
        if op == Op::Capabilities {
            continue;
        }
        if declaration.supports(op) {
            run.expect_code(op, b"{", ErrorCode::BadRequest, "a request that isn't JSON");
        } else {
            run.expect_code(
                op,
                &request(Empty {}),
                ErrorCode::Unsupported,
                "an undeclared op",
            );
        }
    }
    let locations = run.answer::<LocationsResponse>(
        &declaration,
        LocationsRequest {
            home: fixture.home.clone(),
            env: BTreeMap::new(),
        },
    );
    let transcript_root = match locations {
        Some(Ok(locations)) => {
            if !locations.within(&fixture.home) {
                run.found(
                    Op::Locations.as_str(),
                    "skills must lie inside configDirs or be the shared skills root",
                );
            }
            locations.transcript_root
        }
        Some(Err(error)) => {
            run.found(
                Op::Locations.as_str(),
                format!("a fixture home must have locations: {}", error.message),
            );
            None
        }
        None => None,
    };
    if declaration.supports(Op::Resume) {
        // Either an argv, which decoding checked, or an error answer.
        run.answer::<ResumeResponse>(
            &declaration,
            ResumeRequest {
                session: fixture.session.clone(),
                model: None,
            },
        );
    }
    if let (true, Some(root)) = (declaration.supports(Op::Usage), transcript_root) {
        let answer = run.answer::<UsageResponse>(
            &declaration,
            UsageRequest {
                session: fixture.session.clone(),
                transcript: format!(
                    "{}/tmt-conformance-missing.jsonl",
                    root.trim_end_matches('/')
                ),
            },
        );
        if let Some(Ok(UsageResponse {
            context_tokens: Some(tokens),
        })) = answer
        {
            run.found(
                Op::Usage.as_str(),
                format!("a missing transcript shows no usage, got {tokens}"),
            );
        }
    }
    run.findings
}

struct Run<'a> {
    invoke: &'a mut dyn FnMut(&[&str], &[u8]) -> DriverOutput,
    findings: Vec<Finding>,
}

fn request<T: Serialize>(body: T) -> Vec<u8> {
    serde_json::to_vec(&Request {
        deadline_ms: 250,
        body,
    })
    .expect("a request serializes")
}

impl Run<'_> {
    fn found(&mut self, check: impl Into<String>, detail: impl Into<String>) {
        self.findings.push(Finding {
            check: check.into(),
            detail: detail.into(),
        });
    }

    /// Runs one operation and checks it kept to its deadline.
    fn call(&mut self, op: Op, request: &[u8]) -> DriverOutput {
        let invocation = (self.invoke)(&[SUBCOMMAND, "1", op.as_str()], request);
        let deadline = op.bounds().deadline;
        if invocation.elapsed > deadline {
            self.found(
                op.as_str(),
                format!(
                    "took {:?}, over its {deadline:?} deadline",
                    invocation.elapsed
                ),
            );
        }
        invocation
    }

    /// Runs one operation and decodes its answer against the declaration; a
    /// decode failure is a finding.
    fn answer<T: Answer>(
        &mut self,
        declaration: &T::Declaration,
        body: impl Serialize,
    ) -> Option<Result<T, DriverError>> {
        let output = self.call(T::OP, &request(body)).stdout;
        match decode::<T>(declaration, &output) {
            Ok(answer) => Some(answer),
            Err(error) => {
                self.found(T::OP.as_str(), error.to_string());
                None
            }
        }
    }

    fn error_code(&self, op: Op, output: &[u8]) -> Result<Option<ErrorCode>, DecodeError> {
        decode_done(op, output).map(|answer| answer.err().map(|error| error.code))
    }

    fn expect_code(&mut self, op: Op, request: &[u8], code: ErrorCode, case: &str) {
        let output = self.call(op, request).stdout;
        match self.error_code(op, &output) {
            Ok(Some(got)) if got == code => {}
            other => self.found(
                op.as_str(),
                format!("{case} must answer {code:?}, got {other:?}"),
            ),
        }
    }

    /// Malformed invocations still get one error answer.
    fn envelope_errors(&mut self) {
        for (args, case) in [
            (&[SUBCOMMAND, "0", "snapshot"][..], "an unspoken protocol"),
            (&[SUBCOMMAND, "1", "status"][..], "an unknown op"),
            (&[SUBCOMMAND][..], "a missing op"),
        ] {
            let output = (self.invoke)(args, b"{}").stdout;
            if !matches!(self.error_code(Op::Snapshot, &output), Ok(Some(_))) {
                self.found("envelope", format!("{case} must print one error answer"));
            }
        }
    }
}

/// Each declared operation, asked about a pane or target that doesn't exist.
struct Missing<'r, 'a> {
    run: &'r mut Run<'a>,
    grammar: &'r Grammar,
    fixture: &'r Fixture,
}

impl Missing<'_, '_> {
    fn pane(&self) -> String {
        format!("{}zzzzzzzzzzzz", self.grammar.pane_id_prefix())
    }

    fn socket(&self) -> String {
        self.fixture.socket.clone()
    }

    fn answer<T: Answer<Declaration = Grammar>>(
        &mut self,
        body: impl Serialize,
    ) -> Option<Result<T, DriverError>> {
        self.run.answer(self.grammar, body)
    }

    fn not_found(&mut self, op: Op, body: impl Serialize) {
        self.run
            .expect_code(op, &request(body), ErrorCode::NotFound, "a missing pane");
    }

    fn wrong(&mut self, op: Op, detail: impl Into<String>) {
        self.run.found(op.as_str(), detail);
    }

    fn check(mut self, op: Op) {
        let (socket, pane) = (self.socket(), self.pane());
        match op {
            // Runtime operations: a host that lists one answers `unsupported`,
            // which the `bad_request` check above already reports.
            Op::Capabilities | Op::Locations | Op::Resume | Op::Usage => {}
            Op::Caller => {
                if let Some(Ok(CallerResponse { pane: Some(_) })) =
                    self.answer::<CallerResponse>(CallerRequest {
                        env: BTreeMap::new(),
                    })
                {
                    self.wrong(op, "an empty environment names no pane");
                }
            }
            Op::Server => {
                if let Some(Err(error)) = self.answer::<ServerResponse>(ServerRequest {
                    socket: Some(socket),
                }) {
                    self.wrong(op, format!("the fixture's server: {}", error.message));
                }
            }
            Op::ResolveTarget => {
                let Some(target) = self.grammar.target_numbered("999999999") else {
                    return;
                };
                match self.answer::<ResolveTargetResponse>(ResolveTargetRequest { socket, target })
                {
                    Some(Ok(ResolveTargetResponse { pane_id: None })) | None => {}
                    other => self.wrong(
                        op,
                        format!("an unused target resolves to none, got {other:?}"),
                    ),
                }
            }
            Op::Snapshot => {
                let all = self.answer::<SnapshotResponse>(SnapshotRequest {
                    socket: socket.clone(),
                    panes: None,
                });
                if let Some(Err(error)) = all {
                    self.wrong(op, format!("every pane: {}", error.message));
                }
                match self.answer::<SnapshotResponse>(SnapshotRequest {
                    socket,
                    panes: Some(vec![pane]),
                }) {
                    Some(Ok(SnapshotResponse { panes })) if panes.is_empty() => {}
                    None => {}
                    other => self.wrong(op, format!("a missing pane lists nothing, got {other:?}")),
                }
            }
            Op::Probe => {
                // A server that never ran: its pid is the largest allowed.
                let server = ServerIncarnation {
                    socket,
                    pid: (1 << 53) - 1,
                    start_time: "never".into(),
                };
                match self.answer::<ProbeResponse>(ProbeRequest {
                    server,
                    panes: vec![pane],
                }) {
                    Some(Ok(ProbeResponse::Live { .. })) => {
                        self.wrong(op, "a server that never ran is not live")
                    }
                    Some(Err(error)) => {
                        self.wrong(op, format!("answer dead or unknown: {}", error.message))
                    }
                    _ => {}
                }
            }
            Op::Clear => match self.answer::<ClearResponse>(ClearRequest {
                socket,
                pane_id: pane,
                binding_id: "00000000-0000-4000-8000-000000000000".into(),
            }) {
                Some(Ok(ClearResponse { cleared: false })) | None => {}
                other => self.wrong(
                    op,
                    format!("nothing to clear on a missing pane, got {other:?}"),
                ),
            },
            Op::Publish => self.not_found(
                op,
                PublishRequest {
                    socket,
                    pane_id: pane,
                    pane_pid: 1,
                    marker: Marker {
                        name: "Conformance".into(),
                        canonical_name: "conformance".into(),
                        identity_id: "00000000-0000-4000-8000-000000000001".into(),
                        binding_id: "00000000-0000-4000-8000-000000000002".into(),
                        server_id: "00000000-0000-4000-8000-000000000003".into(),
                        pane_pid: 1,
                    },
                },
            ),
            Op::Capture => self.not_found(
                op,
                CaptureRequest {
                    socket,
                    pane_id: pane,
                    lines: 10,
                },
            ),
            Op::Input => self.not_found(
                op,
                InputRequest {
                    socket,
                    pane_id: pane,
                    text: "conformance".into(),
                    enter: false,
                },
            ),
            Op::Prompt => self.not_found(
                op,
                PromptRequest {
                    socket,
                    pane_id: pane,
                    text: "conformance".into(),
                },
            ),
            Op::Focus => self.not_found(
                op,
                FocusRequest {
                    socket,
                    pane_id: pane,
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Handler, grammar::tests::herdr_like, serve};
    use std::time::Instant;

    /// A driver for a host with no panes, right or wrong in chosen ways.
    #[derive(Default)]
    struct NoPanes {
        clears_anything: bool,
        publishes_anywhere: bool,
    }

    fn gone() -> DriverError {
        DriverError::new(ErrorCode::NotFound, "no such pane")
    }

    impl Handler for NoPanes {
        fn capabilities(&mut self) -> Capabilities {
            let mut capabilities = herdr_like();
            capabilities.ops = [
                // Listing `capabilities` is allowed and changes nothing.
                "capabilities",
                "caller",
                "server",
                "resolve-target",
                "snapshot",
                "probe",
                "publish",
                "clear",
                "capture",
                "focus",
            ]
            .map(String::from)
            .to_vec();
            capabilities
        }
        fn caller(&mut self, _: Request<CallerRequest>) -> Result<CallerResponse, DriverError> {
            Ok(CallerResponse { pane: None })
        }
        fn server(
            &mut self,
            request: Request<ServerRequest>,
        ) -> Result<ServerResponse, DriverError> {
            Ok(ServerResponse {
                server: request.body.socket.map(|socket| ServerIncarnation {
                    socket,
                    pid: 10,
                    start_time: "t".into(),
                }),
            })
        }
        fn resolve_target(
            &mut self,
            _: Request<ResolveTargetRequest>,
        ) -> Result<ResolveTargetResponse, DriverError> {
            Ok(ResolveTargetResponse { pane_id: None })
        }
        fn snapshot(
            &mut self,
            _: Request<SnapshotRequest>,
        ) -> Result<SnapshotResponse, DriverError> {
            Ok(SnapshotResponse { panes: Vec::new() })
        }
        fn probe(&mut self, _: Request<ProbeRequest>) -> Result<ProbeResponse, DriverError> {
            Ok(ProbeResponse::Dead)
        }
        fn publish(&mut self, _: Request<PublishRequest>) -> Result<(), DriverError> {
            if self.publishes_anywhere {
                Ok(())
            } else {
                Err(gone())
            }
        }
        fn clear(&mut self, _: Request<ClearRequest>) -> Result<ClearResponse, DriverError> {
            Ok(ClearResponse {
                cleared: self.clears_anything,
            })
        }
        fn capture(&mut self, _: Request<CaptureRequest>) -> Result<CaptureResponse, DriverError> {
            Err(gone())
        }
        fn focus(&mut self, _: Request<FocusRequest>) -> Result<(), DriverError> {
            Err(gone())
        }
    }

    /// Runs the check in process; `slow` adds time to one op's invocations,
    /// as a slow host would.
    fn findings_with(mut driver: NoPanes, slow: Option<(Op, Duration)>) -> Vec<Finding> {
        let mut invoke = |args: &[&str], request: &[u8]| {
            let owned: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            let started = Instant::now();
            let mut stdout = Vec::new();
            serve(&owned, request, &mut stdout, &mut driver);
            let extra = slow
                .filter(|(op, _)| args.get(2) == Some(&op.as_str()))
                .map_or(Duration::ZERO, |(_, extra)| extra);
            DriverOutput {
                stdout,
                elapsed: started.elapsed() + extra,
            }
        };
        check(
            &mut invoke,
            &Fixture {
                socket: "/tmp/conformance.sock".into(),
            },
        )
    }

    fn findings(driver: NoPanes) -> Vec<Finding> {
        findings_with(driver, None)
    }

    #[test]
    fn a_conforming_driver_has_no_findings() {
        assert_eq!(findings(NoPanes::default()), []);
    }

    #[test]
    fn each_broken_rule_is_found() {
        let checks = |driver| -> Vec<String> {
            findings(driver)
                .into_iter()
                .map(|finding| finding.check)
                .collect()
        };
        assert_eq!(
            checks(NoPanes {
                clears_anything: true,
                ..NoPanes::default()
            }),
            ["clear"]
        );
        assert_eq!(
            checks(NoPanes {
                publishes_anywhere: true,
                ..NoPanes::default()
            }),
            ["publish"]
        );
        let slow: Vec<String> = findings_with(
            NoPanes::default(),
            Some((Op::Snapshot, Duration::from_millis(2001))),
        )
        .into_iter()
        .map(|finding| finding.check)
        .collect();
        assert!(
            !slow.is_empty() && slow.iter().all(|check| check == "snapshot"),
            "{slow:?}"
        );
    }

    /// A runtime driver for an agent with no transcripts, right or wrong in
    /// chosen ways.
    #[derive(Default)]
    struct Agent {
        resumes_through_a_shell: bool,
        reads_a_missing_transcript: bool,
        forgets_the_transcript_root: bool,
        installs_skills_anywhere: bool,
    }

    impl crate::RuntimeHandler for Agent {
        fn capabilities(&mut self) -> RuntimeCapabilities {
            crate::runtime::tests::claude_like()
        }
        fn locations(
            &mut self,
            request: Request<LocationsRequest>,
        ) -> Result<LocationsResponse, DriverError> {
            let home = request.body.home;
            Ok(LocationsResponse {
                config_dirs: vec![format!("{home}/.kimi")],
                skills: if self.installs_skills_anywhere {
                    format!("{home}/.ssh")
                } else {
                    format!("{home}/.kimi/skills")
                },
                hook_settings: Some(format!("{home}/.kimi/settings.json")),
                transcript_root: (!self.forgets_the_transcript_root)
                    .then(|| format!("{home}/.kimi/sessions")),
            })
        }
        fn resume(
            &mut self,
            request: Request<ResumeRequest>,
        ) -> Result<ResumeResponse, DriverError> {
            let argv = if self.resumes_through_a_shell {
                vec![
                    "sh".into(),
                    "-c".into(),
                    format!("kimi -r {}", request.body.session),
                ]
            } else {
                vec!["kimi".into(), "-r".into(), request.body.session]
            };
            Ok(ResumeResponse { argv })
        }
        fn usage(&mut self, _: Request<UsageRequest>) -> Result<UsageResponse, DriverError> {
            Ok(UsageResponse {
                context_tokens: self.reads_a_missing_transcript.then_some(9),
            })
        }
    }

    fn runtime_findings_with(mut driver: Agent, slow: Option<(Op, Duration)>) -> Vec<Finding> {
        let mut invoke = |args: &[&str], request: &[u8]| {
            let owned: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            let mut stdout = Vec::new();
            crate::serve_runtime(&owned, request, &mut stdout, &mut driver);
            let elapsed = slow
                .filter(|(op, _)| args.get(2) == Some(&op.as_str()))
                .map_or(Duration::ZERO, |(_, extra)| extra);
            DriverOutput { stdout, elapsed }
        };
        check_runtime(
            &mut invoke,
            &RuntimeFixture {
                home: "/nonexistent/home".into(),
                session: "conformance-session".into(),
            },
        )
    }

    fn runtime_checks(driver: Agent) -> Vec<String> {
        runtime_findings_with(driver, None)
            .into_iter()
            .map(|finding| finding.check)
            .collect()
    }

    #[test]
    fn a_conforming_runtime_driver_has_no_findings() {
        assert_eq!(runtime_findings_with(Agent::default(), None), []);
    }

    #[test]
    fn each_broken_runtime_rule_is_found() {
        assert_eq!(
            runtime_checks(Agent {
                resumes_through_a_shell: true,
                ..Agent::default()
            }),
            ["resume"]
        );
        assert_eq!(
            runtime_checks(Agent {
                reads_a_missing_transcript: true,
                ..Agent::default()
            }),
            ["usage"]
        );
        assert_eq!(
            runtime_checks(Agent {
                forgets_the_transcript_root: true,
                ..Agent::default()
            }),
            ["locations"]
        );
        assert_eq!(
            runtime_checks(Agent {
                installs_skills_anywhere: true,
                ..Agent::default()
            }),
            ["locations"]
        );
        let slow: Vec<String> = runtime_findings_with(
            Agent::default(),
            Some((Op::Resume, Duration::from_millis(1001))),
        )
        .into_iter()
        .map(|finding| finding.check)
        .collect();
        assert!(
            !slow.is_empty() && slow.iter().all(|check| check == "resume"),
            "{slow:?}"
        );
    }

    #[test]
    fn a_host_driver_is_no_runtime_driver() {
        let mut driver = NoPanes::default();
        let mut invoke = |args: &[&str], request: &[u8]| {
            let owned: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            let mut stdout = Vec::new();
            serve(&owned, request, &mut stdout, &mut driver);
            DriverOutput {
                stdout,
                elapsed: Duration::ZERO,
            }
        };
        let found = check_runtime(
            &mut invoke,
            &RuntimeFixture {
                home: "/h".into(),
                session: "s".into(),
            },
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].check, "capabilities");
    }

    #[test]
    fn a_driver_without_valid_capabilities_stops_the_run() {
        let mut invoke = |_: &[&str], _: &[u8]| DriverOutput {
            stdout: b"not json".to_vec(),
            elapsed: Duration::ZERO,
        };
        let found = check(
            &mut invoke,
            &Fixture {
                socket: "/s".into(),
            },
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].check, "capabilities");
    }
}
