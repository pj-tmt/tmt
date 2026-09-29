//! Thin composition of typed commands, one storage handle and binding policy.
//! Preflight caller/target checks precede configuration and database effects.

mod presentation;

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    invocation::{Invocation, ListScope, OutputMode},
    output::{Failure, after_cleanup},
};
use std::io::{self, Write};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    storage::Storage,
    tmux::{BindingSession, CallerEnvironment, OperationOptions, Tmux},
};
use tmt_core::{
    binding::session::{RememberedSession, RuntimeState},
    binding::{
        self, BindingEntry, BindingTargetEvidence, BoundIdentity, IdentityPresence, UnboundIdentity,
    },
    endpoint::PaneObservation,
    identity::Identity,
    names::is_pane_target,
    settings::PaneBadge,
};

/// One `tmt ls` row: its presence and its remembered session, if any.
struct ListedRow {
    presence: IdentityPresence,
    remembered: Option<RememberedSession>,
    /// The existing `resume` JSON projection, kept byte for byte.
    resume: Option<serde_json::Value>,
}

struct ResolvedPane {
    id: String,
    frozen: Option<BindingTargetEvidence>,
}

enum Report {
    Bound(BoundIdentity),
    Caller {
        pane: String,
        identity: Option<Identity>,
        runtime: RuntimeState,
    },
    Unbound {
        pane: String,
        result: UnboundIdentity,
    },
    Removed(BindingEntry),
    /// The rows `tmt ls` shows, after its filters, in storage order.
    Listed {
        rows: Vec<ListedRow>,
        scope: ListScope,
    },
    Named {
        target: String,
        row: IdentityPresence,
        remembered: Option<RememberedSession>,
    },
    Pane {
        target: String,
        pane: PaneObservation,
        identity: Option<Identity>,
        remembered: Option<RememberedSession>,
    },
}

fn remembered_session(
    storage: &Storage,
    identity: &Identity,
) -> Result<Option<RememberedSession>, Failure> {
    storage
        .session_preferences(&identity.id)
        .map(|preferences| preferences.remembered)
        .map_err(|error| {
            Failure::new("IDENTITY_ERROR", "Could not read remembered sessions.", 1)
                .caused_by(error)
        })
}

fn preflight(
    request: &Invocation,
    tmux: &Tmux,
    environment: &CallerEnvironment,
) -> Result<Option<ResolvedPane>, Failure> {
    let target = match request {
        Invocation::BindMarked { .. } => {
            let target = tmux
                .marked_pane(environment, OperationOptions::default())
                .map_err(endpoint_failure)?
                .ok_or_else(|| {
                    Failure::new(
                        "MARKED_PANE_NOT_FOUND",
                        "No marked pane was found on the selected tmux server.",
                        3,
                    )
                    .suggestion("Mark the intended pane in tmux, then retry.".into())
                })?;
            return Ok(Some(ResolvedPane {
                id: target.pane_id.clone(),
                frozen: Some(target),
            }));
        }
        Invocation::Bind {
            pane: Some(pane),
            name,
            ..
        } => {
            if !is_pane_target(pane) && is_pane_target(name) {
                return Err(Failure::new(
                    "LEGACY_ADD_ORDER",
                    "The v4 add argument order is no longer supported.",
                    1,
                )
                .suggestion(format!("Use: tmt add {name} {pane}")));
            }
            Some(pane.as_str())
        }
        Invocation::List {
            target: Some(target),
            ..
        } if is_pane_target(target) => Some(target.as_str()),
        Invocation::List {
            target: None,
            scope: ListScope { here: true, .. },
            ..
        } => {
            // `--here` is caller-scoped like `whoami`: it needs a tmux pane.
            crate::caller_context::require_independent_host()?;
            return tmux
                .caller_pane(environment)
                .map_err(endpoint_failure)?
                .map(|id| Some(ResolvedPane { id, frozen: None }))
                .ok_or_else(|| {
                    Failure::new(
                        "PANE_NOT_FOUND",
                        "`tmt ls --here` needs a tmux pane, and this is not one.",
                        3,
                    )
                    .suggestion("Run it inside tmux, or drop --here to list every agent.".into())
                });
        }
        Invocation::Bind { pane: None, .. } | Invocation::Whoami | Invocation::Unbind => {
            crate::caller_context::require_independent_host()?;
            return tmux
                .caller_pane(environment)
                .map_err(endpoint_failure)?
                .map(|id| Some(ResolvedPane { id, frozen: None }))
                .ok_or_else(|| {
                    Failure::new(
                        "PANE_NOT_FOUND",
                        "Not running inside a resolvable tmux pane.",
                        3,
                    )
                });
        }
        _ => None,
    };
    target
        .map(|target| {
            tmux.resolve_target(target, OperationOptions::default())
                .map_err(endpoint_failure)?
                .ok_or_else(|| {
                    Failure::new(
                        "PANE_NOT_FOUND",
                        format!("Pane target '{target}' was not found."),
                        3,
                    )
                })
                .map(|id| ResolvedPane { id, frozen: None })
        })
        .transpose()
}

