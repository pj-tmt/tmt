use super::*;
use crate::invocation::ListScope;
use tmt_core::{
    binding::{
        Binding,
        session::{
            BindingSessionState, HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeMode,
        },
    },
    endpoint::{PaneLocation, ProcessIncarnation, ServerEvidence},
};

const CLAUDE: &str = "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f";
const CODEX: &str = "019a2f4c-1111-2222-3333-444455556666";

fn identity(name: &str, lifetime: Lifetime) -> Identity {
    Identity {
        id: format!("id-{name}"),
        name: name.into(),
        canonical_name: name.to_lowercase(),
        lifetime,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn pane(id: &str, command: &str, cwd: &str) -> PaneObservation {
    PaneObservation {
        agent_name: None,
        id: id.into(),
        target: Some(format!("work:1.{}", id.trim_start_matches('%'))),
        cwd: Some(cwd.into()),
        command: command.into(),
        pane_pid: 10,
        pane_incarnation: None,
        suggested_name: None,
        marker: None,
        location: None,
    }
}

/// A binding whose runtime is `state`, observing `session` when given.
fn binding(state: RuntimeState, session: Option<&str>) -> Binding {
    Binding {
        id: "binding".into(),
        identity_id: "identity".into(),
        server: ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux".into(),
            server_pid: 1,
            server_start_time: "0".into(),
        },
        pane_id: "%1".into(),
        pane_pid: 10,
        pane_incarnation: None,
        session: BindingSessionState {
            last_transition: None,
            state,
            key: session.map(|session| ObservedSessionKey {
                incarnation: ProcessIncarnation::new(10, "start").unwrap(),
                provider_session: Some(ProviderSessionId::new(session).unwrap()),
            }),
            launch_owner: None,
        },
    }
}

fn remembered(harness: &str, session: &str, stale: bool) -> RememberedSession {
    RememberedSession {
        harness: HarnessId::new(harness).unwrap(),
        mode: RuntimeMode::new("interactive").unwrap(),
        provider_session: ProviderSessionId::new(session).unwrap(),
        state: None,
        stale_at_ms: stale.then_some(1),
        resume_pending_at_ms: None,
    }
}

fn row(
    name: &str,
    lifetime: Lifetime,
    live: Option<(PaneObservation, Binding)>,
    remembered: Option<RememberedSession>,
) -> ListedRow {
    let presence = if live.is_some() {
        Presence::Active
    } else {
        Presence::Offline
    };
    let (pane, binding) = live.map_or((None, None), |(pane, binding)| (Some(pane), Some(binding)));
    ListedRow {
        activity: serde_json::json!({"state":"unknown", "sinceMs":null, "lastActivityMs":null, "providers":{}}),
        presence: IdentityPresence {
            identity: identity(name, lifetime),
            presence,
            pane,
            binding,
        },
        remembered,
        resume: None,
        running_driver: None,
        driver_support: None,
    }
}

/// The issue's example: saved and temporary agents, a resumable offline
/// identity, an idle shell pane, a stale session and one to fold.
fn agents() -> Vec<ListedRow> {
    vec![
        row(
            "opus-tmt-peer-2",
            Lifetime::Saved,
            Some((
                pane("%12", "2.1.283", "/Users/ada/dev/tmt-local"),
                binding(RuntimeState::Running, Some(CLAUDE)),
            )),
            Some(remembered("claude", CLAUDE, false)),
        ),
        row(
            "astra",
            Lifetime::Saved,
            Some((
                pane("%7", "codex", "/Users/ada/dev/tmt-local"),
                binding(RuntimeState::Running, Some(CODEX)),
            )),
            Some(remembered("codex", CODEX, false)),
        ),
        row(
            "sol",
            Lifetime::Saved,
            None,
            Some(remembered(
                "claude",
                "3f9a1c07-aaaa-bbbb-cccc-dddd0000ffff",
                false,
            )),
        ),
        row("gemini", Lifetime::Saved, None, None),
        row("codex-old", Lifetime::Saved, None, None),
        row(
            "mamezu",
            Lifetime::Temporary,
            None,
            Some(remembered(
                "codex",
                "01a9c3b8-0000-1111-2222-333344445555",
                true,
            )),
        ),
        row(
            "opus-1",
            Lifetime::Temporary,
            Some((
                pane("%31", "-zsh", "/srv/builds/nightly"),
                binding(RuntimeState::Unknown, None),
            )),
            None,
        ),
    ]
}

/// Rendered against the fixture's home, `/Users/ada`.
fn render(rows: Vec<ListedRow>, terminal: Terminal, all: bool) -> String {
    if terminal.color {
        // Production anstream honors NO_COLOR before enabling color; these explicit
        // color fixtures override crossterm's cached choice.
        static COLOR: std::sync::Once = std::sync::Once::new();
        COLOR.call_once(|| crossterm::style::force_color_output(true));
    }
    let mut output = Vec::new();
    write_listing(
        &mut output,
        terminal,
        &rows,
        all,
        Some(Path::new("/Users/ada")),
    )
    .unwrap();
    String::from_utf8(output)
        .unwrap()
        .replace('\u{1b}', "\\u{1b}")
}

#[test]
fn the_shell_set_decides_between_an_idle_pane_and_an_unmanaged_agent() {
    for shell in [
        "sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "nu", "-zsh",
    ] {
        assert!(is_shell(shell), "{shell}");
    }
    for agent in ["2.1.283", "codex", "node", "tmt", "claude", "vim"] {
        assert!(!is_shell(agent), "{agent}");
    }
    let unmanaged = row(
        "plain",
        Lifetime::Saved,
        Some((
            pane("%3", "2.1.283", "/tmp"),
            binding(RuntimeState::Unknown, None),
        )),
        None,
    );
    assert_eq!(state(&unmanaged.presence), State::Running);
    assert_eq!(
        address(&unmanaged.presence, None).unwrap().full(),
        "tmux:%3"
    );
    let idle = row(
        "idle",
        Lifetime::Saved,
        Some((
            pane("%4", "zsh", "/tmp"),
            binding(RuntimeState::Ended, None),
        )),
        None,
    );
    assert_eq!(state(&idle.presence), State::Shell);
    assert_eq!(action(&idle.presence, None).as_deref(), Some("shell"));
    // A shell running under `tmt run` is still a running agent.
    let wrapped = row(
        "wrapped",
        Lifetime::Saved,
        Some((
            pane("%5", "zsh", "/tmp"),
            binding(RuntimeState::Running, None),
        )),
        None,
    );
    assert_eq!(state(&wrapped.presence), State::Running);
}

#[test]
fn the_address_is_the_driver_session_only_when_the_binding_observed_it() {
    let codex = remembered("codex", CODEX, false);
    // Codex has not disclosed its thread yet: the pane, never a guess.
    let undisclosed = row(
        "astra",
        Lifetime::Saved,
        Some((
            pane("%7", "codex", "/tmp"),
            binding(RuntimeState::Running, None),
        )),
        Some(codex.clone()),
    );
    assert_eq!(
        address(&undisclosed.presence, Some(&codex)).unwrap().full(),
        "tmux:%7"
    );
    let disclosed = row(
        "astra",
        Lifetime::Saved,
        Some((
            pane("%7", "codex", "/tmp"),
            binding(RuntimeState::Running, Some(CODEX)),
        )),
        Some(codex.clone()),
    );
    let address = address(&disclosed.presence, Some(&codex)).unwrap();
    assert_eq!(address.full(), format!("codex:{CODEX}"));
    assert_eq!(address.short(), "codex:019a2f4c");
    // Offline: the remembered session, or nothing.
    let offline = row("sol", Lifetime::Saved, None, Some(codex.clone()));
    assert_eq!(
        super::address(&offline.presence, Some(&codex))
            .unwrap()
            .driver,
        "codex"
    );
    let unknown = row("gemini", Lifetime::Saved, None, None);
    assert_eq!(super::address(&unknown.presence, None), None);
}

#[test]
fn actions_resume_only_stopped_agents_and_mark_stale_sessions() {
    let rows = agents();
    let actions: Vec<_> = rows
        .iter()
        .map(|row| action(&row.presence, row.remembered.as_ref()))
        .collect();
    assert_eq!(
        actions,
        [
            None,
            None,
            Some("↻ tmt resume sol".into()),
            None,
            None,
            Some("stale".into()),
            Some("shell".into()),
        ]
    );
}

/// The action is a command the user pastes: the name stays exactly one operand, and a name that
/// starts with a dash follows `--` so it cannot be read as an option.
#[test]
fn the_resume_action_keeps_each_name_as_one_operand() {
    let resume = |name: &str| {
        let row = row(
            name,
            Lifetime::Saved,
            None,
            Some(remembered("claude", CLAUDE, false)),
        );
        action(&row.presence, row.remembered.as_ref())
    };
    for (name, expected) in [
        ("sol", "↻ tmt resume sol"),
        ("my agent", "↻ tmt resume 'my agent'"),
        ("o'brien", "↻ tmt resume 'o'\\''brien'"),
        ("-dash", "↻ tmt resume -- -dash"),
        ("-a b", "↻ tmt resume -- '-a b'"),
    ] {
        assert_eq!(resume(name).as_deref(), Some(expected), "{name}");
    }
}

#[test]
fn the_list_through_a_pipe() {
    insta::assert_snapshot!(render(agents(), Terminal::PLAIN, false));
}

#[test]
fn the_list_with_all_offline_identities() {
    insta::assert_snapshot!(render(agents(), Terminal::PLAIN, true));
}

#[test]
fn the_list_on_a_narrow_terminal() {
    let narrow = Terminal {
        color: true,
        width: Some(40),
        theme: None,
    };
    insta::assert_snapshot!(render(agents(), narrow, false));
}

#[test]
fn an_empty_list_says_so_and_how_to_name_an_agent() {
    assert_eq!(
        render(Vec::new(), Terminal::PLAIN, false),
        "No identities found.\nhint: tmt name <name>\n"
    );
}

#[test]
fn json_rows_gain_address_driver_and_activity() {
    let rows = agents();
    let before: Vec<Value> = rows
        .iter()
        .map(|row| presence_document(&row.presence))
        .collect();
    let document = document(&Report::Listed {
        rows,
        scope: ListScope::default(),
    });
    for (row, mut original) in document["identities"]
        .as_array()
        .unwrap()
        .iter()
        .zip(before)
    {
        let mut row = row.clone();
        let object = row.as_object_mut().unwrap();
        let address = object.remove("address").unwrap();
        let driver = object.remove("driver").unwrap();
        assert_eq!(
            object.remove("session").unwrap(),
            serde_json::json!({"activity":{"state":"unknown", "sinceMs":null, "lastActivityMs":null, "providers":{}}})
        );
        assert_eq!(row, original.take());
        assert_eq!(
            address.is_null(),
            driver.is_null(),
            "address and driver come together"
        );
    }
    assert_eq!(
        document["identities"][0]["address"],
        format!("claude:{CLAUDE}")
    );
    assert_eq!(document["identities"][6]["address"], "tmux:%31");
    assert_eq!(document["identities"][3]["address"], Value::Null);
}

fn running_row(driver: &str, session: &str) -> ListedRow {
    let mut row = row(
        "worker",
        Lifetime::Saved,
        Some((
            pane("%1", driver, "/tmp"),
            binding(RuntimeState::Running, Some(session)),
        )),
        Some(remembered(driver, session, false)),
    );
    row.presence.binding.as_mut().unwrap().identity_id = row.presence.identity.id.clone();
    row
}

/// Exercise the projection with the command's already-observed runtime, while
/// comparing every existing JSON field before and after the additive field.
fn running_document(mut row: ListedRow, runtime: RuntimeState) -> Value {
    let baseline = document(&Report::Listed {
        rows: vec![ListedRow {
            presence: row.presence.clone(),
            remembered: row.remembered.clone(),
            resume: row.resume.clone(),
            activity: row.activity.clone(),
            running_driver: None,
            driver_support: None,
        }],
        scope: ListScope::default(),
    });
    row.running_driver = running_driver(
        &row.presence,
        row.remembered.as_ref(),
        runtime,
        &tmt_adapters::runtime::RuntimeRegistry::first_party(),
    );
    let projected = document(&Report::Listed {
        rows: vec![row],
        scope: ListScope::default(),
    });
    let mut existing = projected.clone();
    existing["identities"][0]
        .as_object_mut()
        .unwrap()
        .remove("runningDriver");
    assert_eq!(existing, baseline, "existing list fields must not change");
    projected["identities"][0].clone()
}

#[test]
fn running_driver_needs_no_activity_telemetry_or_stored_running_claim() {
    for (driver, session) in [("claude", CLAUDE), ("codex", CODEX)] {
        for stored in [
            RuntimeState::Running,
            RuntimeState::Unknown,
            RuntimeState::Ended,
        ] {
            let mut row = running_row(driver, session);
            row.presence.binding.as_mut().unwrap().session.state = stored;
            let value = running_document(row, RuntimeState::Running);
            assert_eq!(value["runningDriver"], driver);
            assert_eq!(value["session"]["activity"]["state"], "unknown");
        }
    }
}

/// A live row whose driver stored `usage`, with `resume` projected the way the
/// list command does it, from the stored state alone.
fn running_row_with_usage(
    driver: &str,
    session: &str,
    usage: Option<tmt_adapters::runtime::driver_state::Usage>,
) -> ListedRow {
    let mut row = running_row(driver, session);
    row.remembered.as_mut().unwrap().state = usage.and_then(|usage| {
        tmt_adapters::runtime::driver_state::after_observation(Some(usage), None, None)
    });
    row.resume = crate::output::resume_document(
        &tmt_core::binding::session::SessionPreferences {
            channel: None,
            preferred_harness: None,
            remembered: row.remembered.clone(),
        },
        &tmt_adapters::runtime::RuntimeRegistry::first_party(),
    );
    row
}

#[test]
fn a_live_row_pairs_running_driver_with_its_stored_context_use() {
    use tmt_adapters::runtime::driver_state::Usage;
    // Codex reports its window; Claude does not, so the key is absent, never null.
    let codex = running_row_with_usage(
        "codex",
        CODEX,
        Some(Usage::new(146_577, Some(258_400), 1_000).unwrap()),
    );
    let value = running_document(codex, RuntimeState::Running);
    assert_eq!(value["runningDriver"], "codex");
    assert_eq!(
        value["resume"]["usage"],
        json!({"tokens":146_577,"windowTokens":258_400,"observedAtMs":1_000})
    );
    let claude = running_row_with_usage(
        "claude",
        CLAUDE,
        Some(Usage::new(195_664, None, 2_000).unwrap()),
    );
    let value = running_document(claude, RuntimeState::Running);
    assert_eq!(value["runningDriver"], "claude");
    assert_eq!(
        value["resume"]["usage"],
        json!({"tokens":195_664,"observedAtMs":2_000})
    );
    // The same stored usage on an ended row has no `runningDriver`: consumers
    // show a context figure only beside that fence.
    let ended = running_row_with_usage(
        "codex",
        CODEX,
        Some(Usage::new(146_577, Some(258_400), 1_000).unwrap()),
    );
    let value = running_document(ended, RuntimeState::Ended);
    assert!(value.get("runningDriver").is_none());
    // A driver that stored nothing has no figure to show, not a zero.
    let value = running_document(
        running_row_with_usage("claude", CLAUDE, None),
        RuntimeState::Running,
    );
    assert_eq!(value["runningDriver"], "claude");
    assert!(value["resume"].get("usage").is_none());
}

#[test]
fn ls_and_list_json_select_the_same_collection_projection() {
    let parse =
        |command: &str| crate::parser::parse_core(&[command.into(), "--json".into()]).unwrap();
    let ls = parse("ls");
    let list = parse("list");
    assert_eq!(ls.invocation, list.invocation);
    assert!(ls.mode.json && list.mode.json);
    assert_eq!(
        ls.invocation,
        crate::invocation::Invocation::List {
            target: None,
            room: None,
            scope: ListScope::default(),
        }
    );
}

#[test]
fn ended_or_stopped_runtime_in_a_live_owned_pane_omits_running_driver() {
    // A remembered, stored-Running session can still name the old driver after
    // the process ends, or while stopped-process evidence is Unknown.
    for runtime in [RuntimeState::Ended, RuntimeState::Unknown] {
        let value = running_document(running_row("codex", CODEX), runtime);
        assert_eq!(value["presence"], "active");
        assert_eq!(value["driver"], "codex");
        assert_eq!(value["address"], format!("codex:{CODEX}"));
        assert!(value.get("runningDriver").is_none());
    }
}

#[test]
fn missing_mismatched_and_historical_evidence_omits_running_driver() {
    type RowChange = fn(&mut ListedRow);
    let cases: &[(&str, RowChange)] = &[
        ("undisclosed", |row| {
            row.presence
                .binding
                .as_mut()
                .unwrap()
                .session
                .key
                .as_mut()
                .unwrap()
                .provider_session = None
        }),
        ("unadmitted", |row| {
            row.presence.binding.as_mut().unwrap().session.key = None
        }),
        ("mismatched", |row| {
            row.remembered.as_mut().unwrap().provider_session =
                ProviderSessionId::new("other-session").unwrap()
        }),
        ("not remembered", |row| row.remembered = None),
        ("unregistered", |row| {
            row.remembered.as_mut().unwrap().harness = HarnessId::new("unregistered").unwrap()
        }),
        ("offline historical binding", |row| {
            row.presence.presence = Presence::Offline
        }),
        ("unknown presence", |row| {
            row.presence.presence = Presence::Unknown
        }),
        ("no current binding", |row| row.presence.binding = None),
        ("no live pane", |row| row.presence.pane = None),
    ];
    for (label, change) in cases {
        let mut row = running_row("codex", CODEX);
        change(&mut row);
        assert!(
            running_document(row, RuntimeState::Running)
                .get("runningDriver")
                .is_none(),
            "{label}"
        );
    }
}

#[test]
fn activity_projection_requires_current_scope_and_runtime_evidence() {
    use tmt_core::binding::session::activity::Event;
    let registry = tmt_adapters::runtime::RuntimeRegistry::first_party();
    let mut binding = binding(RuntimeState::Running, Some(CLAUDE));
    let mut remembered = remembered("claude", CLAUDE, false);
    let key = binding.session.key.as_ref().unwrap();
    remembered.state = tmt_adapters::runtime::driver_state::after_activity(
        None,
        &remembered.provider_session,
        &key.incarnation,
        &Event {
            kind: tmt_core::binding::session::activity::EventKind::TurnStarted,
            turn: None,
        },
        None,
        10,
    );
    let project = |binding: &Binding, remembered: &RememberedSession, runtime| {
        crate::output::activity_document(Some(binding), Some(remembered), runtime, &registry)
    };
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Running),
        json!({
            "state":"working", "sinceMs":10, "lastActivityMs":10, "providers":{},
        })
    );
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Unknown),
        json!({
            "state":"unknown", "sinceMs":null, "lastActivityMs":10, "providers":{},
        })
    );
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Ended)["state"],
        "ended"
    );
    remembered.state = tmt_adapters::runtime::driver_state::after_activity(
        remembered.state.as_ref(),
        &remembered.provider_session,
        &binding.session.key.as_ref().unwrap().incarnation,
        &Event {
            kind: tmt_core::binding::session::activity::EventKind::ApprovalPrompt,
            turn: None,
        },
        None,
        11,
    );
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Running),
        json!({
            "state":"waiting", "sinceMs":11, "lastActivityMs":11, "providers":{},
        })
    );
    binding.session.key.as_mut().unwrap().incarnation =
        ProcessIncarnation::new(10, "replacement").unwrap();
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Running),
        json!({
            "state":"unknown", "sinceMs":null, "lastActivityMs":null, "providers":{},
        })
    );
    remembered.provider_session = ProviderSessionId::new("other").unwrap();
    assert_eq!(
        project(&binding, &remembered, RuntimeState::Running)["state"],
        "unknown"
    );
}

