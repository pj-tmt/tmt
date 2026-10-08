use super::*;
use crate::{runtime::hook_protocol::HookLaunch, skill_installation::ProviderEnvironment};
use serde_json::Value;
use std::fs;

fn fixture(
    args: &[&str],
    global: Option<&str>,
) -> (crate::test_support::TestDirectory, RuntimeCommand) {
    let directory = crate::test_support::TestDirectory::new();
    let home = directory.path.as_path();
    fs::create_dir(home.join(".claude")).unwrap();
    if let Some(global) = global {
        fs::write(home.join(".claude/settings.json"), global).unwrap();
    }
    let environment = ProviderEnvironment::from_parts(home, home, vec![], []);
    let command = RuntimeCommand {
        executable: "claude".into(),
        args: args.iter().map(|s| (*s).into()).collect(),
    };
    let launch = HookLaunch {
        identity_id: "11111111-1111-4111-8111-111111111111".into(),
        binding_id: "22222222-2222-4222-8222-222222222222".into(),
        owner_pid: 42,
        owner_start: "start' token".into(),
    };
    let prepared = prepare(&LaunchHooks {
        command: &command,
        launch: &launch,
        tmt: Path::new("/a b/tm't"),
        environment: &environment,
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json"))
            .ok()
            .as_deref(),
        global
    );
    (directory, prepared)
}

fn settings(command: &RuntimeCommand) -> Value {
    let index = command
        .args
        .iter()
        .position(|arg| arg == "--settings")
        .unwrap();
    serde_json::from_str(command.args[index + 1].to_str().unwrap()).unwrap()
}

#[test]
fn launch_options_precede_the_literal_prompt_boundary() {
    let (_directory, command) = fixture(&["--", "--settings", "literal prompt"], None);
    assert_eq!(command.args[0], "--settings");
    assert_eq!(
        &command.args[2..],
        &["--", "--settings", "literal prompt"].map(std::ffi::OsString::from)
    );
    assert!(settings(&command)["hooks"]["Stop"].is_array());
}

#[test]
fn explicit_regular_file_is_read_without_rewriting_it() {
    let directory = crate::test_support::TestDirectory::new();
    let original = "{\"model\":\"kept\",\"opaque\":123456789012345678901234567890}";
    fs::write(directory.path.join("session.json"), original).unwrap();
    let environment = ProviderEnvironment::from_parts(&directory.path, &directory.path, vec![], []);
    let launch = HookLaunch {
        identity_id: "11111111-1111-4111-8111-111111111111".into(),
        binding_id: "22222222-2222-4222-8222-222222222222".into(),
        owner_pid: 42,
        owner_start: "start".into(),
    };
    let command = RuntimeCommand {
        executable: "claude".into(),
        args: vec!["--settings=session.json".into()],
    };
    let prepared = prepare(&LaunchHooks {
        command: &command,
        launch: &launch,
        tmt: Path::new("/tmt"),
        environment: &environment,
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path.join("session.json")).unwrap(),
        original
    );
    assert_eq!(settings(&prepared)["model"], "kept");
    assert!(
        prepared.args[1]
            .to_str()
            .unwrap()
            .contains("123456789012345678901234567890")
    );
    assert!(!directory.path.join(".claude").exists());
}

#[test]
fn absent_setup_installs_one_observation_per_event_without_usage_consent() {
    let (_directory, command) = fixture(&["--resume", "session", "--model", "model"], None);
    assert_eq!(
        &command.args[..4],
        &["--resume", "session", "--model", "model"].map(std::ffi::OsString::from)
    );
    let settings = settings(&command);
    for event in ["SessionStart", "SessionEnd", "UserPromptSubmit"] {
        assert_eq!(settings["hooks"][event].as_array().unwrap().len(), 1);
    }
    let stops = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stops.len(), 2);
    assert!(
        stops[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("__hook claude --activity-only")
    );
    let focus = stops[1]["hooks"][0]["command"].as_str().unwrap();
    assert!(focus.starts_with("'/a b/tm'\\''t' __focus-hook claude --launch '"));
    assert!(focus.contains("start'\\'' token"));
}

#[test]
fn existing_setup_hooks_are_the_only_observers_and_user_hooks_remain() {
    let observer = crate::runtime::hook_protocol::command_entry("claude", "/stable/tmt");
    let global = json!({"hooks":{"SessionStart":[observer], "SessionEnd":[observer], "UserPromptSubmit":[observer], "Stop":[observer]}}).to_string();
    let inline = r#"{"permissions":{"allow":["Read"]},"unknown":123456789012345678901234567890,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user-hook"}]}]}}"#;
    let (_directory, command) = fixture(&["--settings", inline], Some(&global));
    let raw = command.args[1].to_str().unwrap();
    assert!(raw.contains("123456789012345678901234567890"));
    let settings: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(settings["permissions"]["allow"][0], "Read");
    assert!(settings["hooks"].get("SessionStart").is_none());
    let stops = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stops.len(), 2);
    assert_eq!(stops[0]["hooks"][0]["command"], "user-hook");
    assert!(
        stops[1]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("__focus-hook")
    );
}