fn run(request: Invocation) -> Result<Report, Failure> {
    let tmux = Tmux::default();
    let environment = CallerEnvironment::current();
    let pane = preflight(&request, &tmux, &environment)?;
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let badge = if matches!(
        request,
        Invocation::Bind { .. } | Invocation::BindMarked { .. }
    ) {
        ConfigFiles {
            paths: paths.clone(),
        }
        .load()
        .map_err(Failure::from)?
        .settings
        .pane_badge
    } else {
        PaneBadge::Off
    };
    let mut storage = Storage::open(paths.database).map_err(|error| {
        Failure::new("IDENTITY_ERROR", "Could not open identity storage.", 1).caused_by(error)
    })?;
    let mut endpoint = BindingSession::new(&tmux);
    let pending = operation(&mut storage, &mut endpoint, request, pane, environment.selected_socket())
        .and_then(|report| {
            // Presentation follows successful durable effects, never decides
            // them. The adapter preserves user themes and changed endpoints.
            let update = match &report {
                Report::Bound(result) => result.presence.binding.as_ref().map(|binding|
                    (binding, (badge == PaneBadge::On).then_some(result.presence.identity.name.as_str()))),
                Report::Unbound { result, .. } => result.binding.as_ref().map(|binding| (binding, None)),
                Report::Removed(entry) => entry.binding.as_ref().map(|binding| (binding, None)),
                _ => None,
            };
            if let Some((binding, name)) = update {
                tmux.update_binding_badge(binding, name).map_err(|error|
                    Failure::new("CLEANUP_ERROR", "Could not clean up cosmetic operation resources. Effects may already have occurred.", 1).caused_by(error))?;
            }
            Ok(report)
        });
    after_cleanup(pending, || storage.close())
}

