//! A finite advisory JSON observation, without post-command learning or capture.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
};
use tmt_core::discovery::*;

pub(super) fn candidate_document(c: &DiscoveryCandidate) -> Value {
    let fence = c.fence.as_ref();
    json!({"host":tmt_core::host::HostKind::Tmux.as_str(),"socket":fence.map(|f|&f.socket),"server":fence.map(|f|json!({"pid":f.server.pid(),"start":f.server.start_identity(),"identity":f.server_id})),"session":c.session,"window":c.window,"pane":c.pane,"target":c.target,"native":fence.map(|f|json!({"pid":f.process.pid(),"start":f.process.start_identity()})),"runtime":{"driver":fence.and_then(|f|f.runtime.as_ref()),"state":"unknown","reason":"executable_hint_is_advisory","recordedState":c.identity.as_ref().and_then(|e|e.binding.as_ref()).map(|b|b.session.state.as_str())},"identity":c.identity.as_ref().map(|e|json!({"id":e.identity.id,"name":e.identity.name,"binding":e.binding.as_ref().map(|b|&b.id)})),"state":c.state,"reason":c.reason,"duplicate":c.duplicate,"display":{"title":c.title,"command":c.command,"cwd":c.cwd,"truncated":c.text_truncated},"adoptable":c.adoptable(),"expect":fence.filter(|_|c.adoptable()).map(DiscoveryTarget::digest)})
}

pub fn execute(
    selected_host: Option<&str>,
    socket: Option<&str>,
    mode: OutputMode,
) -> io::Result<u8> {
    let deadline = Instant::now() + Duration::from_millis(SCAN_BUDGET_MS);
    let mut document = json!({"version":1,"observedAtMs":tmt_adapters::request_runtime::wall_time_ms(),"sources":[],"candidates":[],"truncated":false});
    let mut source = json!({"host":selected_host.unwrap_or(tmt_core::host::HostKind::Tmux.as_str()),"socket":socket,"status":"unknown","reason":null});
    if selected_host.is_some_and(|h| h != tmt_core::host::HostKind::Tmux.as_str()) {
        source["status"] = "unsupported".into();
        source["reason"] = "Candidate discovery is unavailable for this host.".into();
    } else {
        let caller = CallerEnvironment::current();
        let host =
            if selected_host == Some(tmt_core::host::HostKind::Tmux.as_str()) || socket.is_some() {
                Host::for_caller_with(&caller, tmt_adapters::process::UnixCommandRunner)
            } else {
                Host::for_caller(&caller)
            };
        match host.discover(socket, None, deadline) {
            Ok(Some(mut observation)) => {
                source["socket"] = observation.socket.clone().into();
                source["status"] = "ok".into();
                match ConfigPaths::discover() {
                    Ok(paths) => {
                        if let Err(error) = tmt_adapters::discovery::annotate(
                            &mut observation,
                            &paths.database,
                            deadline,
                        ) {
                            source["status"] = "unknown".into();
                            source["reason"] = error.to_string().into();
                            for c in &mut observation.candidates {
                                c.reason = Some("storage_unknown");
                            }
                        }
                    }
                    Err(error) => {
                        source["status"] = "unknown".into();
                        source["reason"] = error.to_string().into();
                        for c in &mut observation.candidates {
                            c.reason = Some("storage_unknown");
                        }
                    }
                }
                let mut rows = Vec::new();
                let mut bytes = 1024;
                for c in &observation.candidates {
                    let row = candidate_document(c);
                    let size = row.to_string().len() + 1;
                    if bytes + size > MAX_RESULT_BYTES {
                        observation.truncated = true;
                        break;
                    }
                    bytes += size;
                    rows.push(row);
                }
                if observation.truncated {
                    source["reason"] = "scan_limit".into();
                    for row in &mut rows {
                        row["adoptable"] = false.into();
                        row["expect"] = Value::Null;
                        row["reason"] = "scan_truncated".into();
                    }
                }
                document["candidates"] = rows.into();
                document["truncated"] = observation.truncated.into();
            }
            Ok(None) => {
                source["host"] = host.kind().as_str().into();
                source["status"] = "unsupported".into();
                source["reason"] = "Candidate discovery is unavailable for this host.".into();
            }
            Err(error) => {
                if error.cleanup_failed() {
                    return Failure::new("CLEANUP_ERROR", error.to_string(), 1).publish(mode);
                }
                source["status"] = tmt_adapters::discovery::source_status(&error, socket).into();
                source["reason"] = error.to_string().into();
                // An unavailable bounded acquisition never claims a complete scan.
                document["truncated"] = true.into();
            }
        }
    }
    document["sources"] = json!([source]);
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{document}")?;
    } else {
        for source in document["sources"].as_array().expect("source array") {
            writeln!(
                output,
                "{:?} on {:?}: {}",
                source["host"].as_str().unwrap_or("unknown"),
                source["socket"],
                source["status"].as_str().unwrap_or("unknown")
            )?;
        }
        for row in document["candidates"].as_array().expect("candidate array") {
            writeln!(
                output,
                "  {} {:?}: {}",
                row["pane"].as_str().unwrap_or("unknown"),
                row["display"]["title"],
                row["state"].as_str().unwrap_or("unknown")
            )?;
        }
    }
    Ok(0)
}
