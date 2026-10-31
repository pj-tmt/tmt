use super::*;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

fn value() -> Value {
    json!({"version":1,"rail":{"label":"Colab","icon":"file-text","path":"/","order":100},"menu":[{"id":"pages","label":"<b>Pages</b>","path":"/pages","group":"Pages","count":{"value":2,"kind":"needs-you"}}],"settings":[],"needsYou":2,"home":[{"id":"latest","title":"Latest","path":"/latest"}]})
}
fn parse(v: &Value) -> Option<Description> {
    Description::parse(&serde_json::to_vec(v).unwrap())
}
fn source(name: &'static str) -> Source {
    Source {
        name,
        socket: PathBuf::from(format!("/tmp/{name}/door.sock")),
        identity: Some((1, 1)),
        host: "127.0.0.1:1".into(),
        mount: format!("/r/abc/x/{name}/"),
    }
}

fn extra() -> Value {
    json!({"id":"design","rail":{"label":"Design","icon":"palette","path":"/design/","order":100},"menu":[{"id":"library","label":"Library","path":"/design/library"}],"settings":[{"id":"preferences","label":"Preferences","path":"/design/settings"}],"entry":"/assets/design.js"})
}
fn with_extra() -> Value {
    let mut v = value();
    v["modules"] = json!([extra()]);
    v
}

#[test]
fn primary_only_composed_bytes_are_unchanged_and_extras_are_separate_mount_confined_entries() {
    let baseline = parse(&value())
        .unwrap()
        .module("colab", "https://door/r/abc/x/colab/");
    let expected = json!({"rail":{"label":"Colab","icon":"file-text","path":"https://door/r/abc/x/colab/","order":100},"menu":[{"id":"pages","label":"<b>Pages</b>","path":"https://door/r/abc/x/colab/pages","group":"Pages","count":{"value":2,"kind":"needs-you"}}],"settings":[],"needsYou":2,"home":[{"id":"latest","title":"Latest","path":"https://door/r/abc/x/colab/latest"}],"extension":"colab"});
    assert_eq!(
        serde_json::to_vec(&baseline).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    let shell = Shell::with_reader(Arc::new(|_| parse(&with_extra())), Arc::new(Instant::now));
    let answer = shell.snapshot(
        vec![source("colab")],
        "https://door",
        Instant::now() + Duration::from_secs(1),
    );
    let rows = answer["modules"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], baseline);
    assert!(rows[0].get("module").is_none());
    assert_eq!(rows[1]["extension"], "colab");
    assert_eq!(rows[1]["module"], "design");
    assert_eq!(rows[1]["rail"]["icon"], "palette");
    assert_eq!(
        rows[1]["rail"]["path"],
        "https://door/r/abc/x/colab/design/"
    );
    assert_eq!(
        rows[1]["menu"][0]["path"],
        "https://door/r/abc/x/colab/design/library"
    );
    assert_eq!(
        rows[1]["settings"][0]["path"],
        "https://door/r/abc/x/colab/design/settings"
    );
    assert_eq!(
        rows[1]["entry"],
        "https://door/r/abc/x/colab/assets/design.js"
    );
    for key in ["id", "version", "needsYou", "home", "modules"] {
        assert!(rows[1].get(key).is_none(), "extra leaked {key}");
    }
    assert_eq!(rows[2]["extension"], "remote");
    shell.shutdown();
}

