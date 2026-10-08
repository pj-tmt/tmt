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
        lead_marker: None,
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
        colors: Default::default(),
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
fn provider_durations_preserve_accepted_numbers_units_and_ranges() {
    for (key, text, seconds) in [
        ("every", "10s", 10),
        ("every", "+10s", 10),
        ("every", "00010s", 10),
        ("every", "5m", 300),
        ("every", "+5m", 300),
        ("every", "1h", 3600),
        ("every", "+1h", 3600),
        ("every", "24h", 86400),
        ("every", "1440m", 86400),
        ("every", "86400s", 86400),
        ("timeout", "1s", 1),
        ("timeout", "+5s", 5),
        ("timeout", "030s", 30),
    ] {
        let body = format!("[squad.p.fields.x]\nrun = [\"gh\"]\n{key} = {text:?}\n");
        let parsed = providers(&body).unwrap();
        let actual = if key == "every" {
            parsed[0].every
        } else {
            parsed[0].timeout
        };
        assert_eq!(actual, Duration::from_secs(seconds), "{key} = {text}");
    }
}

#[test]
fn provider_non_ascii_duration_reports_the_setting_without_panicking() {
    // Multi-byte suffixes reproduce the old byte-index split panic.
    for key in ["every", "timeout"] {
        for text in ["5分", "5秒"] {
            let body = format!("[squad.p.fields.x]\nrun = [\"gh\"]\n{key} = {text:?}\n");
            let result = providers(&body).unwrap_err();
            assert_eq!(result.code, "SQUAD_CONFIG_INVALID");
            assert!(
                result.message.contains(&format!("squad.p.fields.x.{key}")),
                "{result}"
            );
        }
    }
}

