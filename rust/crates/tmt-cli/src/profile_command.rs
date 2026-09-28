//! Role and preamble presentation over shared profile and identity owners.

use crate::{
    identity_context,
    invocation::{ContentInput, Invocation, OutputMode, PreambleRequest, RoleOperation},
    output::{Failure, after_cleanup, identity_document},
};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::{
    bounded_file::{self, FileReadError},
    config::ConfigPaths,
    storage::Storage,
};
use tmt_core::{
    identity::Identity,
    profile::{self, ContentError, Profile, ProfileKind, ProfileReader},
};

enum Action {
    Show,
    Set(String),
    Clear,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Shown,
    Set,
    Cleared,
    NotSet,
}
impl Change {
    fn status(self) -> Option<&'static str> {
        match self {
            Self::Shown => None,
            Self::Set => Some("set"),
            Self::Cleared => Some("cleared"),
            Self::NotSet => Some("not_set"),
        }
    }
}
enum Report {
    Role {
        identity: Identity,
        profile: Option<Profile>,
        change: Change,
    },
    Preamble {
        name: String,
        profile: Option<Profile>,
        change: Change,
    },
    List(Vec<(Identity, Profile)>),
}

fn error_code(kind: ProfileKind) -> &'static str {
    match kind {
        ProfileKind::Role => "ROLE_ERROR",
        ProfileKind::Preamble => "PREAMBLE_ERROR",
    }
}

fn input_failure(kind: ProfileKind, error: ContentError) -> Failure {
    let code = match (kind, error) {
        (ProfileKind::Role, ContentError::TooLarge) => "ROLE_INPUT_TOO_LARGE",
        (ProfileKind::Preamble, ContentError::TooLarge) => "PREAMBLE_INPUT_TOO_LARGE",
        (ProfileKind::Role, _) => "ROLE_INPUT_INVALID",
        (ProfileKind::Preamble, _) => "PREAMBLE_INPUT_INVALID",
    };
    Failure::new(code, error.to_string(), 1).caused_by(error)
}

fn role_input(input: ContentInput) -> Result<String, Failure> {
    match input {
        ContentInput::Inline(content) => Ok(content),
        ContentInput::File(file) => {
            let bytes = bounded_file::read(Path::new(&file), profile::MAX_PROFILE_BYTES).map_err(
                |error| match error {
                    FileReadError::TooLarge => {
                        input_failure(ProfileKind::Role, ContentError::TooLarge)
                    }
                    error => {
                        Failure::new("ROLE_FILE_ERROR", "Could not read a regular role file.", 1)
                            .caused_by(error)
                    }
                },
            )?;
            String::from_utf8(bytes).map_err(|error| {
                Failure::new("ROLE_INPUT_INVALID", "Role file must be valid UTF-8.", 1)
                    .caused_by(error)
            })
        }
        ContentInput::Stdin => Err(Failure::new(
            "ROLE_INPUT_INVALID",
            "Role input requires inline text or a regular file.",
            1,
        )),
    }
}

