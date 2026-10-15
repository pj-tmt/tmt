//! The driver over a scripted runner, through `serve` so every answer is the
//! JSON core reads. Herdr's JSON shapes are the ones a disposable Herdr 0.9.1
//! server printed (the built-in host's fixtures, #498).

use super::*;
use crate::run::{Output, RunError};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::VecDeque, time::Instant};

const SOCKET: &str = "/tmp/hdr/herdr.sock";
const BINDING: &str = "123e4567-e89b-42d3-a456-426614174002";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    program: String,
    env: Vec<(String, String)>,
    args: Vec<String>,
}

#[derive(Default)]
struct Scripted {
    answers: RefCell<VecDeque<Result<Output, RunError>>>,
    calls: RefCell<Vec<Call>>,
}

impl Runner for &Scripted {
    fn run(
        &self,
        program: &str,
        env: &[(&str, &str)],
        args: &[String],
        _: Instant,
    ) -> Result<Output, RunError> {
        self.calls.borrow_mut().push(Call {
            program: program.into(),
            env: env
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
            args: args.to_vec(),
        });
        self.answers
            .borrow_mut()
            .pop_front()
            .expect("an unscripted call")
    }
}

impl Scripted {
    fn answer(&self, code: u32, stdout: &str, stderr: &str) -> &Self {
        self.answers.borrow_mut().push_back(Ok(Output {
            code: Some(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }));
        self
    }

    fn json(&self, value: Value) -> &Self {
        self.answer(0, &value.to_string(), "")
    }

    fn text(&self, text: &str) -> &Self {
        self.answer(0, text, "")
    }

    fn refuse(&self, code: &str) -> &Self {
        let error = json!({"error": {"code": code, "message": "refused"}, "id": "cli"});
        self.answer(1, "", &error.to_string())
    }

    fn fail(&self, error: RunError) -> &Self {
        self.answers.borrow_mut().push_back(Err(error));
        self
    }

    /// Each call as `program args…`.
    fn commands(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .map(|call| format!("{} {}", call.program, call.args.join(" ")))
            .collect()
    }

    fn done(&self) {
        assert!(self.answers.borrow().is_empty(), "unused scripted answers");
    }
}

fn serve(runner: &Scripted, op: &str, request: Value) -> Value {
    let args: Vec<String> = ["__tmt-driver", "1", op].map(String::from).into();
    let mut request = request;
    request["deadlineMs"] = json!(1000);
    let mut output = Vec::new();
    let status = tmt_driver_protocol::serve(
        &args,
        request.to_string().as_bytes(),
        &mut output,
        &mut HerdrDriver::new(runner),
    );
    assert_eq!(status, 0);
    let mut answer: Value = serde_json::from_slice(&output).unwrap();
    // An answer is `{"ok": …}`; an error stays `{"error": …}`.
    match answer.get_mut("ok") {
        Some(ok) => ok.take(),
        None => answer,
    }
}

fn error_code(answer: &Value) -> &str {
    answer["error"]["code"].as_str().unwrap_or("none")
}

fn status(version: &str) -> Value {
    json!({"status": "running", "running": true, "version": version, "protocol": 22,
           "socket": SOCKET, "session": null})
}

fn pane(target: &str, terminal: &str, tokens: Option<Value>) -> Value {
    let mut pane = json!({"agent_status": "unknown", "cwd": "/repo", "focused": true,
        "foreground_cwd": "/repo/src", "pane_id": target, "revision": 0,
        "tab_id": "w1:t1", "terminal_id": terminal, "workspace_id": "w1"});
    if let Some(tokens) = tokens {
        pane["tokens"] = tokens;
    }
    pane
}

fn list(panes: Vec<Value>) -> Value {
    json!({"id": "cli:pane:list", "result": {"panes": panes, "type": "pane_list"}})
}

/// One `agent list` row: the terminal that hosts the agent and, when it has
/// one, its agent name. Herdr leaves an unset name out.
fn agent(terminal: &str, name: Option<&str>) -> Value {
    let mut agent = json!({"agent": "claude", "agent_status": "idle", "launch_pending": false,
        "pane_id": "w1:p1", "terminal_id": terminal});
    if let Some(name) = name {
        agent["name"] = json!(name);
    }
    agent
}

fn agents(agents: Vec<Value>) -> Value {
    json!({"id": "cli:agent:list", "result": {"agents": agents, "type": "agent_list"}})
}

fn got(pane: Value) -> Value {
    json!({"id": "cli:pane:get", "result": {"pane": pane, "type": "pane_info"}})
}

fn process(target: &str, shell: u64, command: &str) -> Value {
    json!({"id": "cli:pane:process_info", "result": {"process_info": {
        "foreground_process_group_id": shell,
        "foreground_processes": [{"argv": [command], "argv0": command, "cwd": "/repo",
            "name": command, "pid": shell}],
        "pane_id": target, "shell_pid": shell}, "type": "pane_process_info"}})
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/builtin-marker.json")).unwrap()
}

/// The fixture's tokens as Herdr lists them on a pane.
fn fixture_tokens() -> Value {
    let mut tokens = serde_json::Map::new();
    for token in fixture()["tokens"].as_array().unwrap() {
        let (key, value) = token.as_str().unwrap().split_once('=').unwrap();
        tokens.insert(key.into(), json!(value));
    }
    Value::Object(tokens)
}

/// Tokens the built-in Herdr host wrote decode to the same marker, and this
/// driver writes them byte for byte (the built-in asserts the same fixture).
#[test]
fn the_built_in_hosts_marker_tokens_decode_and_encode_byte_for_byte() {
    let fixture = fixture();
    let marker: tmt_driver_protocol::Marker =
        serde_json::from_value(fixture["marker"].clone()).unwrap();
    assert_eq!(
        marker::decode(fixture_tokens().as_object()),
        Some(marker.clone())
    );
    let expected: Vec<String> = serde_json::from_value(fixture["tokens"].clone()).unwrap();
    assert_eq!(marker::encode(&marker), Some(expected));
    let mut long = marker.clone();
    long.name = "n".repeat(321);
    assert_eq!(marker::encode(&long), None);
    let mut bad_pid = fixture_tokens();
    bad_pid["tmt_pid"] = json!("0");
    assert_eq!(marker::decode(bad_pid.as_object()), None);
}

#[test]
fn the_declared_capabilities_decode_with_the_read_side_ops_only() {
    let runner = Scripted::default();
    let answer = serve(&runner, "capabilities", json!({}));
    let bytes = json!({"ok": answer}).to_string().into_bytes();
    let (capabilities, grammar) = tmt_driver_protocol::decode_capabilities(&bytes).unwrap();
    assert_eq!(
        capabilities.ops,
        [
            "caller",
            "server",
            "resolve-target",
            "snapshot",
            "publish",
            "clear",
            "capture",
            "input",
            "prompt",
            "name-agent"
        ]
    );
    assert!(grammar.is_pane_id("term_65ca1161edc141"));
    assert!(grammar.is_target("w12:p3"));
    // Focus has no Herdr command by pane ID; core leads every probe.
    for op in ["focus", "probe"] {
        assert_eq!(error_code(&serve(&runner, op, json!({}))), "unsupported");
    }
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn the_server_is_the_parent_of_a_pane_shell_on_the_named_socket() {
    let runner = Scripted::default();
    runner
        .json(status("0.9.1"))
        .json(list(vec![
            pane("w1:p1", "term_a1", None),
            pane("w1:p2", "term_a2", None),
        ]))
        .refuse("pane_not_found")
        .json(process("w1:p2", 90593, "zsh"))
        .text("  4100\n")
        .text("Sun Sep 27 10:00:00 2026\n");
    let answer = serve(&runner, "server", json!({"socket": SOCKET}));
    assert_eq!(
        answer["server"],
        json!({"socket": SOCKET, "pid": 4100, "startTime": "Sun Sep 27 10:00:00 2026"})
    );
    assert_eq!(
        runner.commands(),
        [
            "herdr status server --json",
            "herdr pane list",
            "herdr pane process-info --pane w1:p1",
            "herdr pane process-info --pane w1:p2",
            "ps -o ppid= -p 90593",
            "ps -o lstart= -p 4100",
        ]
    );
    let calls = runner.calls.borrow();
    assert!(
        calls[..4]
            .iter()
            .all(|call| call.env == [("HERDR_SOCKET_PATH".to_owned(), SOCKET.to_owned())])
    );
    assert!(
        calls[4..]
            .iter()
            .all(|call| call.env == [("LC_ALL".to_owned(), "C".to_owned())])
    );
    runner.done();
}

#[test]
fn no_server_or_no_pane_is_no_server_and_an_old_one_is_unavailable() {
    let runner = Scripted::default();
    runner.json(json!({"running": false, "socket": SOCKET}));
    assert_eq!(
        serve(&runner, "server", json!({"socket": null}))["server"],
        json!(null)
    );
    // Herdr's own default: no socket is named.
    assert!(runner.calls.borrow()[0].env.is_empty());

    let runner = Scripted::default();
    runner.json(status("0.9.1")).json(list(vec![]));
    assert_eq!(
        serve(&runner, "server", json!({"socket": SOCKET}))["server"],
        json!(null)
    );
    runner.done();

    let runner = Scripted::default();
    runner.json(status("0.9.1-rc.1"));
    assert_eq!(
        error_code(&serve(&runner, "server", json!({"socket": SOCKET}))),
        "unavailable"
    );

    let runner = Scripted::default();
    assert_eq!(
        error_code(&serve(
            &runner,
            "server",
            json!({"socket": "relative.sock"})
        )),
        "bad_request"
    );
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn herdr_failures_map_to_protocol_codes() {
    for (script, code) in [
        (Err("server_not_running"), "unavailable"),
        (Err("permission_denied"), "failed"),
        (Ok(RunError::NotStarted), "unavailable"),
        (Ok(RunError::Unfinished), "failed"),
    ] {
        let runner = Scripted::default();
        match script {
            Err(herdr) => runner.refuse(herdr),
            Ok(error) => runner.fail(error),
        };
        let answer = serve(
            &runner,
            "snapshot",
            json!({"socket": SOCKET, "panes": null}),
        );
        assert_eq!(error_code(&answer), code, "{script:?}");
    }
    for (stdout, stderr, exit) in [("{", "", 0), ("", "not json", 1), ("", "", 2)] {
        let runner = Scripted::default();
        runner.answer(exit, stdout, stderr);
        let answer = serve(
            &runner,
            "snapshot",
            json!({"socket": SOCKET, "panes": null}),
        );
        assert_eq!(error_code(&answer), "failed");
    }
}

#[test]
fn a_snapshot_reports_the_scoped_panes_their_shells_and_markers() {
    let runner = Scripted::default();
    runner
        .json(list(vec![
            pane("w1:p1", "term_a1", Some(fixture_tokens())),
            pane("w1:p2", "term_a2", None),
            pane("w1:p3", "term_a3", None),
        ]))
        .json(process("w1:p1", 90593, "claude"))
        // `w1:p3` closed after the list.
        .refuse("pane_not_found")
        .json(agents(vec![agent("term_a1", Some("reviewer"))]));
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": ["term_a1", "term_a3", "term_gone"]}),
    );
    assert_eq!(
        answer["panes"],
        json!([{"id": "term_a1", "target": "w1:p1", "cwd": "/repo/src",
                "command": "claude", "panePid": 90593, "suggestedName": null,
                "agentName": "reviewer", "marker": fixture()["marker"]}])
    );
    runner.done();
}

/// The agent name is Herdr's `agent list` name of the terminal, and nothing
/// else: an agent with no name is `null`, and a pane with no agent, a name
/// outside Herdr's rule and a terminal listed twice leave the member out
/// (unknown). Agent names are read last and are no evidence: whatever stops
/// Herdr from listing its agents, the pane evidence is the answer.
#[test]
fn a_snapshot_reads_each_agent_name_last_and_never_fails_for_it() {
    let panes = || {
        list(vec![
            pane("w1:p1", "term_a1", None),
            pane("w1:p2", "term_a2", None),
            pane("w1:p3", "term_a3", None),
            pane("w1:p4", "term_a4", None),
            pane("w1:p5", "term_a5", None),
        ])
    };
    let evidence = |runner: &Scripted| {
        runner.json(panes());
        for (target, shell) in [
            ("w1:p1", 11),
            ("w1:p2", 12),
            ("w1:p3", 13),
            ("w1:p4", 14),
            ("w1:p5", 15),
        ] {
            runner.json(process(target, shell, "claude"));
        }
    };
    let snapshot = |runner: &Scripted| {
        let answer = serve(runner, "snapshot", json!({"socket": SOCKET, "panes": null}));
        let panes = answer["panes"].as_array().expect("a snapshot").clone();
        let pids: Vec<u64> = panes
            .iter()
            .map(|pane| pane["panePid"].as_u64().unwrap())
            .collect();
        assert_eq!(pids, [11, 12, 13, 14, 15], "the pane evidence is intact");
        panes
            .into_iter()
            .map(|pane| pane.get("agentName").cloned())
            .collect::<Vec<Option<Value>>>()
    };

    let runner = Scripted::default();
    evidence(&runner);
    runner.json(agents(vec![
        agent("term_a1", Some("reviewer")),
        agent("term_a2", None),
        agent("term_a4", Some("Not A Herdr Name")),
        agent("term_a5", Some("first")),
        agent("term_a5", Some("second")),
    ]));
    assert_eq!(
        snapshot(&runner),
        [Some(json!("reviewer")), Some(json!(null)), None, None, None]
    );
    assert_eq!(runner.commands().last().unwrap(), "herdr agent list");
    assert_eq!(runner.commands().len(), 7);
    runner.done();

    for failure in [
        |runner: &Scripted| {
            runner.refuse("unknown_method");
        },
        |runner: &Scripted| {
            runner.json(json!({"id": "cli:agent:list", "result": {"type": "agent_list"}}));
        },
        |runner: &Scripted| {
            runner.json(agents(vec![json!({"name": "no-terminal"})]));
        },
        |runner: &Scripted| {
            runner.fail(RunError::Unfinished);
        },
        |runner: &Scripted| {
            runner.fail(RunError::NotStarted);
        },
    ] {
        let runner = Scripted::default();
        evidence(&runner);
        failure(&runner);
        assert_eq!(snapshot(&runner), vec![None; 5]);
        runner.done();
    }

    // A scope that selects no pane asks Herdr for no agents.
    let runner = Scripted::default();
    runner.json(panes());
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": ["term_gone"]}),
    );
    assert_eq!(answer["panes"], json!([]));
    runner.done();
}

/// Herdr numbers workspaces, tabs and panes with `1`-`9`, uppercase letters
/// and then `0`: the tenth pane is `pA`, the 32nd `p0` and the 33rd `p11`.
/// Every listed target must validate, including panes outside the scope.
#[test]
fn panes_numbered_with_letters_are_listed_resolved_and_the_caller() {
    let runner = Scripted::default();
    runner
        .json(list(vec![
            pane("w1:p1E", "term_a1", None),
            pane("wA:pG", "term_a2", None),
            pane("w3:p0", "term_a3", None),
            pane("w3:p11", "term_a4", None),
        ]))
        .json(process("w1:p1E", 90593, "claude"))
        .json(process("wA:pG", 90594, "zsh"))
        .json(process("w3:p0", 90595, "zsh"))
        .json(process("w3:p11", 90596, "zsh"))
        .json(agents(vec![]));
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": null}),
    );
    let targets: Vec<&str> = answer["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pane| pane["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        ["w1:p1E", "wA:pG", "w3:p0", "w3:p11"],
        "{answer:#}"
    );
    runner.done();