fn located(host: &str, session: Option<&str>, window: Option<&str>) -> Option<PaneLocation> {
    Some(PaneLocation {
        host: host.into(),
        session: session.map(Into::into),
        window: window.map(Into::into),
    })
}

fn listed_row(pane: PaneObservation) -> Value {
    let row = row(
        "lead",
        Lifetime::Saved,
        Some((pane, binding(RuntimeState::Running, None))),
        None,
    );
    document(&Report::Listed {
        rows: vec![row],
        scope: ListScope::default(),
    })["identities"][0]
        .clone()
}

#[test]
fn a_live_row_publishes_the_hosts_location_and_leaves_the_target_untouched() {
    let mut tmux = pane("%7", "codex", "/repo");
    tmux.location = located("tmux", Some("my session"), Some("12"));
    let json = listed_row(tmux.clone());
    assert_eq!(
        json["location"],
        serde_json::json!({"host": "tmux", "session": "my session", "window": "12"})
    );
    assert_eq!(json["target"], "work:1.7");
    assert_eq!(pane_document(&tmux)["location"], json["location"]);
    assert_eq!(pane_document(&tmux)["target"], "work:1.7");
}

#[test]
fn an_external_host_publishes_only_its_host() {
    let mut herdr = pane("%7", "codex", "/repo");
    herdr.location = located("herdr", None, None);
    let json = listed_row(herdr.clone());
    assert_eq!(json["location"], serde_json::json!({"host": "herdr"}));
    assert_eq!(pane_document(&herdr)["location"], json["location"]);
}

