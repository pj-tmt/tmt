//! Development-only tmux adapter probe for isolated E2E fixtures.
//! This is not a supported tmt command and is never an installed artifact.

use std::{
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use tmt_adapters::{
    host::DeliveryError,
    process::{CommandFailure, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner},
    tmux::{CallerEnvironment, OperationOptions, Tmux, TmuxError},
};
use tmt_core::endpoint::{
    BindingMarker, EndpointProbe, EndpointSnapshot, PaneObservation, ServerEvidence,
    valid_process_id,
};

const USAGE: &str = "Usage: tmux-probe [--json] caller | snapshot [pane-id ...] | server | probe <socket> <pid> [pane-id ...] | probe-server <socket> <pid> | target <target> | set-marker <pane> <name> <canonical-name> <identity-id> <binding-id> <server-id> <pane-pid> | clear-marker <pane> <binding-id> | send <socket> <pane> <message> | capture <socket> <pane> <lines>";

#[derive(Debug)]
enum ProbeError {
    Environment,
    Usage(&'static str),
    Tmux(TmuxError),
    Delivery(DeliveryError),
}

impl ProbeError {
    fn code(&self) -> &'static str {
        match self {
            Self::Environment => "ENVIRONMENT_ERROR",
            Self::Usage(_) => "USAGE_ERROR",
            Self::Tmux(_) => "TMUX_ERROR",
            Self::Delivery(error) if error.uncertain() => "DELIVERY_UNCERTAIN",
            Self::Delivery(_) => "DELIVERY_PREPARATION_FAILED",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Environment => "TMT_E2E_SOCKET must be set for the isolated probe.".into(),
            Self::Usage(message) => (*message).into(),
            Self::Tmux(error) => error.to_string(),
            Self::Delivery(error) => error.to_string(),
        }
    }
}

impl From<TmuxError> for ProbeError {
    fn from(error: TmuxError) -> Self {
        Self::Tmux(error)
    }
}

impl From<DeliveryError> for ProbeError {
    fn from(error: DeliveryError) -> Self {
        Self::Delivery(error)
    }
}

struct CountingRunner<R> {
    inner: R,
    count: Arc<AtomicUsize>,
    diagnostic: Option<Arc<Mutex<Option<serde_json::Value>>>>,
}

impl<R: CommandRunner> CommandRunner for CountingRunner<R> {
    fn execute(
        &self,
        request: CommandRequest<'_>,
    ) -> Result<CommandOutput, tmt_adapters::process::CommandError> {
        self.count.fetch_add(1, Ordering::Relaxed);
        let Some(diagnostic) = &self.diagnostic else {
            return self.inner.execute(request);
        };
        let (program, args, deadline, limit) = (
            request.program,
            request.args,
            request.deadline,
            request.max_output_bytes,
        );
        let began = Instant::now();
        let budget = deadline.saturating_duration_since(began);
        let result = self.inner.execute(request);
        {
            // Keep only the last bounded command, and publish it only if caller
            // observation fails. Never record stdin or add another observation.
            let elapsed = began.elapsed();
            let (failure, code, signal, raw_os_error, cleanup_failed, output) = match &result {
                Ok(output) => (None, None, None, None, false, Some(output)),
                Err(error) => {
                    let (kind, code, signal) = match error.kind {
                        CommandFailure::Spawn => ("spawn", None, None),
                        CommandFailure::Timeout => ("timeout", None, None),
                        CommandFailure::OutputLimit => ("output_limit", None, None),
                        CommandFailure::Io => ("io", None, None),
                        CommandFailure::Exit { code, signal } => ("exit", code, signal),
                    };
                    (
                        Some(kind),
                        code,
                        signal,
                        error.raw_os_error(),
                        error.cleanup_failed(),
                        error.output.as_ref(),
                    )
                }
            };
            let (stdout, stderr) = output.map_or((&[][..], &[][..]), |output| {
                (output.stdout.as_slice(), output.stderr.as_slice())
            });
            *diagnostic.lock().unwrap() = Some(serde_json::json!({
                "program": bounded_text(program.as_encoded_bytes(), 256),
                "args": args.iter().take(16).map(|arg| bounded_text(arg.as_encoded_bytes(), 256)).collect::<Vec<_>>(),
                "argsTruncated": args.len() > 16 || args.iter().any(|arg| arg.as_encoded_bytes().len() > 256),
                "budgetMs": budget.as_secs_f64() * 1000.0,
                "elapsedMs": elapsed.as_secs_f64() * 1000.0,
                "outputLimitBytes": limit,
                "commandFailure": failure,
                "exitCode": code,
                "signal": signal,
                "rawOsError": raw_os_error,
                "cleanupFailed": cleanup_failed,
                "outputAvailable": output.is_some(),
                "stdout": bounded_text(stdout, 4096),
                "stderr": bounded_text(stderr, 4096),
                "stdoutBytes": stdout.len(),
                "stderrBytes": stderr.len(),
                "outputTruncated": stdout.len() > 4096 || stderr.len() > 4096,
            }));
        }
        result
    }
}

fn bounded_text(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).into_owned()
}

