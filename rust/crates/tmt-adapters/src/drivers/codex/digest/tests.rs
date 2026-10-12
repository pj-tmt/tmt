use super::*;
use crate::{
    runtime::hook_protocol::HookLaunch, skill_installation::ProviderEnvironment,
    test_support::TestDirectory,
};
use std::{ffi::OsString, fs};

fn launch(id: &str) -> HookLaunch {
    HookLaunch {
        identity_id: id.into(),
        binding_id: "22222222-2222-4222-8222-222222222222".into(),
        owner_pid: 42,
        owner_start: "start".into(),
    }
}
fn command(args: &[&str]) -> RuntimeCommand {
    RuntimeCommand {
        executable: "codex".into(),
        args: args.iter().map(OsString::from).collect(),
    }
}
fn prepare_in(
    directory: &TestDirectory,
    command: &RuntimeCommand,
    launch: &HookLaunch,
) -> io::Result<RuntimeCommand> {
    let environment = ProviderEnvironment::from_parts(&directory.path, &directory.path, vec![], []);
    let plan = LaunchHooks {
        command,
        launch,
        tmt: Path::new("/a b/tm't"),
        environment: &environment,
    };
    let (session, cwd) = settings::launch_settings(&plan)?;
    let sources = settings::sources(&environment, &cwd, &directory.path.join("system"))?;
    compose(command, plan.tmt.to_str().unwrap(), session, &sources)
}
fn hook_setting(command: &RuntimeCommand) -> String {
    command
        .args
        .windows(2)
        .filter(|p| p[0] == "-c")
        .map(|p| p[1].to_str().unwrap())
        .find(|s| s.starts_with("hooks="))
        .unwrap()
        .into()
}
fn hooks(command: &RuntimeCommand) -> Value {
    let doc = hook_setting(command).parse::<DocumentMut>().unwrap();
    // Round-trip independently through the generated TOML's literal arrays.
    let mut result = json!({});
    for event in ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop"] {
        if let Some(entries) = doc["hooks"].get(event).and_then(Item::as_array) {
            result[event] = entries.iter().map(|v| {
                let group = v.as_inline_table().unwrap();
                let handlers = group.get("hooks").unwrap().as_array().unwrap();
                json!({"hooks":handlers.iter().map(|h| { let h=h.as_inline_table().unwrap(); json!({"command":h.get("command").unwrap().as_str().unwrap(),"type":h.get("type").unwrap().as_str().unwrap(),"timeout":h.get("timeout").unwrap().as_integer().unwrap()}) }).collect::<Vec<_>>()})
            }).collect::<Vec<_>>().into();
        }
    }
    result
}
#[test]
fn fresh_and_resume_different_identities_have_byte_identical_definitions() {
    let d = TestDirectory::new();
    let first = prepare_in(
        &d,
        &command(&[]),
        &launch("11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    let mut next = launch("33333333-3333-4333-8333-333333333333");
    next.owner_pid = 77;
    next.owner_start = "different".into();
    let resumed = prepare_in(
        &d,
        &command(&["resume", "exact-session", "--no-daemon"]),
        &next,
    )
    .unwrap();
    assert_eq!(
        hook_setting(&first).as_bytes(),
        hook_setting(&resumed).as_bytes()
    );
    let definitions = hooks(&first);
    assert_eq!(definitions["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(
        definitions["Stop"][1]["hooks"][0]["command"],
        "'/a b/tm'\\''t' __digest-hook codex --discover-launch"
    );
    assert!(!hook_setting(&first).contains("--launch"));
    assert!(!hook_setting(&first).contains(&next.identity_id));
    assert!(!d.path.join(".codex").exists());
}
#[test]
fn setup_is_the_only_observer_and_global_bytes_are_untouched() {
    let d = TestDirectory::new();
    fs::create_dir(d.path.join(".codex")).unwrap();
    let observer = crate::runtime::hook_protocol::command_entry(super::super::NAME, "/stable/tmt");
    let global=json!({"hooks":{"SessionStart":[observer],"SessionEnd":[observer],"UserPromptSubmit":[observer],"Stop":[observer]}}).to_string();
    let path = d.path.join(".codex/hooks.json");
    fs::write(&path, &global).unwrap();
    let prepared = prepare_in(
        &d,
        &command(&[]),
        &launch("11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    let h = hooks(&prepared);
    assert!(h.get("SessionStart").is_none());
    assert!(h.get("SessionEnd").is_none());
    assert!(h.get("UserPromptSubmit").is_none());
    assert_eq!(h["Stop"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(path).unwrap(), global);
}
#[test]
fn inline_config_owned_hooks_are_not_observed_twice() {
    let d = TestDirectory::new();
    let owned = crate::runtime::hook_protocol::command_entry(super::super::NAME, "/stable/tmt");
    let session = json!({"SessionStart":[owned],"Stop":[owned]});
    let value = format!("hooks={}", settings::toml(&session).unwrap());
    let original = command(&["-c", &value, "-m", "kept"]);
    let prepared = prepare_in(
        &d,
        &original,
        &launch("11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    let doc = prepared
        .args
        .last()
        .unwrap()
        .to_str()
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    assert_eq!(doc["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert_eq!(doc["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(&prepared.args[..original.args.len()], original.args);
}

#[test]
fn persistent_inline_hooks_refuse_composition_without_changing_foreign_or_owned_entries() {
    let owned = crate::runtime::hook_protocol::command_entry(super::super::NAME, "/stable/tmt");
    let foreign = json!({"hooks":[{"type":"command","command":"/user/hook","timeout":7}]});
    for root in [".codex", "system"] {
        for name in ["config.toml", "requirements.toml"] {
            for entry in [&foreign, &owned] {
                let d = TestDirectory::new();
                let path = d.path.join(root).join(name);
                fs::create_dir(path.parent().unwrap()).unwrap();
                let text = format!(
                    "model='kept'\n[hooks]\nStop={}\n",
                    settings::toml(&json!([entry])).unwrap()
                );
                fs::write(&path, &text).unwrap();
                assert!(
                    prepare_in(
                        &d,
                        &command(&["-m", "kept"]),
                        &launch("11111111-1111-4111-8111-111111111111")
                    )
                    .is_err(),
                    "{root}/{name}: {entry}"
                );
                assert_eq!(fs::read_to_string(path).unwrap(), text);
            }
        }
    }
}

#[test]
fn empty_persistent_hook_tables_allow_stable_launch_composition() {
    let d = TestDirectory::new();
    for root in [".codex", "system"] {
        fs::create_dir(d.path.join(root)).unwrap();
        for name in ["config.toml", "requirements.toml"] {
            fs::write(d.path.join(root).join(name), "model='kept'\n[hooks]\n").unwrap();
        }
    }
    let prepared = prepare_in(
        &d,
        &command(&[]),
        &launch("11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    assert_eq!(hooks(&prepared)["Stop"].as_array().unwrap().len(), 2);
}
#[test]
fn disable_ambiguous_edited_and_unreadable_sources_refuse_without_mutation() {
    let d = TestDirectory::new();
    let launch = launch("11111111-1111-4111-8111-111111111111");
    for args in [
        vec!["--disable", "hooks"],
        vec!["-c", "features.hooks=false"],
        vec!["-c", "allow_managed_hooks_only=true"],
        vec!["--profile", "other"],
        vec!["-pother"],
        vec!["-cfeatures.hooks=false"],
        vec!["--ignore-user-config"],
        vec!["--dangerously-bypass-hook-trust"],
        vec!["-c", "hooks.Stop=[]", "-c", "hooks.Stop=[]"],
        vec![
            "-c",
            r#"hooks.Stop=[{hooks=[{command="'/stable/tmt' __hook codex --edited",type="command",timeout=3}]}]"#,
        ],
    ] {
        assert!(
            prepare_in(&d, &command(&args), &launch).is_err(),
            "{args:?}"
        );
    }
    fs::create_dir(d.path.join(".codex")).unwrap();
    fs::create_dir(d.path.join(".codex/hooks.json")).unwrap();
    assert!(prepare_in(&d, &command(&[]), &launch).is_err());
    assert!(d.path.join(".codex/hooks.json").is_dir());
}
#[test]
fn project_hooks_with_uncertain_eligibility_leave_original_sources_alone() {
    let d = TestDirectory::new();
    fs::create_dir(d.path.join(".codex")).unwrap();
    let text = "[hooks]\n";
    fs::write(d.path.join(".codex/config.toml"), text).unwrap();
    // Here the project layer overlaps the user layer; avoid scanning it twice.
    let cwd = d.path.join("project");
    fs::create_dir(&cwd).unwrap();
    let env = ProviderEnvironment::from_parts(&d.path, &cwd, vec![], []);
    assert!(settings::sources(&env, &cwd, &d.path.join("system")).is_ok());
    fs::create_dir(cwd.join(".codex")).unwrap();
    fs::write(cwd.join(".codex/hooks.json"), "{}").unwrap();
    assert!(settings::sources(&env, &cwd, &d.path.join("system")).is_err());
    assert_eq!(
        fs::read_to_string(d.path.join(".codex/config.toml")).unwrap(),
        text
    );
}
#[test]
fn only_unrecursive_main_stop_with_turn_and_session_can_continue() {
    let good = json!({"hook_event_name":"Stop","session_id":"session","turn_id":"turn","stop_hook_active":false});
    assert!(decode(good.to_string().as_bytes()).is_some());
    for key in ["Stop", "SubagentStop", "Interrupt", "UserPromptSubmit"] {
        let mut v = good.clone();
        v["hook_event_name"] = json!(key);
        v["stop_hook_active"] = json!(true);
        assert!(decode(v.to_string().as_bytes()).is_none());
    }
    for event in ["SubagentStop", "Interrupt", "UserPromptSubmit"] {
        let mut v = good.clone();
        v["hook_event_name"] = json!(event);
        assert!(decode(v.to_string().as_bytes()).is_none());
    }
    for field in ["session_id", "turn_id", "stop_hook_active"] {
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove(field);
        assert!(decode(v.to_string().as_bytes()).is_none());
    }
    assert_eq!(
        serde_json::from_str::<Value>(&encode("digest").unwrap()).unwrap(),
        json!({"decision":"block","reason":"digest"})
    );
    assert!(encode("").is_none());
    assert!(encode(&"x".repeat(CONTEXT_LIMIT + 1)).is_none());
}

#[test]
fn attached_config_and_cd_preserve_foreign_hooks_and_inspect_the_selected_project() {
    let d = TestDirectory::new();
    let foreign = json!({"matcher":"startup|resume","hooks":[{"type":"command","command":"/foreign/hook","timeout":7,"statusMessage":"Kept"}]});
    let text = format!(
        "-c=hooks={}",
        settings::toml(&json!({"SessionStart":[foreign]})).unwrap()
    );
    let prepared = prepare_in(
        &d,
        &command(&[&text]),
        &launch("11111111-1111-4111-8111-111111111111"),
    )
    .unwrap();
    let doc = prepared
        .args
        .last()
        .unwrap()
        .to_str()
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    let retained = doc["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(retained.len(), 2);
    let group = retained.get(0).unwrap().as_inline_table().unwrap();
    assert_eq!(
        group.get("matcher").unwrap().as_str(),
        Some("startup|resume")
    );
    let handler = group
        .get("hooks")
        .unwrap()
        .as_array()
        .unwrap()
        .get(0)
        .unwrap()
        .as_inline_table()
        .unwrap();
    assert_eq!(
        handler.get("command").unwrap().as_str(),
        Some("/foreign/hook")
    );
    assert_eq!(handler.get("timeout").unwrap().as_integer(), Some(7));
    assert_eq!(handler.get("statusMessage").unwrap().as_str(), Some("Kept"));
    let project = d.path.join("project");
    fs::create_dir(&project).unwrap();
    fs::create_dir(project.join(".codex")).unwrap();
    fs::write(project.join(".codex/hooks.json"), "{}").unwrap();
    let cd = format!("-C={}", project.display());
    assert!(
        prepare_in(
            &d,
            &command(&[&cd]),
            &launch("11111111-1111-4111-8111-111111111111")
        )
        .is_err()
    );
}