#[test]
fn a_row_without_a_live_observation_has_no_location_key_at_all() {
    // Unobserved location on a live pane.
    let json = listed_row(pane("%7", "codex", "/repo"));
    assert!(
        json.as_object().unwrap().get("location").is_none(),
        "{json}"
    );
    assert!(
        pane_document(&pane("%7", "codex", "/repo"))
            .as_object()
            .unwrap()
            .get("location")
            .is_none()
    );
    // An offline identity has no pane at all, so nothing to locate.
    let offline = row("gone", Lifetime::Saved, None, None);
    let json = document(&Report::Listed {
        rows: vec![offline],
        scope: ListScope::default(),
    })["identities"][0]
        .clone();
    assert!(
        json.as_object().unwrap().get("location").is_none(),
        "{json}"
    );
}

/// A live row in `command`, with no remembered session, as `tmt ls` sees an
/// agent that TMT cannot observe beyond its pane.
fn pane_only_row(name: &str, command: &str) -> ListedRow {
    row(
        name,
        Lifetime::Saved,
        Some((
            pane("%9", command, "/tmp"),
            binding(RuntimeState::Unknown, None),
        )),
        None,
    )
}

fn support_of(row: &ListedRow, preferred: Option<&str>) -> Option<&'static str> {
    let preferred = preferred.map(|harness| HarnessId::new(harness).unwrap());
    driver_support(&row.presence, row.remembered.as_ref(), preferred.as_ref()).map(Support::as_str)
}