    let runner = Scripted::default();
    runner.json(got(pane("w1:p2C", "term_a2", None)));
    let request = json!({"socket": SOCKET, "target": "w1:p2C"});
    assert_eq!(
        serve(&runner, "resolve-target", request)["paneId"],
        "term_a2"
    );
    assert_eq!(runner.commands(), ["herdr pane get w1:p2C"]);
    // Herdr's letters are uppercase; lowercase is not its target.
    let answer = serve(
        &runner,
        "resolve-target",
        json!({"socket": SOCKET, "target": "w1:p2c"}),
    );
    assert_eq!(error_code(&answer), "bad_request");
    runner.done();

    let runner = Scripted::default();
    runner
        .json(status("0.9.3"))
        .json(got(pane("w1:pA", "term_a2", None)))
        .json(process("w1:pA", 90593, "zsh"));
    let env = json!({"HERDR_PANE_ID": "w1:pA", "HERDR_SOCKET_PATH": SOCKET});
    assert_eq!(
        serve(&runner, "caller", json!({"env": env}))["pane"],
        json!({"id": "term_a2", "socket": SOCKET, "shellPid": 90593})
    );
    runner.done();
}

#[test]
fn an_invalid_or_repeated_pane_list_is_not_evidence() {
    for panes in [
        vec![
            pane("w1:p1", "term_a1", None),
            pane("w1:p1", "term_a2", None),
        ],
        vec![pane("w1:p1", "term_A1", None)],
        vec![pane("pane-1", "term_a1", None)],
        vec![
            pane("w1:p1", "term_a1", None),
            pane("w1:p2e", "term_a2", None),
        ],
    ] {
        let runner = Scripted::default();
        runner.json(list(panes));
        let answer = serve(
            &runner,
            "snapshot",
            json!({"socket": SOCKET, "panes": null}),
        );
        assert_eq!(error_code(&answer), "failed");
    }
    let runner = Scripted::default();
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": ["%1"]}),
    );
    assert_eq!(error_code(&answer), "bad_request");
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn a_target_resolves_to_its_terminal_or_to_nothing() {
    let runner = Scripted::default();
    runner
        .json(got(pane("w1:p2", "term_a2", None)))
        .refuse("pane_not_found");
    let request = json!({"socket": SOCKET, "target": "w1:p2"});
    assert_eq!(
        serve(&runner, "resolve-target", request.clone())["paneId"],
        "term_a2"
    );
    assert_eq!(
        serve(&runner, "resolve-target", request)["paneId"],
        json!(null)
    );
    assert_eq!(runner.commands(), ["herdr pane get w1:p2"; 2]);
    let answer = serve(
        &runner,
        "resolve-target",
        json!({"socket": SOCKET, "target": "%1"}),
    );
    assert_eq!(error_code(&answer), "bad_request");
    runner.done();
}

