//! Identity command composition. Policy and SQL remain in core/service and
//! adapter owners; this layer resolves optional callers and projects output.

use crate::{
    identity_context,
    invocation::{IdentityFilterRequest, IdentityMetadataRequest, IdentityRequest, OutputMode},
    output::{Failure, after_cleanup, identity_document},
};
use serde_json::json;
use std::{
    error::Error,
    io::{self, Write},
};
use tmt_adapters::{
    config::ConfigPaths,
    storage::{Storage, StorageError},
};
use tmt_core::{
    identity::{self, Identity, IdentityError, IdentityReader, Lifetime},
    identity_metadata::{self, MetadataError, MetadataFilter},
};

mod status;

enum Report {
    Status(status::Report),
    Created(identity::CreatedIdentity),
    /// The identity with its remembered-session projection (JSON only).
    Shown(Identity, Option<serde_json::Value>),
    Listed(Vec<Identity>),
    MetadataSet {
        identity_id: String,
        key: String,
        value: String,
        changed: bool,
    },
    MetadataGet {
        identity_id: String,
        key: String,
        value: String,
    },
    MetadataList {
        identity_id: String,
        metadata: std::collections::BTreeMap<String, String>,
    },
    MetadataRemoved {
        identity_id: String,
        key: String,
        removed: bool,
    },
}

fn unavailable(error: impl Error + 'static) -> Failure {
    Failure::new(
        "IDENTITY_ERROR",
        "Could not complete the identity operation.",
        1,
    )
    .caused_by(error)
}

fn show_selection_failure(error: Failure) -> Failure {
    if error.code == "IDENTITY_REQUIRED" {
        Failure::new(
            "IDENTITY_REQUIRED",
            "An identity is required; use identity show <name> or run from a verified bound pane.",
            1,
        )
    } else {
        error
    }
}

fn identity_failure(error: IdentityError<StorageError>) -> Failure {
    match error {
        IdentityError::InvalidName(error) => {
            Failure::new("INVALID_NAME", error.to_string(), 1).caused_by(error)
        }
        IdentityError::Repository(error) => unavailable(error),
    }
}

fn metadata_failure(error: MetadataError<StorageError>) -> Failure {
    match error {
        MetadataError::Invalid(error) => {
            Failure::new("IDENTITY_METADATA_INVALID", error.to_string(), 1).caused_by(error)
        }
        MetadataError::IdentityNotFound => {
            Failure::new("NAME_NOT_FOUND", "The selected identity was not found.", 3)
        }
        MetadataError::KeyNotFound => Failure::new(
            "METADATA_KEY_NOT_FOUND",
            "The metadata key was not found.",
            3,
        ),
        MetadataError::Repository(error) => unavailable(error),
    }
}

fn run(request: IdentityRequest) -> Result<Report, Failure> {
    // Explicit named identity operations remain storage-only and do not probe tmux.
    let selector = match &request {
        IdentityRequest::Metadata { identity, .. } | IdentityRequest::Status { identity, .. } => {
            Some(identity_context::required(identity.as_deref())?)
        }
        IdentityRequest::Show(None) => {
            Some(identity_context::required(None).map_err(show_selection_failure)?)
        }
        _ => None,
    };
    let paths = ConfigPaths::discover().map_err(unavailable)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No identity was changed.",
            "IDENTITY_ERROR",
            "Could not complete the identity operation.",
        )
    })?;
    let pending = operation(&mut storage, request, selector);
    after_cleanup(pending, || storage.close())
}

fn shown(storage: &Storage, identity: Identity) -> Result<Report, Failure> {
    let preferences = storage
        .session_preferences(&identity.id)
        .map_err(unavailable)?;
    let registry = tmt_adapters::runtime::RuntimeRegistry::first_party();
    let resume = crate::output::resume_document(&preferences, &registry);
    Ok(Report::Shown(identity, resume))
}

