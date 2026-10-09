//! `tmt resume --forget <name>`: clear an identity's remembered session
//! explicitly. Resuming itself shares the foreground launch in run_command.

use crate::{invocation::OutputMode, output::Failure};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    storage::{Storage, StorageError},
};
use tmt_core::{binding::BindingRepository, identity::IdentityReader, names::normalize_name};

fn storage_failure(error: StorageError) -> Failure {
    Failure::new(
        "IDENTITY_ERROR",
        "Could not access remembered session state.",
        1,
    )
    .caused_by(error)
}

/// Named inspection stays storage-only. An unnamed operation uses the shared
/// caller selector and refuses an empty pane with the saved-name inventory.
pub fn execute(
    name: Option<&str>,
    show: bool,
    forget_launch: bool,
    forget: bool,
    retry: bool,
    options: tmt_adapters::runtime::launch_preset::LaunchSettings,
    channel: crate::invocation::ChannelMode,
) -> io::Result<u8> {
    // Preserve named launch preflight and error behavior. Only inspection,
    // clearing, and implicit caller selection need this storage composition.
    if let Some(name) = name {
        if forget {
            return self::forget(name);
        }
        if !show && !forget_launch {
            return crate::run_command::resume_with_options(name, retry, &options, channel);
        }
    }
    let selected = (|| {
        if name.is_none() {
            crate::caller_context::require_independent_host()?;
        }

        if !options.valid() {
            return Err(Failure::new(
                "RESUME_SETTINGS_INVALID",
                "Model and effort must be bounded, non-option tokens.",
                1,
            ));
        }
        let paths = ConfigPaths::discover().map_err(Failure::from)?;
        let mut storage = Storage::open(&paths.database).map_err(storage_failure)?;
        let host =
            tmt_adapters::host::Host::for_caller(&tmt_adapters::host::CallerEnvironment::current());
        let identity = match crate::identity_context::optional(&mut storage, &host, name)? {
            Some(identity) => identity,
            None => {
                let names = storage
                    .list_identities()
                    .map_err(storage_failure)?
                    .into_iter()
                    .filter(|i| i.lifetime == tmt_core::identity::Lifetime::Saved)
                    .map(|i| i.name)
                    .collect::<Vec<_>>();
                return Err(Failure::new(
                    "IDENTITY_REQUIRED",
                    format!(
                        "No identity is bound to this pane. Specify a saved identity: {}",
                        if names.is_empty() {
                            "(none)".into()
                        } else {
                            names.join(", ")
                        }
                    ),
                    1,
                ));
            }
        };
        let report = if show {
            let mut preset = tmt_adapters::runtime::launch_preset::read(&storage, &identity.id)
                .map_err(storage_failure)?;
            let preferences = storage
                .session_preferences(&identity.id)
                .map_err(storage_failure)?;
            if let (Some(preset), Some(session)) = (&mut preset, &preferences.remembered)
                && preset.matches(session)
            {
                preset.associate(
                    session,
                    &tmt_adapters::runtime::RuntimeRegistry::first_party(),
                );
            }
            Some(
                serde_json::to_string_pretty(
                    &serde_json::json!({"identity": identity.name, "launch": preset.map(|preset|
                        serde_json::json!({"executable": preset.executable,
                            "model": preset.model, "effort": preset.effort,
                            "env": preset.env, "associated": preset.session.is_some()}))}),
                )
                .expect("launch preset serializes"),
            )
        } else if forget_launch {
            if !tmt_adapters::runtime::launch_preset::clear(&mut storage, &identity.id)
                .map_err(storage_failure)?
            {
                return Err(Failure::new(
                    "NAME_NOT_FOUND",
                    "The identity changed before clearing its launch preset.",
                    3,
                ));
            }
            Some(format!("Forgot {}'s launch preset.", identity.name))
        } else {
            None
        };
        storage.close().map_err(storage_failure)?;
        Ok((identity.name, report))
    })();
    match selected {
        Ok((_, Some(report))) => {
            let mut stdout = tmt_cli_style::stream::stdout(false);
            writeln!(stdout, "{report}")?;
            Ok(0)
        }
        Ok((name, None)) if forget => self::forget(&name),
        Ok((name, None)) => {
            crate::run_command::resume_with_options(&name, retry, &options, channel)
        }
        Err(error) => error.publish(OutputMode::default()),
    }
}

pub fn forget(name: &str) -> io::Result<u8> {
    match forgotten(name) {
        Ok((forgot, message)) => {
            let mut stdout = tmt_cli_style::stream::stdout(false);
            let terminal = stdout.terminal();
            if forgot {
                tmt_cli_style::message::success(&mut stdout, terminal, &message)?;
            } else {
                writeln!(stdout, "{message}")?;
            }
            Ok(0)
        }
        Err(error) => error.publish(OutputMode::default()),
    }
}

/// Whether a session was forgotten, and the message saying so.
fn forgotten(name: &str) -> Result<(bool, String), Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No remembered session was changed.",
            "IDENTITY_ERROR",
            "Could not access remembered session state.",
        )
    })?;
    let result = (|| {
        let identity = storage
            .find_identity(&normalize_name(name))
            .map_err(storage_failure)?
            .ok_or_else(|| {
                Failure::new(
                    "NAME_NOT_FOUND",
                    format!("Identity '{name}' was not found."),
                    3,
                )
            })?;
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                let mut preferences = records.session_preferences(&identity.id)?;
                let Some(session) = preferences.remembered.take() else {
                    return Ok((
                        false,
                        format!("{} has no remembered session.", identity.name),
                    ));
                };
                records.set_session_preferences(&identity.id, &preferences)?;
                Ok((
                    true,
                    format!(
                        "Forgot {}'s remembered {} session.",
                        identity.name,
                        session.harness.as_str()
                    ),
                ))
            })
            .map_err(storage_failure)
    })();
    let closed = storage.close().map_err(storage_failure);
    let message = result?;
    closed?;
    Ok(message)
}