#[test]
fn the_caller_is_the_pane_its_environment_names_on_a_running_server() {
    let runner = Scripted::default();
    runner
        .json(status("0.10.0"))
        .json(got(pane("w1:p2", "term_a2", None)))
        .json(process("w1:p2", 90593, "zsh"));
    let env = json!({"HERDR_PANE_ID": "w1:p2", "HERDR_SOCKET_PATH": SOCKET});
    assert_eq!(
        serve(&runner, "caller", json!({"env": env}))["pane"],
        json!({"id": "term_a2", "socket": SOCKET, "shellPid": 90593})
    );
    runner.done();
    // Nothing to ask without a pane and an absolute socket, or a server.
    for env in [
        json!({"HERDR_PANE_ID": "w1:p2"}),
        json!({"HERDR_PANE_ID": "term_a2", "HERDR_SOCKET_PATH": SOCKET}),
        json!({"HERDR_PANE_ID": "w1:p2", "HERDR_SOCKET_PATH": "herdr.sock"}),
    ] {
        let runner = Scripted::default();
        assert_eq!(
            serve(&runner, "caller", json!({"env": env}))["pane"],
            json!(null)
        );
        assert!(runner.calls.borrow().is_empty());
    }
    let runner = Scripted::default();
    runner.json(json!({"running": false, "socket": SOCKET}));
    let env = json!({"HERDR_PANE_ID": "w1:p2", "HERDR_SOCKET_PATH": SOCKET});
    assert_eq!(
        serve(&runner, "caller", json!({"env": env}))["pane"],
        json!(null)
    );
}

