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
    host::{BindingSession, CallerEnvironment, Host, OperationOptions, PaneCosmetics, PaneRefresh},
    storage::Storage,
};
use tmt_core::{
    binding::session::{RememberedSession, RuntimeState},
    binding::{
        self, BindingEntry, BindingTargetEvidence, BoundIdentity, IdentityPresence,
        RenamedIdentity, UnboundIdentity,
    },
    endpoint::PaneObservation,
    host::ServerSelector,
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
    activity: serde_json::Value,
}

struct ResolvedPane {
    id: String,
    frozen: Option<BindingTargetEvidence>,
    /// The explicit target, whose host observes the pane.
    target: Option<String>,
    /// How the user names the pane.
    label: String,
}

enum Report {
    Bound(BoundIdentity),
    Caller {
        pane: String,
        /// How the user names the pane (`pane` on tmux, `wN:pM` on Herdr).
        label: String,
        identity: Option<Identity>,
        runtime: RuntimeState,
    },
    Unbound {
        pane: String,
        label: String,
        result: UnboundIdentity,
    },
    Removed(BindingEntry),
    /// `pane` is set after commit when a live pane showed the identity.
    Renamed {
        result: RenamedIdentity,
        pane: Option<(String, PaneRefresh)>,
    },
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

/// The caller's own pane with the label the user knows it by, or `missing`.
fn labeled_caller_pane(
    host: &Host,
    environment: &CallerEnvironment,
    missing: impl FnOnce() -> Failure,
) -> Result<Option<ResolvedPane>, Failure> {
    let id = host
        .caller_pane(environment)
        .map_err(endpoint_failure)?
        .ok_or_else(missing)?;
    let label = host.caller_label(&id).map_err(endpoint_failure)?;
    Ok(Some(ResolvedPane {
        label,
        id,
        frozen: None,
        target: None,
    }))
}

/// `listed_pane`: whether `ls <text>` addresses a pane, decided once by
/// [`listed_target`] before any host is resolved.
fn preflight(
    request: &Invocation,
    host: &Host,
    environment: &CallerEnvironment,
    listed_pane: bool,
) -> Result<Option<ResolvedPane>, Failure> {
    let target = match request {
        Invocation::BindMarked { .. } => {
            let target = host
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
                label: target.pane_id.clone(),
                frozen: Some(target),
                target: None,
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
        } if listed_pane => Some(target.as_str()),
        Invocation::List {
            target: None,
            scope: ListScope { here: true, .. },
            ..
        } => {
            // `--here` is caller-scoped like `whoami`: it needs a tmux pane.
            crate::caller_context::require_independent_host()?;
            return labeled_caller_pane(host, environment, || {
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
            return labeled_caller_pane(host, environment, || {
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
            Host::for_target(target)
                .resolve_target(target, OperationOptions::default())
                .map_err(endpoint_failure)?
                .ok_or_else(|| {
                    Failure::new(
                        "PANE_NOT_FOUND",
                        format!("Pane target '{target}' was not found."),
                        3,
                    )
                })
                .map(|id| ResolvedPane {
                    label: tmt_core::host::HostKind::label(&id, Some(target)).to_owned(),
                    id,
                    frozen: None,
                    target: Some(target.to_owned()),
                })
        })
        .transpose()
}

fn open_storage(paths: &ConfigPaths) -> Result<Storage, Failure> {
    Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No identity was changed.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })
}

/// Whether `ls <text>` addresses a pane rather than an identity, decided
/// once before any host is resolved. Only text an existing identity could
/// hold (a Herdr target) needs storage, which is then kept for the command;
/// other requests open storage only after their preflight, as before.
struct ListedTarget {
    pane: bool,
    opened: Option<(ConfigPaths, Storage)>,
}

fn listed_target(request: &Invocation) -> Result<ListedTarget, Failure> {
    let target = match request {
        Invocation::List {
            target: Some(target),
            ..
        } if is_pane_target(target) => target,
        _ => {
            return Ok(ListedTarget {
                pane: false,
                opened: None,
            });
        }
    };
    if tmt_core::names::validate_existing_name(target).is_err() {
        return Ok(ListedTarget {
            pane: true,
            opened: None,
        });
    }
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let storage = open_storage(&paths)?;
    let pane = tmt_core::identity::addresses_pane(&storage, target).map_err(|error| {
        Failure::new("IDENTITY_ERROR", "Could not read identity storage.", 1).caused_by(error)
    })?;
    Ok(ListedTarget {
        pane,
        opened: Some((paths, storage)),
    })
}

fn run(request: Invocation) -> Result<Report, Failure> {
    let environment = CallerEnvironment::current();
    let caller = Host::for_caller(&environment);
    let listed = listed_target(&request)?;
    let pane = preflight(&request, &caller, &environment, listed.pane)?;
    // An explicit target's own host observes it; stored bindings are always
    // probed on theirs.
    let host = match pane.as_ref().and_then(|pane| pane.target.as_deref()) {
        Some(target) => Host::for_target(target),
        None => caller,
    };
    let (paths, storage) = match listed.opened {
        Some((paths, storage)) => (paths, Some(storage)),
        None => (ConfigPaths::discover().map_err(Failure::from)?, None),
    };
    let badge = if matches!(
        request,
        Invocation::Bind { .. } | Invocation::BindMarked { .. } | Invocation::Rename { .. }
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
    let mut storage = match storage {
        Some(storage) => storage,
        None => open_storage(&paths)?,
    };
    // A pane observation needs its server resolved before any binding
    // transaction; listing and name operations only probe stored bindings.
    let resolved = if pane.is_some()
        || matches!(
            request,
            Invocation::Bind { .. } | Invocation::BindMarked { .. }
        ) {
        host.resolve_servers(&mut storage).map_err(endpoint_failure)
    } else {
        Ok(())
    };
    let mut endpoint = host.session();
    let operated = resolved.and_then(|()| {
        operation(
            &mut storage,
            &mut endpoint,
            request,
            pane,
            host.selected_server(&environment),
        )
    });
    let pending = operated.and_then(|mut report| {
        // Presentation follows successful durable effects, never decides
        // them. The adapter preserves user themes and changed endpoints.
        let badge = badge == PaneBadge::On;
        let update = match &report {
            Report::Bound(result) => result.presence.binding.as_ref().map(|binding| {
                (
                    binding,
                    PaneCosmetics::Bound {
                        identity: &result.presence.identity,
                        badge,
                        border_hint: true,
                    },
                )
            }),
            Report::Unbound { result, .. } => result
                .binding
                .as_ref()
                .map(|binding| (binding, PaneCosmetics::Ended)),
            Report::Removed(entry) => entry
                .binding
                .as_ref()
                .map(|binding| (binding, PaneCosmetics::Ended)),
            _ => None,
        };
        if let Some((binding, cosmetics)) = update {
            Host::for_server(&binding.server)
                .update_binding_cosmetics(binding, cosmetics)
                .map_err(cosmetic_cleanup)?;
        }
        // The rename is committed; its pane only shows it, and a stale
        // marker name never detaches, so a failure here is a warning.
        if let Report::Renamed { result, pane } = &mut report
            && result.changed()
            && let Some(binding) = &result.binding
        {
            let cosmetics = PaneCosmetics::Bound {
                identity: &result.identity,
                badge,
                border_hint: false,
            };
            let refresh = Host::for_server(&binding.server)
                .update_binding_cosmetics(binding, cosmetics)
                .map_err(cosmetic_cleanup)?;
            if refresh != PaneRefresh::Absent {
                *pane = Some((binding.pane_id.clone(), refresh));
            }
        }
        Ok(report)
    });
    after_cleanup(pending, || storage.close())
}

fn cosmetic_cleanup(error: tmt_adapters::host::HostError) -> Failure {
    Failure::new(
        "CLEANUP_ERROR",
        "Could not clean up cosmetic operation resources. Effects may already have occurred.",
        1,
    )
    .caused_by(error)
}

fn operation(
    storage: &mut Storage,
    endpoint: &mut BindingSession<'_, tmt_adapters::process::UnixCommandRunner>,
    request: Invocation,
    pane: Option<ResolvedPane>,
    current_server: Option<ServerSelector<'_>>,
) -> Result<Report, Failure> {
    match request {
        Invocation::Bind {
            name,
            save,
            pane: selector,
        } => {
            let pane = &pane.as_ref().expect("binding preflight").id;
            if selector.is_none()
                && let Some(bound) =
                    binding::name_auto_identity(storage, endpoint, pane, &name, save)
                        .map_err(binding_failure)?
            {
                return Ok(Report::Bound(bound));
            }
            binding::bind_identity_with_creation(storage, endpoint, pane, &name, save)
                .map(Report::Bound)
                .map_err(binding_failure)
        }
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
            let ResolvedPane {
                id: pane, label, ..
            } = pane.expect("caller preflight");
            let observed =
                binding::pane_presence(storage, endpoint, &pane).map_err(binding_failure)?;
            let runtime = observed
                .binding
                .as_ref()
                .and_then(|binding| endpoint.observed_runtime(binding).ok())
                .unwrap_or(RuntimeState::Unknown);
            Ok(Report::Caller {
                pane,
                label,
                identity: observed.identity,
                runtime,
            })
        }
        Invocation::Unbind => {
            let ResolvedPane {
                id: pane, label, ..
            } = pane.expect("caller preflight");
            let result = binding::unbind_identity(storage, endpoint, &pane)
                .map_err(binding_failure)?
                .ok_or_else(|| {
                    Failure::new("UNBOUND_PANE", "Pane has no active global name.", 1)
                })?;
            Ok(Report::Unbound {
                pane,
                label,
                result,
            })
        }
        Invocation::Remove { name, force } => {
            binding::remove_identity(storage, endpoint, &name, force)
                .map(Report::Removed)
                .map_err(binding_failure)
        }
        Invocation::Rename { old, new } => binding::rename_identity(storage, &old, &new)
            .map(|result| Report::Renamed { result, pane: None })
            .map_err(binding_failure),
        Invocation::List {
            target: None,
            room,
            scope,
        } => {
            // Resolve scope first: a typo must not trigger global reconciliation.
            let room = room
                .map(|selector| crate::room_command::resolve(storage, &selector))
                .transpose()?;
            let mut rows = binding::list_presence(storage, endpoint, current_server)
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
                    let runtime = row
                        .binding
                        .as_ref()
                        .filter(|_| row.presence == tmt_core::binding::Presence::Active)
                        .and_then(|binding| {
                            // Activity ended requires a process fact, not a stored SessionEnd hook.
                            let mut observed = binding.clone();
                            if observed.session.state == RuntimeState::Ended {
                                observed.session.state = RuntimeState::Unknown;
                            }
                            endpoint.observed_runtime(&observed).ok()
                        })
                        .unwrap_or(RuntimeState::Unknown);
                    let activity = crate::output::activity_document(
                        row.binding.as_ref(),
                        preferences.remembered.as_ref(),
                        runtime,
                        &registry,
                    );
                    Ok(ListedRow {
                        presence: row,
                        remembered: preferences.remembered,
                        resume,
                        activity,
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
    if !mode.json {
        presentation::warnings(&mut tmt_cli_style::stream::stderr(), &report)?;
    }
    use crate::skill_reminder::Outcome;
    let outcome = match &report {
        Report::Bound(result) if result.created => {
            if result.presence.identity.lifetime == tmt_core::identity::Lifetime::Temporary {
                Outcome::TemporaryIdentityCreated {
                    name: result.presence.identity.name.clone(),
                }
            } else {
                Outcome::SavedIdentityCreated {
                    name: result.presence.identity.name.clone(),
                }
            }
        }
        _ => Outcome::None,
    };
    crate::skill_reminder::present(outcome, mode, true);
    let changed_binding = match &report {
        Report::Bound(result) => result.presence.binding.as_ref(),
        Report::Unbound { result, .. } => result.binding.as_ref(),
        Report::Removed(entry) => entry.binding.as_ref(),
        Report::Renamed { result, .. } if result.changed() => result.binding.as_ref(),
        _ => None,
    };
    if let Some(binding) = changed_binding
        && let Ok(paths) = ConfigPaths::discover()
    {
        let host = Host::for_server(&binding.server);
        tmt_adapters::workspace::refresh_server(&paths, &host, &binding.server);
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("Use: tmt add {name} {pane}", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "`tmt ls --here` needs a tmux pane, and this is not one.",
        &["`"],
        &[],
    ),
];

#[cfg(test)]
pub(crate) use presentation::PRINTED_HINTS as PRESENTATION_HINTS;
