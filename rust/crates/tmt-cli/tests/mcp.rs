//! Real stdio clients: command parity, pinned attribution and child lifetime.
#![cfg(unix)]

mod support;

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_adapters::process::{CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DEADLINE: Duration = Duration::from_secs(30);

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-mcp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        // Reuse the fixture's environment definition without launching a child.
        support::command(&root, &[]);
        Self { root }
    }
    fn raw(&self, args: &[&str], input: &[u8]) -> (CommandOutput, bool) {
        let command = support::command(&self.root, args);
        let mut argv = vec![OsString::from("-i")];
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                let mut entry = key.to_os_string();
                entry.push("=");
                entry.push(value);
                argv.push(entry);
            }
        }
        // A fixed shell trampoline gives the shared bounded runner the fixture cwd.
        // All paths and CLI arguments are positional data, never shell source.
        argv.extend([
            OsString::from("/bin/sh"),
            OsString::from("-c"),
            OsString::from("cd \"$1\" && shift && exec \"$@\""),
            OsString::from("tmt-mcp-fixture"),
            self.root.as_os_str().to_os_string(),
        ]);
        argv.push(command.get_program().to_os_string());
        argv.extend(command.get_args().map(OsStr::to_os_string));
        let output = UnixCommandRunner.execute(CommandRequest {
            program: OsStr::new("/usr/bin/env"),
            args: &argv,
            input,
            deadline: Instant::now() + DEADLINE,
            max_output_bytes: tmt_adapters::mcp::OUTPUT_LIMIT,
        });
        let (output, ok) = match output {
            Ok(output) => (output, true),
            Err(error) => (
                error
                    .output
                    .expect("observed CLI failure, not timeout or leaked child"),
                false,
            ),
        };
        (output, ok)
    }
    fn cli(&self, args: &[&str], input: &[u8]) -> (Value, bool) {
        let (output, ok) = self.raw(args, input);
        (
            serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "CLI JSON: {error}; stderr: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
            }),
            ok,
        )
    }
    fn json(&self, args: &[&str]) -> Value {
        let (value, ok) = self.cli(args, &[]);
        assert!(ok, "CLI failed: {value}");
        value
    }
    fn api(&self, operation: &str, input: Value, identity: Option<&str>) -> Value {
        let mut request = json!({"version":1,"operation":operation,"input":input});
        if let Some(identity) = identity {
            request["identity"] = identity.into();
        }
        let (value, ok) = self.cli(&["api"], &serde_json::to_vec(&request).unwrap());
        assert!(ok, "API failed: {value}");
        value
    }
    fn create(&self, name: &str) -> String {
        self.json(&["identity", "create", name, "--json"])["identity"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn dispatch(&self, recipient: &str, operation: &str, message: &str) -> (Value, String) {
        let receipt = self.api(
            "dispatch.create",
            json!({"operationId":operation,"recipientIds":[recipient],"message":message}),
            Some("Sender"),
        );
        let id = receipt["items"][0]["requestId"].as_str().unwrap().into();
        (receipt, id)
    }
    fn attention(&self) -> Value {
        let db = Connection::open(support::state_dir(&self.root).join("tmux-team.db")).unwrap();
        let mut statement = db.prepare("SELECT request_id, attention_revision, attention_acknowledged_revision, recipient_attention_revision, recipient_attention_acknowledged_revision, status FROM request_attempts ORDER BY request_id").unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok(json!([
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?
                ]))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        json!(rows)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(fs::read_dir(self.root.join("tmux")).unwrap().count(), 0);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

struct Server {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<Receiver<Value>>,
    reader: Option<JoinHandle<()>>,
    next: u64,
}
impl Server {
    fn new(fixture: &Fixture, identity: &str) -> Self {
        let mut child = support::command(&fixture.root, &["mcp", "--identity", identity])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let count = stdout
                    .by_ref()
                    .take(tmt_adapters::mcp::OUTPUT_LIMIT as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .unwrap();
                if count == 0 {
                    break;
                }
                assert!(
                    count <= tmt_adapters::mcp::OUTPUT_LIMIT && line.ends_with(b"\n"),
                    "bounded complete MCP frame"
                );
                if tx
                    .send(serde_json::from_slice(&line).expect("only JSON on stdout"))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            child: Some(child),
            input,
            output: Some(rx),
            reader: Some(reader),
            next: 0,
        }
    }
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}));
        let value = self
            .output
            .as_ref()
            .unwrap()
            .recv_timeout(DEADLINE)
            .expect("bounded MCP response");
        assert_eq!(value["id"], self.next);
        value
    }
    fn initialize(&mut self, version: &str) {
        let value = self.rpc("initialize", json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"Other identity is not authority","version":"1"}}));
        assert_eq!(value["result"]["protocolVersion"], version);
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn tool(&mut self, name: &str, args: Value, expected: &Value, failed: bool) {
        let value = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        assert_eq!(value["result"]["isError"], failed, "{value}");
        assert_eq!(&value["result"]["structuredContent"], expected);
        assert_eq!(
            serde_json::from_str::<Value>(value["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap(),
            *expected
        );
    }
    fn finish(&mut self, success: bool) {
        self.input.take();
        let child = support::wait(&mut self.child, Duration::from_secs(10));
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.output.take();
        self.reader.take().unwrap().join().unwrap();
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        self.input.take();
        self.output.take();
        if let Some(reader) = self.reader.take() {
            let joined = reader.join();
            if !thread::panicking() {
                joined.unwrap();
            }
        }
    }
}

#[test]
fn mcp_reads_match_command_resources_without_acknowledging_or_dispatching() {
    let fixture = Fixture::new();
    fixture.create("Sender");
    let receiver = fixture.create("Receiver");
    fixture.create("Other");
    let operation = "694648f4-a55d-47e0-9ee5-337e6f67d8ab";
    let (_, id) = fixture.dispatch(&receiver, operation, "read parity 日本語 😀\r\n");
    let before = fixture.attention();
    for version in tmt_adapters::mcp::PROTOCOLS {
        let mut server = Server::new(&fixture, "Receiver");
        server.initialize(version);
        server.tool(
            "tmt_list",
            json!({}),
            &fixture.json(&["identity", "list", "--json"]),
            false,
        );
        server.tool(
            "tmt_operation",
            json!({"operationId":operation}),
            &fixture.api("dispatch.show", json!({"operationId":operation}), None),
            false,
        );
        server.tool(
            "tmt_inbox",
            json!({"from":"Sender","limit":1}),
            &fixture.json(&[
                "inbox",
                "--identity",
                "Receiver",
                "--from",
                "Sender",
                "--limit",
                "1",
                "--json",
            ]),
            false,
        );
        server.tool(
            "tmt_request",
            json!({"requestId":id}),
            &fixture.json(&[
                "x",
                "show",
                &id,
                "--incoming",
                "--identity",
                "Receiver",
                "--json",
            ]),
            false,
        );
        let (unavailable, ok) = fixture.cli(&["result", &id, "--json"], &[]);
        assert!(!ok);
        server.tool("tmt_result", json!({"requestId":id}), &unavailable, true);
        let (denied, ok) = fixture.cli(
            &[
                "x",
                "show",
                &id,
                "--incoming",
                "--identity",
                "Other",
                "--json",
            ],
            &[],
        );
        assert!(!ok);
        let mut other = Server::new(&fixture, "Other");
        other.initialize(version);
        other.tool("tmt_request", json!({"requestId":id}), &denied, true);
        other.finish(true);
        for name in ["tmt_send", "tmt_answer", "tmt_ack"] {
            assert_eq!(
                server.rpc("tools/call", json!({"name":name,"arguments":{}}))["error"]["code"],
                -32602
            );
        }
        assert_eq!(
            server.rpc(
                "tools/call",
                json!({"name":"tmt_list","arguments":{"identity":"Sender"}})
            )["error"]["code"],
            -32602
        );
        server.send(json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"tmt_list"}}));
        assert_eq!(server.rpc("ping", json!({}))["result"], json!({}));
        server.finish(true);
        assert_eq!(fixture.attention(), before);
    }
}