fn publish_request(pane_pid: u64) -> Value {
    json!({"socket": SOCKET, "paneId": "term_a1", "panePid": pane_pid,
           "marker": fixture()["marker"]})
}

#[test]
fn publishing_reports_the_tokens_under_tmt_on_the_same_shell_and_reads_them_back() {
    let runner = Scripted::default();
    let panes = || list(vec![pane("w1:p1", "term_a1", None)]);
    runner
        .json(panes())
        .json(process("w1:p1", 90593, "claude"))
        .text("")
        .json(list(vec![pane("w1:p1", "term_a1", Some(fixture_tokens()))]));
    assert_eq!(serve(&runner, "publish", publish_request(90593)), json!({}));
    let report = runner.calls.borrow()[2].args.clone();
    let mut expected: Vec<String> = ["pane", "report-metadata", "w1:p1", "--source", "tmt"]
        .map(String::from)
        .into();
    for token in fixture()["tokens"].as_array().unwrap() {
        expected.extend(["--token".into(), token.as_str().unwrap().into()]);
    }
    for key in ["tmt_name_4", "tmt_cname_4"] {
        expected.extend(["--clear-token".into(), key.into()]);
    }
    assert_eq!(report, expected);
    runner.done();

    // Herdr silently dropped the report.
    let runner = Scripted::default();
    runner
        .json(panes())
        .json(process("w1:p1", 90593, "claude"))
        .text("")
        .json(panes());
    assert_eq!(
        error_code(&serve(&runner, "publish", publish_request(90593))),
        "failed"
    );
}