fn operation(
    storage: &mut Storage,
    request: IdentityRequest,
    selector: Option<identity_context::Selector>,
) -> Result<Report, Failure> {
    match request {
        IdentityRequest::Status { operation, .. } => {
            let identity = identity_context::resolve(
                storage,
                selector.expect("status request resolved a selector"),
            )?;
            status::run(storage, identity.id, operation).map(Report::Status)
        }
        IdentityRequest::Create(name) => {
            identity::create_or_resolve(storage, &name, Lifetime::Saved)
                .map(Report::Created)
                .map_err(identity_failure)
        }
        IdentityRequest::Show(Some(name)) => {
            tmt_core::names::validate_name(&name).map_err(|error| {
                Failure::new("INVALID_NAME", error.to_string(), 1).caused_by(error)
            })?;
            let identity = storage
                .resolve_identity(&name)
                .map_err(unavailable)?
                .ok_or_else(|| {
                    Failure::new(
                        "NAME_NOT_FOUND",
                        format!("Identity '{name}' was not found."),
                        3,
                    )
                })?;
            shown(storage, identity)
        }
        IdentityRequest::Show(None) => {
            let identity = identity_context::resolve(
                storage,
                selector.expect("unnamed show resolved a selector"),
            )
            .map_err(show_selection_failure)?;
            shown(storage, identity)
        }
        IdentityRequest::List(filters) => {
            if filters.is_empty() {
                return storage
                    .list_identities()
                    .map(Report::Listed)
                    .map_err(unavailable);
            }
            let filters = filters
                .into_iter()
                .map(|filter| match filter {
                    IdentityFilterRequest::Equals { key, value } => {
                        MetadataFilter::equals(&key, &value)
                    }
                    IdentityFilterRequest::Has(key) => MetadataFilter::has(&key),
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| metadata_failure(MetadataError::Invalid(error)))?;
            identity_metadata::search_identities_by_metadata(storage, &filters)
                .map(Report::Listed)
                .map_err(metadata_failure)
        }
        IdentityRequest::Metadata { operation, .. } => {
            let identity = identity_context::resolve(
                storage,
                selector.expect("metadata request resolved a selector"),
            )?;
            match operation {
                IdentityMetadataRequest::Set { key, value } => {
                    let result = identity_metadata::set_identity_metadata(
                        storage,
                        &identity.id,
                        &key,
                        &value,
                    )
                    .map_err(metadata_failure)?;
                    Ok(Report::MetadataSet {
                        identity_id: result.identity_id,
                        key: result.key,
                        value: result.value,
                        changed: result.changed,
                    })
                }
                IdentityMetadataRequest::Get { key } => {
                    let result =
                        identity_metadata::get_identity_metadata(storage, &identity.id, &key)
                            .map_err(metadata_failure)?;
                    Ok(Report::MetadataGet {
                        identity_id: result.identity_id,
                        key: result.key,
                        value: result.value,
                    })
                }
                IdentityMetadataRequest::List => {
                    let result = identity_metadata::list_identity_metadata(storage, &identity.id)
                        .map_err(metadata_failure)?;
                    Ok(Report::MetadataList {
                        identity_id: result.identity_id,
                        metadata: result.metadata,
                    })
                }
                IdentityMetadataRequest::Remove { key } => {
                    let result =
                        identity_metadata::remove_identity_metadata(storage, &identity.id, &key)
                            .map_err(metadata_failure)?;
                    Ok(Report::MetadataRemoved {
                        identity_id: result.identity_id,
                        key: result.key,
                        removed: result.removed,
                    })
                }
            }
        }
    }
}

/// `SAVED n` and `TEMPORARY n` sections, each sorted by name; the id is
/// shortened because no command takes it as input.
fn write_list(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    mut identities: Vec<Identity>,
) -> io::Result<()> {
    use tmt_cli_style::{
        Token,
        list::{self, Section},
        table::{Cell, Column, Table},
        value,
    };
    identities.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
    let sections: Vec<Section> = [
        ("saved", tmt_core::identity::Lifetime::Saved),
        ("temporary", tmt_core::identity::Lifetime::Temporary),
    ]
    .into_iter()
    .filter_map(|(title, lifetime)| {
        let matching: Vec<&Identity> = identities
            .iter()
            .filter(|identity| identity.lifetime == lifetime)
            .collect();
        let mut rows = Table::new(&[Column::Name, Column::Fixed]);
        for identity in &matching {
            rows.row([
                Cell::from(&identity.name),
                Cell::styled(value::short_id(&identity.id), Token::Dim),
            ]);
        }
        (!matching.is_empty()).then_some(Section {
            title,
            count: Some(matching.len()),
            rows,
            note: None,
            hint: None,
        })
    })
    .collect();
    list::write(output, terminal, &sections)
}

pub fn execute(request: IdentityRequest, mode: OutputMode) -> io::Result<u8> {
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let outcome = match &report {
        Report::Created(result) if result.created => {
            crate::skill_reminder::Outcome::SavedIdentityCreated {
                name: result.identity.name.clone(),
            }
        }
        _ => crate::skill_reminder::Outcome::None,
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        let document = document(&report);
        writeln!(stdout, "{document}")?;
    } else {
        match report {
            Report::Status(report) => report.write(&mut stdout, terminal)?,
            Report::Created(result) => {
                if result.created {
                    tmt_cli_style::message::success(
                        &mut stdout,
                        terminal,
                        &format!(
                            "Created saved identity '{}' ({})",
                            result.identity.name, result.identity.id
                        ),
                    )?;
                } else {
                    writeln!(
                        stdout,
                        "Saved identity '{}' already exists ({})",
                        result.identity.name, result.identity.id
                    )?;
                }
            }
            Report::Shown(identity, _) => tmt_cli_style::detail::write(
                &mut stdout,
                terminal,
                &identity.name,
                &[
                    ("lifetime", identity.lifetime.as_str().to_owned()),
                    ("canonical", identity.canonical_name),
                    ("id", identity.id),
                ],
            )?,
            Report::Listed(identities) if identities.is_empty() => {
                writeln!(stdout, "No identities found.")?;
                tmt_cli_style::message::hint(&mut stdout, terminal, "tmt name <name>")?;
            }
            Report::Listed(identities) => write_list(&mut stdout, terminal, identities)?,
            Report::MetadataSet {
                key,
                value,
                changed,
                ..
            } => {
                if changed {
                    tmt_cli_style::message::success(
                        &mut stdout,
                        terminal,
                        &format!("Set {key}={value}"),
                    )?;
                } else {
                    writeln!(stdout, "{key}={value} is unchanged")?;
                }
            }
            Report::MetadataGet { value, .. } => writeln!(stdout, "{value}")?,
            Report::MetadataList { metadata, .. } if metadata.is_empty() => {
                writeln!(stdout, "No metadata found.")?;
            }
            Report::MetadataList { metadata, .. } => tmt_cli_style::detail::write(
                &mut stdout,
                terminal,
                "METADATA",
                &metadata
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.clone()))
                    .collect::<Vec<_>>(),
            )?,
            Report::MetadataRemoved { key, removed, .. } => {
                if removed {
                    tmt_cli_style::message::success(
                        &mut stdout,
                        terminal,
                        &format!("Removed {key}"),
                    )?;
                } else {
                    writeln!(stdout, "{key} was already absent")?;
                }
            }
        }
    }
    drop(stdout);
    crate::skill_reminder::present(outcome, mode, true);
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "tmt name <name>",
        &[""],
        &[],
    )];