fn caller_document<R: CommandRunner>(
    tmux: &Tmux<CountingRunner<R>>,
    environment: &CallerEnvironment,
) -> Result<serde_json::Value, ProbeError> {
    Ok(serde_json::json!({"pane": tmux.caller_pane(environment)?}))
}

fn marker_json(marker: Option<&BindingMarker>) -> serde_json::Value {
    marker.map_or(serde_json::Value::Null, |marker| {
        serde_json::json!({
            "name": marker.name,
            "canonicalName": marker.canonical_name,
            "identityId": marker.identity_id,
            "bindingId": marker.binding_id,
            "serverId": marker.server_id,
            "panePid": marker.pane_pid,
        })
    })
}

fn server_json(server: &ServerEvidence) -> serde_json::Value {
    serde_json::json!({
        "serverId": server.server_id,
        "socketPath": server.socket_path,
        "serverPid": server.server_pid,
        "serverStartTime": server.server_start_time,
    })
}

fn pane_json(pane: &PaneObservation) -> serde_json::Value {
    serde_json::json!({
        "id": pane.id,
        "target": pane.target,
        "cwd": pane.cwd,
        "command": pane.command,
        "panePid": pane.pane_pid,
        "suggestedName": pane.suggested_name,
        "marker": marker_json(pane.marker.as_ref()),
    })
}

fn snapshot_json(snapshot: &EndpointSnapshot) -> serde_json::Value {
    serde_json::json!({
        "server": server_json(&snapshot.server),
        "panes": snapshot.panes.iter().map(pane_json).collect::<Vec<_>>(),
    })
}

fn probe_json(probe: EndpointProbe) -> serde_json::Value {
    match probe {
        EndpointProbe::Live(snapshot) => serde_json::json!({
            "status": "live",
            "snapshot": snapshot_json(&snapshot),
        }),
        EndpointProbe::Dead => serde_json::json!({"status": "dead"}),
        EndpointProbe::Unknown => serde_json::json!({"status": "unknown"}),
    }
}

fn parse_pid(text: &str) -> Result<u64, ProbeError> {
    let pid = text
        .parse::<u64>()
        .map_err(|_| ProbeError::Usage("PID must be a positive safe integer."))?;
    if !valid_process_id(pid) {
        return Err(ProbeError::Usage("PID must be a positive safe integer."));
    }
    Ok(pid)
}

fn options<'a>(pane_ids: &'a [String]) -> OperationOptions<'a> {
    OperationOptions {
        deadline: None,
        pane_ids: (!pane_ids.is_empty()).then_some(pane_ids),
    }
}