#[test]
fn publishing_refuses_a_pane_that_no_longer_runs_that_shell() {
    let runner = Scripted::default();
    runner
        .json(list(vec![pane("w1:p1", "term_a1", None)]))
        .json(process("w1:p1", 777, "zsh"))
        .json(list(vec![pane("w1:p1", "term_a1", None)]))
        .refuse("pane_not_found")
        .json(list(vec![]));
    for _ in 0..3 {
        assert_eq!(
            error_code(&serve(&runner, "publish", publish_request(90593))),
            "not_found"
        );
    }
    assert!(
        !runner
            .commands()
            .iter()
            .any(|call| call.contains("report-metadata"))
    );
    runner.done();
}

#[test]
fn clearing_removes_only_this_bindings_marker() {
    let request = json!({"socket": SOCKET, "paneId": "term_a1", "bindingId": BINDING});
    let runner = Scripted::default();
    let mut other = fixture_tokens();
    other["tmt_binding"] = json!("123e4567-e89b-42d3-a456-426614174099");
    runner
        .json(list(vec![pane("w1:p1", "term_a1", Some(other))]))
        .json(list(vec![]))
        .json(list(vec![pane("w1:p1", "term_a1", Some(fixture_tokens()))]))
        .text("");
    assert_eq!(serve(&runner, "clear", request.clone())["cleared"], false);
    assert_eq!(serve(&runner, "clear", request.clone())["cleared"], false);
    assert_eq!(serve(&runner, "clear", request)["cleared"], true);
    let mut expected: Vec<String> = ["pane", "report-metadata", "w1:p1", "--source", "tmt"]
        .map(String::from)
        .into();
    for key in [
        "tmt_identity",
        "tmt_binding",
        "tmt_server",
        "tmt_pid",
        "tmt_name",
        "tmt_name_2",
        "tmt_name_3",
        "tmt_cname",
        "tmt_cname_2",
        "tmt_cname_3",
    ] {
        expected.extend(["--clear-token".into(), key.into()]);
    }
    assert_eq!(runner.calls.borrow()[3].args, expected);
    assert_eq!(runner.calls.borrow().len(), 4);
    runner.done();
}

#[test]
fn a_capture_reads_the_recent_lines_of_the_pane_by_its_current_target() {
    let runner = Scripted::default();
    runner
        .json(list(vec![pane("w2:p1", "term_a1", None)]))
        .text("one\ntwo\n")
        .json(list(vec![]));
    let request = json!({"socket": SOCKET, "paneId": "term_a1", "lines": 40});
    assert_eq!(
        serve(&runner, "capture", request.clone()),
        json!({"text": "one\ntwo\n"})
    );
    assert_eq!(
        runner.commands()[1],
        "herdr pane read w2:p1 --source recent --lines 40 --format text"
    );
    assert_eq!(error_code(&serve(&runner, "capture", request)), "not_found");
    let zero = json!({"socket": SOCKET, "paneId": "term_a1", "lines": 0});
    assert_eq!(error_code(&serve(&runner, "capture", zero)), "bad_request");
    runner.done();
}

fn input(text: &str, enter: bool) -> Value {
    json!({"socket": SOCKET, "paneId": "term_a1", "text": text, "enter": enter})
}

#[test]
fn input_types_one_line_literally_and_enter_only_when_asked() {
    let runner = Scripted::default();
    let panes = || list(vec![pane("w1:p1", "term_a1", None)]);
    runner.json(panes()).text("").json(panes()).text("");
    // Core stages input: the text, then Enter alone.
    assert_eq!(
        serve(&runner, "input", input("--help ！ é", false)),
        json!({})
    );
    assert_eq!(serve(&runner, "input", input("", true)), json!({}));
    assert_eq!(
        runner.calls.borrow()[1].args,
        ["pane", "send-text", "w1:p1", "--help ！ é"]
    );
    assert_eq!(runner.commands()[3], "herdr pane send-keys w1:p1 Enter");
    runner.done();
}

