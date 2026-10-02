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
            "input"
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
        .refuse("pane_not_found");
    let answer = serve(
        &runner,
        "snapshot",
        json!({"socket": SOCKET, "panes": ["term_a1", "term_a3", "term_gone"]}),
    );
    assert_eq!(
        answer["panes"],
        json!([{"id": "term_a1", "target": "w1:p1", "cwd": "/repo/src",
                "command": "claude", "panePid": 90593, "suggestedName": null,
                "marker": fixture()["marker"]}])
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