#[test]
fn invalid_extra_refuses_the_entire_description_and_reuses_primary_limits() {
    let valid = with_extra();
    for field in ["version", "needsYou", "home", "future"] {
        let mut v = valid.clone();
        v["modules"][0][field] = json!(1);
        assert!(parse(&v).is_none(), "accepted extra {field}");
    }
    for (pointer, bad) in [
        ("/modules", Value::Null),
        ("/modules", json!([extra(), extra()])),
        ("/modules/0", Value::Null),
        ("/modules/0/id", json!("")),
        ("/modules/0/id", json!("Design")),
        ("/modules/0/id", json!("x".repeat(33))),
        ("/modules/0/rail/path", json!("//elsewhere/")),
        ("/modules/0/rail/path", json!("/../ops/")),
        ("/modules/0/menu/0/path", json!("https://elsewhere/")),
        ("/modules/0/settings/0/path", json!("/%2e%2e/")),
        ("/modules/0/rail/label", json!("x".repeat(17))),
        ("/modules/0/rail/order", json!(1000)),
        ("/modules/0/menu/0/label", json!("bad\n")),
        ("/modules/0/settings/0/label", json!("x".repeat(41))),
        ("/modules/0/entry", Value::Null),
    ] {
        let mut v = valid.clone();
        *v.pointer_mut(pointer).unwrap() = bad;
        assert!(parse(&v).is_none(), "accepted {pointer}");
    }
    for (key, n, item) in [
        (
            "menu",
            limits::DESCRIBE_MENU + 1,
            extra()["menu"][0].clone(),
        ),
        (
            "settings",
            limits::DESCRIBE_SETTINGS + 1,
            extra()["settings"][0].clone(),
        ),
    ] {
        let mut v = valid.clone();
        v["modules"][0][key] = json!(vec![item; n]);
        assert!(parse(&v).is_none());
    }
    let mut oversized = serde_json::to_vec(&valid).unwrap();
    oversized.resize(limits::DESCRIBE_BYTES + 1, b' ');
    assert!(Description::parse(&oversized).is_none());
    let mut v = valid;
    v["modules"][0]["rail"]["icon"] = json!("future-icon");
    v["modules"][0]["entry"] = json!("/../escape.js");
    let d = parse(&v).unwrap();
    assert_eq!(d.modules.as_ref().unwrap()[0].rail.icon, "box");
    assert!(
        d.modules.as_ref().unwrap()[0].entry.is_none(),
        "invalid entry keeps primary link-only fallback"
    );
}

#[test]
fn remote_rail_opens_the_existing_my_remotes_list() {
    let shell = Shell::new();
    let answer = shell.snapshot(vec![], "http://127.0.0.1:1", Instant::now());
    let remote = &answer["modules"][0];
    assert_eq!(remote["rail"]["path"], "http://127.0.0.1:1/remote/");
    assert_eq!(remote["menu"][0]["path"], remote["rail"]["path"]);
    assert_eq!(remote["settings"][0]["label"], "My Remotes");
}

#[test]
fn strict_description_bounds_and_every_path_stays_inside_its_mount() {
    let valid = value();
    let result = parse(&valid)
        .unwrap()
        .module("colab", "http://127.0.0.1:1/r/abc/x/colab/");
    assert_eq!(
        result["menu"][0]["label"], "<b>Pages</b>",
        "labels remain text data, not stripped HTML"
    );
    assert_eq!(
        result["menu"][0]["path"],
        "http://127.0.0.1:1/r/abc/x/colab/pages"
    );
    assert!(result.get("version").is_none());
    for bad in [
        "..",
        "/../x",
        "/a/../b",
        "//evil",
        "/a//b",
        "https://evil/x",
        "/x?q=1",
        "/x#hash",
        "/%2e%2e/x",
        "/a\\b",
    ] {
        for pointer in ["/rail/path", "/menu/0/path", "/home/0/path"] {
            let mut v = valid.clone();
            *v.pointer_mut(pointer).unwrap() = json!(bad);
            assert!(parse(&v).is_none(), "accepted {bad} at {pointer}");
        }
    }
    for (pointer, bad) in [
        ("/version", json!(2)),
        ("/version", json!(1.0)),
        ("/rail/order", json!(1000)),
        ("/rail/label", json!("字".repeat(17))),
        ("/menu/0/label", json!("x".repeat(41))),
        ("/menu/0/group", json!("x".repeat(25))),
        ("/menu/0/group", Value::Null),
        ("/menu/0/count/value", json!(10000)),
        ("/menu/0/count/kind", json!("urgent")),
        ("/needsYou", json!(-1)),
        ("/needsYou", json!(10000)),
        ("/home", Value::Null),
        ("/menu/0/id", json!("A")),
        ("/menu/0/id", json!("x".repeat(33))),
        ("/rail/icon", json!("<svg>")),
        ("/rail/icon", json!("https://evil")),
        ("/rail/label", json!("a\n")),
        ("/rail/label", json!("a\u{202e}b")),
    ] {
        let mut v = valid.clone();
        *v.pointer_mut(pointer).unwrap() = bad.clone();
        assert!(parse(&v).is_none(), "accepted {pointer}={bad}");
    }
    for (key, n, item) in [
        ("menu", 25, valid["menu"][0].clone()),
        ("settings", 17, json!({"id":"x","label":"x","path":"/"})),
        ("home", 9, valid["home"][0].clone()),
    ] {
        let mut v = valid.clone();
        v[key] = json!(vec![item; n]);
        assert!(parse(&v).is_none(), "{key}");
    }
    let mut v = valid.clone();
    v["menu"] = json!([valid["menu"][0], valid["menu"][0]]);
    assert!(parse(&v).is_none());
    let mut v = valid.clone();
    v["rail"]["active"] = json!(true);
    assert!(parse(&v).is_none());
    let text = serde_json::to_string(&valid)
        .unwrap()
        .replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(Description::parse(text.as_bytes()).is_none());
    assert!(Description::parse(&[0xff]).is_none());
    assert!(Description::parse(&vec![b' '; limits::DESCRIBE_BYTES + 1]).is_none());
    let mut v = valid;
    v["rail"]["icon"] = json!("future-icon");
    v["rail"]["label"] = json!("字".repeat(16));
    v["needsYou"] = json!(9999);
    v["rail"]["order"] = json!(999);
    assert_eq!(parse(&v).unwrap().rail.icon, "box");
}

