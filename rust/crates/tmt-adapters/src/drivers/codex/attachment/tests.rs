use super::*;

fn command(args: &[&str]) -> RuntimeCommand {
    RuntimeCommand {
        executable: "/owned/bin/codex".into(),
        args: args.iter().map(OsString::from).collect(),
    }
}

#[test]
fn resumes_exact_thread_on_explicit_endpoint_without_prompt_or_fork() {
    let selected = command(&["-m", "gpt-6-luna", "-C", "project"]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    let foreground = options
        .foreground(
            &selected,
            "ws://127.0.0.1:50000",
            &ProviderSessionId::new("exact-thread").unwrap(),
        )
        .unwrap();
    assert_eq!(foreground.executable, selected.executable);
    assert_eq!(
        foreground.args,
        command(&[
            "resume",
            "--remote",
            "ws://127.0.0.1:50000",
            "--remote-auth-token-env",
            TOKEN_ENV,
            "-C",
            "/owned/project",
            "-m",
            "gpt-6-luna",
            "exact-thread"
        ])
        .args
    );
    assert_eq!(options.working_directory(), Path::new("/owned/project"));
    assert_eq!(
        options.server_arguments(),
        command(&["-c", "model=\"gpt-6-luna\""]).args
    );
}

#[test]
fn fresh_tui_owns_thread_creation_and_typed_permission_options() {
    let selected = command(&[
        "-m",
        "gpt-6-luna",
        "-C",
        "project",
        "--sandbox",
        "read-only",
        "--ask-for-approval",
        "on-request",
        "--no-alt-screen",
    ]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    let fresh = options
        .fresh_foreground(&selected, "ws://127.0.0.1:50000")
        .unwrap();
    assert_eq!(fresh.executable, selected.executable);
    assert_eq!(
        fresh.args,
        command(&[
            "--remote",
            "ws://127.0.0.1:50000",
            "--remote-auth-token-env",
            TOKEN_ENV,
            "-C",
            "/owned/project",
            "-m",
            "gpt-6-luna",
            "--no-alt-screen",
            "--sandbox",
            "read-only",
            "--ask-for-approval",
            "on-request",
        ])
        .args
    );
}

#[test]
fn fresh_tui_preserves_provider_permission_defaults() {
    let selected = command(&[]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    assert_eq!(
        options
            .fresh_foreground(&selected, "ws://127.0.0.1:50000")
            .unwrap()
            .args,
        command(&[
            "--remote",
            "ws://127.0.0.1:50000",
            "--remote-auth-token-env",
            TOKEN_ENV,
            "-C",
            "/owned"
        ])
        .args
    );
}

#[test]
fn initial_input_and_session_switches_are_rejected_before_launch() {
    for args in [
        &["hello"][..],
        &["resume", "another-thread"],
        &["fork"],
        &["--last"],
        &["--remote", "unix://"],
        &["--no-daemon"],
        &["--", "prompt"],
    ] {
        assert!(
            matches!(
                LaunchOptions::parse(&command(args), Path::new("/owned")),
                Err(AttachmentError::UnsupportedArgument(_))
            ),
            "{args:?}"
        );
    }
}

#[test]
fn forwards_configuration_without_shell_interpolation_or_token_value() {
    let selected = command(&[
        "--config=model=\"value\"",
        "--sandbox",
        "read-only",
        "--ask-for-approval",
        "on-request",
        "--no-alt-screen",
    ]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    assert_eq!(
        options.server_arguments(),
        command(&[
            "--config",
            "model=\"value\"",
            "-c",
            "sandbox_mode=\"read-only\"",
            "-c",
            "approval_policy=\"on-request\""
        ])
        .args
    );
    assert!(matches!(
        LaunchOptions::parse(&command(&["--model"]), Path::new("/owned")),
        Err(AttachmentError::MissingValue(_))
    ));
}

#[test]
fn shared_default_and_nonlocal_endpoints_are_rejected() {
    let selected = command(&[]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    let thread = ProviderSessionId::new("exact-thread").unwrap();
    assert!(
        options
            .foreground(&selected, "unix:///owned/control.sock", &thread)
            .is_ok()
    );
    for endpoint in [
        "unix://",
        "unix://relative",
        "ws://localhost:42",
        "ws://127.0.0.1:0",
        "ws://127.0.0.1:42/path",
        "ws://192.0.2.1:42",
        "wss://remote:42",
    ] {
        assert_eq!(
            options.foreground(&selected, endpoint, &thread),
            Err(AttachmentError::InvalidEndpoint)
        );
        assert_eq!(
            options.fresh_foreground(&selected, endpoint),
            Err(AttachmentError::InvalidEndpoint)
        );
    }
}

#[cfg(unix)]
#[test]
fn relative_and_absolute_cd_have_one_observable_directory_from_either_spawn_cwd() {
    use crate::test_support::TestDirectory;
    use std::{fs, process::Command};

    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    // A foreground argv probe implements the provider's -C semantics and
    // reports its actual process cwd. It never starts a provider or model.
    let probe = root.join("foreground.sh");
    fs::write(&probe, "while [ \"$#\" -gt 0 ]; do\ncase \"$1\" in -C|--cd) shift; cd -- \"$1\" || exit 1;; esac\nshift\ndone\npwd -P\n").unwrap();
    for directory in [Path::new("project"), project.as_path()] {
        let selected = RuntimeCommand {
            executable: "codex".into(),
            args: vec!["-C".into(), directory.as_os_str().into()],
        };
        let options = LaunchOptions::parse(&selected, &root).unwrap();
        // Server spawn and thread/start must consume this same resolved cwd.
        assert_eq!(options.working_directory(), project);
        let server = Command::new("/bin/pwd")
            .arg("-P")
            .current_dir(options.working_directory())
            .output()
            .unwrap();
        assert!(server.status.success());
        let foreground = options
            .foreground(
                &selected,
                "unix:///owned/endpoint",
                &ProviderSessionId::new("exact-thread").unwrap(),
            )
            .unwrap();
        for spawn_cwd in [&root, &project] {
            let attached = Command::new("/bin/sh")
                .arg(&probe)
                .args(&foreground.args)
                .current_dir(spawn_cwd)
                .output()
                .unwrap();
            assert!(attached.status.success());
            assert_eq!(attached.stdout, server.stdout);
        }
    }
}

#[test]
fn permission_flags_belong_to_fresh_tui_never_remote_resume() {
    for args in [
        vec!["-s", "read-only", "-a", "on-request"],
        vec!["--sandbox=read-only", "--ask-for-approval=on-request"],
    ] {
        let selected = command(&args);
        let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
        let fresh = options
            .fresh_foreground(&selected, "ws://127.0.0.1:50000")
            .unwrap();
        assert!(
            fresh
                .args
                .windows(2)
                .any(|pair| pair == command(&["--sandbox", "read-only"]).args)
        );
        assert!(
            fresh
                .args
                .windows(2)
                .any(|pair| pair == command(&["--ask-for-approval", "on-request"]).args)
        );
        let foreground = options
            .foreground(
                &selected,
                "ws://127.0.0.1:50000",
                &ProviderSessionId::new("exact-thread").unwrap(),
            )
            .unwrap();
        assert_eq!(
            foreground.args,
            command(&[
                "resume",
                "--remote",
                "ws://127.0.0.1:50000",
                "--remote-auth-token-env",
                TOKEN_ENV,
                "-C",
                "/owned",
                "exact-thread"
            ])
            .args
        );
        assert_eq!(
            options.server_arguments(),
            command(&[
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"on-request\""
            ])
            .args
        );
    }
    let options = LaunchOptions::parse(&command(&[]), Path::new("/owned")).unwrap();
    assert!(options.server_arguments().is_empty());
}

#[test]
fn unmapped_permission_config_and_values_are_explicitly_refused() {
    for key in [
        "approval_policy",
        "approvals_reviewer",
        "sandbox_mode",
        "default_permissions",
        "permissions.profile",
        "network.enabled",
        "sandbox_workspace_write.network_access",
        "\"approval_policy\"",
        "'sandbox_mode'",
        "permi\\ssions",
    ] {
        assert_eq!(
            LaunchOptions::parse(
                &command(&["-c", &format!("{key}=true")]),
                Path::new("/owned")
            )
            .unwrap_err(),
            AttachmentError::UnsupportedConfig,
            "{key}"
        );
    }
    for (flag, value, named) in [
        ("-s", "unknown", "--sandbox"),
        ("-a", "on-failure", "--ask-for-approval"),
    ] {
        assert_eq!(
            LaunchOptions::parse(&command(&[flag, value]), Path::new("/owned")).unwrap_err(),
            AttachmentError::UnsupportedPermission(named)
        );
    }
    let selected = command(&["-c", "model_reasoning_effort=\"low\""]);
    let options = LaunchOptions::parse(&selected, Path::new("/owned")).unwrap();
    assert_eq!(options.server_arguments(), selected.args);
}

#[test]
fn exact_resume_requires_typed_target_and_preserves_model_without_extra_input() {
    let session = ProviderSessionId::new("11111111-1111-4111-8111-111111111111").unwrap();
    let command = RuntimeCommand {
        executable: "codex".into(),
        args: [
            "resume",
            "-m",
            "fixture-model",
            session.as_str(),
            "--no-daemon",
        ]
        .map(OsString::from)
        .to_vec(),
    };
    let cwd = Path::new("/fixture");
    assert!(LaunchOptions::parse(&command, cwd).is_err());
    let options = LaunchOptions::for_launch(&command, cwd, Some(&session)).unwrap();
    assert_eq!(
        options.thread_resume_params(&session),
        serde_json::json!({"threadId":session.as_str(),"cwd":"/fixture","model":"fixture-model"})
    );
    assert_eq!(
        options.server_arguments(),
        ["-c", "model=\"fixture-model\""]
    );
    let mut extra = command.clone();
    extra.args.push("prompt".into());
    assert!(LaunchOptions::for_launch(&extra, cwd, Some(&session)).is_err());
    let other = ProviderSessionId::new("22222222-2222-4222-8222-222222222222").unwrap();
    assert!(LaunchOptions::for_launch(&command, cwd, Some(&other)).is_err());
    assert!(
        LaunchOptions::for_launch(
            &command,
            cwd,
            Some(&ProviderSessionId::new("invalid").unwrap())
        )
        .is_err()
    );
}
