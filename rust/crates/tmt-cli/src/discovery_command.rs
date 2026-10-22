//! A finite advisory JSON observation, without post-command learning or capture.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    driver_protocol::registry::Admitted,
    host::{CallerEnvironment, Host, HostError, Unselectable},
    process::{CommandRunner, UnixCommandRunner},
};
use tmt_core::{
    discovery::*,
    host::{HostKind, HostName},
};

/// `host` is the observed host, for a row without a fence. A host's agent
/// name is display only, and a row without one has no member.
pub(super) fn candidate_document(c: &DiscoveryCandidate, host: HostKind) -> Value {
    let mut row = row_document(c, host);
    if let Some(name) = &c.agent_name {
        row["display"]["agentName"] = name.as_str().into();
    }
    row
}

fn row_document(c: &DiscoveryCandidate, host: HostKind) -> Value {
    let fence = c.fence.as_ref();
    json!({"host":fence.map_or(host, |f| f.host).as_str(),"socket":fence.map(|f|&f.socket),"server":fence.map(|f|json!({"pid":f.server.pid(),"start":f.server.start_identity(),"identity":f.server_id})),"session":c.session,"window":c.window,"pane":c.pane,"target":c.target,"native":fence.map(|f|json!({"pid":f.process.pid(),"start":f.process.start_identity()})),"runtime":{"driver":fence.and_then(|f|f.runtime.as_ref()),"state":"unknown","reason":"executable_hint_is_advisory","recordedState":c.identity.as_ref().and_then(|e|e.binding.as_ref()).map(|b|b.session.state.as_str())},"identity":c.identity.as_ref().map(|e|json!({"id":e.identity.id,"name":e.identity.name,"binding":e.binding.as_ref().map(|b|&b.id)})),"state":c.state,"reason":c.reason,"duplicate":c.duplicate,"display":{"title":c.title,"command":c.command,"cwd":c.cwd,"truncated":c.text_truncated},"adoptable":c.adoptable(),"expect":fence.filter(|_|c.adoptable()).map(DiscoveryTarget::digest)})
}

/// The sources a scan reports instead of observing a host.
struct Refusal {
    hosts: Vec<String>,
    status: &'static str,
    reason: &'static str,
}

/// What selection chose: a host to observe, or the sources a scan reports
/// instead.
enum Selected<R> {
    Host(Box<Host<R>>),
    Refused(Refusal),
}

/// The host a scan observes. An explicit tmux selection or socket is never
/// affected by driver approvals. An explicit external host runs only its
/// admitted driver, on the caller's own server of that host when no socket
/// is given. Implicit selection refuses, running nothing, when the caller's
/// host cannot be known without a driver it must not run. Finding the
/// caller's pane spends at most 1 s of the scan; a read that fails to clean
/// up is an error, and nothing is observed.
fn select<R: CommandRunner + Clone>(
    runner: R,
    selected_host: Option<&str>,
    socket: Option<&str>,
    admitted: &Admitted,
    caller: &CallerEnvironment,
    deadline: Instant,
) -> Result<Selected<R>, HostError> {
    let refuse = |host: &str, status, reason| {
        Selected::Refused(Refusal {
            hosts: vec![host.to_owned()],
            status,
            reason,
        })
    };
    let records = || admitted.records.clone();
    let within = deadline.min(Instant::now() + Duration::from_secs(1));
    Ok(match selected_host {
        Some(name) if name == HostKind::Tmux.as_str() => {
            Selected::Host(Box::new(Host::for_caller_with(caller, runner)))
        }
        None if socket.is_some() => Selected::Host(Box::new(Host::for_caller_with(caller, runner))),
        Some(name) => {
            let host = HostName::new(name)
                .filter(|_| admitted.records.iter().any(|record| record.name == name));
            if let Some(host) = host {
                let host = HostKind::External(host);
                return Ok(Selected::Host(Box::new(match socket {
                    Some(_) => Host::for_host(host, runner, records()),
                    None => Host::for_selected_host(host, caller, runner, records(), within)?,
                })));
            }
            match admitted.gaps.iter().find(|gap| gap.name == name) {
                Some(gap) => refuse(name, "unsupported", gap.reason),
                None if admitted.unreadable.is_some() => {
                    refuse(name, "unknown", "driver_registry_unreadable")
                }
                None => refuse(name, "unsupported", "driver_not_approved"),
            }
        }
        None => match caller.unselectable(admitted) {
            Some(Unselectable::Gap(gap)) => refuse(&gap.name, "unsupported", gap.reason),
            // Every named host is unknown; none is chosen.
            Some(Unselectable::Ambiguous(hosts)) => Selected::Refused(Refusal {
                hosts: hosts.into_iter().map(str::to_owned).collect(),
                status: "unknown",
                reason: "ambiguous_caller_host",
            }),
            // Without the registry only tmux is known, and it is not scanned.
            Some(Unselectable::RegistryUnreadable) => refuse(
                HostKind::Tmux.as_str(),
                "unknown",
                "driver_registry_unreadable",
            ),
            None => Selected::Host(Box::new(Host::for_caller_within(
                caller,
                runner,
                records(),
                within,
            )?)),
        },
    })
}

