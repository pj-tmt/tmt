//! Development-only tmux adapter probe for isolated E2E fixtures.
//! This is not a supported tmt command and is never an installed artifact.

use std::{
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tmt_adapters::{
    host::DeliveryError,
    process::{CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner},
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

struct CountingRunner {
    inner: UnixCommandRunner,
    count: Arc<AtomicUsize>,
}

impl CommandRunner for CountingRunner {
    fn execute(
        &self,
        request: CommandRequest<'_>,
    ) -> Result<CommandOutput, tmt_adapters::process::CommandError> {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.inner.execute(request)
    }
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

fn run(args: &[String], tmux: &Tmux<CountingRunner>) -> Result<serde_json::Value, ProbeError> {
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
        "caller" if args.len() == 1 => Ok(serde_json::json!({
            "pane": tmux.caller_pane(&CallerEnvironment::current())?,
        })),
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
    let tmux = Tmux::new(CountingRunner {
        inner: UnixCommandRunner,
        count: Arc::clone(&count),
    });
    let document = match run(&args, &tmux) {
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
            object.insert(
                "commandCount".into(),
                serde_json::json!(count.load(Ordering::Relaxed)),
            );
            serde_json::Value::Object(object)
        }
        Err(error) => serde_json::json!({
            "error": {"code": error.code(), "message": error.message()},
            "commandCount": count.load(Ordering::Relaxed),
        }),
    };
    if writeln!(io::stdout().lock(), "{document}").is_err() {
        return ExitCode::FAILURE;
    }
    if document.get("error").is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
