//! The built-in host as a user meets it with no tmux: one private world, two members that
//! reply, one attached, lost and explicitly resumed. Each leg has a focused test beside this one; this proves
//! they compose, and that the end state is honest and clean.
use super::attach::{output_until, quote, wait_for};
use super::*;

fn lines(world: &World, name: &str) -> Vec<String> {
    fs::read_to_string(world.root.join(format!("{name}.lines")))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn received(world: &World, name: &str, text: &str) -> usize {
    lines(world, name)
        .iter()
        .filter(|line| *line == text)
        .count()
}

/// A member that logs every line it is given, and answers a pasted request the way a
/// cooperating agent does. The log shows a replayed request twice.
fn replying_script(world: &World, name: &str) -> String {
    let root = world.root.display();
    let executable = quote(env!("CARGO_BIN_EXE_tmt"));
    format!(
        r#"printf '%s' "$$" > '{root}/{name}.pid'; printf 'provider-ready\n'; while IFS= read -r line; do printf '%s\n' "$line" >> '{root}/{name}.lines'; case "$line" in quit) exit 0;; *"tmt reply req_"*) set -- $line; while [ "$1" != tmt ]; do shift; done; {executable} reply "$3" --receipt "$5" --message {name}-processed;; esac; done"#
    )
}

const SESSION: &str = "11111111-1111-4111-8111-111111111111";

// Reuse the native provider-shaped parent: hooks must come from a real admitted
// provider descendant, and resume must discover that same executable/session.
fn replying_provider(world: &mut World) -> PathBuf {
    let example = std::path::Path::new(env!("CARGO_BIN_EXE_tmt"))
        .parent()
        .unwrap()
        .join("examples/provider-script-fixture");
    let provider = world.root.join("bin/claude");
    tmt_test_support::write_executable(
        &provider,
        &fs::read(&example).expect("build tmt-adapters --example provider-script-fixture first"),
        0o755,
    )
    .unwrap();
    let root = world.root.display();
    let script = format!(
        r#"#!/bin/sh
printf '%s' "$PPID" > '{root}/alpha.pid'
printf '%s' "$$" > '{root}/alpha-peer.pid'
printf '%s\n' "$@" >> '{root}/provider-argv'
IFS= read -r release_hook
printf '%s' '{{"hook_event_name":"SessionStart","session_id":"{SESSION}","source":"startup","cwd":"{root}","model":"fixture-model"}}' | {cli} __hook claude > '{root}/hook.out' 2> '{root}/hook.err'
printf '%s' "$?" > '{root}/hook.status.tmp'
/bin/mv '{root}/hook.status.tmp' '{root}/hook.status'
{reply}
"#,
        cli = quote(env!("CARGO_BIN_EXE_tmt")),
        reply = replying_script(world, "alpha").replace("\"$$\"", "\"$PPID\""),
    );
    let interpreter = world.root.join("fixture-interpreter");
    tmt_test_support::write_executable(&interpreter, script.as_bytes(), 0o755).unwrap();
    world.interpreter = Some(interpreter);
    // An exact resume requires a readable transcript, entirely inside this world.
    let transcripts = world.root.join("home/.claude/projects/fixture");
    fs::create_dir_all(&transcripts).unwrap();
    fs::write(transcripts.join(format!("{SESSION}.jsonl")), "{}\n").unwrap();
    provider
}

fn exchanges(world: &World) -> Vec<Vec<rusqlite::types::Value>> {
    let db = rusqlite::Connection::open_with_flags(
        support::state_dir(&world.root).join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = db
        .prepare(
            "SELECT a.request_id,a.attempt_id,a.recipient_identity_id,a.message_text,\
         a.status,a.wake_state,r.body,r.submitted_at_ms FROM request_attempts a \
         LEFT JOIN request_responses r USING(request_id) ORDER BY a.rowid",
        )
        .unwrap();
    statement
        .query_map([], |row| (0..8).map(|i| row.get(i)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn attempts(world: &World, request: &str) -> Vec<(String, String)> {
    let db = rusqlite::Connection::open_with_flags(
        support::state_dir(&world.root).join("tmt.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = db
        .prepare("SELECT status,wake_state FROM request_attempts WHERE request_id=?")
        .unwrap();
    statement
        .query_map([request], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn gone(process: &ProcessIncarnation) -> bool {
    match observe_runtime_process(
        &UnixCommandRunner,
        process.pid(),
        Instant::now() + Duration::from_secs(3),
    ) {
        Ok(ProcessObservation::Gone) => true,
        Ok(ProcessObservation::Live(now) | ProcessObservation::Stopped(now)) => now != *process,
        _ => false,
    }
}

impl World {
    fn talk_for_reply(&mut self, name: &str, text: &str) -> Value {
        let reply = self.json(&[
            "talk",
            name,
            text,
            "--identity",
            "sender",
            "--timeout",
            "30",
            "--json",
        ]);
        assert_eq!(reply["response"], format!("{name}-processed"), "{reply:#}");
        reply
    }

    fn member_row(&mut self, name: &str) -> Value {
        let listed = self.json(&["ls", "--json"]);
        listed["identities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap_or_else(|| panic!("no identity {name}: {listed:#}"))
            .clone()
    }

    fn supervisor_of(&mut self, name: &str) -> ProcessIncarnation {
        let pane = self.member_row(name)["pane"].as_str().unwrap().to_owned();
        let mut client =
            Connection::connect(&support::state_dir(&self.root).join("pty"), &pane).unwrap();
        let ControlReply::State { supervisor_pid, .. } =
            client.control(Control::Inspect, WAIT).unwrap()
        else {
            panic!("supervisor evidence")
        };
        self.own_pid(u64::from(supervisor_pid))
    }

    fn program_of(&mut self, name: &str) -> ProcessIncarnation {
        let pid = fs::read_to_string(self.root.join(format!("{name}.pid")))
            .unwrap()
            .parse()
            .unwrap();
        self.own_pid(pid)
    }
}

#[test]
fn two_members_talk_attach_and_resume_after_supervisor_loss_without_tmux() {
    let mut world = World::new();
    let agent = world.agent();
    world.json(&["identity", "create", "sender", "--json"]);
    let provider = replying_provider(&mut world);
    for name in ["alpha", "beta"] {
        let script = replying_script(&world, name);
        let started = if name == "alpha" {
            world.run(&[
                "run",
                "--no-channel",
                "--save",
                name,
                provider.to_str().unwrap(),
            ])
        } else {
            world.run(&["run", "--no-channel", "--save", name, &agent, "-c", &script])
        };
        assert!(started.status.success(), "{name}: {started:?}");
        assert!(String::from_utf8_lossy(&started.stderr).contains("host pty"));
        wait_for("member pid", || {
            world.root.join(format!("{name}.pid")).exists()
        });
    }
    let (identity, original_pane, session) = record_provider_session(&world, None, SESSION);
    assert_eq!(session, SESSION);
    assert!(!world.root.join("bin/tmux").exists());
    world.remember();
    world.remember_programs();
    let alpha_supervisor = world.supervisor_of("alpha");
    let beta_supervisor = world.supervisor_of("beta");
    let alpha_program = world.program_of("alpha");
    let beta_program = world.program_of("beta");

    // Both members answer, each request reaches only its own member, once.
    world.talk_for_reply("alpha", "story-one-alpha");
    world.talk_for_reply("beta", "story-one-beta");
    assert_eq!(received(&world, "alpha", "story-one-alpha"), 1);
    assert_eq!(received(&world, "beta", "story-one-beta"), 1);
    assert_eq!(received(&world, "alpha", "story-one-beta"), 0);
    assert_eq!(received(&world, "beta", "story-one-alpha"), 0);

    // A person attaches to alpha: typing reaches the program and beta is not disturbed.
    let mut attached = world.outer("story_attach", false);
    attached.input(b"typed-while-attached\r").unwrap();
    wait_for("typed line reaches alpha", || {
        received(&world, "alpha", "typed-while-attached") == 1
    });
    world.talk_for_reply("beta", "story-two-beta");
    // Detaching ends the viewer, not the member, which still answers.
    attached.input(b"\x1c").unwrap();
    output_until(&mut attached, b"OUTER-DONE");
    world.restored("story_attach", 0);
    world.talk_for_reply("alpha", "story-two-alpha");
    assert_eq!(received(&world, "alpha", "story-two-alpha"), 1);
    assert_eq!(received(&world, "alpha", "typed-while-attached"), 1);

    // The supervisor of an attached member is lost.
    let mut watching = world.outer("story_loss", false);
    watching.input(b"typed-before-loss\r").unwrap();
    wait_for("typed line reaches alpha", || {
        received(&world, "alpha", "typed-before-loss") == 1
    });
    assert!(
        matches!(observe_runtime_process(&UnixCommandRunner, alpha_supervisor.pid(),
        Instant::now() + Duration::from_secs(3)), Ok(ProcessObservation::Live(now)) if now == alpha_supervisor)
    );
    kill(
        Pid::from_raw(alpha_supervisor.pid() as i32),
        Signal::SIGKILL,
    )
    .unwrap();
    let ending = output_until(&mut watching, b"OUTER-DONE");
    let text = String::from_utf8_lossy(&ending);
    assert!(text.contains("does not prove the agent ended"), "{text}");
    world.restored("story_loss", 1);

    // The loss is reported as it is: alpha is offline and cannot be checked, and a new
    // request is queued for its inbox rather than pasted anywhere.
    let before = lines(&world, "alpha");
    let row = world.member_row("alpha");
    assert_eq!(row["presence"], "offline", "{row:#}");
    assert!(row["pane"].is_null(), "{row:#}");
    let checked = world.run(&["check", "alpha"]);
    assert_eq!(checked.status.code(), Some(3), "{checked:?}");
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("not active"),
        "{checked:?}"
    );
    let queued = world.json(&[
        "talk",
        "alpha",
        "story-after-loss",
        "--identity",
        "sender",
        "--detach",
        "--json",
    ]);
    assert_eq!(queued["status"], "queued", "{queued:#}");
    assert_eq!(queued["offline"], true, "{queued:#}");
    let request = queued["requestId"].as_str().unwrap().to_owned();
    let recorded = attempts(&world, &request);
    assert_eq!(recorded, [("queued".into(), "not_attempted".into())]);

    // The other member is untouched, and nothing replays the lost one's request.
    world.talk_for_reply("beta", "story-three-beta");
    assert_eq!(received(&world, "beta", "story-three-beta"), 1);
    assert_eq!(received(&world, "beta", "story-after-loss"), 0);
    world.json(&["ls", "--json"]);
    assert_eq!(
        attempts(&world, &request),
        recorded,
        "reads must not replay a request"
    );
    assert_eq!(
        lines(&world, "alpha"),
        before,
        "nothing reached the lost member"
    );

    // The lost member's program does not outlive its supervisor.
    wait_for("alpha's program to end with its supervisor", || {
        gone(&alpha_supervisor) && gone(&alpha_program)
    });

    // Resume is an explicit launch, never a retry of any exchange. The old
    // provider and supervisor are confirmed gone before this action begins.
    let before_resume = exchanges(&world);
    fs::remove_file(world.root.join("hook.status")).unwrap();
    let resumed = world.run(&["resume", "--host", "pty", "--no-channel", "alpha"]);
    support::with_launch_failure_packet(
        &support::state_dir(&world.root).join("tmt.db"),
        &[alpha_supervisor.pid(), alpha_program.pid()],
        || assert!(resumed.status.success(), "{resumed:?}"),
    );
    let (resumed_identity, resumed_pane, resumed_session) =
        record_provider_session(&world, Some(&original_pane), SESSION);
    assert_eq!(resumed_identity, identity);
    assert_eq!(resumed_session, SESSION);
    assert_ne!(resumed_pane, original_pane);
    let restarted_supervisor = world.supervisor_of("alpha");
    let restarted_program = world.program_of("alpha");
    assert_ne!(restarted_supervisor, alpha_supervisor);
    let argv = fs::read_to_string(world.root.join("provider-argv")).unwrap();
    assert!(
        argv.lines()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|words| words == ["--resume", SESSION]),
        "{argv}"
    );
    assert_eq!(
        exchanges(&world),
        before_resume,
        "resume must preserve every exchange"
    );
    assert_eq!(
        attempts(&world, &request),
        recorded,
        "queued work must not replay on resume"
    );
    assert_eq!(lines(&world, "alpha"), before);
    world.talk_for_reply("alpha", "story-resumed-alpha");
    world.talk_for_reply("beta", "story-after-resume-beta");
    for text in ["story-one-alpha", "story-two-alpha", "story-resumed-alpha"] {
        assert_eq!(
            received(&world, "alpha", text),
            1,
            "{text} processed exactly once"
        );
    }
    assert_eq!(received(&world, "alpha", "story-after-loss"), 0);
    assert_eq!(received(&world, "beta", "story-after-resume-beta"), 1);
    let after_resume = exchanges(&world);
    assert_eq!(after_resume.len(), before_resume.len() + 2);
    assert!(
        after_resume.starts_with(&before_resume),
        "old exchanges changed"
    );

    let directory = support::state_dir(&world.root).join("pty");
    let mut terminal = Connection::connect(&directory, &resumed_pane).unwrap();
    terminal.input(b"quit\r").unwrap();
    drop(terminal);
    wait_for("resumed member and supervisor to end", || {
        gone(&restarted_supervisor) && gone(&restarted_program)
    });
    for pane in [&original_pane, &resumed_pane] {
        for extension in ["sock", "lock"] {
            assert!(
                !directory.join(format!("{pane}.{extension}")).exists(),
                "{pane} left its .{extension} behind after resume and exit"
            );
        }
    }

    // Beta ends on its own; its supervisor follows and removes its socket and lock.
    let directory = support::state_dir(&world.root).join("pty");
    let pane = world.member_row("beta")["pane"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut terminal = Connection::connect(&directory, &pane).unwrap();
    terminal.input(b"quit\r").unwrap();
    drop(terminal);
    wait_for("beta's supervisor and program to end", || {
        gone(&beta_supervisor) && gone(&beta_program)
    });
    for extension in ["sock", "lock"] {
        assert!(
            !directory.join(format!("{pane}.{extension}")).exists(),
            "beta left its .{extension} behind"
        );
    }
    assert_eq!(world.member_row("beta")["presence"], "offline");
    assert!(world.clean());
}