#[test]
fn input_that_would_submit_early_or_reach_no_pane_is_not_sent() {
    // Herdr would submit at a line break, so nothing is asked of it.
    for text in ["one\ntwo", "one\r", "\n"] {
        let runner = Scripted::default();
        assert_eq!(
            error_code(&serve(&runner, "input", input(text, false))),
            "bad_request"
        );
        assert!(runner.calls.borrow().is_empty());
    }
    let runner = Scripted::default();
    runner
        .json(list(vec![]))
        .json(list(vec![pane("w1:p1", "term_a1", None)]))
        .refuse("pane_not_found");
    for _ in 0..2 {
        assert_eq!(
            error_code(&serve(&runner, "input", input("hi", false))),
            "not_found"
        );
    }
    runner.done();
    // Once text was typed, a pane gone before Enter is uncertain, not unsent.
    let runner = Scripted::default();
    runner
        .json(list(vec![pane("w1:p1", "term_a1", None)]))
        .text("")
        .refuse("pane_not_found");
    assert_eq!(
        error_code(&serve(&runner, "input", input("hi", true))),
        "failed"
    );
    runner.done();
}

#[test]
fn a_prompt_goes_to_the_agent_herdr_recognizes_and_maps_its_refusals() {
    let request = json!({"socket": SOCKET, "paneId": "term_a1", "text": "hi\nthere"});
    let runner = Scripted::default();
    runner
        .json(list(vec![pane("w1:p1", "term_a1", None)]))
        .text("");
    assert_eq!(serve(&runner, "prompt", request.clone()), json!({}));
    // The whole message in one call; Herdr submits it.
    assert_eq!(
        runner.calls.borrow()[1].args,
        ["agent", "prompt", "w1:p1", "hi\nthere"]
    );
    runner.done();
    for (herdr, code) in [
        ("agent_not_found", "no_agent"),
        ("agent_blocked", "blocked"),
        ("agent_not_ready", "not_ready"),
        ("pane_not_found", "not_found"),
        ("server_not_running", "unavailable"),
        ("permission_denied", "failed"),
    ] {
        let runner = Scripted::default();
        runner
            .json(list(vec![pane("w1:p1", "term_a1", None)]))
            .refuse(herdr);
        assert_eq!(
            error_code(&serve(&runner, "prompt", request.clone())),
            code,
            "{herdr}"
        );
        runner.done();
    }
    let runner = Scripted::default();
    runner.json(list(vec![]));
    assert_eq!(error_code(&serve(&runner, "prompt", request)), "not_found");
    // Prompt-only codes never leak into another operation's answer.
    let runner = Scripted::default();
    runner.refuse("agent_not_found");
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": null}),
    );
    assert_eq!(error_code(&answer), "failed");
}

/// Herdr 0.9.1 reads the argument after the target as the text, whatever it
/// looks like, and has no `--` separator (`agent prompt` refuses one, and
/// `send-text` would type it). So the text is passed as the one last argv
/// entry, as is; the real-Herdr test shows option-like text arrives intact.
#[test]
fn option_like_text_is_the_last_argument_with_no_separator() {
    for text in ["-h", "--version", "--wait", "--", "- item"] {
        let runner = Scripted::default();
        let panes = || list(vec![pane("w1:p1", "term_a1", None)]);
        runner.json(panes()).text("").json(panes()).text("");
        assert_eq!(serve(&runner, "input", input(text, false)), json!({}));
        let request = json!({"socket": SOCKET, "paneId": "term_a1", "text": text});
        assert_eq!(serve(&runner, "prompt", request), json!({}));
        let calls = runner.calls.borrow();
        assert_eq!(calls[1].args, ["pane", "send-text", "w1:p1", text]);
        assert_eq!(calls[3].args, ["agent", "prompt", "w1:p1", text]);
        drop(calls);
        runner.done();
    }
}

/// A `name-agent` request: a set with `name`, or a clear of `expected`.
fn name_agent(name: Option<&str>, expected: Option<&str>) -> Value {
    let mut request = json!({"socket": SOCKET, "paneId": "term_a1", "panePid": 90593,
                             "bindingId": BINDING, "name": name});
    if let Some(expected) = expected {
        request["expected"] = json!(expected);
    }
    request
}

fn set(name: &str) -> Value {
    name_agent(Some(name), None)
}

fn take_away(expected: &str) -> Value {
    name_agent(None, Some(expected))
}

fn listed(tokens: Value) -> Value {
    list(vec![pane("w1:p1", "term_a1", Some(tokens))])
}

/// Scripts the reads every `name-agent` call makes before it decides: the
/// agents, then the pane with its marker and its shell (the fence, last).
/// `agent_name` is the name of the agent in the pane.
fn observed<'r>(runner: &'r Scripted, agent_name: Option<&str>) -> &'r Scripted {
    runner
        .json(agents(vec![
            agent("term_other", Some("reviewer")),
            agent("term_a1", agent_name),
        ]))
        .json(listed(fixture_tokens()))
        .json(process("w1:p1", 90593, "claude"))
}