fn run(request: Invocation) -> Result<Report, Failure> {
    let (kind, name, action) = match request {
        Invocation::Role {
            identity,
            operation,
        } => (
            ProfileKind::Role,
            identity,
            match operation {
                RoleOperation::Show => Some(Action::Show),
                RoleOperation::Clear => Some(Action::Clear),
                RoleOperation::Set(input) => Some(Action::Set(role_input(input)?)),
            },
        ),
        Invocation::Preamble(request) => match request {
            PreambleRequest::Show(name) => (
                ProfileKind::Preamble,
                name.clone(),
                name.map(|_| Action::Show),
            ),
            PreambleRequest::Set { name, content } => (
                ProfileKind::Preamble,
                Some(name),
                Some(Action::Set(content)),
            ),
            PreambleRequest::Clear(name) => {
                (ProfileKind::Preamble, Some(name), Some(Action::Clear))
            }
        },
        _ => unreachable!("profile dispatcher accepts only role and preamble"),
    };
    let unavailable = |error| {
        Failure::new(error_code(kind), "Could not access profile storage.", 1).caused_by(error)
    };
    let selector = action
        .as_ref()
        .map(|_| identity_context::required(name.as_deref()))
        .transpose()?;
    let paths = ConfigPaths::discover().map_err(|error| {
        Failure::new(error_code(kind), "Could not discover profile storage.", 1).caused_by(error)
    })?;
    let mut storage = Storage::open(paths.database).map_err(unavailable)?;
    let pending = (|| {
        let Some(action) = action else {
            return storage
                .list_preambles()
                .map(Report::List)
                .map_err(unavailable);
        };
        let identity = identity_context::resolve(
            &mut storage,
            selector.expect("profile action selects identity"),
        )
        .map_err(|error| {
            if error.code == "IDENTITY_ERROR" {
                Failure::new(error_code(kind), "Could not access profile identity.", 1)
                    .caused_by(error)
            } else {
                error
            }
        })?;
        let (value, change) = match action {
            Action::Show => (
                storage
                    .find_profile(&identity.id, kind)
                    .map_err(unavailable)?,
                Change::Shown,
            ),
            Action::Set(content) => {
                let content = profile::normalize_content(&content)
                    .map_err(|error| input_failure(kind, error))?;
                let value = profile::set_profile(&mut storage, &identity, kind, &content)
                    .map_err(unavailable)?
                    .ok_or_else(|| crate::output::identity_missing(&identity.name))?;
                (Some(value), Change::Set)
            }
            Action::Clear => {
                let cleared = profile::clear_profile(&mut storage, &identity, kind)
                    .map_err(unavailable)?
                    .ok_or_else(|| crate::output::identity_missing(&identity.name))?;
                (
                    None,
                    if cleared {
                        Change::Cleared
                    } else {
                        Change::NotSet
                    },
                )
            }
        };
        Ok(match kind {
            ProfileKind::Role => Report::Role {
                identity,
                profile: value,
                change,
            },
            ProfileKind::Preamble => Report::Preamble {
                name: identity.name,
                profile: value,
                change,
            },
        })
    })();
    after_cleanup(pending, || storage.close())
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = io::stdout().lock();
    if mode.json {
        let value = match report {
            Report::Role {
                identity, profile, ..
            } => {
                serde_json::json!({"identity": identity_document(&identity), "role": profile.map(|profile| serde_json::json!({"content": profile.content, "updatedAt": profile.updated_at}))})
            }
            Report::Preamble {
                name,
                profile,
                change,
            } => {
                let mut value = serde_json::json!({"agent": name});
                if matches!(change, Change::Shown | Change::Set) {
                    value["preamble"] = profile.map(|profile| profile.content).into();
                }
                if let Some(status) = change.status() {
                    value["status"] = status.into();
                }
                value
            }
            Report::List(rows) => {
                serde_json::json!({"preambles": rows.into_iter().map(|(identity, profile)| serde_json::json!({"agent": identity.name, "preamble": profile.content})).collect::<Vec<_>>()})
            }
        };
        writeln!(stdout, "{value}")?;
    } else {
        match report {
            Report::Role {
                identity,
                profile,
                change,
            } => match change {
                Change::Set => writeln!(stdout, "Set role profile for '{}'.", identity.name)?,
                Change::Cleared | Change::NotSet => {
                    writeln!(stdout, "Cleared role profile for '{}'.", identity.name)?
                }
                Change::Shown => {
                    writeln!(stdout, "Identity '{}'", identity.name)?;
                    writeln!(
                        stdout,
                        "{}",
                        profile.map_or_else(
                            || "No role profile is set.".into(),
                            |profile| profile.content
                        )
                    )?;
                }
            },
            Report::Preamble {
                name,
                profile,
                change,
            } => match change {
                Change::Set => writeln!(stdout, "Set preamble for {name}")?,
                Change::Cleared => writeln!(stdout, "Cleared preamble for {name}")?,
                Change::NotSet => writeln!(stdout, "No preamble was set for {name}")?,
                Change::Shown => {
                    if let Some(profile) = profile {
                        writeln!(stdout, "Preamble for {name}:\n{}", profile.content)?;
                    } else {
                        writeln!(stdout, "No preamble set for {name}")?;
                    }
                }
            },
            Report::List(rows) => {
                if rows.is_empty() {
                    writeln!(stdout, "No preambles configured")?;
                }
                for (identity, profile) in rows {
                    writeln!(stdout, "─── {} ───\n{}\n", identity.name, profile.content)?;
                }
            }
        }
    }
    Ok(0)
}