#[test]
fn mcp_result_keeps_the_full_maximum_exact_final() {
    let fixture = Fixture::new();
    fixture.create("Sender");
    let receiver = fixture.create("Receiver");
    let (_, id) = fixture.dispatch(
        &receiver,
        "89ae2b8a-e009-4c47-80f2-fac9dc0f5af3",
        "exact final",
    );
    let request = fixture.json(&[
        "x",
        "show",
        &id,
        "--incoming",
        "--identity",
        "Receiver",
        "--json",
    ]);
    let receipt = request["exchange"]["reply"]["receipt"].as_str().unwrap();
    let prefix = "\u{feff}\r\n\0日本語 😀\n";
    let final_text = format!(
        "{prefix}{}",
        "\0".repeat(tmt_core::exact_text::MAX_EXCHANGE_TEXT_BYTES - prefix.len())
    );
    let (submitted, ok) = fixture.cli(
        &["reply", &id, "--receipt", receipt, "--stdin", "--json"],
        final_text.as_bytes(),
    );
    assert!(ok, "{submitted}");
    let expected = fixture.json(&["result", &id, "--json"]);
    let mut server = Server::new(&fixture, &receiver);
    server.initialize(tmt_adapters::mcp::PROTOCOLS[0]);
    server.tool("tmt_result", json!({"requestId":id}), &expected, false);
    server.finish(true);
    assert_eq!(
        expected["response"].as_str().unwrap().as_bytes(),
        final_text.as_bytes()
    );
}