#[test]
fn failed_refresh_retains_last_good_but_never_renews_its_age() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let now = Instant::now();
    let seconds = Arc::new(AtomicU64::new(0));
    let offset = Arc::clone(&seconds);
    let shell = Shell::with_reader(
        Arc::new(move |_| {
            (counter.fetch_add(1, Ordering::SeqCst) == 0).then(|| parse(&with_extra()).unwrap())
        }),
        Arc::new(move || now + Duration::from_secs(offset.load(Ordering::SeqCst))),
    );
    let get = |source| {
        shell.snapshot(
            vec![source],
            "http://127.0.0.1:1",
            Instant::now() + Duration::from_secs(1),
        )
    };
    assert_eq!(get(source("colab"))["modules"].as_array().unwrap().len(), 3);
    assert_eq!(get(source("colab"))["modules"][0]["extension"], "colab");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    seconds.store(5, Ordering::SeqCst);
    let mut missing = source("colab");
    missing.identity = None;
    assert_eq!(get(missing.clone())["modules"].as_array().unwrap().len(), 3);
    seconds.store(30, Ordering::SeqCst);
    assert_eq!(get(missing.clone())["modules"].as_array().unwrap().len(), 3);
    seconds.store(31, Ordering::SeqCst);
    assert_eq!(get(missing)["modules"].as_array().unwrap().len(), 1);
    shell.shutdown();
}

#[test]
fn refreshes_are_parallel_single_flight_and_late_results_only_fill_live_cache() {
    let (started, starts) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let gate = Arc::clone(&release);
    let shell = Arc::new(Shell::with_reader(
        Arc::new(move |s| {
            started.send(s.name).unwrap();
            let (lock, cv) = &*gate;
            let held = lock.lock().unwrap();
            let (held, timed) = cv
                .wait_timeout_while(held, Duration::from_secs(3), |open| !*open)
                .unwrap();
            assert!(*held && !timed.timed_out(), "fixture gate unreleased");
            parse(&value())
        }),
        Arc::new(Instant::now),
    ));
    let sources = vec![source("colab"), source("notes")];
    let before = Instant::now();
    let first = shell.snapshot(
        sources.clone(),
        "http://127.0.0.1:1",
        before + limits::SHELL_WAIT,
    );
    assert!(
        before.elapsed() < Duration::from_millis(800),
        "composition waited for the one-second callbacks"
    );
    assert_eq!(first["modules"].as_array().unwrap().len(), 1);
    let mut names = vec![
        starts.recv_timeout(Duration::from_secs(1)).unwrap(),
        starts.recv_timeout(Duration::from_secs(1)).unwrap(),
    ];
    names.sort();
    assert_eq!(names, ["colab", "notes"]);
    shell.snapshot(sources.clone(), "http://127.0.0.1:1", Instant::now());
    assert!(
        starts.try_recv().is_err(),
        "a second request stacked refreshes"
    );
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    let state = shell.shared.state.lock().unwrap();
    let (settled, timed) = shell
        .shared
        .changed
        .wait_timeout_while(state, Duration::from_secs(1), |s| {
            s.entries.values().any(|e| e.running.is_some())
        })
        .unwrap();
    assert!(!timed.timed_out());
    drop(settled);
    let answer = shell.snapshot(sources, "http://127.0.0.1:1", Instant::now());
    assert_eq!(answer["modules"].as_array().unwrap().len(), 3);
    shell.shutdown();
    assert!(shell.workers.lock().unwrap().is_empty());
    assert_eq!(
        shell.snapshot(vec![source("colab")], "http://127.0.0.1:1", Instant::now())["modules"],
        json!([])
    );
}