fn run<R: CommandRunner>(
    args: &[String],
    tmux: &Tmux<CountingRunner<R>>,
) -> Result<serde_json::Value, ProbeError> {
    let Some(mode) = args.first().map(String::as_str) else {
        return Err(ProbeError::Usage(USAGE));
    };
    match mode {
        "send" if args.len() == 4 => {
            // As production sends: core's pane input policy, then the transport.
            let text = tmt_core::driver::pane_input_text(&args[3]);
            tmux.send_on(&args[1], &args[2], &text, Duration::ZERO)?;
            Ok(serde_json::json!({"sent": true}))
        }
        "capture" if args.len() == 4 => {
            let lines = args[3]
                .parse::<u64>()
                .map_err(|_| ProbeError::Usage("Capture lines must be an unsigned integer."))?;
            Ok(serde_json::json!({"output": tmux.capture_on(&args[1], &args[2], lines)?}))
        }
        "caller" if args.len() == 1 => caller_document(tmux, &CallerEnvironment::current()),
        "snapshot" => {
            let pane_ids = args[1..].to_vec();
            let snapshot = tmux.snapshot(options(&pane_ids))?;
            Ok(snapshot_json(&snapshot))
        }
        "server" if args.len() == 1 => {
            let pane_ids = Vec::new();
            let snapshot = tmux.snapshot(OperationOptions {
                pane_ids: Some(&pane_ids),
                ..Default::default()
            })?;
            Ok(snapshot_json(&snapshot))
        }
        "probe" if args.len() >= 3 => {
            let socket = &args[1];
            if socket.is_empty() {
                return Err(ProbeError::Usage("probe requires a nonempty socket."));
            }
            let recorded_pid = parse_pid(&args[2])?;
            let pane_ids = args[3..].to_vec();
            let probe = tmux.probe(socket, recorded_pid, options(&pane_ids))?;
            Ok(probe_json(probe))
        }
        "probe-server" if args.len() == 3 => {
            let socket = &args[1];
            if socket.is_empty() {
                return Err(ProbeError::Usage(
                    "probe-server requires a nonempty socket.",
                ));
            }
            let recorded_pid = parse_pid(&args[2])?;
            let pane_ids = Vec::new();
            let probe = tmux.probe(
                socket,
                recorded_pid,
                OperationOptions {
                    pane_ids: Some(&pane_ids),
                    ..Default::default()
                },
            )?;
            Ok(probe_json(probe))
        }
        "target" if args.len() == 2 => Ok(serde_json::json!({
            "pane": tmux.resolve_target(&args[1], OperationOptions::default())?,
        })),
        "set-marker" if args.len() == 8 => {
            let marker = BindingMarker {
                name: args[2].clone(),
                canonical_name: args[3].clone(),
                identity_id: args[4].clone(),
                binding_id: args[5].clone(),
                server_id: args[6].clone(),
                pane_pid: parse_pid(&args[7])?,
            };
            tmux.set_marker(&args[1], &marker, OperationOptions::default())?;
            Ok(serde_json::json!({"changed": true}))
        }
        "clear-marker" if args.len() == 3 => {
            let changed =
                tmux.clear_marker(&args[1], Some(&args[2]), OperationOptions::default())?;
            Ok(serde_json::json!({"changed": changed}))
        }
        _ => Err(ProbeError::Usage(USAGE)),
    }
}

fn result_document(
    result: Result<serde_json::Value, ProbeError>,
    count: usize,
    diagnostic: Option<&Mutex<Option<serde_json::Value>>>,
) -> serde_json::Value {
    let mut document = match result {
        Ok(result) => {
            let mut object = match result {
                serde_json::Value::Object(object) => object,
                _ => {
                    let error = ProbeError::Usage("Probe result must be a JSON object.");
                    serde_json::Map::from_iter([(
                        "error".into(),
                        serde_json::json!({"code": error.code(), "message": error.message()}),
                    )])
                }
            };
            object.insert("commandCount".into(), serde_json::json!(count));
            serde_json::Value::Object(object)
        }
        Err(error) => serde_json::json!({
            "error": {"code": error.code(), "message": error.message()},
            "commandCount": count,
        }),
    };
    if (document.get("pane").is_some_and(serde_json::Value::is_null)
        || document.get("error").is_some())
        && let Some(last) = diagnostic.and_then(|last| last.lock().unwrap().take())
    {
        document["diagnostic"] = last;
    }
    document
}

