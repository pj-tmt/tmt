//! Deterministic Codex protocol peer for real CLI/tmux E2E tests. No model,
//! credentials, host provider configuration or external network is used.
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufRead, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, client::IntoClientRequest};

fn log(value: Value) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(env::var("MOCK_CHANNEL_LOG").unwrap())
        .unwrap();
    file.write_all(format!("{value}\n").as_bytes()).unwrap();
}
fn event(name: &str) {
    log(json!({"event":name,"pid":std::process::id()}));
}
fn flag(args: &[String], name: &str) -> String {
    args.windows(2).find(|pair| pair[0] == name).unwrap()[1].clone()
}
fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!(
            "codex-cli {}",
            env::var("MOCK_VERSION").unwrap_or_else(|_| "0.160.0".into())
        );
    } else if args.iter().any(|arg| arg == "app-server") {
        if env::var_os("MOCK_SERVER_FAILURE").is_some() {
            std::process::exit(1);
        }
        server(&args);
    } else {
        foreground(&args);
    }
}
#[derive(Clone)]
struct LoadedThread {
    id: String,
    cwd: String,
}

fn server(args: &[String]) {
    let token = fs::read_to_string(flag(args, "--ws-token-file")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    eprintln!("listening on: ws://{}", listener.local_addr().unwrap());
    event("server-started");
    let state = Arc::new(Mutex::new(None::<LoadedThread>));
    if env::var_os("MOCK_DIGEST_HOOKS").is_some() {
        let state = state.clone();
        let args = args.to_vec();
        thread::spawn(move || {
            digest_steps(&args, || {
                state.lock().unwrap().as_ref().map(|s| s.id.clone())
            })
        });
    }
    for stream in listener.incoming() {
        let token = token.clone();
        let state = state.clone();
        thread::spawn(move || connection(stream.unwrap(), &token, state));
    }
}
#[expect(
    clippy::result_large_err,
    reason = "tungstenite requires its handshake HTTP error response by value"
)]
fn connection(stream: TcpStream, token: &str, state: Arc<Mutex<Option<LoadedThread>>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(90)))
        .unwrap();
    let expected = format!("Bearer {token}");
    let mut socket = tungstenite::accept_hdr(
        stream,
        |request: &tungstenite::handshake::server::Request, response| {
            assert_eq!(
                request.headers().get("authorization").unwrap(),
                expected.as_str()
            );
            Ok(response)
        },
    )
    .unwrap();
    while let Ok(Message::Text(text)) = socket.read() {
        let request: Value = serde_json::from_str(text.as_str()).unwrap();
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap();
        let result = match method {
            "initialize" => json!({"userAgent":"codex_cli_rs/0.160.0"}),
            "initialized" => continue,
            "thread/start" => {
                let mut state = state.lock().unwrap();
                assert!(state.is_none(), "only one thread per owned endpoint");
                let thread = uuid::Uuid::new_v4().to_string();
                *state = Some(LoadedThread {
                    id: thread.clone(),
                    cwd: fs::canonicalize(request["params"]["cwd"].as_str().unwrap())
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                });
                log(json!({"event":"thread-start","params":request["params"],"thread":thread}));
                json!({"cwd":request["params"]["cwd"],"thread":{"id":thread}})
            }
            "thread/loaded/list" => {
                json!({"data":state.lock().unwrap().as_ref().map(|t| vec![t.id.clone()]).unwrap_or_default()})
            }
            "thread/read" => {
                let state = state.lock().unwrap();
                let loaded = state.as_ref().unwrap();
                assert_eq!(request["params"]["threadId"], loaded.id);
                json!({"thread":{"id":loaded.id,"cwd":loaded.cwd,"ephemeral":false}})
            }
            "thread/resume" => {
                let mut state = state.lock().unwrap();
                let requested = request["params"]["threadId"].as_str().unwrap();
                if state.is_none() {
                    log(
                        json!({"event":"thread-resume","params":request["params"],"thread":requested}),
                    );
                    if env::var("MOCK_RESUME_FAILURE").as_deref() == Ok("refused") {
                        socket.send(Message::text(json!({"id":id,"error":{"code":-32603,"message":"fixture resume refused"}}).to_string())).unwrap();
                        continue;
                    }
                    let returned = if env::var("MOCK_RESUME_FAILURE").as_deref() == Ok("mismatch") {
                        "33333333-3333-4333-8333-333333333333"
                    } else {
                        requested
                    };
                    *state = Some(LoadedThread {
                        id: returned.to_owned(),
                        cwd: request["params"]["cwd"].as_str().unwrap().to_owned(),
                    });
                    json!({"cwd":request["params"]["cwd"],"thread":{"id":returned}})
                } else {
                    assert_eq!(Some(requested), state.as_ref().map(|t| t.id.as_str()));
                    event("attached");
                    json!({})
                }
            }
            "thread/queue/add" => {
                assert_eq!(
                    request["params"]["threadId"].as_str(),
                    state.lock().unwrap().as_ref().map(|t| t.id.as_str())
                );
                let params = &request["params"];
                let content = params["input"][0]["text"].as_str().unwrap();
                log(json!({"event":"queue","content":content,"id":id}));
                let mode = env::var("MOCK_RECEIPT").unwrap_or_default();
                if mode == "lost" {
                    return;
                }
                if mode == "internal" || mode == "archived" {
                    let thread = params["threadId"].as_str().unwrap();
                    let (code, message) = if mode == "archived" {
                        (
                            -32600,
                            format!(
                                "session {thread} is archived. Run `codex unarchive {thread}` to unarchive it first."
                            ),
                        )
                    } else {
                        (-32603, "internal failure after possible enqueue".into())
                    };
                    socket
                        .send(Message::text(
                            json!({"id":id,"error":{"code":code,"message":message}}).to_string(),
                        ))
                        .unwrap();
                    continue;
                }
                let result = json!({"queuedSubmission":{"id":uuid::Uuid::new_v4().to_string(),"clientUserMessageId":params["clientUserMessageId"],"input":params["input"]}});
                // Write the native receipt before exposing processed input to the mock foreground.
                socket
                    .send(Message::text(json!({"id":id,"result":result}).to_string()))
                    .unwrap();
                if let Ok(model) = env::var("MOCK_HOOK_MODEL") {
                    hook(params["threadId"].as_str().unwrap(), "resume", &model);
                }
                if env::var_os("MOCK_ACTIVITY_HOOKS").is_some() {
                    let turn = uuid::Uuid::new_v4().to_string();
                    let session = params["threadId"].as_str().unwrap();
                    run_hook(
                        session,
                        "prompt-hook",
                        json!({"hook_event_name":"UserPromptSubmit","session_id":session,"turn_id":turn,"prompt":"fixture turn"}),
                    );
                    let finish = format!("{}.finish-turn", env::var("MOCK_CHANNEL_LOG").unwrap());
                    let deadline = Instant::now() + Duration::from_secs(15);
                    while !std::path::Path::new(&finish).exists() {
                        assert!(
                            Instant::now() < deadline,
                            "activity assertion did not release fixture turn"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    run_hook(
                        session,
                        "stop-hook",
                        json!({"hook_event_name":"Stop","session_id":session,"turn_id":turn,"transcript_path":null}),
                    );
                }
                log(json!({"event":"channel","content":content}));
                continue;
            }
            _ => panic!("unexpected method {method}"),
        };
        if socket
            .send(Message::text(json!({"id":id,"result":result}).to_string()))
            .is_err()
        {
            return;
        }
        if method == "thread/start"
            && let Ok(model) = env::var("MOCK_EAGER_HOOK_MODEL")
        {
            let id = state.lock().unwrap().as_ref().unwrap().id.clone();
            hook(&id, "startup", &model);
        }
    }
}
fn hook(session: &str, source: &str, model: &str) {
    run_hook(
        session,
        "hook",
        json!({"hook_event_name":"SessionStart","source":source,"session_id":session,"model":model}),
    );
}
fn run_hook(session: &str, event: &str, payload: Value) {
    let peer: Value = serde_json::from_str(&env::var("MOCK_PEER").unwrap()).unwrap();
    let started = Instant::now();
    log(json!({"event":"hook-arrived","thread":session}));
    let mut hook = Command::new(peer["executable"].as_str().unwrap())
        .args(
            peer["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap()),
        )
        .args(["__hook", "codex"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let result = hook.wait_with_output().unwrap();
    log(
        json!({"event":event,"thread":session,"elapsedMs":started.elapsed().as_millis(),"ok":result.status.success(),"stdout":String::from_utf8_lossy(&result.stdout),"stderr":String::from_utf8_lossy(&result.stderr)}),
    );
}
fn foreground(args: &[String]) {
    let _attachment = if args.iter().any(|arg| arg == "--remote") {
        let resume = args.first().is_some_and(|arg| arg == "resume");
        if resume {
            assert!(!args.iter().any(|arg| matches!(
                arg.as_str(),
                "-s" | "--sandbox" | "-a" | "--ask-for-approval"
            )));
        }
        let endpoint = flag(args, "--remote");
        let thread = args.last().unwrap();
        let mut request = endpoint.into_client_request().unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", env::var("TMT_CODEX_ENDPOINT_TOKEN").unwrap())
                .parse()
                .unwrap(),
        );
        let (mut socket, _) = tungstenite::connect(request).unwrap();
        let (method, params) = if resume {
            ("thread/resume", json!({"threadId":thread}))
        } else {
            ("thread/start", json!({"cwd":flag(args, "-C")}))
        };
        socket
            .send(Message::text(
                json!({"id":"attach","method":method,"params":params}).to_string(),
            ))
            .unwrap();
        socket.read().unwrap();
        Some(socket)
    } else {
        None
    };
    log(json!({"event":"started","pid":std::process::id(),"args":args}));
    if env::var_os("MOCK_DIGEST_HOOKS").is_some() && !args.iter().any(|a| a == "--remote") {
        let session = if args.first().is_some_and(|a| a == "resume") {
            args.iter()
                .find(|a| uuid::Uuid::parse_str(a).is_ok())
                .unwrap()
                .clone()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let args = args.to_vec();
        thread::spawn(move || digest_steps(&args, || Some(session.clone())));
    }
    thread::spawn(|| {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            log(json!({"event":"paste","line":line}));
        }
    });
    let path = env::var("MOCK_CHANNEL_LOG").unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut processed = 0;
    while !std::path::Path::new(&format!("{path}.quit")).exists() {
        assert!(Instant::now() < deadline, "fixture lifetime expired");
        let text = fs::read_to_string(&path).unwrap();
        let messages: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        for value in messages.iter().skip(processed) {
            if value["event"] != "channel" {
                continue;
            }
            let content = value["content"].as_str().unwrap();
            if env::var("MOCK_AUTOREPLY").as_deref() == Ok("1") {
                let words: Vec<_> = content.split_whitespace().collect();
                if let Some(index) = words.windows(2).position(|pair| pair == ["tmt", "reply"]) {
                    assert_eq!(words[index + 3], "--receipt");
                    let peer: Value =
                        serde_json::from_str(&env::var("MOCK_PEER").unwrap()).unwrap();
                    let result = Command::new(peer["executable"].as_str().unwrap())
                        .args(
                            peer["args"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|arg| arg.as_str().unwrap()),
                        )
                        .args([
                            "reply",
                            words[index + 2],
                            "--receipt",
                            words[index + 4],
                            "--message",
                            "channel-ok",
                            "--json",
                        ])
                        .output()
                        .unwrap();
                    log(
                        json!({"event":"reply","ok":result.status.success(),"stdout":String::from_utf8_lossy(&result.stdout),"stderr":String::from_utf8_lossy(&result.stderr)}),
                    );
                }
            }
        }
        processed = messages.len();
        thread::sleep(Duration::from_millis(20));
    }
}

// Model-free hook interpreter: consume the actual composed invocation and its
// isolated user hooks, with explicit fixture-only trust. No host provider runs.
fn digest_steps(args: &[String], session: impl Fn() -> Option<String>) {
    let path = env::var("MOCK_CHANNEL_LOG").unwrap();
    let mut definitions = serde_json::Map::new();
    let home = std::path::PathBuf::from(env::var_os("CODEX_HOME").unwrap());
    if let Ok(text) = fs::read_to_string(home.join("hooks.json")) {
        let user: Value = serde_json::from_str(&text).unwrap();
        definitions = user["hooks"].as_object().unwrap().clone();
    }
    let mut session_definitions = serde_json::Map::new();
    for pair in args
        .windows(2)
        .filter(|p| p[0] == "-c" || p[0] == "--config")
    {
        let doc = pair[1].parse::<toml_edit::DocumentMut>().unwrap();
        if let Some(hooks) = doc.get("hooks").and_then(toml_edit::Item::as_table_like) {
            for (event, entries) in hooks.iter() {
                let handlers = entries.as_array().unwrap().iter().map(|group| {
                    let group = group.as_inline_table().unwrap();
                    json!({"hooks":group.get("hooks").unwrap().as_array().unwrap().iter().map(|handler| {
                        let handler = handler.as_inline_table().unwrap();
                        json!({"command":handler.get("command").unwrap().as_str().unwrap()})
                    }).collect::<Vec<_>>()})
                }).collect::<Vec<_>>();
                // CLI overrides make one session layer. The final composed root
                // replaces an earlier explicit root; separate user hooks still merge.
                session_definitions.insert(event.to_owned(), json!(handlers));
            }
        }
    }
    for (event, handlers) in session_definitions {
        definitions
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .extend(handlers.as_array().unwrap().iter().cloned());
    }
    log(json!({"event":"digest-definitions","hooks":definitions}));
    while !std::path::Path::new(&format!("{path}.quit")).exists() {
        let step = format!("{path}.digest-step");
        if let Ok(bytes) = fs::read(&step) {
            fs::remove_file(&step).unwrap();
            let step: Value = serde_json::from_slice(&bytes).unwrap();
            let current = session().unwrap();
            let payload = json!({"hook_event_name":step["hookEvent"].as_str().unwrap_or("Stop"),"source":step["source"].as_str().unwrap_or("startup"),"session_id":step["session"].as_str().unwrap_or(&current),"turn_id":step["turn"].as_str().unwrap_or("fixture-turn"),"stop_hook_active":step["active"].as_bool().unwrap_or(false),"transcript_path":null,"model":"fixture-digest-model","prompt":"fixture prompt"});
            let event = payload["hook_event_name"].as_str().unwrap();
            if env::var("MOCK_TRUST_HOOKS").as_deref() != Ok("1") {
                log(json!({"event":"digest-skipped","step":step["name"]}));
            } else if let Some(groups) = definitions.get(event).and_then(Value::as_array) {
                for group in groups {
                    for handler in group["hooks"].as_array().unwrap() {
                        let original = handler["command"].as_str().unwrap();
                        let command = if env::var("MOCK_LEGACY_HOOKS").as_deref() == Ok("1") {
                            original.replace(" __digest-hook ", " __focus-hook ")
                        } else {
                            original.to_owned()
                        };
                        let mut child = Command::new("/bin/sh")
                            .args(["-c", &command])
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .unwrap();
                        child
                            .stdin
                            .take()
                            .unwrap()
                            .write_all(payload.to_string().as_bytes())
                            .unwrap();
                        let output = child.wait_with_output().unwrap();
                        let text = String::from_utf8(output.stdout).unwrap();
                        let parsed = serde_json::from_str::<Value>(&text).ok();
                        log(
                            json!({"event":"digest-handler","step":step["name"],"command":command,"ok":output.status.success(),"stdout":text,"stderr":String::from_utf8_lossy(&output.stderr)}),
                        );
                        if parsed.as_ref().is_some_and(|v| v["decision"] == "block") {
                            let reason = parsed.unwrap()["reason"].as_str().unwrap().to_owned();
                            log(
                                json!({"event":"digest-continuation","step":step["name"],"reason":reason}),
                            );
                        }
                    }
                }
            }
            log(json!({"event":"digest-step-done","step":step["name"],"session":current}));
        }
        thread::sleep(Duration::from_millis(10));
    }
}
