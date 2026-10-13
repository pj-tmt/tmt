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
        self as auth, AuthState, Client, Clock, FileStore, Project, RandomJitter, Status,
        SystemClock, UreqHttp, presentation,
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
    details: "Uses --remote, the configured default, then this machine's Firestore project. Public remote references grant no membership; a space owner admits the sign-in identity in Colab.",
};
const SIGN_IN: CommandSpec = CommandSpec {
    name: "sign-in",
    summary: "Sign in to Firestore for native sync",
    examples: &[Example {
        command: "tmt remote --remote team sync sign-in --json",
        note: "Sign in using a public remote reference",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "An active sign-in is reused. After it ends, signing in creates a new identity that the space owner must admit in Colab. An unconfirmed attempt is never retried automatically.\nTo revoke a paired device instead, use tmt remote devices revoke <client-id>.",
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
fn mapped(error: auth::AuthError) -> RemoteError {
    RemoteError::new(error.code(), &error.to_string())
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
    let project = Project::from_public_config(&selected.public_config).map_err(mapped)?;
    let store = FileStore::new(layout, &selected.project).map_err(mapped)?;
    let clock = Arc::new(SystemClock::default());
    let http = UreqHttp::new(clock.clone());
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
            presentation::signed_in(uid.expect("confirmed identity"), reused)
        }
        Ok(()) => format!(
            "{} Run `tmt remote sync sign-in` when you need to sync again, then have the space owner admit the new identity in Colab.",
            presentation::revoked(uid)
        ),
        Err(_) if action == "revoke" && status.state == AuthState::Revoking => {
            presentation::REVOKE_UNCONFIRMED.into()
        }
        Err(error) if action == "sign-in" && error.may_have_effect() => {
            presentation::SIGN_IN_UNCONFIRMED.into()
        }
        Err(error) => error.to_string(),
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
    if let Some(selection) = selection {
        let store =
            FileStore::new(layout.expect("selected layout"), &selection.project).map_err(mapped)?;
        let http = UreqHttp::new(clock.clone());
        let cancel = AtomicBool::new(false);
        let mut client = Client::load(
            Project::from_public_config(&selection.public_config).map_err(mapped)?,
            &store,
            &http,
            clock.as_ref(),
            &RandomJitter,
            &cancel,
        )
        .map_err(mapped)?;
        status = client.status();
        notice = client.take_permission_notice();
    }
    Ok(SignInStatus {
        value: serde_json::to_value(presentation::project(&status))
            .expect("safe projection serializes"),
        line: presentation::status_line(&status, clock.now().wall_ms),
        notice,
    })
}
