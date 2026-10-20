//! The price file's validation, fallback order and fetch, against a loopback
//! server and a private directory; nothing here touches the network.

use super::{
    BUILT_IN, Lookup, Resolved, Table, built_in, cache,
    fetch::{self, Target},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const NOW: u64 = 1_800_000_000_000;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        // The private-path rule refuses symlinks, and macOS's temp dir is under `/var`.
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "tmt-digest-pricing-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Clone)]
enum Answer {
    Body(Vec<u8>),
    Status(u16),
    Redirect,
    Stall,
}

struct Server {
    address: SocketAddr,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
}
impl Server {
    fn answering(answer: Answer) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (seen, heads, done) = (hits.clone(), requests.clone(), stopped.clone());
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                if done.load(Ordering::SeqCst) {
                    return;
                }
                seen.fetch_add(1, Ordering::SeqCst);
                let mut head = [0u8; 4096];
                let read = stream.read(&mut head).unwrap_or(0);
                heads
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head[..read]).into_owned());
                let answer = answer.clone();
                std::thread::spawn(move || match answer {
                    Answer::Body(body) => reply(&mut stream, "200 OK", "", &body),
                    Answer::Status(code) => reply(&mut stream, &format!("{code} Nope"), "", b"{}"),
                    Answer::Redirect => reply(
                        &mut stream,
                        "301 Moved Permanently",
                        "Location: /elsewhere\r\n",
                        b"",
                    ),
                    Answer::Stall => std::thread::sleep(Duration::from_secs(6)),
                });
            }
        });
        Self {
            address,
            hits,
            requests,
            stopped,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn target(&self) -> Target {
        Target::local(format!("http://{}/pricing-v1.json", self.address))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
    }
}
fn reply(stream: &mut TcpStream, status: &str, extra: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn base() -> Value {
    serde_json::from_str(BUILT_IN).unwrap()
}
fn bytes(file: &Value) -> Vec<u8> {
    serde_json::to_vec(file).unwrap()
}
/// The built-in file after `edit`, as bytes.
fn edited(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut file = base();
    edit(&mut file);
    bytes(&file)
}
/// A later version with one changed price.
fn newer() -> Value {
    let mut file = base();
    file["version"] = json!("2026-11-01");
    file["asOf"] = json!("2026-11-01");
    file["models"][0]["usdMicrosPerMillionInputTokens"] = json!(5_000_000);
    file
}
fn table(file: &Value) -> Table {
    Table::parse(&bytes(file)).unwrap()
}
fn state(file: &Value, attempted: u64) -> cache::State {
    cache::State {
        attempted_at_ms: attempted,
        fetched_at_ms: Some(attempted),
        file: Some(table(file)),
    }
}
fn price(resolved: &Resolved, model: &str) -> u64 {
    resolved.document()["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["modelId"] == model)
        .unwrap()["usdMicrosPerMillionInputTokens"]
        .as_u64()
        .unwrap()
}

#[test]
fn the_embedded_file_is_the_documented_table() {
    let document = Resolved::built_in().document();
    assert_eq!(document["version"], "2026-10-14");
    assert_eq!(document["asOf"], "2026-10-14");
    assert_eq!(document["basis"], "standard-base-input");
    assert_eq!(document["origin"], "built-in");
    assert!(document.get("fetchedAtMs").is_none());
    let models: Vec<_> = document["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["modelId"].as_str().unwrap())
        .collect();
    assert_eq!(
        models,
        [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "gpt-6-sol",
            "gpt-6.1-sol"
        ]
    );
    // The file on disk is what the build embeds; the release asset is checked against it.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pricing/pricing-v1.json");
    assert_eq!(fs::read_to_string(path).unwrap(), BUILT_IN);
    assert_eq!(built_in(), &Table::parse(BUILT_IN.as_bytes()).unwrap());
}

#[test]
fn every_malformed_file_is_dropped() {
    let text = BUILT_IN.to_owned();
    let price_key = "usdMicrosPerMillionInputTokens";
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("extra top-level key", edited(|f| f["extra"] = json!(1))),
        (
            "missing key",
            edited(|f| {
                f.as_object_mut().unwrap().remove("asOf");
            }),
        ),
        (
            "wrong schema",
            edited(|f| f["schema"] = json!("tmt-digest-pricing-v2")),
        ),
        (
            "impossible date",
            edited(|f| f["version"] = json!("2026-02-30")),
        ),
        ("month 13", edited(|f| f["asOf"] = json!("2026-13-01"))),
        ("short date", edited(|f| f["version"] = json!("26-10-14"))),
        (
            "unknown basis",
            edited(|f| f["basis"] = json!("cached-input")),
        ),
        ("no models", edited(|f| f["models"] = json!([]))),
        (
            "duplicate model",
            edited(|f| {
                let first = f["models"][0].clone();
                f["models"].as_array_mut().unwrap().push(first);
            }),
        ),
        (
            "model id with a space",
            edited(|f| f["models"][0]["modelId"] = json!("a b")),
        ),
        (
            "empty model id",
            edited(|f| f["models"][0]["modelId"] = json!("")),
        ),
        (
            "negative price",
            edited(|f| f["models"][0][price_key] = json!(-1)),
        ),
        (
            "fractional price",
            edited(|f| f["models"][0][price_key] = json!(1.5)),
        ),
        (
            "price above the safe integer",
            edited(|f| f["models"][0][price_key] = json!(9_007_199_254_740_992u64)),
        ),
        (
            "price as text",
            edited(|f| f["models"][0][price_key] = json!("4000000")),
        ),
        (
            "plain http source",
            edited(|f| f["models"][0]["source"] = json!("http://example.com/prices")),
        ),
        (
            "source with a space",
            edited(|f| f["models"][0]["source"] = json!("https://example.com/a b")),
        ),
        (
            "source over 256 bytes",
            edited(|f| {
                f["models"][0]["source"] = json!(format!("https://example.com/{}", "a".repeat(250)))
            }),
        ),
        (
            "unknown model key",
            edited(|f| f["models"][0]["cached"] = json!(1)),
        ),
        (
            "65 models",
            edited(|f| {
                f["models"] = (0..65)
                    .map(|index| {
                        json!({"modelId":format!("m{index}"),price_key:1,
                            "source":"https://example.com/p"})
                    })
                    .collect()
            }),
        ),
        ("trailing text", format!("{text} x").into_bytes()),
        (
            "byte order mark",
            [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
        ),
        ("not UTF-8", [text.as_bytes(), b"\xff"].concat()),
        (
            "duplicate key",
            text.replacen("\"version\"", "\"version\": \"2026-10-14\", \"version\"", 1)
                .into_bytes(),
        ),
        (
            "over 16 KiB",
            format!("{text}{}", " ".repeat(16 * 1024)).into_bytes(),
        ),
        ("empty", Vec::new()),
        ("an array", b"[]".to_vec()),
    ];
    assert!(Table::parse(text.as_bytes()).is_some());
    for (name, body) in cases {
        assert!(Table::parse(&body).is_none(), "{name} must be dropped");
    }
}

#[test]
fn a_version_is_one_table_and_never_older_than_what_is_known() {
    let known = table(&base());
    let mut repriced = base();
    repriced["models"][1]["usdMicrosPerMillionInputTokens"] = json!(2_000_001);
    let mut re_sourced = base();
    re_sourced["models"][1]["source"] = json!("https://example.com/other");
    let mut older = base();
    older["version"] = json!("2026-10-13");
    assert!(known.follows(&known), "the same table again");
    assert!(
        table(&newer()).follows(&known),
        "a later version may differ"
    );
    assert!(
        !table(&repriced).follows(&known),
        "a known version with other prices"
    );
    assert!(
        !table(&re_sourced).follows(&known),
        "a known version with other content"
    );
    assert!(
        !table(&older).follows(&known),
        "an older version never rolls back"
    );
    assert!(!table(&repriced).consistent_with(&known));
    assert!(table(&older).consistent_with(&known));
}

#[test]
fn quotes_use_the_table_in_hand() {
    let fetched = table(&newer());
    let quote = fetched.quote("claude-opus-5-5", 7, 1_000).unwrap();
    assert_eq!(quote["pricingVersion"], "2026-11-01");
    assert_eq!(quote["usdMicrosPerMillionInputTokens"], 5_000_000);
    assert_eq!(quote["usdMicros"], 5_000);
    let old = built_in().quote("claude-opus-5-5", 7, 1_000).unwrap();
    assert_eq!(old["pricingVersion"], "2026-10-14");
    assert_eq!(old["usdMicros"], 4_000);
    assert!(fetched.quote("unknown-model", 7, 1).is_none());
}

#[test]
fn the_address_override_admits_https_and_this_machine_only() {
    for url in [
        "https://example.com/p.json",
        "http://127.0.0.1/p.json",
        "http://127.0.0.1:8080/p.json",
        "http://localhost:1/x",
        "http://[::1]:9/x",
        "http://[::1]",
    ] {
        assert!(fetch::admit(url).is_some(), "{url}");
    }
    for url in [
        "",
        "off",
        "http://example.com/p.json",
        "http://127.0.0.1.example.com/p.json",
        "http://localhost@example.com/p.json",
        "http://localhostx/p.json",
        "ftp://127.0.0.1/p.json",
        "file:///etc/passwd",
        "https://exa mple.com/p.json",
        "https://example.com/p.json\n",
    ] {
        assert!(fetch::admit(url).is_none(), "{url:?}");
    }
    let unset = std::env::VarError::NotPresent;
    assert_eq!(fetch::target_from(Err(unset)).unwrap().url(), fetch::URL);
    assert!(fetch::target_from(Ok("off".into())).is_none());
    assert!(fetch::target_from(Ok(String::new())).is_none());
    assert!(fetch::target_from(Err(std::env::VarError::NotUnicode("x".into()))).is_none());
}

#[test]
fn the_default_address_is_the_raw_main_url_of_the_embedded_file() {
    assert_eq!(
        fetch::URL,
        "https://raw.githubusercontent.com/pj-tmt/tmt/main/extensions/tmt-digest/pricing/pricing-v1.json"
    );
    let default = fetch::target_from(Err(std::env::VarError::NotPresent)).unwrap();
    assert!(!default.is_loopback());
}

#[test]
fn a_valid_fetch_is_used_and_cached_privately() {
    let temp = Temp::new();
    let server = Server::answering(Answer::Body(bytes(&newer())));
    let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    let document = resolved.document();
    assert_eq!(document["version"], "2026-11-01");
    assert_eq!(document["origin"], "fetched");
    assert_eq!(document["fetchedAtMs"], NOW);
    assert_eq!(price(&resolved, "claude-opus-5-5"), 5_000_000);
    let request = server.requests.lock().unwrap().join("\n").to_lowercase();
    assert!(request.starts_with("get /pricing-v1.json http/1.1"));
    assert!(request.contains("user-agent: tmt-digest/"));
    for header in ["authorization", "cookie", "x-github", "referer"] {
        assert!(!request.contains(header), "{header} was sent");
    }
    let stored = temp.0.join("digest/pricing/fetched-v1.json");
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&stored), 0o600);
    assert_eq!(mode(&temp.0.join("digest/pricing")), 0o700);
    assert_eq!(mode(&temp.0.join("digest")), 0o700);
    let kept = cache::read(&temp.0).unwrap();
    assert_eq!(kept.attempted_at_ms, NOW);
    assert_eq!(kept.fetched_at_ms, Some(NOW));
    assert_eq!(kept.file, Some(table(&newer())));
    assert_eq!(
        fs::read_dir(temp.0.join("digest/pricing")).unwrap().count(),
        1
    );
    // The next status inside the spacing reads the cache and says so.
    let again = Lookup::begin(&temp.0, NOW + 1_000, Some(server.target())).finish();
    assert_eq!(again.document()["origin"], "cached");
    assert_eq!(again.document()["fetchedAtMs"], NOW);
    assert_eq!(server.hits(), 1);
}

#[test]
fn an_unusable_answer_falls_back_to_the_cache_and_then_the_built_in_table() {
    let mut bad_schema = newer();
    bad_schema["schema"] = json!("x");
    let older = edited(|f| f["version"] = json!("2026-10-13"));
    let repriced = edited(|f| f["models"][0]["usdMicrosPerMillionInputTokens"] = json!(1));
    let answers = [
        ("not found", Answer::Status(404)),
        ("server error", Answer::Status(500)),
        ("redirect", Answer::Redirect),
        ("garbage", Answer::Body(b"<html>".to_vec())),
        ("oversized", Answer::Body(vec![b' '; 17 * 1024])),
        ("wrong schema", Answer::Body(bytes(&bad_schema))),
        ("older than the built-in", Answer::Body(older)),
        ("built-in version, other prices", Answer::Body(repriced)),
    ];
    for (name, answer) in answers {
        let server = Server::answering(answer);
        let temp = Temp::new();
        let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
        assert_eq!(resolved.document()["origin"], "built-in", "{name}");
        assert_eq!(price(&resolved, "claude-opus-5-5"), 4_000_000, "{name}");
        let kept = cache::read(&temp.0).unwrap();
        assert_eq!((kept.attempted_at_ms, kept.file), (NOW, None), "{name}");

        let temp = Temp::new();
        cache::write(&temp.0, &state(&newer(), NOW - 60_000)).unwrap();
        let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
        let document = resolved.document();
        assert_eq!(document["origin"], "cached", "{name}");
        assert_eq!(document["version"], "2026-11-01", "{name}");
        assert_eq!(document["fetchedAtMs"], NOW - 60_000, "{name}");
        let kept = cache::read(&temp.0).unwrap();
        assert_eq!(
            kept.attempted_at_ms, NOW,
            "{name}: the failed attempt counts"
        );
        assert_eq!(
            kept.file,
            Some(table(&newer())),
            "{name}: the cache keeps its file"
        );
    }
}

#[test]
fn a_fetched_file_cannot_change_the_prices_of_a_known_version() {
    // Known from the cache: the same version with other prices is dropped.
    let temp = Temp::new();
    cache::write(&temp.0, &state(&newer(), NOW - 60_000)).unwrap();
    let mut conflicting = newer();
    conflicting["models"][2]["usdMicrosPerMillionInputTokens"] = json!(9);
    let server = Server::answering(Answer::Body(bytes(&conflicting)));
    let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    assert_eq!(resolved.document()["origin"], "cached");
    assert_eq!(price(&resolved, "gpt-6-sol"), 2_000_000);

    // The same file again is the same table and is simply fetched.
    let server = Server::answering(Answer::Body(bytes(&newer())));
    let resolved = Lookup::begin(&temp.0, NOW + 60_000, Some(server.target())).finish();
    assert_eq!(resolved.document()["origin"], "fetched");
    assert_eq!(resolved.document()["fetchedAtMs"], NOW + 60_000);

    // Known from the built-in table: its own bytes are accepted, other prices are not.
    let temp = Temp::new();
    let server = Server::answering(Answer::Body(BUILT_IN.as_bytes().to_vec()));
    let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    assert_eq!(resolved.document()["origin"], "fetched");
    assert_eq!(resolved.table(), built_in());
}

#[test]
fn a_stalled_server_ends_at_the_deadline_and_the_attempt_counts() {
    let temp = Temp::new();
    let server = Server::answering(Answer::Stall);
    let started = Instant::now();
    let resolved = Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    let waited = started.elapsed();
    assert_eq!(resolved.document()["origin"], "built-in");
    assert!(waited >= Duration::from_millis(1_800), "{waited:?}");
    assert!(waited < Duration::from_millis(3_200), "{waited:?}");
    assert_eq!(cache::read(&temp.0).unwrap().attempted_at_ms, NOW);
    assert_eq!(server.hits(), 1);
}

#[test]
fn attempts_are_thirty_seconds_apart_and_a_failed_one_counts() {
    let temp = Temp::new();
    let server = Server::answering(Answer::Status(500));
    Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    assert_eq!(server.hits(), 1);
    let soon = Lookup::begin(&temp.0, NOW + 29_999, Some(server.target())).finish();
    assert_eq!(soon.document()["origin"], "built-in");
    assert_eq!(server.hits(), 1, "no second attempt inside the spacing");
    Lookup::begin(&temp.0, NOW + 30_000, Some(server.target())).finish();
    assert_eq!(server.hits(), 2);
    // A clock that moved back is not a reason to wait.
    Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
    assert_eq!(server.hits(), 3);
}

#[test]
fn with_the_fetch_off_the_cache_still_serves_and_nothing_is_written() {
    let temp = Temp::new();
    let none = Lookup::begin(&temp.0, NOW, None).finish();
    assert_eq!(none.document()["origin"], "built-in");
    assert!(!temp.0.join("digest").exists());
    cache::write(&temp.0, &state(&newer(), NOW - 60_000)).unwrap();
    let path = temp.0.join("digest/pricing/fetched-v1.json");
    let before = fs::read(&path).unwrap();
    let cached = Lookup::begin(&temp.0, NOW, None).finish();
    assert_eq!(cached.document()["origin"], "cached");
    assert_eq!(price(&cached, "claude-opus-5-5"), 5_000_000);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_cache_that_no_longer_follows_the_built_in_table_is_ignored() {
    let mut older = base();
    older["version"] = json!("2026-10-13");
    let mut repriced = base();
    repriced["models"][0]["usdMicrosPerMillionInputTokens"] = json!(1);
    for (name, file) in [("older", older), ("same version, other prices", repriced)] {
        let temp = Temp::new();
        cache::write(&temp.0, &state(&file, NOW - 60_000)).unwrap();
        let resolved = Lookup::begin(&temp.0, NOW, None).finish();
        assert_eq!(resolved.document()["origin"], "built-in", "{name}");
        assert_eq!(price(&resolved, "claude-opus-5-5"), 4_000_000, "{name}");
        // A failed attempt does not carry the stale file forward.
        let server = Server::answering(Answer::Status(500));
        Lookup::begin(&temp.0, NOW, Some(server.target())).finish();
        assert_eq!(cache::read(&temp.0).unwrap().file, None, "{name}");
    }
}

#[test]
fn the_cache_is_private_and_untrusted_on_read() {
    let temp = Temp::new();
    let directory = temp.0.join("digest/pricing");
    let path = directory.join("fetched-v1.json");
    let fresh = || {
        cache::write(&temp.0, &state(&newer(), NOW)).unwrap();
        fs::read_to_string(&path).unwrap()
    };
    let chmod = |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode));
    let valid = fresh();
    assert!(cache::read(&temp.0).is_some());

    chmod(&path, 0o644).unwrap();
    assert!(cache::read(&temp.0).is_none(), "group/world readable file");
    fresh();
    chmod(&directory, 0o755).unwrap();
    assert!(cache::read(&temp.0).is_none(), "open directory");
    chmod(&directory, 0o700).unwrap();

    let elsewhere = temp.0.join("elsewhere.json");
    fs::write(&elsewhere, &valid).unwrap();
    chmod(&elsewhere, 0o600).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&elsewhere, &path).unwrap();
    assert!(cache::read(&temp.0).is_none(), "symlink");
    fs::remove_file(&path).unwrap();

    let file: Value = serde_json::from_str(&valid).unwrap();
    let broken = |name: &str, edit: &dyn Fn(&mut Value)| {
        let mut changed = file.clone();
        edit(&mut changed);
        fs::write(&path, changed.to_string()).unwrap();
        chmod(&path, 0o600).unwrap();
        assert!(cache::read(&temp.0).is_none(), "{name}");
        fs::remove_file(&path).unwrap();
    };
    broken("unknown key", &|f| f["extra"] = json!(1));
    broken("other cache version", &|f| f["version"] = json!(2));
    broken("time without a file", &|f| f["file"] = Value::Null);
    broken("file without a time", &|f| f["fetchedAtMs"] = Value::Null);
    broken("unsafe time", &|f| {
        f["attemptedAtMs"] = json!(9_007_199_254_740_992u64)
    });
    broken("invalid price file", &|f| f["file"]["basis"] = json!("x"));
    broken("over its size bound", &|f| {
        f["padding"] = json!("x".repeat(70 * 1024))
    });
    fs::write(&path, "{").unwrap();
    chmod(&path, 0o600).unwrap();
    assert!(cache::read(&temp.0).is_none(), "truncated");

    // A bad cache is replaced by the next valid write.
    fresh();
    assert!(cache::read(&temp.0).is_some());
}
