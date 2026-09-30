use super::*;
use crate::invocation::ListScope;
use tmt_core::{
    binding::{
        Binding,
        session::{
            BindingSessionState, HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeMode,
        },
    },
    endpoint::{ProcessIncarnation, ServerEvidence},
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
        id: id.into(),
        target: Some(format!("work:1.{}", id.trim_start_matches('%'))),
        cwd: Some(cwd.into()),
        command: command.into(),
        pane_pid: 10,
        suggested_name: None,
        marker: None,
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
        presence: IdentityPresence {
            identity: identity(name, lifetime),
            presence,
            pane,
            binding,
        },
        remembered,
        resume: None,
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
                pane("%12", "2.1.283", "/Users/ada/dev/tmux-team"),
                binding(RuntimeState::Running, Some(CLAUDE)),
            )),
            Some(remembered("claude", CLAUDE, false)),
        ),
        row(
            "astra",
            Lifetime::Saved,
            Some((
                pane("%7", "codex", "/Users/ada/dev/tmux-team"),
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
fn json_rows_only_gain_the_full_address_and_driver() {
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