pub fn execute(
    selected_host: Option<&str>,
    socket: Option<&str>,
    mode: OutputMode,
) -> io::Result<u8> {
    let deadline = Instant::now() + Duration::from_millis(SCAN_BUDGET_MS);
    let mut document = json!({"version":1,"observedAtMs":tmt_adapters::request_runtime::wall_time_ms(),"sources":[],"candidates":[],"truncated":false});
    let mut source = json!({"host":selected_host.unwrap_or(HostKind::Tmux.as_str()),"socket":socket,"status":"unknown","reason":null});
    // One read of the approvals; discovery never adopts or rewrites one.
    let admitted = tmt_adapters::host::external::admitted();
    tmt_adapters::host::external::register(&admitted.records);
    let caller = CallerEnvironment::admitted(&admitted);
    let selected = select(
        UnixCommandRunner,
        selected_host,
        socket,
        &admitted,
        &caller,
        deadline,
    );
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => return Failure::new("CLEANUP_ERROR", error.to_string(), 1).publish(mode),
    };
    match selected {
        Selected::Refused(refusal) => {
            source["status"] = refusal.status.into();
            source["reason"] = refusal.reason.into();
            let sources: Vec<Value> = refusal
                .hosts
                .iter()
                .map(|host| {
                    let mut source = source.clone();
                    source["host"] = host.as_str().into();
                    source
                })
                .collect();
            // An unknown source never claims a complete scan.
            document["truncated"] = (refusal.status == "unknown").into();
            document["sources"] = sources.into();
            return print(document, mode);
        }
        Selected::Host(host) => match host.discover(socket, None, deadline) {
            Ok(mut observation) => {
                let (status, reason) = observation.source.status();
                source["host"] = observation.host.as_str().into();
                source["socket"] = observation.socket.clone().into();
                source["status"] = status.into();
                source["reason"] = reason.into();
                // A source with no inventory (offline, unsupported or empty)
                // has nothing to annotate; its outcome stands.
                let paths = (!observation.candidates.is_empty()).then(ConfigPaths::discover);
                match paths {
                    None => {}
                    Some(Ok(paths)) => {
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
                    Some(Err(error)) => {
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
                    let row = candidate_document(c, observation.host);
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
            Err(error) => {
                if error.cleanup_failed() {
                    return Failure::new("CLEANUP_ERROR", error.to_string(), 1).publish(mode);
                }
                source["host"] = host.kind().as_str().into();
                // A driver failure is never evidence that its server is gone.
                source["status"] = match host.kind() {
                    HostKind::Tmux => tmt_adapters::discovery::tmux_source_status(&error, socket),
                    HostKind::External(_) => "unknown",
                }
                .into();
                source["reason"] = error.to_string().into();
                // An unavailable bounded acquisition never claims a complete scan.
                document["truncated"] = true.into();
            }
        },
    }
    document["sources"] = json!([source]);
    print(document, mode)
}

fn print(document: Value, mode: OutputMode) -> io::Result<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::endpoint::ProcessIncarnation;

    fn candidate(fence: Option<DiscoveryTarget>) -> DiscoveryCandidate {
        DiscoveryCandidate {
            pane: "term_7".into(),
            session: None,
            window: None,
            target: None,
            title: String::new(),
            command: String::new(),
            cwd: String::new(),
            agent_name: None,
            fence,
            identity: None,
            state: "unknown",
            reason: None,
            duplicate: false,
            text_truncated: false,
        }
    }

    #[test]
    fn a_row_names_its_fenced_host_or_else_the_observed_host() {
        let herdr = HostKind::parse("herdr").unwrap();
        let fence = DiscoveryTarget {
            host: herdr,
            socket: "/tmp/herdr.sock".into(),
            server: ProcessIncarnation::new(1, "server-start").unwrap(),
            server_id: None,
            server_start_time: "server-start".into(),
            pane: "term_7".into(),
            process: ProcessIncarnation::new(2, "pane-start").unwrap(),
            marker: None,
            runtime: None,
        };
        let row = candidate_document(&candidate(Some(fence.clone())), HostKind::Tmux);
        assert_eq!(row["host"], "herdr");
        assert_eq!(row["expect"], fence.digest());
        let row = candidate_document(&candidate(None), herdr);
        assert_eq!(row["host"], "herdr");
        assert_eq!(row["expect"], Value::Null);
    }

    /// Answers the caller's pane (its shell is this test process) and logs
    /// every operation.
    const DRIVER: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "$3" >> "$d/calls"
cat > /dev/null
case "$3" in
  capabilities) printf '%s' '{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["caller","server","snapshot"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":["FAKE_PANE_ID"]}}' ;;
  caller) printf '{"ok":{"pane":{"id":"fake-1","socket":"/tmp/fake.sock","shellPid":%s}}}' "$(cat "$d/pid")" ;;
  *) printf '{"error":{"code":"unsupported","message":"no %s"}}' "$3" ;;
