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