#[test]
fn every_built_in_driver_reports_its_declared_level_from_a_remembered_harness() {
    for driver in descriptor::ALL {
        let row = row(
            "agent",
            Lifetime::Saved,
            None,
            Some(remembered(driver.name, "session-1", false)),
        );
        assert_eq!(
            support_of(&row, None),
            Some(driver.support.as_str()),
            "{}",
            driver.name
        );
    }
}

#[test]
fn the_identified_driver_is_remembered_then_preferred_then_the_exact_live_executable() {
    // Remembered and preferred harnesses need no live pane, so a stopped agent
    // keeps its level; the remembered one wins over the preferred one.
    let stopped = row("sol", Lifetime::Saved, None, None);
    assert_eq!(support_of(&stopped, Some("codex")), Some("full"));
    assert_eq!(support_of(&stopped, Some("gemini")), Some("basic"));
    let remembered_claude = row(
        "sol",
        Lifetime::Saved,
        None,
        Some(remembered("claude", "session-1", false)),
    );
    assert_eq!(support_of(&remembered_claude, Some("gemini")), Some("full"));
    // Only without a harness does the live pane's foreground executable count.
    for (command, expected) in [
        ("gemini", Some("basic")),
        ("pi", Some("basic")),
        ("opencode", Some("basic")),
        ("claude", Some("full")),
        ("/opt/bin/muse", Some("basic")),
        ("pi-calc", None),
        ("node", None),
        ("zsh", None),
        ("2.1.283", None),
    ] {
        assert_eq!(
            support_of(&pane_only_row("agent", command), None),
            expected,
            "{command}"
        );
    }
    // A harness that is not a built-in driver has no level, and never falls
    // back to a command that happens to match another driver.
    let mut unregistered = pane_only_row("agent", "gemini");
    unregistered.remembered = Some(remembered("unregistered", "session-1", false));
    assert_eq!(support_of(&unregistered, None), None);
    // The pane counts only while the row is active.
    for presence in [Presence::Offline, Presence::Unknown] {
        let mut row = pane_only_row("agent", "gemini");
        row.presence.presence = presence;
        assert_eq!(support_of(&row, None), None, "{presence:?}");
    }
}