fn main() -> ExitCode {
    if !matches!(std::env::var_os("TMT_E2E_SOCKET"), Some(value) if !value.is_empty()) {
        let document = serde_json::json!({
            "error": {"code": "ENVIRONMENT_ERROR", "message": ProbeError::Environment.message()},
            "commandCount": 0,
        });
        let _ = writeln!(io::stdout().lock(), "{document}");
        return ExitCode::FAILURE;
    }

    let raw_args = std::env::args_os().skip(1).collect::<Vec<OsString>>();
    let mut args = Vec::with_capacity(raw_args.len());
    for arg in raw_args {
        let Ok(arg) = arg.into_string() else {
            let document = serde_json::json!({
                "error": {"code": "USAGE_ERROR", "message": "Arguments must be valid UTF-8."},
                "commandCount": 0,
            });
            let _ = writeln!(io::stdout().lock(), "{document}");
            return ExitCode::FAILURE;
        };
        args.push(arg);
    }
    if args.first().is_some_and(|arg| arg == "--json") {
        args.remove(0);
    }

    let count = Arc::new(AtomicUsize::new(0));
    let diagnostic =
        (args.first().is_some_and(|arg| arg == "caller")).then(|| Arc::new(Mutex::new(None)));
    let tmux = Tmux::new(CountingRunner {
        inner: UnixCommandRunner,
        count: Arc::clone(&count),
        diagnostic: diagnostic.clone(),
    });
    let document = result_document(
        run(&args, &tmux),
        count.load(Ordering::Relaxed),
        diagnostic.as_deref(),
    );
    if writeln!(io::stdout().lock(), "{document}").is_err() {
        return ExitCode::FAILURE;
    }
    if document.get("error").is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    enum Reply {
        Text(&'static str),
        Timeout,
        CleanupFailure,
    }

    impl CommandRunner for Reply {
        fn execute(
            &self,
            request: CommandRequest<'_>,
        ) -> Result<CommandOutput, tmt_adapters::process::CommandError> {
            match self {
                Self::Text(text) => Ok(CommandOutput {
                    stdout: text.as_bytes().to_vec(),
                    stderr: Vec::new(),
                }),
                // The existing process owner refuses an expired deadline before spawn.
                // This constructs its real typed timeout without invoking any tmux.
                Self::Timeout | Self::CleanupFailure => {
                    let mut error = UnixCommandRunner
                        .execute(CommandRequest {
                            deadline: Instant::now(),
                            ..request
                        })
                        .unwrap_err();
                    if matches!(self, Self::CleanupFailure) {
                        error.cleanup_error = Some(io::Error::other("owned cleanup refused"));
                    }
                    Err(error)
                }
            }
        }
    }

    fn caller(reply: Reply) -> serde_json::Value {
        let count = Arc::new(AtomicUsize::new(0));
        let diagnostic = Arc::new(Mutex::new(None));
        let tmux = Tmux::new(CountingRunner {
            inner: reply,
            count: Arc::clone(&count),
            diagnostic: Some(Arc::clone(&diagnostic)),
        });
        let document = caller_document(
            &tmux,
            &CallerEnvironment {
                tmux: Some("/tmp/private.sock,321,0".into()),
                pane: Some("%9".into()),
                process_id: 900,
                driver_env: Default::default(),
            },
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        result_document(document, count.load(Ordering::Relaxed), Some(&diagnostic))
    }

    #[test]
    fn verified_caller_keeps_its_existing_result_without_diagnostics() {
        assert_eq!(
            caller(Reply::Text(
                "%9__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321\n"
            )),
            serde_json::json!({"pane": "%9", "commandCount": 1})
        );
    }

    #[test]
    fn absent_caller_preserves_the_same_commands_typed_timeout() {
        let document = caller(Reply::Timeout);
        assert!(document["pane"].is_null());
        assert_eq!(document["diagnostic"]["commandFailure"], "timeout");
        assert_eq!(document["diagnostic"]["cleanupFailed"], false);
        assert_eq!(document["diagnostic"]["outputAvailable"], false);
    }

    #[test]
    fn cleanup_failure_remains_an_error_and_preserves_its_diagnostic() {
        let document = caller(Reply::CleanupFailure);
        assert_eq!(document["error"]["code"], "TMUX_ERROR");
        assert_eq!(document["diagnostic"]["commandFailure"], "timeout");
        assert_eq!(document["diagnostic"]["cleanupFailed"], true);
    }

    #[test]
    fn rejected_caller_output_is_preserved_without_claiming_a_command_failure() {
        for output in [
            "malformed\n",
            "%9__TMT_CALLER_PANE_4f1c__/tmp/other.sock__TMT_CALLER_PANE_4f1c__321\n",
        ] {
            let document = caller(Reply::Text(output));
            assert!(document["pane"].is_null());
            assert_eq!(document["diagnostic"]["stdout"], output);
            assert_eq!(document["diagnostic"]["outputAvailable"], true);
            assert!(document["diagnostic"]["commandFailure"].is_null());
        }
    }
}