#[test]
fn mcp_identity_is_pinned_through_rename_and_never_rebound_after_retirement() {
    let fixture = Fixture::new();
    fixture.create("Sender");
    let receiver = fixture.create("Receiver");
    let mut server = Server::new(&fixture, "Receiver");
    server.initialize(tmt_adapters::mcp::PROTOCOLS[1]);
    fixture.json(&["identity", "mv", "Receiver", "Renamed", "--json"]);
    fixture.create("Receiver");
    server.tool(
        "tmt_inbox",
        json!({}),
        &fixture.json(&["inbox", "--identity", &receiver, "--json"]),
        false,
    );
    fixture.json(&["rm", "Renamed", "--force", "--json"]);
    // A UUID-shaped display name must not rescue the old pinned UUID.
    fixture.create(&receiver);
    let result = server.rpc("tools/call", json!({"name":"tmt_inbox","arguments":{}}));
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(
        result["result"]["structuredContent"]["error"]["code"],
        "NAME_NOT_FOUND"
    );
    server.finish(true);
}

#[test]
fn mcp_eof_partial_frames_and_signal_have_bounded_child_cleanup() {
    let fixture = Fixture::new();
    fixture.create("Receiver");
    let mut clean = Server::new(&fixture, "Receiver");
    clean.finish(true);
    let mut partial = Server::new(&fixture, "Receiver");
    partial.input.as_mut().unwrap().write_all(b"{").unwrap();
    partial.finish(false);
    let mut stalled = Server::new(&fixture, "Receiver");
    stalled.initialize(tmt_adapters::mcp::PROTOCOLS[0]);
    stalled.input.as_mut().unwrap().write_all(b"{").unwrap();
    // Leave stdin open: acquisition, rather than EOF, must end the child.
    let child = support::wait(&mut stalled.child, Duration::from_secs(10));
    assert!(!child.wait_with_output().unwrap().status.success());
    let mut signalled = Server::new(&fixture, "Receiver");
    signalled.initialize(tmt_adapters::mcp::PROTOCOLS[0]);
    kill(
        Pid::from_raw(signalled.child.as_ref().unwrap().id() as i32),
        Signal::SIGTERM,
    )
    .unwrap();
    signalled.finish(false);
}

#[test]
fn mcp_refuses_implicit_missing_and_temporary_launch_identities() {
    let fixture = Fixture::new();
    fixture.create("Saved");
    let database = support::state_dir(&fixture.root).join("tmux-team.db");
    let mut storage = tmt_adapters::storage::Storage::open(database).unwrap();
    tmt_core::identity::create_or_resolve(
        &mut storage,
        "Temporary",
        tmt_core::identity::Lifetime::Temporary,
    )
    .unwrap();
    storage.close().unwrap();
    for args in [
        vec!["mcp"],
        vec!["mcp", "--identity", "Missing"],
        vec!["mcp", "--identity", "Temporary"],
    ] {
        let (output, ok) = fixture.raw(&args, b"");
        assert!(!ok);
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let mut server = Server::new(&fixture, "Saved");
    server.initialize(tmt_adapters::mcp::PROTOCOLS[0]);
    server.finish(true);
}

#[test]
fn mcp_rejects_an_oversized_frame_and_recovers_after_invalid_complete_json() {
    let fixture = Fixture::new();
    fixture.create("Receiver");
    let mut server = Server::new(&fixture, "Receiver");
    server.input.as_mut().unwrap().write_all(b"{\n").unwrap();
    let error = server
        .output
        .as_ref()
        .unwrap()
        .recv_timeout(DEADLINE)
        .unwrap();
    assert!(error["id"].is_null());
    assert_eq!(error["error"]["code"], -32700);
    server.initialize(tmt_adapters::mcp::PROTOCOLS[0]);
    assert_eq!(server.rpc("ping", json!({}))["result"], json!({}));
    server.finish(true);
    let mut oversized = Server::new(&fixture, "Receiver");
    // The server may close while the final chunk is being written; both outcomes
    // must end in a framing failure, never a successful truncated tool response.
    let _ = oversized
        .input
        .as_mut()
        .unwrap()
        .write_all(&vec![b'X'; tmt_adapters::mcp::INPUT_LIMIT + 1]);
    oversized.finish(false);
}
