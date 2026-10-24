//! Read-only first-run topology, including servers without TMT metadata.

use super::{Tmux, TmuxError, TmuxFailure, metadata};
use crate::process::{CommandRunner, runtime::observe_starts};
use std::{collections::BTreeMap, time::Instant};
use tmt_core::{
    discovery::*,
    endpoint::{valid_process_id, valid_server_id},
    host::HostKind,
};

// Escape framing characters on the server, without shell evaluation or recursive
// format expansion. Every display value is decoded back to its literal bytes.
fn field(name: &str) -> String {
    let mut value = format!(r"#{{s|\\|\\\\|:#{{{name}}}}}");
    for (from, to) in [("\n", r"\\n"), ("\t", r"\\t"), ("\r", r"\\r")] {
        value = format!("#{{s|{from}|{to}|:{value}}}");
    }
    value
}

fn format() -> String {
    [
        "socket_path",
        "pid",
        "@tmt.server-id",
        "pane_id",
        "pane_pid",
        "session_name",
        "window_index",
        "pane_index",
        "pane_title",
        "pane_current_command",
        "pane_current_path",
        "@tmt.agent",
        "start_time",
    ]
    .map(field)
    .join("\t")
}

fn decode(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        result.push(match chars.next()? {
            '\\' => '\\',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            _ => return None,
        });
    }
    Some(result)
}

impl<R: CommandRunner> Tmux<R> {
    pub fn discover(
        &self,
        socket: Option<&str>,
        pane: Option<&str>,
        deadline: Instant,
    ) -> Result<DiscoveryObservation, TmuxError> {
        let mut args = super::socket_args(socket);
        args.extend(["list-panes".into(), "-a".into()]);
        if let Some(pane) = pane {
            if !super::valid_pane_id(pane) {
                return Err(invalid());
            }
            args.extend(["-f".into(), format!("#{{==:#{{pane_id}},{pane}}}")]);
        }
        args.extend(["-F".into(), format()]);
        let output = self.run(
            "tmux",
            args,
            deadline,
            MAX_RESULT_BYTES,
            TmuxFailure::Evidence,
        )?;
        let mut rows = BTreeMap::<String, Vec<String>>::new();
        let mut duplicate = std::collections::BTreeSet::new();
        let mut truncated = false;
        for line in output.split_terminator('\n') {
            let fields = line
                .split('\t')
                .map(decode)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(invalid)?;
            if fields.len() != 13 || !super::valid_pane_id(&fields[3]) {
                return Err(invalid());
            }
            if let Some(old) = rows.get(&fields[3]) {
                if old[..5] != fields[..5] || old[9..] != fields[9..] {
                    return Err(invalid());
                }
                duplicate.insert(fields[3].clone());
            } else if rows.len() < MAX_CANDIDATES {
                rows.insert(fields[3].clone(), fields);
            } else {
                truncated = true;
            }
        }
        let Some(first) = rows.values().next() else {
            // An explicit missing pane is a guarded refusal, not an operational
            // source failure. Unscoped empty topology remains unknown.
            if pane.is_some() {
                return Ok(DiscoveryObservation {
                    host: HostKind::Tmux,
                    source: DiscoverySource::Ok,
                    socket: socket.unwrap_or_default().into(),
                    candidates: Vec::new(),
                    truncated: false,
                });
            }
            return Err(invalid());
        };
        let socket_path = first[0].clone();
        if socket_path.is_empty()
            || socket.is_some_and(|socket| socket != socket_path)
            || rows.values().any(|row| row[..3] != first[..3])
        {
            return Err(invalid());
        }
        let id = |value: &str| value.parse::<u64>().ok().filter(|id| valid_process_id(*id));
        let server_pid = id(&first[1]).ok_or_else(invalid)?;
        let mut pids = vec![server_pid];
        for row in rows.values() {
            pids.push(id(&row[4]).ok_or_else(invalid)?);
        }
        let starts = observe_starts(&self.runner, &pids, deadline)
            .map_err(|error| TmuxError::command(TmuxFailure::Evidence, error))?;
        if Instant::now() >= deadline {
            truncated = true;
        }
        let mut candidates = Vec::new();
        for row in rows.values() {
            let pane_pid = id(&row[4]).ok_or_else(invalid)?;
            let document = metadata::decode(&row[11]);
            let marker = metadata::marker(&document);
            let malformed_marker = (!row[11].is_empty()
                && crate::json_document::parse(&row[11])
                    .ok()
                    .is_none_or(|value| {
                        !value.is_object()
                            || value.get("version").and_then(serde_json::Value::as_u64) != Some(1)
                    }))
                || (document.get("globalIdentity").is_some() && marker.is_none());
            let runtime = super::evidence::suggested_name(&row[9]);
            let fence =
                starts
                    .get(&server_pid)
                    .zip(starts.get(&pane_pid))
                    .map(|(server, process)| DiscoveryTarget {
                        host: HostKind::Tmux,
                        socket: socket_path.clone(),
                        server: server.clone(),
                        server_id: valid_server_id(&row[2]).then(|| row[2].clone()),
                        server_start_time: row[12].clone(),
                        pane: row[3].clone(),
                        process: process.clone(),
                        marker,
                        runtime,
                    });
            let mut reason = if fence.is_none() {
                Some("native_incarnation_unknown")
            } else if malformed_marker || (!row[2].is_empty() && !valid_server_id(&row[2])) {
                Some("ownership_unknown")
            } else {
                None
            };
            if truncated {
                reason = Some("scan_truncated");
            }
            let (title, a) = display_text(&row[8]);
            let (command, b) = display_text(&row[9]);
            let (cwd, c) = display_text(&row[10]);
            let (session, d) = display_text(&row[5]);
            candidates.push(DiscoveryCandidate {
                pane: row[3].clone(),
                session: Some(session),
                window: Some(row[6].clone()),
                target: Some(format!("{}:{}.{}", row[5], row[6], row[7])),
                title,
                command,
                cwd,
                agent_name: None,
                fence,
                identity: None,
                state: "unknown",
                reason,
                duplicate: duplicate.contains(&row[3]),
                text_truncated: a || b || c || d,
            });
        }
        Ok(DiscoveryObservation {
            host: HostKind::Tmux,
            source: DiscoverySource::Ok,
            socket: socket_path,
            candidates,
            truncated,
        })
    }
}