/// The JSON row with `driverSupport` removed must equal the same row without
/// it, so the key is purely additive.
fn support_document(mut row: ListedRow, support: Option<Support>) -> Value {
    let baseline = document(&Report::Listed {
        rows: vec![ListedRow {
            presence: row.presence.clone(),
            remembered: row.remembered.clone(),
            resume: row.resume.clone(),
            activity: row.activity.clone(),
            running_driver: row.running_driver.clone(),
            driver_support: None,
        }],
        scope: ListScope::default(),
    });
    row.driver_support = support;
    let projected = document(&Report::Listed {
        rows: vec![row],
        scope: ListScope::default(),
    });
    let mut existing = projected.clone();
    existing["identities"][0]
        .as_object_mut()
        .unwrap()
        .remove("driverSupport");
    assert_eq!(existing, baseline, "existing list fields must not change");
    projected["identities"][0].clone()
}

#[test]
fn json_driver_support_is_additive_a_plain_string_and_absent_never_null() {
    let mut full = running_row("claude", CLAUDE);
    full.running_driver = Some("claude".into());
    let value = support_document(full, Some(Support::Full));
    assert_eq!(value["driverSupport"], "full");
    assert_eq!(value["runningDriver"], "claude");
    assert_eq!(value["driver"], "claude");

    // A basic driver has no runtime, so there is no `runningDriver` to sit
    // beside; the level is still reported.
    let value = support_document(pane_only_row("pi-1", "pi"), Some(Support::Basic));
    assert_eq!(value["driverSupport"], "basic");
    assert!(value.get("runningDriver").is_none());
    assert_eq!(value["session"]["activity"]["state"], "unknown");

    for row in [
        pane_only_row("unknown", "node"),
        row("offline", Lifetime::Saved, None, None),
    ] {
        let value = support_document(row, None);
        assert!(value.get("driverSupport").is_none(), "{value}");
    }
}