const READS: [&str; 3] = [
    "herdr agent list",
    "herdr pane list",
    "herdr pane process-info --pane w1:p1",
];

/// The calls after the three reads.
fn writes(runner: &Scripted) -> Vec<String> {
    runner.commands()[3..].to_vec()
}

/// A set renames the agent over whatever agent name it has, someone else's
/// too: the explicit command is the intent. It never touches the pane label
/// and writes no token.
#[test]
fn a_set_renames_the_agent_over_any_name_and_never_its_pane_label() {
    for current in [None, Some("users-own"), Some("bea")] {
        let runner = Scripted::default();
        observed(&runner, current).text("");
        assert_eq!(serve(&runner, "name-agent", set("ann")), json!({}));
        let mut expected: Vec<String> = READS.map(String::from).into();
        expected.push("herdr agent rename w1:p1 ann".to_owned());
        assert_eq!(runner.commands(), expected, "{current:?}");
        assert!(
            !runner
                .commands()
                .iter()
                .any(|call| call.contains("pane rename") || call.contains("report-metadata")),
            "{current:?}"
        );
        runner.done();
    }
    // The agent has the name already: nothing to rename.
    let runner = Scripted::default();
    observed(&runner, Some("ann"));
    assert_eq!(serve(&runner, "name-agent", set("ann")), json!({}));
    assert_eq!(runner.commands(), READS);
    runner.done();
}

/// Herdr renames in one step: a refused new name leaves the old one, and no
/// `--clear` comes first. The next read still lists the old name, and the
/// same set as before changes nothing.
#[test]
fn a_refused_new_name_keeps_the_old_name_and_never_clears_first() {
    let runner = Scripted::default();
    observed(&runner, Some("ann")).refuse("agent_name_taken");
    let answer = serve(&runner, "name-agent", set("bea"));
    assert_eq!(error_code(&answer), "refused");
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap()
            .contains("refused"),
        "Herdr's message: {answer}"
    );
    assert_eq!(writes(&runner), ["herdr agent rename w1:p1 bea"]);
    runner.done();
    let runner = Scripted::default();
    observed(&runner, Some("ann"));
    assert_eq!(serve(&runner, "name-agent", set("ann")), json!({}));
    assert_eq!(runner.commands(), READS);
    runner.done();
}

/// `name: null` takes the agent name away only when it equals `expected`,
/// the identity's name, even when someone else gave that equal name: the
/// driver compares names, it does not know who set one. A different name,
/// an agent without a name, or no agent at all changes nothing.
#[test]
fn taking_the_name_away_clears_only_the_expected_name() {
    let runner = Scripted::default();
    observed(&runner, Some("ann")).text("");
    assert_eq!(serve(&runner, "name-agent", take_away("ann")), json!({}));
    assert_eq!(writes(&runner), ["herdr agent rename w1:p1 --clear"]);
    runner.done();
    for (case, current) in [("another name", Some("bea")), ("no name", None)] {
        let runner = Scripted::default();
        observed(&runner, current);
        assert_eq!(
            serve(&runner, "name-agent", take_away("ann")),
            json!({}),
            "{case}"
        );
        assert_eq!(runner.commands(), READS, "{case}");
        runner.done();
    }
    let runner = Scripted::default();
    runner
        .json(agents(vec![agent("term_other", Some("ann"))]))
        .json(listed(fixture_tokens()))
        .json(process("w1:p1", 90593, "zsh"));
    assert_eq!(serve(&runner, "name-agent", take_away("ann")), json!({}));
    assert_eq!(runner.commands(), READS);
    runner.done();
}

/// `expected` goes with `name: null` and only there: without it a request
/// could take away a name someone else gave, so it is refused before any
/// call.
#[test]
fn expected_is_required_to_take_a_name_away_and_refused_with_a_name() {
    let mut empty = take_away("ann");
    empty["expected"] = json!("");
    let mut null = take_away("ann");
    null["expected"] = Value::Null;
    for (case, request) in [
        ("no expected", name_agent(None, None)),
        ("empty expected", empty),
        ("null expected", null),
        ("a name with expected", name_agent(Some("ann"), Some("ann"))),
    ] {
        let runner = Scripted::default();
        assert_eq!(
            error_code(&serve(&runner, "name-agent", request)),
            "bad_request",
            "{case}"
        );
        assert!(runner.calls.borrow().is_empty(), "{case}");
    }
}