esac
"#;

    /// The production runner, except that one read reports its child as not
    /// cleaned up: the driver's `caller` call (`"caller"`), or any ancestry
    /// `ps` read (`"ps"`). It never runs tmux, and logs every program.
    #[derive(Clone)]
    struct Unclean {
        read: &'static str,
        programs: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl tmt_adapters::process::CommandRunner for Unclean {
        fn execute(
            &self,
            request: tmt_adapters::process::CommandRequest<'_>,
        ) -> Result<tmt_adapters::process::CommandOutput, tmt_adapters::process::CommandError>
        {
            use tmt_adapters::process::CommandRequest;
            let program = request.program.to_string_lossy().into_owned();
            let args: Vec<String> = request
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            self.programs.borrow_mut().push(program.clone());
            let failed = |cleanup: bool| {
                let mut error = UnixCommandRunner
                    .execute(CommandRequest {
                        program: std::ffi::OsStr::new("/nonexistent/tmt-select-test"),
                        args: &[],
                        input: b"",
                        deadline: Instant::now() + Duration::from_secs(1),
                        max_output_bytes: 16,
                    })
                    .unwrap_err();
                error.cleanup_error = cleanup.then(|| io::Error::other("not reaped"));
                Err(error)
            };
            if program.ends_with("tmux") {
                return failed(false);
            }
            let ps = program == "ps" || args.iter().any(|arg| arg.ends_with("/ps"));
            let hit = match self.read {
                "caller" => args.iter().any(|arg| arg == "caller"),
                _ => ps,
            };
            if !hit {
                return UnixCommandRunner.execute(request);
            }
            let _ = UnixCommandRunner.execute(request);
            failed(true)
        }
    }

    /// Discovery's selection (#4063): a selection read that fails to clean
    /// up, implicit or explicit, plain or nested, is an error; no inventory,
    /// fallback or tmux call follows. The production runner selects the fake
    /// host from the same environment.
    #[test]
    fn a_failed_cleanup_while_selecting_is_an_error_and_scans_nothing() {
        let root = std::env::temp_dir().join(format!("tmt-select-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("driver")).unwrap();
        let driver = root.join("driver/tmt-driver-fake");
        tmt_test_support::write_executable(&driver, DRIVER.as_bytes(), 0o755).unwrap();
        std::fs::write(root.join("driver/pid"), std::process::id().to_string()).unwrap();
        let record = tmt_adapters::driver_protocol::registry::approve(
            &root.join("global"),
            &driver,
            &UnixCommandRunner,
        )
        .unwrap();
        let admitted = Admitted {
            records: vec![record],
            ..Admitted::default()
        };
        // A private, absent socket: even a tmux call could reach no server.
        let socket = root.join("none.sock");
        let caller = |nested: bool| CallerEnvironment {
            tmux: nested.then(|| format!("{},1,0", socket.display()).into()),
            pane: nested.then(|| "%1".into()),
            process_id: u64::from(std::process::id()),
            driver_env: [("FAKE_PANE_ID".to_owned(), "f1".into())].into(),
        };
        let calls = || std::fs::read_to_string(root.join("driver/calls")).unwrap_or_default();
        let deadline = || Instant::now() + Duration::from_secs(5);
        let fake = HostKind::External(HostName::new("fake").unwrap());
        for host in [None, Some("fake")] {
            let selected = select(
                UnixCommandRunner,
                host,
                None,
                &admitted,
                &caller(false),
                deadline(),
            );
            match selected {
                Ok(Selected::Host(chosen)) => assert_eq!(chosen.kind(), fake, "{host:?}"),
                _ => panic!("{host:?}: the fake host is selected"),
            }
            for (nested, read) in [
                (false, "caller"),
                (false, "ps"),
                (true, "caller"),
                (true, "ps"),
            ] {
                let case = format!("{host:?} nested={nested} {read}");
                let before = calls();
                let runner = Unclean {
                    read,
                    programs: Default::default(),
                };
                let selected = select(
                    runner.clone(),
                    host,
                    None,
                    &admitted,
                    &caller(nested),
                    deadline(),
                );
                match selected {
                    Err(error) => assert!(error.cleanup_failed(), "{case}: {error}"),
                    Ok(_) => panic!("{case}: a cleanup failure selects nothing"),
                }
                let after = calls();
                let new: Vec<&str> = after[before.len()..].lines().collect();
                assert!(new.iter().all(|op| *op == "caller"), "{case}: {new:?}");
                let programs = runner.programs.borrow();
                assert!(
                    !programs.iter().any(|program| program.ends_with("tmux")),
                    "{case}: {programs:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