#[test]
fn a_running_basic_agent_carries_a_dim_basic_note_that_never_replaces_an_action() {
    let mut basic = pane_only_row("pi-1", "pi");
    basic.driver_support = Some(Support::Basic);
    let mut full = running_row("claude", CLAUDE);
    full.driver_support = Some(Support::Full);
    let text = render(vec![basic, full], Terminal::PLAIN, false);
    let lines: Vec<&str> = text.lines().collect();
    let basic_line = lines.iter().find(|line| line.contains("pi-1")).unwrap();
    assert!(basic_line.trim_end().ends_with("basic"), "{text}");
    let full_line = lines.iter().find(|line| line.contains("worker")).unwrap();
    assert!(!full_line.contains("basic"), "{text}");

    // An action keeps the slot; the note appears only where nothing else does.
    let mut stale = row(
        "pi-old",
        Lifetime::Saved,
        Some((
            pane("%3", "pi", "/tmp"),
            binding(RuntimeState::Running, Some(CLAUDE)),
        )),
        Some(remembered("claude", CLAUDE, true)),
    );
    stale.driver_support = Some(Support::Basic);
    let text = render(vec![stale], Terminal::PLAIN, false);
    assert!(text.contains("stale") && !text.contains("basic"), "{text}");

    // An offline basic agent with nothing to resume still folds into one line.
    let mut offline = row("pi-2", Lifetime::Saved, None, None);
    offline.driver_support = Some(Support::Basic);
    let text = render(vec![offline], Terminal::PLAIN, false);
    assert!(
        text.contains("offline: pi-2") && !text.contains("basic"),
        "{text}"
    );
}

#[test]
fn a_herdr_agent_name_mismatch_advises_tmt_this() {
    let mut binding = binding(RuntimeState::Running, None);
    binding.server.host = tmt_core::host::HostKind::parse("herdr").unwrap();
    let mut pane = pane("term_1", "claude", "/repo");
    pane.target = Some("w1:p2".into());
    pane.agent_name = Some("ann".into());
    let rows = [row("worker", Lifetime::Saved, Some((pane, binding)), None)];
    assert_eq!(
        agent_name_advice(&rows),
        [(
            "worker's agent has the herdr agent name ann, not worker".to_owned(),
            "In worker's pane run: tmt this worker".to_owned(),
        )]
    );
}