fn document(report: &Report) -> serde_json::Value {
    match report {
        Report::Status(report) => report.value(),
        Report::Created(result) => {
            json!({"identity": identity_document(&result.identity), "created": result.created})
        }
        Report::Shown(identity, resume) => {
            let mut document = json!({"identity": identity_document(identity)});
            if let Some(resume) = resume {
                document["resume"] = resume.clone();
            }
            document
        }
        Report::Listed(identities) => {
            json!({"identities": identities.iter().map(identity_document).collect::<Vec<_>>()})
        }
        Report::MetadataSet {
            identity_id,
            key,
            value,
            changed,
        } => json!({"identityId": identity_id, "key": key, "value": value, "changed": changed}),
        Report::MetadataGet {
            identity_id,
            key,
            value,
        } => json!({"identityId": identity_id, "key": key, "value": value}),
        Report::MetadataList {
            identity_id,
            metadata,
        } => json!({"identityId": identity_id,
                "metadata": tmt_adapters::identity_projection::metadata_value(metadata)}),
        Report::MetadataRemoved {
            identity_id,
            key,
            removed,
        } => json!({"identityId": identity_id, "key": key, "removed": removed}),
    }
}

pub(crate) fn list_document(paths: &ConfigPaths) -> Result<serde_json::Value, Failure> {
    let mut storage = Storage::open(&paths.database).map_err(unavailable)?;
    let pending = operation(&mut storage, IdentityRequest::List(Vec::new()), None);
    after_cleanup(pending, || storage.close()).map(|report| document(&report))
}