#[test]
fn inline_owned_hooks_are_not_added_twice() {
    let observer = crate::runtime::hook_protocol::command_entry("claude", "/stable/tmt");
    let inline = json!({"hooks":{"SessionStart":[observer], "SessionEnd":[observer], "UserPromptSubmit":[observer], "Stop":[observer]}}).to_string();
    let (_directory, command) = fixture(&["--settings", &inline], None);
    let settings = settings(&command);
    assert_eq!(
        settings["hooks"]["SessionStart"].as_array().unwrap().len(),
        1
    );
    assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
}

#[test]
fn disabled_ambiguous_and_edited_settings_refuse_composition() {
    let directory = crate::test_support::TestDirectory::new();
    let environment = ProviderEnvironment::from_parts(
        directory.path.as_path(),
        directory.path.as_path(),
        vec![],
        [],
    );
    let launch = HookLaunch {
        identity_id: "11111111-1111-4111-8111-111111111111".into(),
        binding_id: "22222222-2222-4222-8222-222222222222".into(),
        owner_pid: 42,
        owner_start: "start".into(),
    };
    for args in [
        vec!["--bare"],
        vec!["--safe-mode"],
        vec!["--setting-sources", "user", "--setting-sources=local"],
        vec!["--settings", r#"{"disableAllHooks":true}"#],
        vec!["--settings", r#"{"allowManagedHooksOnly":true}"#],
        vec!["--settings", r#"{"hooks":{},"hooks":{}}"#],
        vec!["--settings", "{}", "--settings={}"],
        vec![
            "--settings",
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"'/stable/tmt' __hook claude --changed","timeout":3}]}]}}"#,
        ],
    ] {
        let command = RuntimeCommand {
            executable: "claude".into(),
            args: args.into_iter().map(Into::into).collect(),
        };
        assert!(
            prepare(&LaunchHooks {
                command: &command,
                launch: &launch,
                tmt: Path::new("/tmt"),
                environment: &environment
            })
            .is_err()
        );
    }
}

#[test]
fn only_unrecursive_documented_main_stop_claims_and_output_uses_block_reason() {
    assert_eq!(
        decode(br#"{"hook_event_name":"Stop","session_id":"one","stop_hook_active":false}"#)
            .unwrap()
            .as_str(),
        "one"
    );
    for payload in [
        r#"{"hook_event_name":"Stop","session_id":"one","stop_hook_active":true}"#,
        r#"{"hook_event_name":"Stop","session_id":"one"}"#,
        r#"{"hook_event_name":"StopFailure","session_id":"one","stop_hook_active":false}"#,
        r#"{"hook_event_name":"SubagentStop","session_id":"one","stop_hook_active":false}"#,
    ] {
        assert!(decode(payload.as_bytes()).is_none());
    }
    assert_eq!(
        serde_json::from_str::<Value>(&encode("digest").unwrap()).unwrap(),
        json!({"decision":"block","reason":"digest"})
    );
    assert!(encode("").is_none());
    assert!(encode(&"x".repeat(CONTEXT_LIMIT + 1)).is_none());
}

#[test]
fn ignored_global_setup_cannot_suppress_launch_observers() {
    let observer = crate::runtime::hook_protocol::command_entry("claude", "/stable/tmt");
    let global = json!({"hooks":{"SessionStart":[observer], "SessionEnd":[observer], "UserPromptSubmit":[observer], "Stop":[observer]}}).to_string();
    for args in [
        &["--setting-sources", "project,local"][..],
        &["--setting-sources="][..],
    ] {
        let (_directory, command) = fixture(args, Some(&global));
        let settings = settings(&command);
        assert_eq!(
            settings["hooks"]["SessionStart"].as_array().unwrap().len(),
            1
        );
        assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(command.args.iter().any(|arg| arg == args[0]));
    }
}

#[test]
fn lifecycle_ancestry_uses_the_callers_process_owner() {
    use crate::{
        process::{CommandError, CommandOutput, CommandRequest, CommandRunner},
        runtime::lifecycle::RuntimeLifecycle,
    };
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };
    struct Reject(Cell<bool>);
    impl CommandRunner for Reject {
        fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.set(true);
            Err(CommandError::new(crate::process::CommandFailure::Spawn))
        }
    }
    let runner = Reject(Cell::new(false));
    assert!(
        super::super::ClaudeLifecycle
            .observe_in_pane(&runner, 1, 2, Instant::now() + Duration::from_secs(1))
            .is_none()
    );
    assert!(
        runner.0.get(),
        "lifecycle must not substitute a worker-only runner"
    );
}