#[test]
fn provider_mistakes_are_refused_with_their_place() {
    for (text, expected) in [
        (
            "[squad.p.fields.x]\nevery = \"1m\"\n",
            "`squad.p.fields.x` needs `run` or `preset",
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
            "[squad.p.fields.note]\nrun = [\"gh\"]\n",
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
            pr_state: None,
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
        pr_state: None,
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
    crate::test_support::write_ready_executable(&echo, "#!/bin/sh\nprintf '%s|' \"$@\"\n");
    let fail = dir.join("fail");
    crate::test_support::write_ready_executable(&fail, "#!/bin/sh\necho partial\nexit 3\n");
    let slow = dir.join("slow");
    crate::test_support::write_ready_executable(&slow, "#!/bin/sh\nsleep 5\necho late\n");
    let loud = dir.join("loud");
    crate::test_support::write_ready_executable(
        &loud,
        "#!/bin/sh\nhead -c 100000 /dev/zero | tr '\\0' x\n",
    );
    let job = |argv: &[&str], timeout: u64| Job {
        field: "f".into(),
        member: "R".into(),
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        timeout: Duration::from_secs(timeout),
        output: Output::Line,
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
                color: None,
                pr_state: None,
            },
            Outcome::Value {
                value: "on-path".into(),
                color: None,
                pr_state: None,
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
    crate::test_support::write_ready_executable(
        &program,
        &format!(
            "#!/bin/sh\necho x >> '{}'\necho \"open:$1\"\n",
            counter.display()
        ),
    );
    assert!(
        !counter.exists(),
        "readiness does not run the provider payload"
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

#[test]
fn the_github_preset_reads_a_pull_request_from_pr_link() {
    let read = providers("[squad.p.fields.pr]\npreset = \"github-pr\"\nevery = \"2m\"\n").unwrap();
    assert_eq!(read[0].output, Output::GithubPr);
    assert_eq!(read[0].every, Duration::from_secs(120));
    let members = vec![
        member("R", &[("pr_link", "https://github.com/acme/app/pull/412")]),
        member("S", &[]),
    ];
    let jobs = due(&read, &members, &Cache::at(None), 0);
    assert_eq!(jobs.len(), 1, "no pr_link, no run");
    assert_eq!(
        jobs[0].argv,
        [
            "gh",
            "pr",
            "view",
            "https://github.com/acme/app/pull/412",
            "--json",
            "number,state,isDraft,reviewDecision"
        ]
    );
    for (text, expected) in [
        (
            "[squad.p.fields.pr]\npreset = \"gitlab\"\n",
            "must be \"github-pr\"",
        ),
        (
            "[squad.p.fields.pr]\npreset = \"github-pr\"\nrun = [\"gh\"]\n",
            "sets both `run` and `preset`",
        ),
        (
            "[squad.p.fields.pr]\nevery = \"1m\"\n",
            "needs `run` or `preset",
        ),
    ] {
        let message = error(text);
        assert!(message.contains(expected), "{text}: {message}");
    }
}

#[test]
fn github_output_becomes_number_state_and_review() {
    let value = |text: &str| match github_pr(text.as_bytes()) {
        Outcome::Value { value, .. } => value,
        other => panic!("{text}: {other:?}"),
    };
    let pr = |state: &str, draft: bool, review: &str| {
        format!(
            r#"{{"number":412,"state":"{state}","isDraft":{draft},"reviewDecision":"{review}"}}"#
        )
    };
    assert_eq!(value(&pr("OPEN", false, "")), "#412 open");
    assert_eq!(value(&pr("OPEN", true, "")), "#412 draft");
    assert_eq!(
        value(&pr("OPEN", false, "APPROVED")),
        "#412 open · approved"
    );
    assert_eq!(
        value(&pr("OPEN", false, "CHANGES_REQUESTED")),
        "#412 open · changes requested"
    );
    assert_eq!(
        value(&pr("OPEN", false, "REVIEW_REQUIRED")),
        "#412 open · review required"
    );
    assert_eq!(
        value(&pr("MERGED", false, "APPROVED")),
        "#412 merged",
        "review only while open"
    );
    assert_eq!(value(&pr("CLOSED", false, "")), "#412 closed");
    for (state, draft, expected) in [
        ("OPEN", false, "open"),
        ("OPEN", true, "draft"),
        ("MERGED", false, "merged"),
        ("CLOSED", false, "closed"),
    ] {
        let Outcome::Value { pr_state, .. } = github_pr(pr(state, draft, "").as_bytes()) else {
            panic!("valid github JSON must produce a value");
        };
        assert_eq!(pr_state.as_deref(), Some(expected));
    }
    for bad in [
        "",
        "not json",
        r#"{"state":"OPEN"}"#,
        &pr("LOCKED", false, ""),
    ] {
        assert_eq!(github_pr(bad.as_bytes()), Outcome::Failed, "{bad}");
    }
}

/// gh missing, logged out (exit 4) or printing something else is a failed
/// run, shown as `?`; its output is read only after a successful exit.
#[test]
fn the_github_preset_degrades_to_a_failed_run() {
    let dir = scratch("gh");
    let fake = |name: &str, script: &str| {
        let path = dir.join(name);
        crate::test_support::write_ready_executable(&path, script);
        path.to_str().unwrap().to_owned()
    };
    let ok = fake(
        "ok",
        "#!/bin/sh\necho '{\"number\":7,\"state\":\"OPEN\",\"isDraft\":false,\"reviewDecision\":\"APPROVED\"}'\n",
    );
    let logged_out = fake(
        "out",
        "#!/bin/sh\necho 'To get started with GitHub CLI, please run:  gh auth login' >&2\nexit 4\n",
    );
    let job = |program: String| Job {
        field: "pr".into(),
        member: "R".into(),
        argv: vec![program, "https://github.com/acme/app/pull/7".into()],
        timeout: Duration::from_secs(5),
        output: Output::GithubPr,
    };
    assert_eq!(
        run(&[
            job(ok),
            job(logged_out),
            job(dir.join("gh").to_str().unwrap().to_owned())
        ]),
        [
            Outcome::Value {
                value: "#7 open · approved".into(),
                color: None,
                pr_state: Some("open".into()),
            },
            Outcome::Failed,
            Outcome::Failed,
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_current_successful_github_preset_states_are_activity_evidence() {
    let providers =
        providers("[squad.p.fields.pr_state]\npreset = \"github-pr\"\nevery = \"1m\"\n").unwrap();
    let linked = member("R", &[("pr_link", "https://example.com/pull/1")]);
    let jobs = due(
        &providers,
        std::slice::from_ref(&linked),
        &Cache::at(None),
        1000,
    );
    let mut cache = Cache::at(None);
    let mut parsed = github_pr(br#"{"number":1,"state":"OPEN","isDraft":false}"#);
    if let Outcome::Value { value, .. } = &mut parsed {
        *value = "display format changed completely".into();
    }
    cache.record(&jobs[0], &parsed, 1000);
    assert_eq!(
        cache
            .github_pr_states(&providers, &linked, 2000)
            .get("pr_state")
            .map(String::as_str),
        Some("open")
    );
    assert!(cache.github_pr_states(&providers, &linked, 999).is_empty());
    assert!(
        cache
            .github_pr_states(&providers, &linked, 61001)
            .is_empty()
    );
    let relinked = member("R", &[("pr_link", "https://example.com/pull/2")]);
    assert!(
        cache
            .github_pr_states(&providers, &relinked, 2000)
            .is_empty()
    );
    cache.record(&jobs[0], &Outcome::Failed, 2000);
    assert!(cache.github_pr_states(&providers, &linked, 2000).is_empty());
    let text = gh();
    let jobs = due(&text, std::slice::from_ref(&linked), &Cache::at(None), 1000);
    cache.record(
        &jobs[0],
        &Outcome::Value {
            value: "#1 merged".into(),
            color: None,
            pr_state: None,
        },
        1000,
    );
    assert!(cache.github_pr_states(&text, &linked, 2000).is_empty());
}

/// A provider's color is untrusted output: a theme token's name suggests a
/// color, anything else none, and a failed or empty run clears it. The value
/// is shown either way.
#[test]
fn a_provider_suggests_a_color_only_by_a_theme_tokens_name() {
    let dir = scratch("colors");
    let mut cache = Cache::at(Some(dir.join("p.json")));
    let providers = gh();
    let linked = member("R", &[("pr_link", "https://example.com/pull/1")]);
    let job = due(&providers, std::slice::from_ref(&linked), &cache, 0).remove(0);
    let shown_with = |cache: &Cache| {
        let mut shown = vec![linked.clone()];
        apply(&providers, &mut shown, cache);
        shown.remove(0)
    };
    for (color, expected) in [
        (Some("review"), Some("review")),
        (Some("magenta"), Some("magenta")),
        (Some("#ff0000"), None),
        (Some("\u{1b}[31m"), None),
        (Some("default"), None),
        (Some("orange"), None),
        (None, None),
    ] {
        cache.record(
            &job,
            &Outcome::Value {
                value: "OPEN".into(),
                color: color.map(str::to_owned),
                pr_state: None,
            },
            1_000,
        );
        let shown = shown_with(&cache);
        assert_eq!(shown.fields["pr_state"], "OPEN", "{color:?}");
        assert_eq!(
            shown.colors.get("pr_state").map(String::as_str),
            expected,
            "{color:?}"
        );
    }
    cache.record(&job, &Outcome::Failed, 2_000);
    assert!(
        shown_with(&cache).colors.is_empty(),
        "a failed run has no color"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn github_pr_never_schedules_a_missing_or_empty_link() {
    let read = providers("[squad.p.fields.pr]\npreset = \"github-pr\"\n").unwrap();
    let members = [member("missing", &[]), member("empty", &[("pr_link", "")])];
    assert!(due(&read, &members, &Cache::at(None), 1_000).is_empty());
}
