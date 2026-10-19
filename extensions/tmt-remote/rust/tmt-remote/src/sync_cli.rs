//! Command composition uses the public Core root and ordinary-client selector.
//! Nothing opens, and no deployment authority enters the Auth lifecycle.
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{Arc, atomic::AtomicBool},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{
    error::RemoteError,
    firebase_auth::{
        self as auth, AuthState, Client, Clock, FileStore, ProductionHttp, Project, RandomJitter,
        Status, SystemClock, presentation,
    },
    remote_selector,
    state::Layout,
};
const SYNC: CommandSpec = CommandSpec {
    name: "sync",
    summary: "Manage this machine's Firestore sign-in",
    examples: &[Example {
        command: "tmt remote sync sign-in --json",
        note: "Sign in to the selected Firestore remote",
    }],
    outputs: OutputModes::Human,
    details: "Uses --remote, the configured default, then this machine's Firestore project. Public remote references grant no membership. For a remote added with tmt remote add, page access for this machine is not available yet. To open a shared page in your browser, use the page link from its owner.",
};
const SIGN_IN: CommandSpec = CommandSpec {
    name: "sign-in",
    summary: "Sign this machine in to a remote",
    examples: &[Example {
        command: "tmt remote --remote team sync sign-in --json",
        note: "Sign this machine in to the saved remote",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Use the selected remote's anonymous sign-in. No personal account is needed. An active sign-in is reused. After it ends, signing in creates a new identity. For a remote added with tmt remote add, page access for this machine is not available yet. To open a shared page in your browser, use the page link from its owner. If a sign-in is not confirmed, it is not retried automatically; run the command again.\nTo revoke a paired device instead, use tmt remote devices revoke <client-id>.",
};
const REVOKE: CommandSpec = CommandSpec {
    name: "revoke",
    summary: "Revoke this machine's Firestore sign-in",
    examples: &[Example {
        command: "tmt remote sync revoke --json",
        note: "End the selected Firestore sign-in",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "An unconfirmed revoke keeps the sign-in blocked; run this command again to finish. Other machines' identities and paired devices are unchanged.\nTo revoke a paired device instead, use tmt remote devices revoke <client-id>.",
};
pub(crate) fn command() -> Command {
    tmt_cli_style::command(&SYNC)
        .subcommand_required(true)
        .subcommand(action(&SIGN_IN))
        .subcommand(action(&REVOKE))
}
fn action(spec: &CommandSpec) -> Command {
    tmt_cli_style::command(spec).arg(
        Arg::new("project")
            .long("project")
            .help("Assert the selected Firebase project id"),
    )
}
fn mapped(error: auth::AuthError, kind: remote_selector::ClientKind) -> RemoteError {
    RemoteError::new(error.code(), &presentation::error_line(&error, kind))
}
pub(crate) fn run(
    layout: &Layout,
    explicit: Option<&str>,
    arguments: &ArgMatches,
) -> Result<(), RemoteError> {
    let (action, words) = arguments.subcommand().expect("required sync action");
    let selected = remote_selector::client(
        layout,
        explicit,
        words.get_one::<String>("project").map(String::as_str),
    )?;
    let mapped = |error| mapped(error, selected.kind);
    let project = Project::from_public_config(&selected.public_config).map_err(mapped)?;
    let store = FileStore::new(layout, &selected.project).map_err(mapped)?;
    let clock = Arc::new(SystemClock::default());
    let http = ProductionHttp::new(&project, clock.clone()).map_err(mapped)?;
    let cancel = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, cancel.clone())?;
    }
    let mut client = Client::load(
        project,
        &store,
        &http,
        clock.as_ref(),
        &RandomJitter,
        cancel.as_ref(),
    )
    .map_err(mapped)?;
    let before = client.status();
    let reused = action == "sign-in" && before.state == AuthState::Active;
    // Reusing an active identity needs no network confirmation. Token demand is
    // handled by the serving Auth owner, never by this idempotent command.
    let result = if reused {
        Ok(())
    } else if action == "sign-in" {
        client.sign_in()
    } else {
        client.revoke()
    };
    let status = client.status();
    let notice = client.take_permission_notice();
    client.stop();
    let json_output = words.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let uid = if action == "revoke" {
        before.uid.as_deref()
    } else {
        status.uid.as_deref()
    };
    let message = match result {
        Ok(()) if action == "sign-in" => {
            presentation::signed_in(uid.expect("confirmed identity"), reused, selected.kind)
        }
        Ok(()) => presentation::revoked(uid, selected.kind),
        Err(_) if action == "revoke" && status.state == AuthState::Revoking => {
            presentation::REVOKE_UNCONFIRMED.into()
        }
        Err(error) if action == "sign-in" && error.may_have_effect() => {
            presentation::sign_in_unconfirmed(selected.kind)
        }
        Err(error) => presentation::error_line(&error, selected.kind),
    };
    let sign_in = if result.is_err()
        && action == "sign-in"
        && result.as_ref().is_err_and(|e| e.may_have_effect())
    {
        presentation::unconfirmed(None)
    } else {
        presentation::project(&status)
    };
    if json_output {
        let mut answer = json!({"project":selected.project,"uid":uid,"signIn":sign_in});
        if let Err(error) = result {
            answer["error"] = json!({"code":error.code(),"message":message});
        } else {
            answer["reused"] = json!(reused);
        }
        if let Some(notice) = notice {
            answer["notices"] = json!([notice]);
        }
        writeln!(output, "{answer}")?;
    } else {
        writeln!(output, "{message}")?;
        if let Some(notice) = notice {
            writeln!(output, "{notice}")?;
        }
    }
    if result.is_err() {
        Err(RemoteError::new("REMOTE_SYNC_REPORTED", ""))
    } else {
        Ok(())
    }
}
pub(crate) struct SignInStatus {
    pub value: Value,
    pub line: Option<String>,
    pub notice: Option<&'static str>,
}
pub(crate) fn status(
    layout: Option<&Layout>,
    explicit: Option<&str>,
    project: Option<&str>,
) -> Result<SignInStatus, RemoteError> {
    let selection = match layout {
        Some(layout) => remote_selector::client_status(layout, explicit, project)?,
        None if explicit.is_some() || project.is_some() => {
            return Err(remote_selector::missing(explicit.or(project).unwrap()));
        }
        None => None,
    };
    let clock = Arc::new(SystemClock::default());
    let mut status = Status {
        state: AuthState::Unenrolled,
        uid: None,
        expires_at_ms: None,
        expiry_notice: false,
        permissions_repaired: false,
    };
    let mut notice = None;
    if let Some(selection) = selection.as_ref() {
        let mapped = |error| mapped(error, selection.kind);
        let project = Project::from_public_config(&selection.public_config).map_err(mapped)?;
        let loaded = (|| {
            let store = FileStore::new(layout.expect("selected layout"), &selection.project)?;
            let http = ProductionHttp::new(&project, clock.clone())?;
            let cancel = AtomicBool::new(false);
            let mut client = Client::load(
                project,
                &store,
                &http,
                clock.as_ref(),
                &RandomJitter,
                &cancel,
            )?;
            Ok::<_, auth::AuthError>((client.status(), client.take_permission_notice()))
        })();
        match loaded {
            Ok((loaded_status, loaded_notice)) => {
                status = loaded_status;
                notice = loaded_notice;
            }
            Err(error) => {
                return Ok(SignInStatus {
                    value: serde_json::to_value(presentation::diagnostic(&error))
                        .expect("safe diagnostic serializes"),
                    line: Some(presentation::error_line(&error, selection.kind)),
                    notice: None,
                });
            }
        }
    }
    Ok(SignInStatus {
        value: serde_json::to_value(presentation::project(&status))
            .expect("safe projection serializes"),
        line: selection.as_ref().and_then(|selection| {
            presentation::status_line(&status, clock.now().wall_ms, selection.kind)
        }),
        notice,
    })
}