/// A change needs exact evidence. A list that does not say what the
/// terminal hosts changes nothing, where a snapshot would only show no name.
#[test]
fn naming_refuses_an_agent_list_it_cannot_read_exactly() {
    let unreadable = |name: Value| {
        let mut row = agent("term_a1", None);
        row["name"] = name;
        vec![row]
    };
    for (case, rows) in [
        (
            "two rows",
            vec![agent("term_a1", None), agent("term_a1", Some("ann"))],
        ),
        ("a number", unreadable(json!(7))),
        ("an object", unreadable(json!({"name": "ann"}))),
        ("outside Herdr's rule", unreadable(json!("Users Own"))),
        ("empty", unreadable(json!(""))),
        (
            "a row without a terminal",
            vec![agent("term_a1", None), json!({"name": "ann"})],
        ),
    ] {
        for request in [set("ann"), take_away("ann")] {
            let runner = Scripted::default();
            runner.json(agents(rows.clone()));
            assert_eq!(
                error_code(&serve(&runner, "name-agent", request.clone())),
                "failed",
                "{case} {request}"
            );
            assert_eq!(runner.commands(), ["herdr agent list"], "{case}");
            runner.done();
        }
    }
    // An explicit null is no name, as an absent member is.
    let runner = Scripted::default();
    runner
        .json(agents(unreadable(json!(null))))
        .json(listed(fixture_tokens()))
        .json(process("w1:p1", 90593, "claude"))
        .text("");
    assert_eq!(serve(&runner, "name-agent", set("ann")), json!({}));
    assert_eq!(writes(&runner), ["herdr agent rename w1:p1 ann"]);
    runner.done();
}

#[test]
fn a_name_herdr_would_not_take_is_refused_before_any_call() {
    for name in [
        "",
        "Ann",
        "9lives",
        "has space",
        "--clear",
        "-x",
        "ann.b",
        "\u{e9}cole",
        &"a".repeat(33),
    ] {
        let runner = Scripted::default();
        assert_eq!(
            error_code(&serve(&runner, "name-agent", set(name))),
            "refused",
            "{name:?}"
        );
        assert!(runner.calls.borrow().is_empty(), "{name:?}");
    }
    let runner = Scripted::default();
    let longest = format!("a{}", "b-_9".repeat(7) + "bcd");
    assert_eq!(longest.len(), 32);
    observed(&runner, None).text("");
    assert_eq!(serve(&runner, "name-agent", set(&longest)), json!({}));
    runner.done();
}

#[test]
fn herdrs_rename_refusals_map_to_the_agent_and_name_codes() {
    for (herdr, code) in [
        ("agent_name_taken", "refused"),
        ("invalid_agent_name", "refused"),
        ("agent_not_found", "no_agent"),
        ("agent_launch_pending", "not_ready"),
        ("pane_not_found", "not_found"),
        ("permission_denied", "failed"),
    ] {
        let runner = Scripted::default();
        observed(&runner, None).refuse(herdr);
        assert_eq!(
            error_code(&serve(&runner, "name-agent", set("ann"))),
            code,
            "{herdr}"
        );
        assert_eq!(writes(&runner), ["herdr agent rename w1:p1 ann"], "{herdr}");
        runner.done();
    }
    // A pane Herdr sees no agent in has nothing to name.
    let runner = Scripted::default();
    runner
        .json(agents(vec![agent("term_other", Some("reviewer"))]))
        .json(listed(fixture_tokens()))
        .json(process("w1:p1", 90593, "zsh"));
    assert_eq!(
        error_code(&serve(&runner, "name-agent", set("ann"))),
        "no_agent"
    );
    assert_eq!(writes(&runner), [] as [String; 0]);
    runner.done();
}

/// The fence is the last read before the rename: a pane that is gone,
/// unmarked, bound again or running another shell is never renamed.
#[test]
fn naming_refuses_a_pane_that_is_gone_rebound_or_runs_another_shell() {
    let no_agents = || agents(vec![agent("term_a1", Some("ann"))]);
    for (case, request, script) in [
        (
            "gone",
            set("ann"),
            (|runner: &Scripted| {
                runner.json(list(vec![]));
            }) as fn(&Scripted),
        ),
        ("unmarked", set("ann"), |runner| {
            runner.json(list(vec![pane("w1:p1", "term_a1", None)]));
        }),
        ("rebound", set("ann"), |runner| {
            let mut other = fixture_tokens();
            other["tmt_binding"] = json!("123e4567-e89b-42d3-a456-426614174099");
            runner.json(listed(other));
        }),
        ("another shell", set("ann"), |runner| {
            runner
                .json(listed(fixture_tokens()))
                .json(process("w1:p1", 777, "zsh"));
        }),
        ("closed", set("ann"), |runner| {
            runner
                .json(listed(fixture_tokens()))
                .refuse("pane_not_found");
        }),
        ("gone, taking away", take_away("ann"), |runner| {
            runner.json(list(vec![]));
        }),
        ("rebound, taking away", take_away("ann"), |runner| {
            let mut other = fixture_tokens();
            other["tmt_binding"] = json!("123e4567-e89b-42d3-a456-426614174099");
            runner.json(listed(other));
        }),
    ] {
        let runner = Scripted::default();
        runner.json(no_agents());
        script(&runner);
        assert_eq!(
            error_code(&serve(&runner, "name-agent", request)),
            "not_found",
            "{case}"
        );
        assert!(
            !runner
                .commands()
                .iter()
                .any(|call| call.contains("agent rename") || call.contains("report-metadata")),
            "{case}"
        );
        runner.done();
    }
    let runner = Scripted::default();
    assert_eq!(
        error_code(&serve(&runner, "name-agent", {
            let mut request = set("ann");
            request["panePid"] = json!(0);
            request
        })),
        "bad_request"
    );
}