fn invalid() -> TmuxError {
    TmuxError::evidence("tmux discovery evidence is incomplete or ambiguous")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process::{CommandError, CommandOutput, CommandRequest},
        scripted_runner::ScriptedRunner,
    };
    use tmt_core::endpoint::ProcessIncarnation;
    struct NativeRunner(ScriptedRunner);
    impl CommandRunner for NativeRunner {
        fn execute(&self, r: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.execute(r)
        }
        fn process_observation(
            &self,
            pid: u64,
            _: Instant,
        ) -> Option<crate::process::runtime::ProcessObservation> {
            Some(crate::process::runtime::ProcessObservation::Live(
                ProcessIncarnation::new(pid, "native-start").unwrap(),
            ))
        }
    }
    fn row(pane: &str, title: &str) -> String {
        [
            "/tmp/a",
            "41",
            "",
            pane,
            "100",
            "work",
            "0",
            "1",
            title,
            "codex",
            "/tmp",
            r#"{"version":1}"#,
            "1",
        ]
        .join("\t")
            + "\n"
    }
    #[test]
    fn discovery_decodes_untrusted_fields_as_data_without_metadata_initialization() {
        let runner = ScriptedRunner::default();
        runner.push_output(
            row("%1", r"line\n\t\\#{run-shell:bad};").into_bytes(),
            vec![],
        );
        let tmux = Tmux::new(NativeRunner(runner));
        let result = tmux
            .discover(
                Some("/tmp/a"),
                None,
                Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap();
        let c = &result.candidates[0];
        assert_eq!(c.title, "line\n\t\\#{run-shell:bad};");
        assert!(c.adoptable());
        assert_eq!(c.state, "unknown");
        assert_eq!(c.fence.as_ref().unwrap().server_id, None);
        let calls = tmux.runner.0.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            &calls[0].args[..5],
            ["-u", "-S", "/tmp/a", "list-panes", "-a"]
        );
        assert_eq!(calls[0].max_output_bytes, MAX_RESULT_BYTES);
        assert!(!calls[0].args.iter().any(|a| a == "set-option"));
        assert!(format().contains(r"s|\\|\\\\|"));
    }
    #[test]
    fn candidate_cap_is_explicit_and_never_authorizes_an_incomplete_scan() {
        let runner = ScriptedRunner::default();
        let output = (0..257)
            .map(|i| row(&format!("%{i}"), "candidate"))
            .collect::<String>();
        runner.push_output(output.into_bytes(), vec![]);
        let tmux = Tmux::new(NativeRunner(runner));
        let result = tmux
            .discover(
                Some("/tmp/a"),
                None,
                Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(result.candidates.len(), MAX_CANDIDATES);
        assert!(result.truncated);
        assert!(result.candidates.iter().all(|c| !c.adoptable()));
    }
    #[test]
    fn escaped_delimiters_do_not_create_candidates_and_bad_framing_refuses() {
        assert_eq!(decode(r"a\tb\nc\r\\d"), Some("a\tb\nc\r\\d".into()));
        assert_eq!(decode(r"a\x"), None);
        let runner = ScriptedRunner::default();
        runner.push_output(b"forged\tpane\n".to_vec(), vec![]);
        let tmux = Tmux::new(NativeRunner(runner));
        assert!(
            tmux.discover(
                Some("/tmp/a"),
                None,
                Instant::now() + std::time::Duration::from_secs(2)
            )
            .is_err()
        );
    }
    #[test]
    fn a_missing_scoped_pane_supplies_no_admission_while_empty_inventory_is_unknown() {
        let runner = ScriptedRunner::default();
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(NativeRunner(runner));
        let scoped = tmux
            .discover(
                Some("/tmp/a"),
                Some("%99"),
                Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap();
        assert!(scoped.candidates.is_empty());
        assert!(!scoped.truncated);
        let runner = ScriptedRunner::default();
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(NativeRunner(runner));
        assert!(
            tmux.discover(
                Some("/tmp/a"),
                None,
                Instant::now() + std::time::Duration::from_secs(2)
            )
            .is_err()
        );
    }
}
