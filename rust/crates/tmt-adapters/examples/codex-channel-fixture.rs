//! Deterministic Codex protocol peer for real CLI/tmux E2E tests. No model,
//! credentials, provider configuration or external network is used.
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufRead, Write},
    net::{TcpListener, TcpStream},
    process::Command,
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
        println!("codex-cli 0.159.3");
    } else if args.iter().any(|arg| arg == "app-server") {
        server(&args);
    } else {
        foreground(&args);
    }
}
fn server(args: &[String]) {
    let token = fs::read_to_string(flag(args, "--ws-token-file")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    eprintln!("listening on: ws://{}", listener.local_addr().unwrap());
    event("server-started");
    let state = Arc::new(Mutex::new(None::<String>));
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
fn connection(stream: TcpStream, token: &str, state: Arc<Mutex<Option<String>>>) {
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
            "initialize" => json!({"userAgent":"codex_cli_rs/0.159.3"}),
            "initialized" => continue,
            "thread/start" => {
                let mut state = state.lock().unwrap();
                assert!(state.is_none(), "only one thread per owned endpoint");
                let thread = uuid::Uuid::new_v4().to_string();
                *state = Some(thread.clone());
                log(json!({"event":"thread-start","params":request["params"],"thread":thread}));
                json!({"cwd":request["params"]["cwd"],"thread":{"id":thread}})
            }
            "thread/resume" => {
                assert_eq!(
                    request["params"]["threadId"].as_str(),
                    state.lock().unwrap().as_deref()
                );
                event("attached");
                json!({})
            }
            "thread/queue/add" => {
                assert_eq!(
                    request["params"]["threadId"].as_str(),
                    state.lock().unwrap().as_deref()
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
    }
}
fn foreground(args: &[String]) {
    let _attachment = if args.iter().any(|arg| arg == "--remote") {
        assert!(!args.iter().any(|arg| matches!(
            arg.as_str(),
            "-s" | "--sandbox" | "-a" | "--ask-for-approval"
        )));
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
        socket
            .send(Message::text(
                json!({"id":"attach","method":"thread/resume","params":{"threadId":thread}})
                    .to_string(),
            ))
            .unwrap();
        socket.read().unwrap();
        Some(socket)
    } else {
        None
    };
    event("started");
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