#[test]
fn every_collection_has_unique_ids_and_exact_boundaries() {
    let mut v = value();
    v["rail"]["path"] = json!(format!("/{}", "a".repeat(255)));
    for (key, cap, title) in [
        ("menu", 24, "label"),
        ("settings", 16, "label"),
        ("home", 8, "title"),
    ] {
        v[key] = Value::Array(
            (0..cap)
                .map(|i| {
                    let mut row = json!({"id":format!("item-{i}"),"path":"/"});
                    row[title] = json!("字".repeat(40));
                    row
                })
                .collect(),
        );
    }
    assert!(parse(&v).is_some());
    for key in ["menu", "settings", "home"] {
        let mut duplicate = v.clone();
        duplicate[key][1]["id"] = duplicate[key][0]["id"].clone();
        assert!(parse(&duplicate).is_none(), "duplicate {key} ID");
        let mut oversized = v.clone();
        let mut extra = oversized[key][0].clone();
        extra["id"] = json!("extra");
        oversized[key].as_array_mut().unwrap().push(extra);
        assert!(parse(&oversized).is_none(), "oversized {key}");
    }
    v["rail"]["path"] = json!(format!("/{}", "a".repeat(256)));
    assert!(parse(&v).is_none());
    for key in ["rail", "menu", "settings", "version"] {
        let mut absent = value();
        absent.as_object_mut().unwrap().remove(key);
        assert!(parse(&absent).is_none(), "missing {key}");
    }
    for pointer in ["/menu/0/count", "/menu/0/group", "/home"] {
        let mut null = value();
        *null.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(parse(&null).is_none(), "null {pointer}");
    }
}

#[test]
fn replaced_socket_fences_a_late_answer_and_shutdown_joins_the_worker() {
    let (started, starts) = mpsc::channel();
    let (released, release) = mpsc::channel();
    let release = Mutex::new(release);
    let shell = Arc::new(Shell::with_reader(
        Arc::new(move |s| {
            started.send(s.identity).unwrap();
            if s.identity == Some((1, 1)) {
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
            }
            let mut v = with_extra();
            v["rail"]["label"] = json!(if s.identity == Some((1, 1)) {
                "Old"
            } else {
                "New"
            });
            v["modules"][0]["rail"]["label"] = json!(if s.identity == Some((1, 1)) {
                "Old Design"
            } else {
                "New Design"
            });
            parse(&v)
        }),
        Arc::new(Instant::now),
    ));
    shell.snapshot(vec![source("colab")], "http://127.0.0.1:1", Instant::now());
    assert_eq!(
        starts.recv_timeout(Duration::from_secs(1)).unwrap(),
        Some((1, 1))
    );
    let mut replacement = source("colab");
    replacement.identity = Some((1, 2));
    shell.snapshot(
        vec![replacement.clone()],
        "http://127.0.0.1:1",
        Instant::now(),
    );
    released.send(()).unwrap();
    let state = shell.shared.state.lock().unwrap();
    let (settled, timed) = shell
        .shared
        .changed
        .wait_timeout_while(state, Duration::from_secs(1), |s| {
            s.entries["colab"].running.is_some()
        })
        .unwrap();
    assert!(!timed.timed_out());
    assert!(
        settled.entries["colab"].good.is_none(),
        "old socket answer was installed"
    );
    drop(settled);
    let answer = shell.snapshot(
        vec![replacement],
        "http://127.0.0.1:1",
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(answer["modules"][0]["rail"]["label"], "New");
    assert_eq!(
        starts.recv_timeout(Duration::from_secs(1)).unwrap(),
        Some((1, 2))
    );
    assert_eq!(answer["modules"][1]["rail"]["label"], "New Design");
    let removed = shell.snapshot(vec![], "http://127.0.0.1:1", Instant::now());
    assert_eq!(
        removed["modules"].as_array().unwrap().len(),
        1,
        "removal drops primary and extra together"
    );
    shell.shutdown();
    assert!(shell.workers.lock().unwrap().is_empty());
}

#[test]
fn unknown_count_keeps_the_module_and_null_or_over_bound_counts_refuse() {
    let known = parse(&value())
        .unwrap()
        .module("colab", "http://127.0.0.1:1/r/abc/x/colab/");
    assert_eq!(known["needsYou"], 2);
    let mut unknown = value();
    unknown.as_object_mut().unwrap().remove("needsYou");
    unknown["menu"][0].as_object_mut().unwrap().remove("count");
    let module = parse(&unknown)
        .expect("unknown counts must not remove a module")
        .module("colab", "http://127.0.0.1:1/r/abc/x/colab/");
    assert!(module.get("needsYou").is_none());
    assert!(module["menu"][0].get("count").is_none());
    assert_eq!(module["rail"]["label"], "Colab");
    unknown["needsYou"] = Value::Null;
    assert!(parse(&unknown).is_none(), "null is not omission");
    unknown["needsYou"] = json!(10000);
    assert!(parse(&unknown).is_none());
}