fn operation(
    storage: &mut Storage,
    endpoint: &mut BindingSession<'_, tmt_adapters::process::UnixCommandRunner>,
    request: Invocation,
    pane: Option<ResolvedPane>,
    current_socket: Option<&str>,
) -> Result<Report, Failure> {
    match request {
        Invocation::Bind { name, save, .. } => binding::bind_identity_with_creation(
            storage,
            endpoint,
            &pane.as_ref().expect("binding preflight").id,
            &name,
            save,
        )
        .map(Report::Bound)
        .map_err(binding_failure),
        Invocation::BindMarked { name, save } => {
            let pane = pane.as_ref().expect("marked binding preflight");
            binding::bind_identity_with_creation_at(
                storage,
                endpoint,
                &pane.id,
                pane.frozen.as_ref(),
                &name,
                save,
            )
            .map(Report::Bound)
            .map_err(binding_failure)
        }
        Invocation::Whoami => {
            let pane = pane.expect("caller preflight").id;
            let observed =
                binding::pane_presence(storage, endpoint, &pane).map_err(binding_failure)?;
            let runtime = observed
                .binding
                .as_ref()
                .and_then(|binding| endpoint.observed_runtime(binding).ok())
                .unwrap_or(RuntimeState::Unknown);
            Ok(Report::Caller {
                pane,
                identity: observed.identity,
                runtime,
            })
        }
        Invocation::Unbind => {
            let pane = pane.expect("caller preflight").id;
            let result = binding::unbind_identity(storage, endpoint, &pane)
                .map_err(binding_failure)?
                .ok_or_else(|| {
                    Failure::new("UNBOUND_PANE", "Pane has no active global name.", 1)
                })?;
            Ok(Report::Unbound { pane, result })
        }
        Invocation::Remove { name, force } => {
            binding::remove_identity(storage, endpoint, &name, force)
                .map(Report::Removed)
                .map_err(binding_failure)
        }
        Invocation::List {
            target: None,
            room,
            scope,
        } => {
            // Resolve scope first: a typo must not trigger global reconciliation.
            let room = room
                .map(|selector| crate::room_command::resolve(storage, &selector))
                .transpose()?;
            let mut rows = binding::list_presence(storage, endpoint, current_socket)
                .map_err(binding_failure)?;
            if let Some(room) = room {
                rows.retain(|row| room.member_ids.contains(&row.identity.id));
            }
            if let Some(lifetime) = scope.lifetime {
                rows.retain(|row| row.identity.lifetime == lifetime);
            }
            if let Some(caller) = &pane {
                // The caller's session is the part of its pane target before ':'.
                let observed = binding::pane_presence(storage, endpoint, &caller.id)
                    .map_err(binding_failure)?;
                let session = |pane: &PaneObservation| {
                    pane.target
                        .as_deref()
                        .and_then(|target| target.split_once(':'))
                        .map(|(session, _)| session.to_owned())
                };
                let here = session(&observed.pane);
                rows.retain(|row| here.is_some() && row.pane.as_ref().and_then(session) == here);
            }
            let registry = tmt_adapters::runtime::RuntimeRegistry::first_party();
            let rows = rows
                .into_iter()
                .map(|row| {
                    let preferences =
                        storage
                            .session_preferences(&row.identity.id)
                            .map_err(|error| {
                                Failure::new(
                                    "IDENTITY_ERROR",
                                    "Could not read remembered sessions.",
                                    1,
                                )
                                .caused_by(error)
                            })?;
                    let resume = crate::output::resume_document(&preferences, &registry);
                    Ok(ListedRow {
                        presence: row,
                        remembered: preferences.remembered,
                        resume,
                    })
                })
                .collect::<Result<Vec<_>, Failure>>()?;
            Ok(Report::Listed { rows, scope })
        }
        Invocation::List {
            target: Some(target),
            ..
        } => {
            if let Some(pane) = pane {
                let observed =
                    binding::pane_presence(storage, endpoint, &pane.id).map_err(binding_failure)?;
                let remembered = match &observed.identity {
                    Some(identity) => remembered_session(storage, identity)?,
                    None => None,
                };
                Ok(Report::Pane {
                    target,
                    pane: observed.pane,
                    identity: observed.identity,
                    remembered,
                })
            } else {
                let row =
                    binding::name_presence(storage, endpoint, &target).map_err(binding_failure)?;
                let remembered = remembered_session(storage, &row.identity)?;
                Ok(Report::Named {
                    target,
                    row,
                    remembered,
                })
            }
        }
        _ => Err(Failure::new(
            "INTERNAL_ERROR",
            "Unexpected binding command.",
            1,
        )),
    }
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(stdout, "{}", presentation::document(&report))?;
    } else {
        let terminal = stdout.terminal();
        presentation::text(&mut stdout, terminal, &report)?;
    }
    drop(stdout);
    use crate::skill_reminder::Outcome;
    let outcome = match &report {
        Report::Bound(result) if result.created => {
            if result.presence.identity.lifetime == tmt_core::identity::Lifetime::Temporary {
                Outcome::TemporaryIdentityCreated
            } else {
                Outcome::SavedIdentityCreated
            }
        }
        _ => Outcome::None,
    };
    crate::skill_reminder::present(outcome, mode, true);
    Ok(0)
}
