use super::*;
use std::collections::BTreeMap;

fn providers(text: &str) -> Result<Vec<Provider>, SquadError> {
    let config: toml_edit::DocumentMut = text.parse().unwrap();
    read(
        config["squad"]["p"].as_table_like(),
        "p",
        crate::rows::field_name,
        |field| crate::rows::OWN_FIELDS.contains(&field),
    )
}

fn error(text: &str) -> String {
    let error = providers(text).expect_err(text);
    assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{text}");
    error.message
}

fn member(id: &str, fields: &[(&str, &str)]) -> Member {
    Member {
        id: id.into(),
        name: id.to_lowercase(),
        lifetime: "saved".into(),
        presence: "active".into(),
        pane: Value::Null,
        activity: Value::Null,
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
        meta: BTreeMap::new(),
        seen: Value::Null,
        numbers: BTreeMap::new(),
        failed: Default::default(),
    }
}

/// A scratch directory with a fake program in it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("squad-provider-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_provider_is_a_program_with_bounded_timing() {
    let read = providers(
        "[squad.p.fields.pr_state]\nrun = [\"gh\", \"pr\", \"view\", \"{pr_link}\", \"--json\", \"state\"]\n\
         every = \"5m\"\ntimeout = \"2s\"\n\
         [squad.p.fields.ctx]\nrun = [\"/usr/local/bin/ctx\", \"{member}\"]\n",
    )
    .unwrap();
    let names: Vec<&str> = read.iter().map(|provider| provider.name.as_str()).collect();
    assert_eq!(names, ["pr_state", "ctx"]);
    assert_eq!(read[0].every, Duration::from_secs(300));
    assert_eq!(read[0].timeout, Duration::from_secs(2));
    assert_eq!(
        (read[1].every, read[1].timeout),
        (DEFAULT_EVERY, DEFAULT_TIMEOUT)
    );
    assert!(
        providers("[squad.p]\nlayout = \"crew\"\n")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn provider_mistakes_are_refused_with_their_place() {
    for (text, expected) in [
        (
            "[squad.p.fields.x]\nevery = \"1m\"\n",
            "`squad.p.fields.x.run` is required",
        ),
        (
            "[squad.p.fields.x]\nrun = []\n",
            "must be a program and its arguments",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"bin/gh\"]\n",
            "literal program name on PATH or an absolute path",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"{prog}\"]\n",
            "literal program name",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"gh\", \"{Bad}\"]\n",
            "is not a field name",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"gh\"]\nevery = \"5s\"\n",
            "`squad.p.fields.x.every` must be 10s-86400s",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"gh\"]\ntimeout = \"1m\"\n",
            "`squad.p.fields.x.timeout` must be 1s-30s",
        ),
        (
            "[squad.p.fields.x]\nrun = [\"gh\"]\nshell = true\n",
            "not a field provider setting",
        ),
        (
            "[squad.p.fields.state]\nrun = [\"gh\"]\n",
            "a field Squad reads itself",
        ),
        (
            "[squad.p.fields.Bad]\nrun = [\"gh\"]\n",
            "must be named like a field",
        ),
        ("[squad.p]\nfields = 3\n", "must be a table of fields"),
    ] {
        let message = error(text);
        assert!(message.contains(expected), "{text}: {message}");
    }
    let many: String = (0..=MAX_PROVIDERS)
        .map(|index| format!("[squad.p.fields.f{index}]\nrun = [\"gh\"]\n"))
        .collect();
    assert!(error(&many).contains("at most 8"));
}

fn gh() -> Vec<Provider> {
    providers("[squad.p.fields.pr_state]\nrun = [\"gh\", \"pr\", \"view\", \"{pr_link}\"]\nevery = \"1m\"\n")
        .unwrap()
}

/// Runs only what is due; a new `{pr_link}` is due at once and never shows
/// the old link's value; a member without the field, or whose value would
/// be an option, never runs.
#[test]
fn due_runs_follow_the_argv_and_the_interval() {
    let dir = scratch("due");
    let mut cache = Cache::at(Some(dir.join("p.json")));
    let providers = gh();
    let linked = member("R", &[("pr_link", "https://example.com/pull/1")]);
    let members = vec![
        linked.clone(),
        member("S", &[]),
        member("T", &[("pr_link", "--repo=evil/x")]),
    ];
    let jobs = due(&providers, &members, &cache, 1_000);
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!(jobs[0].member, "R");
    assert_eq!(
        jobs[0].argv,
        ["gh", "pr", "view", "https://example.com/pull/1"]
    );

    cache.record(
        &jobs[0],
        &Outcome::Value {
            value: "OPEN".into(),
            color: None,
        },
        1_000,
    );
    assert!(
        due(&providers, &members, &cache, 60_999).is_empty(),
        "fresh"
    );
    assert_eq!(
        due(&providers, &members, &cache, 61_000).len(),
        1,
        "every elapsed"
    );

    let mut shown = members.clone();
    apply(&providers, &mut shown, &cache);
    assert_eq!(shown[0].fields["pr_state"], "OPEN");
    assert!(!shown[1].fields.contains_key("pr_state"));
    assert!(
        !shown[2].fields.contains_key("pr_state"),
        "refused, never run"
    );

    // A new link: due now, and the old value is not shown for it.
    let relinked = vec![member("R", &[("pr_link", "https://example.com/pull/2")])];
    assert_eq!(due(&providers, &relinked, &cache, 2_000).len(), 1);
    let mut shown = relinked.clone();
    apply(&providers, &mut shown, &cache);
    assert!(!shown[0].fields.contains_key("pr_state"));

    // A failed run shows `?`, marked failed; an empty one shows nothing.
    cache.record(&jobs[0], &Outcome::Failed, 3_000);
    let mut shown = vec![linked.clone()];
    apply(&providers, &mut shown, &cache);
    assert_eq!(shown[0].fields["pr_state"], "?");
    assert!(shown[0].failed.contains("pr_state"));
    cache.record(&jobs[0], &Outcome::Empty, 4_000);
    let mut shown = vec![linked];
    apply(&providers, &mut shown, &cache);
    assert!(!shown[0].fields.contains_key("pr_state"));
    assert!(shown[0].failed.is_empty());

    // The cache survives a reload; corrupt content reads as empty.
    cache.save().unwrap();
    assert!(
        Cache::at(Some(dir.join("p.json")))
            .entry("pr_state", "R")
            .is_some()
    );
    std::fs::write(dir.join("p.json"), "{not json").unwrap();
    assert!(Cache::at(Some(dir.join("p.json"))).document.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn output_is_one_clean_line_or_a_value_with_a_color_token() {
    let value = |value: &str, color: Option<&str>| Outcome::Value {
        value: value.into(),
        color: color.map(str::to_owned),
    };
    assert_eq!(outcome(b"OPEN\n"), value("OPEN", None));
    assert_eq!(outcome(b"  first\nsecond\n"), value("first", None));
    assert_eq!(
        outcome(b"a\x1b[31mb\x07"),
        value("a[31mb", None),
        "no escapes reach the board"
    );
    assert_eq!(
        outcome(br#"{"value": "487k", "color": "review"}"#),
        value("487k", Some("review"))
    );
    assert_eq!(
        outcome(br#"{"value": 12, "color": "Not A Token"}"#),
        value("12", None)
    );
    assert_eq!(outcome(b"\n"), Outcome::Empty);
    assert_eq!(outcome(br#"{"color": "review"}"#), Outcome::Empty);
    let long = "x".repeat(500);
    assert_eq!(
        outcome(long.as_bytes()),
        value(&"x".repeat(VALUE_LIMIT), None)
    );
}

/// Real programs: found on PATH or by absolute path, never through a
/// shell, with each argument intact; a failure, a timeout or too much
/// output is a failed run.
#[test]
fn programs_run_directly_and_every_failure_is_a_failed_run() {
    let dir = scratch("run");
    let echo = dir.join("echo-args");
    crate::test_support::write_executable(&echo, "#!/bin/sh\nprintf '%s|' \"$@\"\n");
    let fail = dir.join("fail");
    crate::test_support::write_executable(&fail, "#!/bin/sh\necho partial\nexit 3\n");
    let slow = dir.join("slow");
    crate::test_support::write_executable(&slow, "#!/bin/sh\nsleep 5\necho late\n");
    let loud = dir.join("loud");
    crate::test_support::write_executable(
        &loud,
        "#!/bin/sh\nhead -c 100000 /dev/zero | tr '\\0' x\n",
    );
    let job = |argv: &[&str], timeout: u64| Job {
        field: "f".into(),
        member: "R".into(),
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        timeout: Duration::from_secs(timeout),
    };
    let echo = echo.to_str().unwrap();
    let jobs = [
        job(&[echo, "a b", "$(id)", "; rm -rf ~"], 5),
        job(&["sh", "-c", "echo on-path"], 5),
        job(&[fail.to_str().unwrap()], 5),
        job(&[slow.to_str().unwrap()], 1),
        job(&[loud.to_str().unwrap()], 5),
        job(&[dir.join("missing").to_str().unwrap()], 5),
    ];
    let started = std::time::Instant::now();
    let outcomes = run(&jobs);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the timeout bounds a slow program"
    );
    assert_eq!(
        outcomes,
        [
            Outcome::Value {
                value: "a b|$(id)|; rm -rf ~|".into(),
                color: None
            },
            Outcome::Value {
                value: "on-path".into(),
                color: None
            },
            Outcome::Failed,
            Outcome::Failed,
            Outcome::Failed,
            Outcome::Failed,
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A refresh runs what is due, saves it, and a second one within `every`
/// runs nothing.
#[test]
fn a_refresh_saves_its_values_and_does_not_repeat_fresh_work() {
    let dir = scratch("refresh");
    let counter = dir.join("count");
    let program = dir.join("state");
    crate::test_support::write_executable(
        &program,
        &format!(
            "#!/bin/sh\necho x >> '{}'\necho \"open:$1\"\n",
            counter.display()
        ),
    );
    let providers = providers(&format!(
        "[squad.p.fields.pr_state]\nrun = [\"{}\", \"{{pr_link}}\"]\n",
        program.display()
    ))
    .unwrap();
    let members = vec![
        member("R", &[("pr_link", "https://example.com/pull/1")]),
        member("S", &[("pr_link", "https://example.com/pull/2")]),
    ];
    let path = Some(dir.join("p.json"));
    assert_eq!(
        refresh_in(Cache::at(path.clone()), &providers, &members, 1_000),
        2
    );
    assert_eq!(
        refresh_in(Cache::at(path.clone()), &providers, &members, 2_000),
        0
    );
    assert_eq!(
        std::fs::read_to_string(&counter).unwrap().lines().count(),
        2
    );
    let mut shown = members.clone();
    apply(&providers, &mut shown, &Cache::at(path));
    assert_eq!(
        shown[0].fields["pr_state"],
        "open:https://example.com/pull/1"
    );
    assert_eq!(
        shown[1].fields["pr_state"],
        "open:https://example.com/pull/2"
    );
    let _ = std::fs::remove_dir_all(dir);
}
